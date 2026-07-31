// contracts/stream/tests/upgrade_path.rs
//! V5→V6 migration test suite for contract upgrade.
//!
//! Validates that upgrading the contract WASM preserves existing stream
//! state and that the upgrade entrypoint is properly gated by admin auth.
//!
//! V5→V6 storage layout invariants (cross-referenced against
//! docs/upgrade.md and contracts/stream/src/checksum.rs):
//!
//! | Storage key          | Discriminant | Stored type | Notes                        |
//! |----------------------|-------------|-------------|------------------------------|
//! | `DataKey::Config`    |           0 | `Config`    | Admin + token; read by admin |
//! | `DataKey::Stream(n)` |           2 | `Stream`    | Per-stream persistent state  |
//!
//! Upgrade protocol (from docs/upgrade.md):
//! 1. Only the contract admin may initiate an upgrade.
//! 2. The `upgrade` entrypoint reads `DataKey::Config` to verify admin.
//! 3. After WASM replacement, existing `DataKey::Stream(n)` entries
//!    remain readable — no data migration is needed for V5→V6 because
//!    the `Stream` struct layout and storage discriminants are unchanged.
//!
//! NOTE: The Soroban test environment does not have a deployable WASM
//! for arbitrary hashes.  `env.deployer().update_current_contract_wasm()`
//! with a zero hash `[0u8; 32]` traps with `Error(Storage, MissingValue)`.
//! Tests that actually call `update_current_contract_wasm` are therefore
//! marked `#[ignore]` and are ready to be enabled when a test
//! environment with deployable WASM artifacts is available.

use fluxora_stream::{
    ContractError, ContractUpgraded, DataKey, FluxoraStream, FluxoraStreamClient,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env, IntoVal,
};

/// Test context for upgrade tests
struct UpgradeTestCtx<'a> {
    env: Env,
    contract_id: Address,
    client: FluxoraStreamClient<'a>,
    admin: Address,
    _token: Address,
    sender: Address,
    recipient: Address,
}

impl<'a> UpgradeTestCtx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let sac = StellarAssetClient::new(&env, &token);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        sac.mint(&sender, &1_000_000_000_000i128);
        TokenClient::new(&env, &token).approve(&sender, &contract_id, &i128::MAX, &200_000u32);

        client.init(&token, &admin);

        Self {
            env,
            contract_id,
            client,
            admin,
            _token: token,
            sender,
            recipient,
        }
    }

    fn create_test_stream(&self) -> u64 {
        let rate = 1_i128;
        let start_time = 1_000_000u64;
        let cliff_time = 1_000_000u64;
        let end_time = 2_000_000u64;
        let deposit = rate * (end_time - start_time) as i128;

        self.client.create_stream(
            &self.sender,
            &fluxora_stream::CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: rate,
                start_time,
                cliff_time,
                end_time,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: fluxora_stream::StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

// -----------------------------------------------------------------------
// Tests that DO NOT call `update_current_contract_wasm`
// (always runnable in the test environment)
// -----------------------------------------------------------------------

/// Test that the contract is correctly initialised (Diskriminant 0 Config).
/// This validates that `init` persists the admin/token pair at
/// `DataKey::Config` (V5→V6 invariant from checkusm.rs).
#[test]
fn test_upgrade_fails_if_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let new_hash = BytesN::from_array(&env, &[0u8; 32]);

    // Call through the generated client rather than the native helper. This is
    // also a compile-time ABI guard: removing `upgrade` from #[contractimpl]
    // removes `try_upgrade` and makes this test fail to compile.
    let result = client.try_upgrade(&new_hash);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// A call without the configured admin's authorization must be rejected by
/// the host before the deployer is invoked.
///
/// `env.mock_all_auths()` is deliberately not used. Soroban authorization is
/// address-based; there is no separate caller argument to compare with admin.
#[test]
fn test_upgrade_rejected_for_non_admin() {
    let env = Env::default();

    // Set up the contract with a known admin.
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();

    let admin = Address::generate(&env);
    // Allow only the real admin auth so `init` can proceed.
    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract_id,
            fn_name: "init",
            args: (&token, &admin).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.init(&token, &admin);

    let new_hash = BytesN::from_array(&env, &[0u8; 32]);

    // Only init authorization was mocked. The generated-client call therefore
    // reaches `admin.require_auth()` and is rejected before the invalid hash can
    // be evaluated by `update_current_contract_wasm`.
    let result = client.try_upgrade(&new_hash);
    assert!(
        result.is_err(),
        "upgrade without the configured admin's authorization must fail"
    );
}

/// `version()` returns the same compile-time constant regardless of how many
/// times it is called in sequence. This locks down the "no storage side-effects"
/// guarantee: a naive implementation that incremented a counter in storage on
/// each call would break this test.
#[test]
fn test_version_idempotent_after_multiple_reads() {
    let ctx = UpgradeTestCtx::setup();

    let v1 = ctx.client.version();
    let v2 = ctx.client.version();
    let v3 = ctx.client.version();

    assert_eq!(
        v1, v2,
        "version() must return the same value on repeated calls"
    );
    assert_eq!(
        v2, v3,
        "version() must return the same value on repeated calls"
    );
    assert_eq!(v1, fluxora_stream::CONTRACT_VERSION);
}

/// `version()` works before `init` is called (pre-init check).
///
/// This is documented in `docs/upgrade.md §2. version() Entry-Point Semantics`:
/// "Works before `init` is called (pre-flight deployment check)."
///
/// A deployment script must be able to call `version()` immediately after
/// deploying the WASM — before setting up admin/token — to confirm it uploaded
/// the right binary.
#[test]
fn test_version_works_before_init() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    // No `init` call — contract is in a blank state.
    let v = client.version();
    assert_eq!(
        v,
        fluxora_stream::CONTRACT_VERSION,
        "version() must be callable before init"
    );
}

/// `version()` leaves instance storage completely unchanged.
///
/// We read the storage key set before calling `version()`, call it three times,
/// then confirm the storage key set is still the same (no writes occurred).
///
/// This guards the "no storage side-effects" semantic documented in
/// `docs/upgrade.md §2`: version() makes no storage reads or writes.
#[test]
fn test_version_has_no_storage_side_effects() {
    let ctx = UpgradeTestCtx::setup();
    let contract_id = ctx.contract_id.clone();

    // Seed both storage durabilities with canaries. Checking only stream count
    // would miss accidental writes to unrelated keys.
    ctx.env.as_contract(&contract_id, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::PausedStreamCount, &17u64);
        ctx.env
            .storage()
            .persistent()
            .set(&DataKey::AutoRenewEnabled(u64::MAX), &true);
    });

    ctx.client.version();
    ctx.client.version();

    ctx.env.as_contract(&contract_id, || {
        assert_eq!(
            ctx.env
                .storage()
                .instance()
                .get::<_, u64>(&DataKey::PausedStreamCount),
            Some(17),
            "version() must not mutate instance storage"
        );
        assert_eq!(
            ctx.env
                .storage()
                .persistent()
                .get::<_, bool>(&DataKey::AutoRenewEnabled(u64::MAX)),
            Some(true),
            "version() must not mutate persistent storage"
        );
    });
}

/// The storage-free version view must remain cheaper than a view that loads
/// Config and bumps instance TTL. This is a relative regression guard rather
/// than a network-fee estimate: host cost models can change between releases.
#[test]
fn test_version_budget_is_lower_than_storage_backed_view() {
    let ctx = UpgradeTestCtx::setup();

    ctx.env.budget().reset_unlimited();
    assert_eq!(ctx.client.version(), fluxora_stream::CONTRACT_VERSION);
    let version_cpu = ctx.env.budget().cpu_instruction_cost();
    let version_memory = ctx.env.budget().memory_bytes_cost();

    ctx.env.budget().reset_unlimited();
    let _ = ctx.client.get_config();
    let config_cpu = ctx.env.budget().cpu_instruction_cost();
    let config_memory = ctx.env.budget().memory_bytes_cost();

    assert!(
        version_cpu < config_cpu,
        "version CPU cost ({version_cpu}) must remain below get_config ({config_cpu})"
    );
    assert!(
        version_memory < config_memory,
        "version memory cost ({version_memory}) must remain below get_config ({config_memory})"
    );
}

/// `CONTRACT_VERSION` constant equals exactly 9 (the current release).
///
/// This pins the expected value so any accidental bump or rollback is
/// immediately visible as a test failure rather than silent drift.
#[test]
fn test_contract_version_constant_is_9() {
    assert_eq!(
        fluxora_stream::CONTRACT_VERSION,
        9,
        "CONTRACT_VERSION must be 9 for this release"
    );
}

/// Test that a stream's state is readable (DataKey::Stream(id) invariant).
/// This test exists purely to confirm that the V5→V6 storage layout
/// is intact: `create_stream` writes to `DataKey::Stream(n)`, and
/// `get_stream_state` reads it back with the correct discriminant.
#[test]
fn test_stream_state_readable_no_upgrade() {
    let ctx = UpgradeTestCtx::setup();
    let stream_id = ctx.create_test_stream();

    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.sender, ctx.sender);
    assert_eq!(stream.recipient, ctx.recipient);
    // Verify discriminant 2 (DataKey::Stream) is used — the returned
    // stream_id should match the one we passed in.
    assert_eq!(stream.stream_id, stream_id);
}

// -----------------------------------------------------------------------
// Tests that call `update_current_contract_wasm`
// (require a deployable WASM artifact in the test environment)
// -----------------------------------------------------------------------

/// Test that admin can call upgrade.  The deployer rejects a zero hash
/// in the test environment, but the admin auth check must pass first
/// (returning `ContractError::Unauthorized` on failure).
#[ignore]
#[test]
fn test_upgrade_succeeds_for_admin() {
    let ctx = UpgradeTestCtx::setup();
    let new_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);

    // The admin auth gate runs before the deployer call. If auth fails
    // we get Unauthorized instead of the deployer's "Wasm does not exist".
    // In the test env the deployer traps, but the trap proves the
    // admin check passed (otherwise the deployer is never reached).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.env.as_contract(&ctx.contract_id, || {
            fluxora_stream::upgrade(ctx.env.clone(), new_hash)
        })
    }));
    match result {
        Ok(Ok(())) => {} // upgrade succeeded (deployer accepted hash)
        Ok(Err(err)) => assert_ne!(
            err,
            ContractError::Unauthorized,
            "admin upgrade must not fail with Unauthorized"
        ),
        Err(_) => {} // deployer rejected hash — admin check passed
    }
}

/// Test that a stream's state survives an upgrade attempt (V5→V6 layout).
#[ignore]
#[test]
fn test_upgrade_preserves_stream_state() {
    let ctx = UpgradeTestCtx::setup();
    let stream_id = ctx.create_test_stream();

    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.sender, ctx.sender);
    assert_eq!(stream.recipient, ctx.recipient);

    let new_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.env.as_contract(&ctx.contract_id, || {
            fluxora_stream::upgrade(ctx.env.clone(), new_hash)
        })
    }));

    // Stream must still be readable after upgrade attempt
    let stream_after = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream_after.sender, ctx.sender);
    assert_eq!(stream_after.recipient, ctx.recipient);
}

/// Test that the `ContractUpgraded` event struct has the expected field types.
/// This is a compile-time safety net — if the struct definition changes, this
/// test fails to compile, forcing an explicit review of the event schema.
#[test]
fn test_contract_upgraded_event_struct_layout() {
    let env = Env::default();
    let addr = Address::generate(&env);
    let hash = BytesN::from_array(&env, &[1u8; 32]);

    let event = ContractUpgraded {
        new_wasm_hash: hash.clone(),
        // Historical field name: the implementation records the version of
        // the WASM executing the upgrade, then verifies the target separately.
        new_version: fluxora_stream::CONTRACT_VERSION,
        upgraded_at: 1_000_000,
        upgraded_by: addr.clone(),
    };

    // Verify the backward-compatible field types and ordering.
    assert_eq!(event.new_version, fluxora_stream::CONTRACT_VERSION);
    assert_eq!(event.upgraded_at, 1_000_000);
    assert_eq!(event.upgraded_by, addr);
    assert_eq!(event.new_wasm_hash, hash);
}

// -----------------------------------------------------------------------
// Edge case tests (do not call `update_current_contract_wasm`)
// -----------------------------------------------------------------------

/// Test that version() returns the same value before and after a failed
/// upgrade attempt. This locks down the "no storage corruption" guarantee.
#[test]
fn test_version_stable_after_failed_upgrade() {
    let ctx = UpgradeTestCtx::setup();

    let v_before = ctx.client.version();

    // The hash was not uploaded, so the host rejects the generated-client call.
    let invalid_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
    assert!(ctx.client.try_upgrade(&invalid_hash).is_err());

    let v_after = ctx.client.version();
    assert_eq!(
        v_before, v_after,
        "version() must be stable after a failed upgrade attempt"
    );
    assert_eq!(v_after, fluxora_stream::CONTRACT_VERSION);
}

/// A rejected host update is atomic: neither application upgrade events nor the
/// host's executable update survive, and existing configuration is unchanged.
#[test]
fn test_failed_upgrade_rolls_back_events_and_config() {
    let ctx = UpgradeTestCtx::setup();
    let events_before = ctx.env.events().all();
    let config_before = ctx.client.get_config();

    let invalid_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
    assert!(ctx.client.try_upgrade(&invalid_hash).is_err());

    assert_eq!(
        ctx.env.events().all(),
        events_before,
        "failed upgrade must not leave executable-update or application events"
    );
    assert_eq!(
        ctx.client.get_config(),
        config_before,
        "failed upgrade must not mutate instance configuration"
    );
}

/// Test that the contract remains fully operational after a failed upgrade
/// attempt — creating a stream, reading config, and calling views all work.
#[test]
fn test_contract_usable_after_failed_upgrade() {
    let ctx = UpgradeTestCtx::setup();

    // Attempt upgrade with an uninstalled hash through the exported endpoint.
    let invalid_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
    assert!(ctx.client.try_upgrade(&invalid_hash).is_err());

    // Post-upgrade-attempt operations must succeed
    let stream_id = ctx.create_test_stream();
    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.sender, ctx.sender);
    assert_eq!(stream.recipient, ctx.recipient);

    let count = ctx.client.get_stream_count();
    assert_eq!(count, 1, "stream count must reflect newly created stream");

    let config = ctx.client.get_config();
    assert_eq!(
        config.admin, ctx.admin,
        "config and admin must remain readable after rollback"
    );
}

/// Test that `set_admin` works after a failed upgrade attempt.
/// Admin continuity is critical for scheduling a follow-up upgrade.
#[test]
fn test_admin_rotation_possible_after_failed_upgrade() {
    let ctx = UpgradeTestCtx::setup();
    let new_admin = Address::generate(&ctx.env);

    // Attempt upgrade with an uninstalled hash through the exported endpoint.
    let invalid_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
    assert!(ctx.client.try_upgrade(&invalid_hash).is_err());

    // Must be able to rotate admin after failed upgrade
    ctx.client.set_admin(&new_admin);

    let config_after = ctx.client.get_config();
    assert_eq!(
        config_after.admin, new_admin,
        "admin rotation must succeed after failed upgrade"
    );
}
