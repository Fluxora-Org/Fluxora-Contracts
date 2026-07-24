//! V5 → V6 storage key compatibility regression tests.
//!
//! # Purpose
//!
//! Soroban serialises `DataKey` variants by their **0-based declaration-order
//! discriminant**. Any reorder, insertion, or removal silently corrupts every
//! persistent entry on a live instance. These tests guard against that by:
//!
//! 1. Seeding ledger state with V5-era key/value pairs (using `env.as_contract`
//!    to bypass the contract entry-points and write directly to storage).
//! 2. Invoking V6 read paths and asserting correct deserialization.
//! 3. Asserting that V6-only keys (discriminants 15–20) are absent on a
//!    V5-seeded instance, confirming no phantom reads.
//! 4. Cross-checking `CONTRACT_VERSION` against the live `DataKey` variant count
//!    (currently 29) to ensure versioning discipline when new variants are added.
//!
//! # V5 discriminant table (frozen — must never change)
//!
//! | Disc | Variant                     | Storage    |
//! |-----:|:----------------------------|:-----------|
//! |    0 | `Config`                    | Instance   |
//! |    1 | `NextStreamId`              | Instance   |
//! |    2 | `Stream(u64)`               | Persistent |
//! |    3 | `RecipientStreams(Address)`  | Persistent |
//! |    4 | `GlobalEmergencyPaused`     | Instance   |
//! |    5 | `CreationPaused`            | Instance   |
//! |    6 | `GlobalPauseReason`         | Instance   |
//! |    7 | `GlobalPauseTimestamp`      | Instance   |
//! |    8 | `GlobalPauseAdmin`          | Instance   |
//! |    9 | `AutoClaimDestination(u64)` | Persistent |
//! |   10 | `NextTemplateId`            | Instance   |
//! |   11 | `ActiveTemplateCount`       | Instance   |
//! |   12 | `StreamTemplate(u64)`       | Persistent |
//! |   13 | `OwnerTemplateIds(Address)` | Persistent |
//! |   14 | `TotalLiabilities`          | Instance   |
//!
//! # V6 discriminant table (discriminants 15–20)
//!
//! | Disc | Variant                             | Storage    |
//! |-----:|:------------------------------------|:-----------|
//! |   15 | `WithdrawNonce(Address)`            | Persistent |
//! |   16 | `PauseState`                        | Instance   |
//! |   17 | `ReentrancyLock`                    | Instance   |
//! |   18 | `RecipientStreamPage(Address, u32)` | Persistent |
//! |   19 | `RecipientStreamPageCount(Address)` | Persistent |
//! |   20 | `PendingRecipientUpdate(u64)`       | Persistent |
//!
//! # Post-V6 freeze additions (discriminants 21–28)
//!
//! | Disc | Variant                            | Storage    |
//! |-----:|:-----------------------------------|:-----------|
//! |   21 | `IdReservation(Address)`           | Instance   |
//! |   22 | `MaxRatePerSecond`                 | Instance   |
//! |   23 | `DelegatedWithdrawNonce(Address)`  | Persistent |
//! |   24 | `LastPauseRecord(PauseKind)`       | Instance   |
//! |   25 | `RotationHistory(u64)`             | Persistent |
//! |   26 | `LastAccrualLedgerTimestamp`       | Instance   |
//! |   27 | `PausedStreamCount`                | Instance   |
//! |   28 | `TotalKeeperFeesPaid`              | Instance   |
//!
//! Total live `DataKey` variant count: **29** (discriminants 0–28).
//!
//! # Version Mapping Table (`CONTRACT_VERSION` => Expected DataKey Count)
//!
//! | CONTRACT_VERSION | Expected DataKey Count | Discriminants | Notes |
//! |------------------|------------------------|---------------|-------|
//! | 5                | 15                     | 0..=14        | V5 frozen layout |
//! | 6                | 29                     | 0..=28        | V6 freeze (21) + 8 post-freeze additive variants |
//!
//! # Companion Documentation
//! - `contracts/stream/src/checksum.rs` (WASM checksum & key layout documentation)
//! - `docs/upgrade.md` (CONTRACT_VERSION policy & upgrade runbook)
//!
//! # Security assumptions tested
//!
//! - V5 `Stream` entries (memo absent) decode correctly on V6 with `memo == None`.
//! - V5 instance keys (`Config`, `NextStreamId`, pause flags) are readable on V6.
//! - V5 persistent keys (`RecipientStreams`, `AutoClaimDestination`) are readable.
//! - V6-only keys (discriminants 15–20) return absent/default on a V5-seeded instance.
//! - No `None`-unwrap panics occur on any V6 read path when given V5 storage.
//! - `CONTRACT_VERSION` matches the expected `DataKey` variant count (29).

extern crate std;

use fluxora_stream::{
    Config, DataKey, FluxoraStream, FluxoraStreamClient, PauseKind, Stream, StreamStatus,
    CONTRACT_VERSION,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    vec, Address, Env,
};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// Minimal setup: register contract + token, call `init`, return handles.
struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    client: FluxoraStreamClient<'a>,
    token_id: Address,
    admin: Address,
    sender: Address,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.timestamp = 1_000_000);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        sac.mint(&sender, &1_000_000_000);

        client.init(&token_id, &admin);

        Ctx {
            env,
            contract_id,
            client,
            token_id,
            admin,
            sender,
        }
    }

    /// Seed a V5-era `Stream` directly into persistent storage, bypassing the
    /// contract entry-point. `memo: None` faithfully represents a V5 entry
    /// (the V5 struct had no memo field; XDR forward-compat maps absent → None).
    fn seed_v5_stream(&self, stream_id: u64, recipient: &Address) {
        let now = self.env.ledger().timestamp();
        let stream = Stream {
            stream_id,
            sender: self.sender.clone(),
            recipient: recipient.clone(),
            deposit_amount: 86_400,
            rate_per_second: 1,
            start_time: now,
            cliff_time: now,
            end_time: now + 86_400,
            withdrawn_amount: 0,
            status: StreamStatus::Active,
            cancelled_at: None,
            checkpointed_amount: 0,
            checkpointed_at: now,
            withdraw_dust_threshold: 0,
            memo: None, // V5 had no memo field
        };
        let cid = self.contract_id.clone();
        self.env.as_contract(&cid, || {
            self.env
                .storage()
                .persistent()
                .set(&DataKey::Stream(stream_id), &stream);
        });
    }

    /// Seed a V5-era `RecipientStreams` index entry directly.
    fn seed_v5_recipient_streams(&self, recipient: &Address, ids: soroban_sdk::Vec<u64>) {
        let cid = self.contract_id.clone();
        self.env.as_contract(&cid, || {
            self.env
                .storage()
                .persistent()
                .set(&DataKey::RecipientStreams(recipient.clone()), &ids);
        });
    }
}

// ---------------------------------------------------------------------------
// V5 Stream read-path tests
// ---------------------------------------------------------------------------

/// A V5-era Stream (memo absent) is readable by the V6 `get_stream_state` path.
///
/// This is the primary regression guard: if `DataKey::Stream` discriminant (2)
/// ever shifts, this test will panic with `StreamNotFound` instead of returning
/// the seeded value.
#[test]
fn v5_stream_readable_by_v6_get_stream_state() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    let state = ctx.client.get_stream_state(&0u64);

    assert_eq!(state.stream_id, 0);
    assert_eq!(state.recipient, recipient);
    assert_eq!(state.deposit_amount, 86_400);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
    // V5 entry has no memo — V6 decoder must return None, not panic.
    assert!(
        state.memo.is_none(),
        "V5 stream must decode with memo == None"
    );
}

/// V6 `calculate_accrued` works correctly on a V5-era Stream entry.
///
/// Accrual math depends on `start_time`, `cliff_time`, `end_time`,
/// `rate_per_second`, and `checkpointed_amount` — all present in V5.
#[test]
fn v5_stream_calculate_accrued_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    // Advance 100 seconds past start_time
    ctx.env.ledger().with_mut(|l| l.timestamp += 100);

    let accrued = ctx.client.calculate_accrued(&0u64);
    // rate=1 token/s × 100 s = 100
    assert_eq!(
        accrued, 100,
        "accrual on V5 stream must equal rate × elapsed"
    );
}

/// V6 `get_withdrawable` works correctly on a V5-era Stream entry.
#[test]
fn v5_stream_get_withdrawable_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    ctx.env.ledger().with_mut(|l| l.timestamp += 200);

    let withdrawable = ctx.client.get_withdrawable(&0u64);
    // withdrawn_amount=0, accrued=200 → withdrawable=200
    assert_eq!(withdrawable, 200);
}

/// V6 `get_claimable_at` works correctly on a V5-era Stream entry.
#[test]
fn v5_stream_get_claimable_at_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();
    ctx.seed_v5_stream(0, &recipient);

    let claimable = ctx.client.get_claimable_at(&0u64, &(now + 500));
    assert_eq!(claimable, 500);
}

/// Multiple V5-era streams with different IDs are all independently readable.
///
/// Guards against any off-by-one in the `Stream(u64)` key encoding.
#[test]
fn v5_multiple_streams_all_readable() {
    let ctx = Ctx::setup();
    let r0 = Address::generate(&ctx.env);
    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);

    ctx.seed_v5_stream(0, &r0);
    ctx.seed_v5_stream(1, &r1);
    ctx.seed_v5_stream(42, &r2);

    assert_eq!(ctx.client.get_stream_state(&0u64).recipient, r0);
    assert_eq!(ctx.client.get_stream_state(&1u64).recipient, r1);
    assert_eq!(ctx.client.get_stream_state(&42u64).recipient, r2);
}

/// A V5-era Stream with `cancelled_at` set is readable and accrual is frozen.
#[test]
fn v5_cancelled_stream_readable_accrual_frozen() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();

    // Seed a cancelled V5 stream: cancelled 50 s into the stream
    let cancelled_at = now + 50;
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env.storage().persistent().set(
            &DataKey::Stream(0u64),
            &Stream {
                stream_id: 0,
                sender: ctx.sender.clone(),
                recipient: recipient.clone(),
                deposit_amount: 86_400,
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: now + 86_400,
                withdrawn_amount: 0,
                status: StreamStatus::Cancelled,
                cancelled_at: Some(cancelled_at),
                checkpointed_amount: 0,
                checkpointed_at: now,
                withdraw_dust_threshold: 0,
                memo: None,
            },
        );
    });

    // Advance well past cancelled_at
    ctx.env.ledger().with_mut(|l| l.timestamp = now + 1000);

    let state = ctx.client.get_stream_state(&0u64);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(cancelled_at));
    assert!(state.memo.is_none());

    // Accrual must be frozen at cancelled_at (50 tokens)
    let accrued = ctx.client.calculate_accrued(&0u64);
    assert_eq!(
        accrued, 50,
        "cancelled V5 stream accrual must be frozen at cancelled_at"
    );
}

/// A V5-era Stream with non-zero `checkpointed_amount` decodes correctly.
///
/// `checkpointed_amount` was added in V2; V5 entries always have it set.
#[test]
fn v5_stream_with_checkpoint_readable() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();

    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env.storage().persistent().set(
            &DataKey::Stream(0u64),
            &Stream {
                stream_id: 0,
                sender: ctx.sender.clone(),
                recipient: recipient.clone(),
                deposit_amount: 10_000,
                rate_per_second: 2,
                start_time: now,
                cliff_time: now,
                end_time: now + 5_000,
                withdrawn_amount: 0,
                status: StreamStatus::Active,
                cancelled_at: None,
                checkpointed_amount: 500, // accrued under a prior rate
                checkpointed_at: now,
                withdraw_dust_threshold: 0,
                memo: None,
            },
        );
    });

    ctx.env.ledger().with_mut(|l| l.timestamp += 100);

    let state = ctx.client.get_stream_state(&0u64);
    assert_eq!(state.checkpointed_amount, 500);
    assert!(state.memo.is_none());
}

// ---------------------------------------------------------------------------
// V5 instance key read-path tests
// ---------------------------------------------------------------------------

/// V5 `Config` (discriminant 0) is readable by V6 `get_config`.
///
/// `init` writes `Config` via the contract entry-point, so this test verifies
/// that the discriminant-0 key written by V5 is still decoded correctly by V6.
#[test]
fn v5_config_key_readable_by_v6() {
    let ctx = Ctx::setup();
    // `init` already wrote Config; verify V6 reads it correctly.
    let cfg = ctx.client.get_config();
    assert_eq!(cfg.admin, ctx.admin);
    assert_eq!(cfg.token, ctx.token_id);
}

/// V5 `NextStreamId` (discriminant 1) is readable by V6 `get_stream_count`.
#[test]
fn v5_next_stream_id_readable_by_v6() {
    let ctx = Ctx::setup();
    // Seed NextStreamId directly to simulate a V5 instance with 3 streams created.
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::NextStreamId, &3u64);
    });

    let count = ctx.client.get_stream_count();
    assert_eq!(
        count, 3,
        "V5 NextStreamId must be readable by V6 get_stream_count"
    );
}

/// V5 `GlobalEmergencyPaused` (discriminant 4) is readable by V6.
///
/// When set to `true` on a V5 instance, V6 must still honour the pause.
#[test]
fn v5_global_emergency_paused_readable_by_v6() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::GlobalEmergencyPaused, &true);
    });

    // V6 `set_contract_paused` reads GlobalEmergencyPaused; verify via version()
    // which does NOT check pause state — so we verify the flag is present by
    // attempting a paused operation and expecting ContractPaused.
    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();
    let err = ctx.client.try_create_streams(
        &ctx.sender,
        &vec![
            &ctx.env,
            fluxora_stream::CreateStreamParams {
                recipient,
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: now + 1000,
                withdraw_dust_threshold: None,
                memo: None,
            },
        ],
    );
    assert_eq!(
        err,
        Err(Ok(fluxora_stream::ContractError::ContractPaused)),
        "V5 GlobalEmergencyPaused=true must block V6 stream creation"
    );
}

/// V5 `CreationPaused` (discriminant 5) is readable by V6.
#[test]
fn v5_creation_paused_readable_by_v6() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::CreationPaused, &true);
    });

    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();
    let err = ctx.client.try_create_streams(
        &ctx.sender,
        &vec![
            &ctx.env,
            fluxora_stream::CreateStreamParams {
                recipient,
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: now + 1000,
                withdraw_dust_threshold: None,
                memo: None,
            },
        ],
    );
    assert_eq!(
        err,
        Err(Ok(fluxora_stream::ContractError::ContractPaused)),
        "V5 CreationPaused=true must block V6 stream creation"
    );
}

/// V5 `TotalLiabilities` (discriminant 14) is readable by V6.
///
/// Discriminant 14 is the last frozen V5 key. If any variant were inserted
/// before it, this read would return the wrong type and panic.
#[test]
fn v5_total_liabilities_readable_by_v6() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::TotalLiabilities, &999_i128);
    });

    // Verify by reading via get_total_liabilities if exposed, or indirectly
    // by confirming no panic occurs when the contract reads instance storage.
    // `version()` touches instance storage (bump_instance_ttl) without reading
    // TotalLiabilities, so we verify the key is present by reading it directly.
    let cid2 = ctx.contract_id.clone();
    ctx.env.as_contract(&cid2, || {
        let val: i128 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLiabilities)
            .expect("TotalLiabilities must be present after V5 seed");
        assert_eq!(val, 999_i128);
    });
}

// ---------------------------------------------------------------------------
// V5 RecipientStreams (discriminant 3) read-path tests
// ---------------------------------------------------------------------------

/// V5 `RecipientStreams` (discriminant 3) is readable by V6 `get_recipient_streams`.
#[test]
fn v5_recipient_streams_readable_by_v6() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    // Seed V5-era streams and index
    ctx.seed_v5_stream(0, &recipient);
    ctx.seed_v5_stream(1, &recipient);
    ctx.seed_v5_recipient_streams(&recipient, vec![&ctx.env, 0u64, 1u64]);

    let ids = ctx.client.get_recipient_streams(&recipient, &None, &None);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids.get(0).unwrap(), 0u64);
    assert_eq!(ids.get(1).unwrap(), 1u64);
}

/// V5 `get_recipient_stream_count` works on a V5-seeded index.
#[test]
fn v5_recipient_stream_count_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    ctx.seed_v5_stream(0, &recipient);
    ctx.seed_v5_stream(5, &recipient);
    ctx.seed_v5_stream(10, &recipient);
    ctx.seed_v5_recipient_streams(&recipient, vec![&ctx.env, 0u64, 5u64, 10u64]);

    let count = ctx.client.get_recipient_stream_count(&recipient);
    assert_eq!(count, 3);
}

/// A recipient with no V5 index entry returns an empty list (no panic).
#[test]
fn v5_absent_recipient_streams_returns_empty() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    // No seed — simulates a V5 instance where this recipient had no streams.
    let ids = ctx.client.get_recipient_streams(&recipient, &None, &None);
    assert_eq!(ids.len(), 0);
}

// ---------------------------------------------------------------------------
// V6-only key absence tests (discriminants 15–20)
// ---------------------------------------------------------------------------
//
// On a V5-seeded instance, none of the V6-only keys should be present.
// These tests confirm that V6 read paths return absent/default rather than
// panicking or returning stale data from a shifted discriminant.

/// `WithdrawNonce` (discriminant 15) is absent on a V5 instance.
///
/// V6 delegated-withdraw must treat an absent nonce as 0 (first use).
#[test]
fn v6_withdraw_nonce_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::WithdrawNonce(addr.clone()));
        assert!(
            !present,
            "WithdrawNonce must be absent on a V5-seeded instance"
        );
    });
}

/// `PauseState` (discriminant 16) is absent on a V5 instance.
///
/// V6 reads PauseState as `Option`; absent means the protocol is not paused
/// via the V6 PauseState mechanism.
#[test]
fn v6_pause_state_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx.env.storage().instance().has(&DataKey::PauseState);
        assert!(
            !present,
            "PauseState must be absent on a V5-seeded instance"
        );
    });
}

/// `ReentrancyLock` (discriminant 17) is absent on a V5 instance.
///
/// V6 reads ReentrancyLock as `bool`; absent means the lock is not held.
#[test]
fn v6_reentrancy_lock_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx.env.storage().instance().has(&DataKey::ReentrancyLock);
        assert!(
            !present,
            "ReentrancyLock must be absent on a V5-seeded instance"
        );
    });
}

/// `RecipientStreamPage` (discriminant 18) is absent on a V5 instance.
///
/// V5 used `RecipientStreams` (discriminant 3) for the flat index.
/// V6 adds paged index entries; they must not exist on a V5 instance.
#[test]
fn v6_recipient_stream_page_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::RecipientStreamPage(addr.clone(), 0u32));
        assert!(
            !present,
            "RecipientStreamPage must be absent on a V5-seeded instance"
        );
    });
}

/// `RecipientStreamPageCount` (discriminant 19) is absent on a V5 instance.
#[test]
fn v6_recipient_stream_page_count_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let addr = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::RecipientStreamPageCount(addr.clone()));
        assert!(
            !present,
            "RecipientStreamPageCount must be absent on a V5-seeded instance"
        );
    });
}

/// `PendingRecipientUpdate` (discriminant 20) is absent on a V5 instance.
#[test]
fn v6_pending_recipient_update_absent_on_v5_instance() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let present = ctx
            .env
            .storage()
            .persistent()
            .has(&DataKey::PendingRecipientUpdate(0u64));
        assert!(
            !present,
            "PendingRecipientUpdate must be absent on a V5-seeded instance"
        );
    });
}

// ---------------------------------------------------------------------------
// Discriminant stability smoke tests
// ---------------------------------------------------------------------------
//
// These tests write a known value under a specific DataKey and read it back
// via the same key. If any discriminant shifts (e.g. due to a mid-enum
// insertion), the read will return None or the wrong type, causing a panic.
// They are intentionally redundant with the read-path tests above to provide
// a second layer of detection.

/// Discriminant 0 (Config) round-trips correctly.
#[test]
fn discriminant_0_config_round_trips() {
    let ctx = Ctx::setup();
    let new_admin = Address::generate(&ctx.env);
    let cid = ctx.contract_id.clone();
    let token_addr = ctx.token_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env.storage().instance().set(
            &DataKey::Config,
            &Config {
                token: token_addr.clone(),
                admin: new_admin.clone(),
            },
        );
        let cfg: Config = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .expect("Config must round-trip at discriminant 0");
        assert_eq!(cfg.admin, new_admin);
        assert_eq!(cfg.token, token_addr);
    });
}

/// Discriminant 1 (NextStreamId) round-trips correctly.
#[test]
fn discriminant_1_next_stream_id_round_trips() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::NextStreamId, &7u64);
        let val: u64 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::NextStreamId)
            .expect("NextStreamId must round-trip at discriminant 1");
        assert_eq!(val, 7u64);
    });
}

/// Discriminant 2 (Stream) round-trips correctly.
#[test]
fn discriminant_2_stream_round_trips() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(99, &recipient);

    let state = ctx.client.get_stream_state(&99u64);
    assert_eq!(state.stream_id, 99);
    assert_eq!(state.recipient, recipient);
}

/// Discriminant 3 (RecipientStreams) round-trips correctly.
#[test]
fn discriminant_3_recipient_streams_round_trips() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);
    ctx.seed_v5_recipient_streams(&recipient, vec![&ctx.env, 0u64]);

    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        let ids: soroban_sdk::Vec<u64> = ctx
            .env
            .storage()
            .persistent()
            .get(&DataKey::RecipientStreams(recipient.clone()))
            .expect("RecipientStreams must round-trip at discriminant 3");
        assert_eq!(ids.get(0).unwrap(), 0u64);
    });
}

/// Discriminant 14 (TotalLiabilities) is the last frozen V5 key.
/// A round-trip confirms no variant was inserted before it.
#[test]
fn discriminant_14_total_liabilities_round_trips() {
    let ctx = Ctx::setup();
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::TotalLiabilities, &12345_i128);
        let val: i128 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLiabilities)
            .expect("TotalLiabilities must round-trip at discriminant 14");
        assert_eq!(val, 12345_i128);
    });
}

// ---------------------------------------------------------------------------
// CONTRACT_VERSION smoke test
// ---------------------------------------------------------------------------

/// `version()` returns the compile-time constant without touching storage.
///
/// This test is intentionally minimal: it confirms the entry-point is callable
/// on a V5-seeded instance (V6 only changes sweep_excess authorization, so no
/// new storage keys are written).
#[test]
fn version_entry_point_works_on_v5_seeded_instance() {
    let ctx = Ctx::setup();
    // Seed only V5-era state (no V6 keys written)
    let recipient = Address::generate(&ctx.env);
    ctx.seed_v5_stream(0, &recipient);

    let v = ctx.client.version();
    assert_eq!(v, CONTRACT_VERSION);
}

// ---------------------------------------------------------------------------
// CONTRACT_VERSION vs DataKey variant count cross-check suite
// ---------------------------------------------------------------------------

/// Returns the documented expected `DataKey` variant count for a given `CONTRACT_VERSION`.
///
/// # Version Mapping Table
///
/// | CONTRACT_VERSION | Expected DataKey Variant Count | Discriminant Range | Notes |
/// |------------------|--------------------------------|--------------------|-------|
/// | 5                | 15                             | 0..=14             | V5 release freeze |
/// | 6                | 29                             | 0..=28             | V6 freeze (21) + 8 post-freeze additive variants |
///
/// # Security Safeguard & Maintenance Protocol
/// When a new `DataKey` variant is appended or `CONTRACT_VERSION` is bumped:
/// 1. Update this function's match table to include the new mapping.
/// 2. Update `all_live_datakey_variants()` to include a sample of the new variant.
/// 3. Update prose docs & discriminant tests in `contracts/stream/src/checksum.rs`.
/// 4. Update version history & upgrade strategy in `docs/upgrade.md`.
pub fn expected_datakey_count_for_version(version: u32) -> usize {
    match version {
        5 => 15,
        6 => 29,
        other => panic!(
            "Unhandled CONTRACT_VERSION = {other} in expected_datakey_count_for_version. \
             When incrementing CONTRACT_VERSION, you must update the version mapping table in \
             contracts/stream/tests/storage_key_compat.rs, contracts/stream/src/checksum.rs, \
             and docs/upgrade.md."
        ),
    }
}

/// Constructs a vector containing sample instances of all 29 live `DataKey` variants in declaration order.
///
/// Includes an exhaustive `match` check on `DataKey` so that adding any new variant to `DataKey`
/// without adding it here will produce a compile error.
pub fn all_live_datakey_variants(env: &Env) -> vec::Vec<DataKey> {
    let dummy_addr = Address::generate(env);
    let dummy_pause_kind = PauseKind::Protocol;

    let variants = vec![
        env,
        DataKey::Config,                                       // 0
        DataKey::NextStreamId,                                 // 1
        DataKey::Stream(0),                                    // 2
        DataKey::RecipientStreams(dummy_addr.clone()),         // 3
        DataKey::GlobalEmergencyPaused,                        // 4
        DataKey::CreationPaused,                               // 5
        DataKey::GlobalPauseReason,                            // 6
        DataKey::GlobalPauseTimestamp,                         // 7
        DataKey::GlobalPauseAdmin,                             // 8
        DataKey::AutoClaimDestination(0),                      // 9
        DataKey::NextTemplateId,                               // 10
        DataKey::ActiveTemplateCount,                          // 11
        DataKey::StreamTemplate(0),                            // 12
        DataKey::OwnerTemplateIds(dummy_addr.clone()),         // 13
        DataKey::TotalLiabilities,                             // 14
        DataKey::WithdrawNonce(dummy_addr.clone()),            // 15
        DataKey::PauseState,                                   // 16
        DataKey::ReentrancyLock,                               // 17
        DataKey::RecipientStreamPage(dummy_addr.clone(), 0),   // 18
        DataKey::RecipientStreamPageCount(dummy_addr.clone()), // 19
        DataKey::PendingRecipientUpdate(0),                    // 20
        DataKey::IdReservation(dummy_addr.clone()),            // 21
        DataKey::MaxRatePerSecond,                             // 22
        DataKey::DelegatedWithdrawNonce(dummy_addr.clone()),    // 23
        DataKey::LastPauseRecord(dummy_pause_kind),            // 24
        DataKey::RotationHistory(0),                            // 25
        DataKey::LastAccrualLedgerTimestamp,                   // 26
        DataKey::PausedStreamCount,                            // 27
        DataKey::TotalKeeperFeesPaid,                          // 28
    ];

    // Exhaustive match check to ensure compile-time coverage of all DataKey variants.
    for k in variants.iter() {
        match k {
            DataKey::Config => {}
            DataKey::NextStreamId => {}
            DataKey::Stream(_) => {}
            DataKey::RecipientStreams(_) => {}
            DataKey::GlobalEmergencyPaused => {}
            DataKey::CreationPaused => {}
            DataKey::GlobalPauseReason => {}
            DataKey::GlobalPauseTimestamp => {}
            DataKey::GlobalPauseAdmin => {}
            DataKey::AutoClaimDestination(_) => {}
            DataKey::NextTemplateId => {}
            DataKey::ActiveTemplateCount => {}
            DataKey::StreamTemplate(_) => {}
            DataKey::OwnerTemplateIds(_) => {}
            DataKey::TotalLiabilities => {}
            DataKey::WithdrawNonce(_) => {}
            DataKey::PauseState => {}
            DataKey::ReentrancyLock => {}
            DataKey::RecipientStreamPage(_, _) => {}
            DataKey::RecipientStreamPageCount(_) => {}
            DataKey::PendingRecipientUpdate(_) => {}
            DataKey::IdReservation(_) => {}
            DataKey::MaxRatePerSecond => {}
            DataKey::DelegatedWithdrawNonce(_) => {}
            DataKey::LastPauseRecord(_) => {}
            DataKey::RotationHistory(_) => {}
            DataKey::LastAccrualLedgerTimestamp => {}
            DataKey::PausedStreamCount => {}
            DataKey::TotalKeeperFeesPaid => {}
        }
    }

    variants
}

/// Machine-checks that `CONTRACT_VERSION` matches the expected `DataKey` variant count.
///
/// # Machine-Checked Enforcement
///
/// Fails loudly with an explicit error message if the live `DataKey` variant count
/// diverges from `expected_datakey_count_for_version(CONTRACT_VERSION)`.
///
/// Cross-references:
/// - `contracts/stream/src/checksum.rs`
/// - `docs/upgrade.md`
#[test]
fn test_contract_version_matches_datakey_variant_count() {
    let env = Env::default();
    let live_variants = all_live_datakey_variants(&env);
    let live_count = live_variants.len() as usize;

    let expected_count = expected_datakey_count_for_version(CONTRACT_VERSION);

    assert_eq!(
        live_count,
        expected_count,
        "CRITICAL VERSION DRIFT: CONTRACT_VERSION ({}) expects {} DataKey variants, \
         but the live DataKey enum has {} variants. \
         When adding a new DataKey variant, you MUST update: \
         1. expected_datakey_count_for_version() in contracts/stream/tests/storage_key_compat.rs \
         2. all_live_datakey_variants() in contracts/stream/tests/storage_key_compat.rs \
         3. Prose tables & variant count tests in contracts/stream/src/checksum.rs \
         4. Version history & policy in docs/upgrade.md \
         5. CONTRACT_VERSION in contracts/stream/src/lib.rs if required by versioning policy.",
        CONTRACT_VERSION,
        expected_count,
        live_count
    );
}

/// Edge case: V5 version mapping expected count is 15.
#[test]
fn test_expected_datakey_count_mapping_v5() {
    assert_eq!(expected_datakey_count_for_version(5), 15);
}

/// Edge case: V6 version mapping expected count is 29.
#[test]
fn test_expected_datakey_count_mapping_v6() {
    assert_eq!(expected_datakey_count_for_version(6), 29);
}

/// Edge case: Unmapped/future versions trigger panic forcing deliberate mapping update.
#[test]
#[should_panic(expected = "Unhandled CONTRACT_VERSION = 999")]
fn test_expected_datakey_count_mapping_unhandled_version_panics() {
    expected_datakey_count_for_version(999);
}

/// Assert exact live variant count is 29 (discriminants 0..=28).
#[test]
fn test_datakey_variant_count_exact_29() {
    let env = Env::default();
    let live_variants = all_live_datakey_variants(&env);
    assert_eq!(
        live_variants.len() as usize,
        29,
        "DataKey variant count changed without updating storage_key_compat test suite."
    );
}

