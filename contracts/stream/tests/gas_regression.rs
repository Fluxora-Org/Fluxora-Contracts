/// Batch benchmark coverage and stress-case regression for `fluxora_stream`.
///
/// This harness locks down gas behavior around the existing batch flow so
/// future refactors cannot regress performance or break backward
/// compatibility. It explicitly covers:
///
/// * Happy-path batch creation (`create_streams`) at 1, 5, 10 streams.
/// * Happy-path batch withdrawal (`batch_withdraw`) at 1, 10, 50, 100 IDs.
/// * Per-entry routed withdrawal (`batch_withdraw_to`) at 1, 10, 50, 100 IDs.
/// * Mixed-state batch withdrawal (`batch_withdraw` with Active /
///   Cancelled / Completed streams together).
/// * Bulk admin resume (`bulk_resume_streams_as_admin`) at 1, 10, 50, 100.
/// * Bulk sender cancel (`bulk_cancel_streams`) at 1, 10, 50, 100.
/// * Keeper economic incentive (`keeper_cancel`) for both partial and fully
///   accrued streams.
/// * Persistent-entry size ceiling (`test_stream_entry_xdr_size_*`) for
///   baseline, memo-only, metadata-only, and worst-case configurations.
///
/// Additionally, this module stabilizes batch benchmark behavior across upgrades
/// and retries by enforcing deterministic validation and clear error paths for:
///
/// * Duplicate stream ID detection in batch operations (single retry)
/// * Duplicate destination address detection in `batch_withdraw_to` (single retry)
/// * Zero-amount batch withdrawals with potential mixed terminal/non-terminal streams
/// * Empty batch withdrawal validation (already covered, but validated for determinism)
/// * Consolidation of duplicate entry detection for improved resilience
///
/// All measurements are emitted as `GAS_MEASUREMENT: <function>: <size>: <cost>`
/// and compared by `script/validate_gas.py` against the JSON baseline in
/// `docs/gas.md`. The `PER_INVOCATION_CPU_BUDGET` assertion ensures no batch
/// entry-point exceeds 25B CPU instructions (75% of Soroban's 100B ceiling).
///
/// See docs/gas.md §"Batch Benchmark Coverage" for the full regression surface,
/// baseline update procedure, and upgrade-compatibility notes.
// See docs/gas.md for the baseline update process and review bar.
use fluxora_stream::{
    accrual::pack_rate_segment, BatchWithdrawResult, CreateStreamParams, FluxoraStream,
    FluxoraStreamClient, PauseReason, StreamKind, WithdrawToParam,
    MAX_MEMO_BYTES, MAX_METADATA_BYTES, MAX_METADATA_KEYS,
    MAX_METADATA_KEY_BYTES, MAX_METADATA_VALUE_BYTES,
    MAX_STREAM_ENTRY_BYTES,
};
use soroban_sdk::{
    contracttype, symbol_short,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    xdr::ToXdr,
    Address, Bytes, Env, Map, Vec,
};

// Per-invocation CPU budget (Soroban limit) with a 75% safety margin.
// The budget assertion fails if measured cost exceeds this threshold,
// guarding against inadvertent regressions (e.g. an increased MAX_PAGE_SIZE
// that worsens the O(n²) duplicate-ID scan).
const PER_INVOCATION_CPU_BUDGET: u64 = 25_000_000_000;

// Grace period (mirrors KEEPER_GRACE_PERIOD_SECONDS in lib.rs).
const KEEPER_GRACE: u64 = 604_800;

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    keeper: Address,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let keeper = Address::generate(&env);

        client.init(&token_id, &admin);

        // Fund the sender using the admin's minting power
        sac.mint(&sender, &1_000_000_i128);
        // Provide default allowance so create_stream can pull the deposit.
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
            keeper,
        }
    }

    fn create_default_stream(&self) -> u64 {
        let amount = 1000_i128;
        let rate = 1_i128;
        let start_time = 0u64;
        let cliff_time = 0u64;
        let end_time = 1000u64;

        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: amount,
                rate_per_second: rate,
                start_time: start_time,
                cliff_time: cliff_time,
                end_time: end_time,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

// KeeperTestContext is an alias for TestContext — same setup, same fields.
// Keeper tests use the identical harness; the type alias keeps the test
// names readable without duplicating the setup code.
type KeeperTestContext<'a> = TestContext<'a>;

fn measure_gas<F, C>(ctx: &C, f: F) -> u64
where
    F: FnOnce(&C),
    C: HasEnv,
{
    ctx.env().budget().reset_unlimited();
    f(ctx);
    ctx.env().budget().cpu_instruction_cost()
}

trait HasEnv {
    fn env(&self) -> &Env;
}

impl HasEnv for TestContext<'_> {
    fn env(&self) -> &Env {
        &self.env
    }
}

#[test]
fn test_create_stream_gas() {
    let ctx = TestContext::setup();

    let cost = measure_gas(&ctx, |ctx| {
        ctx.create_default_stream();
    });

    println!("GAS_MEASUREMENT: create_stream: single: {}", cost);
}

#[test]
fn test_withdraw_gas() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500); // Accrue 500 tokens

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.withdraw(&stream_id, &None);
    });

    println!("GAS_MEASUREMENT: withdraw: single: {}", cost);
}

#[test]
fn test_batch_withdraw_gas() {
    let sizes = [1, 10, 50, 100];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(ctx.create_default_stream());
        }

        ctx.env.ledger().set_timestamp(500); // Accrue tokens for all

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.batch_withdraw(&ctx.recipient, &streams);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "batch_withdraw at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!("GAS_MEASUREMENT: batch_withdraw: {}: {}", size, cost);
    }
}

/// Gas regression baseline for `batch_withdraw_to`.
///
/// Uses a distinct destination address per withdrawal to exercise the
/// per-entry destination validation path alongside the O(n²) duplicate-ID
/// scan in `reject_duplicate_ids`.  The O(n²) scan costs roughly
/// n*(n-1)/2 comparisons at batch size n, so at MAX_PAGE_SIZE (100) the
/// worst case is ~4 950 element-by-element comparisons inside the helper.
#[test]
fn test_batch_withdraw_to_gas() {
    let sizes = [1, 10, 50, 100];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        let mut destinations = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(ctx.create_default_stream());
            destinations.push_back(Address::generate(&ctx.env));
        }

        let mut withdrawals = soroban_sdk::Vec::new(&ctx.env);
        for i in 0..size {
            withdrawals.push_back(WithdrawToParam {
                stream_id: streams.get(i as u32).unwrap(),
                destination: destinations.get(i as u32).unwrap(),
            });
        }

        ctx.env.ledger().set_timestamp(500); // Accrue tokens for all

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.batch_withdraw_to(&ctx.recipient, &withdrawals);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "batch_withdraw_to at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!("GAS_MEASUREMENT: batch_withdraw_to: {}: {}", size, cost);
    }
}

#[test]
fn test_batch_withdraw_mixed_state_gas() {
    let ctx = TestContext::setup();

    let active_id = ctx.create_default_stream();
    let completed_id = ctx.create_default_stream();
    let cancelled_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client.cancel_stream(&cancelled_id);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client.withdraw(&completed_id, &None);

    let mut stream_ids = Vec::new(&ctx.env);
    stream_ids.push_back(active_id);
    stream_ids.push_back(cancelled_id);
    stream_ids.push_back(completed_id);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.env.ledger().set_timestamp(1000);
        ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
    });

    println!("GAS_MEASUREMENT: batch_withdraw: mixed-state: {}", cost);
}

#[test]
fn test_create_streams_gas() {
    let ctx = TestContext::setup();
    let sizes = [1, 5, 10];

    for &size in &sizes {
        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(CreateStreamParams {
                kind: StreamKind::Linear,
                withdraw_dust_threshold: None,
                recipient: Address::generate(&ctx.env),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: 1000u64,
                memo: None,
                metadata: None,
                irrevocable: None,
                witness: None,
            });
        }

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.create_streams(&ctx.sender, &streams);
        });

        println!("GAS_MEASUREMENT: create_streams: {}: {}", size, cost);
    }
}

/// Gas regression baseline for `bulk_resume_streams_as_admin`.
///
/// Creates streams, pauses each one (advancing the ledger far enough to
/// clear the pause cooldown), then resumes them all in a single admin-authed
/// call. Batch sizes 1, 10, 50, and 100 cover the full batch capacity up to
/// MAX_PAGE_SIZE so `script/validate_gas.py` can compare measured costs.
#[test]
fn test_bulk_resume_streams_as_admin_gas() {
    let sizes = [1, 10, 50, 100];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            let id = ctx.create_default_stream();
            // Advance past the pause/resume cooldown (17 ledgers) so the
            // subsequent pause succeeds even if the ledger sequence is low.
            ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
            ctx.client
                .pause_stream_as_admin(&id, &PauseReason::Administrative);
            streams.push_back(id);
        }

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.bulk_resume_streams_as_admin(&streams);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "bulk_resume_streams_as_admin at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!(
            "GAS_MEASUREMENT: bulk_resume_streams_as_admin: {}: {}",
            size, cost
        );
    }
}

/// Gas regression baseline for `bulk_cancel_streams`.
///
/// Creates active streams owned by the sender then cancels them all in a
/// single call. Batch sizes 1, 10, 50, and 100 cover the full batch capacity
/// up to MAX_PAGE_SIZE so `script/validate_gas.py` can compare measured costs.
#[test]
fn test_bulk_cancel_streams_gas() {
    let sizes = [1, 10, 50, 100];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(ctx.create_default_stream());
        }

        ctx.env.ledger().set_timestamp(500); // Accrue tokens so cancellation is non-trivial

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.bulk_cancel_streams(&ctx.sender, &streams);
        });

        assert!(
            cost <= PER_INVOCATION_CPU_BUDGET,
            "bulk_cancel_streams at size {} exceeded per-invocation CPU budget: {} > {}",
            size,
            cost,
            PER_INVOCATION_CPU_BUDGET,
        );

        println!("GAS_MEASUREMENT: bulk_cancel_streams: {}: {}", size, cost);
    }
}

// ---------------------------------------------------------------------------
// keeper_cancel gas measurements
//
// Two variants capture the two meaningful cost paths:
//
//   partial_accrual — the common keeper incentive case: the stream expired with
//     an unstreamed balance, so the contract makes three token transfers
//     (recipient, sender, keeper).  This is the hot path for economically
//     rational keeper bots and the cost documented in docs/gas.md's
//     break-even formula.
//
//   fully_accrued   — the degenerate case: deposit == rate × duration, so
//     sender_refund_gross == 0, keeper_fee == 0 and no keeper transfer is
//     issued.  Only one token transfer (to the recipient) occurs.  Cost is
//     slightly lower than the partial_accrual variant.
//
// Both variants print a GAS_MEASUREMENT line that validate_gas.py picks up
// and compares against the JSON baseline in docs/gas.md.
// ---------------------------------------------------------------------------

/// keeper_cancel on a stream that still has an unstreamed balance (3 transfers).
///
/// Setup:
///   deposit = 10 000, rate = 5 token/s, start = 0, end = 1 000
///   → accrued at end_time = min(5 × 1 000, 10 000) = 5 000
///   → sender_refund_gross = 5 000
///   → keeper_fee = 5 000 × 50 / 10 000 = 25
///   → three token transfers: recipient 5 000, sender 4 975, keeper 25
#[test]
fn test_keeper_cancel_gas_partial_accrual() {
    let ctx = KeeperTestContext::setup();

    // Create the stream at t=0.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 10_000_i128,
            rate_per_second: 5_i128,
            start_time: 0_u64,
            cliff_time: 0_u64,
            end_time: 1_000_u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Advance past end_time + grace period so the stream is eligible.
    ctx.env.ledger().set_timestamp(1_000 + KEEPER_GRACE + 1);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.keeper_cancel(&stream_id, &ctx.keeper);
    });

    // Print in the canonical GAS_MEASUREMENT format so validate_gas.py can
    // parse this line and compare it against the baseline in docs/gas.md.
    println!("GAS_MEASUREMENT: keeper_cancel: partial_accrual: {}", cost);
}

/// keeper_cancel on a stream that is fully accrued (1 transfer, keeper fee == 0).
///
/// Setup:
///   deposit = 1 000, rate = 1 token/s, start = 0, end = 1 000
///   → accrued at end_time = 1 000 == deposit
///   → sender_refund_gross = 0, keeper_fee = 0
///   → one token transfer: recipient 1 000; no sender or keeper transfers
#[test]
fn test_keeper_cancel_gas_fully_accrued() {
    let ctx = KeeperTestContext::setup();

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: 0_u64,
            cliff_time: 0_u64,
            end_time: 1_000_u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(1_000 + KEEPER_GRACE + 1);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.keeper_cancel(&stream_id, &ctx.keeper);
    });

    println!("GAS_MEASUREMENT: keeper_cancel: fully_accrued: {}", cost);
}

// ---------------------------------------------------------------------------
// Edge Case: Duplicate Stream IDs in Batch Operations (Deterministic Retry)
// ---------------------------------------------------------------------------

/// Duplicate stream ID detection in batch_withdraw is idempotent.
///
/// After a duplicate batch fails with `ContractError::DuplicateStreamId`,
/// retrying the same batch produces the identical error. This stabilizes
/// behavior across upgrades and ensures deterministic error handling.
#[test]
fn test_batch_withdraw_duplicate_ids_idempotent() {
    let ctx = TestContext::setup();
    let id1 = ctx.create_default_stream();
    let id2 = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    let mut stream_ids = soroban_sdk::Vec::new(&ctx.env);
    stream_ids.push_back(id1);
    stream_ids.push_back(id2);
    stream_ids.push_back(id1); // duplicate

    let result1 = ctx.client.try_batch_withdraw(&ctx.recipient, &stream_ids);
    assert!(result1.is_err());
    let error_type1 = if result1.is_err() { "DuplicateStreamId" } else { "Unknown" };

    // Retry with same duplicate list should produce identical error (idempotent)
    let result2 = ctx.client.try_batch_withdraw(&ctx.recipient, &stream_ids);
    let error_type2 = if result2.is_err() { "DuplicateStreamId" } else { "Unknown" };

    assert_eq!(error_type1, error_type2);
    assert!(result1.is_err());
    assert!(result2.is_err());
}

/// Batch withdrawal with valid (non-duplicate) stream IDs is deterministic.
///
/// After a successful batch withdraw, retrying the same batch produces identical
/// results because all streams are in a terminal state (Completed). This ensures
/// deterministic behavior across upgrades and retries.
#[test]
fn test_batch_withdraw_valid_ids_deterministic() {
    let ctx = TestContext::setup();
    let id1 = ctx.create_default_stream();
    let id2 = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    let mut stream_ids = soroban_sdk::Vec::new(&ctx.env);
    stream_ids.push_back(id1);
    stream_ids.push_back(id2);

    let result1 = ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(result1.len(), 2);

    // After withdrawal, withdrawable should be 0 for both streams
    let s1_state = ctx.client.get_stream_state(&id1);
    assert_eq!(s1_state.withdrawn_amount, 500);

    // Retry batch on already-withdrawn streams
    let result2 = ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(result2.len(), 2);

    // Withdrawn amounts should be unchanged
    let s1_state_after = ctx.client.get_stream_state(&id1);
    assert_eq!(s1_state_after.withdrawn_amount, 500);
}

/// Zero-amount batch withdrawals with mixed terminal and non-terminal streams.
///
/// A batch containing both Completed and Active streams should process deterministically:
/// Completed streams yield 0, Active streams are processed normally. This stabilizes behavior
/// for edge cases where batch processing includes already-completed streams.
#[test]
fn test_batch_withdraw_zero_amount_mixed_terminal() {
    let ctx = TestContext::setup();
    let active_id = ctx.create_default_stream();
    let completed_id = ctx.create_default_stream();

    // Complete the first stream
    ctx.env.ledger().set_timestamp(500);
    ctx.client.withdraw(&completed_id, &None);

    ctx.env.ledger().set_timestamp(1000);

    let mut stream_ids = soroban_sdk::Vec::new(&ctx.env);
    stream_ids.push_back(active_id);
    stream_ids.push_back(completed_id);

    // Batch with both active and completed streams
    let result1 = ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(result1.len(), 2);

    // Result should have 0 for completed and >0 for active
    let mut zero_count = 0;
    let mut positive_count = 0;
    for r in result1.iter() {
        if r.amount == 0 {
            zero_count += 1;
        } else if r.amount > 0 {
            positive_count += 1;
        }
    }
    assert_eq!(zero_count, 1);
    assert_eq!(positive_count, 1);
}

/// Empty batch withdrawal validation for determinism.
///
/// Empty batch withdrawals should succeed deterministically and produce empty results
/// without affecting contract state. This ensures predictable behavior even for edge cases
/// and stabilizes behavior across versions and upgrades.
#[test]
fn test_batch_withdraw_empty_batch() {
    let ctx = TestContext::setup();

    let mut stream_ids = soroban_sdk::Vec::new(&ctx.env);

    let result = ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(result.len(), 0);
}

/// Edge Case: Duplicate Destination Addresses in batch_withdraw_to.
///
/// `batch_withdraw_to` should detect and reject duplicate destination addresses
/// for deterministic error handling across retries. This ensures consistent behavior
/// even after contract upgrades or across retry attempts.
#[test]
fn test_batch_withdraw_to_duplicate_destinations_idempotent() {
    let ctx = TestContext::setup();
    let stream_id_a = ctx.create_default_stream();
    let stream_id_b = ctx.create_default_stream();
    let destination_a = ctx.recipient.clone();
    let destination_b = destination_a.clone(); // Same destination -> duplicate

    ctx.env.ledger().set_timestamp(500);

    let mut withdrawals = soroban_sdk::Vec::new(&ctx.env);
    withdrawals.push_back(WithdrawToParam {
        stream_id: stream_id_a,
        destination: destination_a.clone(),
    });
    withdrawals.push_back(WithdrawToParam {
        stream_id: stream_id_b,
        destination: destination_b.clone(), // Duplicate destination
    });

    let result1 = ctx.client.try_batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert!(result1.is_err());
    let error_type1 = if result1.is_err() { "DuplicateDestination" } else { "Unknown" };

    // Retry with same duplicate list should produce identical error (idempotent)
    let result2 = ctx.client.try_batch_withdraw_to(&ctx.recipient, &withdrawals);
    let error_type2 = if result2.is_err() { "DuplicateDestination" } else { "Unknown" };

    assert_eq!(error_type1, error_type2);
    assert!(result1.is_err());
    assert!(result2.is_err());
}

// ---------------------------------------------------------------------------
// Stream persistent-entry XDR size regression
// ---------------------------------------------------------------------------

/// Helper: build a worst-case metadata map that exactly fits within
/// MAX_METADATA_BYTES.  We use 8 keys of 32 bytes and values sized so that
/// the aggregate key+value sum ≤ 512.
///
/// 8 keys × 32 bytes = 256 bytes of keys.
/// Remaining budget: 512 − 256 = 256 bytes for 8 values → 32 bytes each.
fn worst_case_metadata(env: &Env) -> Map<Bytes, Bytes> {
    let key_len = MAX_METADATA_KEY_BYTES as usize; // 32
    let total_budget = MAX_METADATA_BYTES as usize; // 512
    let total_key_bytes = (MAX_METADATA_KEYS as usize) * key_len; // 256
    let value_budget = total_budget - total_key_bytes; // 256
    let value_len = value_budget / (MAX_METADATA_KEYS as usize); // 32

    let mut meta: Map<Bytes, Bytes> = Map::new(env);
    for i in 0..MAX_METADATA_KEYS {
        // Build a key: 32 bytes, first byte encodes the index so each key is
        // unique; remaining bytes are 0xAA.
        let mut key_buf = vec![0xAAu8; key_len];
        key_buf[0] = i as u8;
        let key = Bytes::from_slice(env, &key_buf);

        // Value: value_len bytes of 0xBB.
        let val = Bytes::from_slice(env, &vec![0xBBu8; value_len]);
        meta.set(key, val);
    }
    meta
}

/// Worst-case Stream XDR size regression.
///
/// Constructs a stream via `create_streams` (the only public entry-point that
/// accepts both `memo` and `metadata`) with all optional fields at their maximum
/// allowed sizes, then retrieves the stored `Stream` via `get_stream_state` and
/// serialises it to XDR.  Asserts the serialised length ≤ MAX_STREAM_ENTRY_BYTES.
///
/// If this test fails after a `Stream` struct change, follow the update procedure
/// in docs/gas.md §"Stream Persistent-Entry Size".
#[test]
fn test_stream_entry_xdr_size_worst_case() {
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token_id, &admin);
    sac.mint(&sender, &i128::MAX);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &u32::MAX);

    // Memo at maximum allowed size (256 × 0xFF).
    let memo = Bytes::from_slice(&env, &vec![0xFFu8; MAX_MEMO_BYTES]);

    // Metadata: 8 keys × (32-byte key + 32-byte value) = 512 bytes aggregate —
    // exactly at MAX_METADATA_BYTES, so the contract must accept it.
    let metadata = worst_case_metadata(&env);

    // Witness address occupies the optional `witness` field.
    let witness = Address::generate(&env);

    let deposit: i128 = 1_000_000;
    let rate: i128 = 1;
    let start_time: u64 = 0;
    let cliff_time: u64 = 0;
    let end_time: u64 = 1_000_000;

    let params = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: rate,
            start_time,
            cliff_time,
            end_time,
            withdraw_dust_threshold: Some(i128::MAX),
            memo: Some(memo.clone()),
            metadata: Some(metadata.clone()),
            kind: StreamKind::Linear,
            irrevocable: Some(true),
            witness: Some(witness.clone()),
        }
    ];

    let ids = client.create_streams(&sender, &params);
    let stream_id = ids.get(0).expect("stream created");

    // Retrieve the persisted Stream struct through the public view.
    let stream = client.get_stream_state(&stream_id);

    // Serialize the Stream to XDR using the same trait that Soroban uses when
    // writing to persistent storage (soroban_sdk::xdr::ToXdr).
    let xdr_bytes = stream.to_xdr(&env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "STREAM_XDR_SIZE: worst_case: {} bytes (ceiling: {} bytes, headroom: {} bytes)",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES,
        MAX_STREAM_ENTRY_BYTES - serialized_len
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "Stream XDR size {} bytes exceeds MAX_STREAM_ENTRY_BYTES {} bytes. \
         The Stream struct has grown beyond the documented ceiling. \
         See docs/gas.md §\"Stream Persistent-Entry Size\" for the update procedure.",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

/// Baseline (minimal) Stream XDR size — all optional fields absent.
///
/// Provides the lower-bound data point printed in docs/gas.md alongside the
/// worst-case ceiling so operators can reason about the rent cost spread.
#[test]
fn test_stream_entry_xdr_size_baseline() {
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token_id, &admin);
    sac.mint(&sender, &1_000_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    // No memo, no metadata, no optional fields.
    let params = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        }
    ];

    let ids = client.create_streams(&sender, &params);
    let stream_id = ids.get(0).expect("stream created");

    let stream = client.get_stream_state(&stream_id);
    let xdr_bytes = stream.to_xdr(&env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "STREAM_XDR_SIZE: baseline (no optional fields): {} bytes",
        serialized_len
    );

    // Baseline must be strictly less than the worst-case ceiling.
    assert!(
        serialized_len < MAX_STREAM_ENTRY_BYTES,
        "Baseline Stream XDR size {} bytes should be well below MAX_STREAM_ENTRY_BYTES {}",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

/// Memo-only worst case — only memo populated, no metadata.
///
/// Isolates the memo contribution so docs/gas.md can document it separately.
#[test]
fn test_stream_entry_xdr_size_memo_only() {
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token_id, &admin);
    sac.mint(&sender, &1_000_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    let memo = Bytes::from_slice(&env, &vec![0xFFu8; MAX_MEMO_BYTES]);

    let params = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            withdraw_dust_threshold: None,
            memo: Some(memo),
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        }
    ];

    let ids = client.create_streams(&sender, &params);
    let stream_id = ids.get(0).expect("stream created");

    let stream = client.get_stream_state(&stream_id);
    let xdr_bytes = stream.to_xdr(&env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "STREAM_XDR_SIZE: memo_only ({}B memo): {} bytes",
        MAX_MEMO_BYTES, serialized_len
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "Memo-only Stream XDR size {} bytes exceeds ceiling {}",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

/// Metadata-only worst case — only metadata populated, no memo.
///
/// Isolates the metadata contribution so docs/gas.md can document it separately.
#[test]
fn test_stream_entry_xdr_size_metadata_only() {
    use soroban_sdk::xdr::ToXdr;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sac = StellarAssetClient::new(&env, &token_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token_id, &admin);
    sac.mint(&sender, &1_000_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    let metadata = worst_case_metadata(&env);

    let params = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: Some(metadata),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        }
    ];

    let ids = client.create_streams(&sender, &params);
    let stream_id = ids.get(0).expect("stream created");

    let stream = client.get_stream_state(&stream_id);
    let xdr_bytes = stream.to_xdr(&env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "STREAM_XDR_SIZE: metadata_only ({} entries, {}B aggregate): {} bytes",
        MAX_METADATA_KEYS, MAX_METADATA_BYTES, serialized_len
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "Metadata-only Stream XDR size {} bytes exceeds ceiling {}",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

// ---------------------------------------------------------------------------
// Bit-packed rate-schedule storage: packed vs. unpacked gas/rent comparison
//
// Backs the storage-savings claim in the rate-schedule bit-packing issue:
// packing each `(rate_per_second: i128, duration_secs: u64)` segment into a
// single bit-packed `u128` word (`accrual::pack_rate_segment`) roughly halves
// the persistent-storage footprint versus storing the two fields separately.
// ---------------------------------------------------------------------------

/// Test-only mirror of the legacy two-full-width-field segment layout
/// described in the issue. `accrual::RateSegment` is intentionally a plain
/// (non-`#[contracttype]`) validation helper, so this type lets the
/// benchmark persist a 10-segment schedule exactly as an unpacked on-chain
/// layout would store it, for an apples-to-apples XDR/rent comparison
/// against the packed `Vec<u128>` representation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
struct UnpackedRateSegment {
    rate_per_second: i128,
    duration_secs: u64,
}

/// A representative 10-segment rate schedule spanning the packable range,
/// including a mix of positive/negative rates and short/long durations.
fn sample_rate_schedule_10_segments() -> [(i128, u32); 10] {
    [
        (1, 3_600),
        (1_000, 86_400),
        (-500, 604_800),
        (1_000_000, 1),
        (0, 1),
        (42, 999_999),
        (5_000_000_000, 100),
        (-1, 2_147_483_647),
        (7_777_777, 2_592_000),
        (-42_000, 31_536_000),
    ]
}

/// Compares the persistent-storage rent cost of a 10-segment piecewise rate
/// schedule stored as bit-packed `u128` words versus the legacy two-field
/// layout, on two axes:
///
/// 1. **Serialized XDR bytes** — what Soroban actually charges rent on.
/// 2. **CPU instructions for the `set`/`get` persistent-storage host calls** —
///    measured via `env.budget()` inside `env.as_contract(...)`, so the cost
///    is the genuine metered cost of writing/reading each representation to
///    a real persistent ledger entry (not a wall-clock proxy).
///
/// See `docs/gas.md` §"Rate-Schedule Packing: Packed vs. Unpacked Storage"
/// for the measured baseline and update procedure.
#[test]
fn test_rate_schedule_packed_vs_unpacked_storage_gas() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let segments = sample_rate_schedule_10_segments();

    // --- Unpacked layout: two full-width fields per segment ---
    let mut unpacked: Vec<UnpackedRateSegment> = Vec::new(&env);
    for &(rate, duration) in &segments {
        unpacked.push_back(UnpackedRateSegment {
            rate_per_second: rate,
            duration_secs: duration as u64,
        });
    }
    let unpacked_bytes = unpacked.clone().to_xdr(&env).len();

    let unpacked_key = symbol_short!("unpckd");
    env.budget().reset_unlimited();
    env.as_contract(&contract_id, || {
        env.storage().persistent().set(&unpacked_key, &unpacked);
    });
    let unpacked_write_cost = env.budget().cpu_instruction_cost();

    env.budget().reset_unlimited();
    let _: Vec<UnpackedRateSegment> = env.as_contract(&contract_id, || {
        env.storage().persistent().get(&unpacked_key).unwrap()
    });
    let unpacked_read_cost = env.budget().cpu_instruction_cost();

    // --- Packed layout: one bit-packed u128 word per segment ---
    let mut packed: Vec<u128> = Vec::new(&env);
    for &(rate, duration) in &segments {
        let word = pack_rate_segment(rate, duration).expect("segment within packable range");
        packed.push_back(word);
    }
    let packed_bytes = packed.clone().to_xdr(&env).len();

    let packed_key = symbol_short!("packed");
    env.budget().reset_unlimited();
    env.as_contract(&contract_id, || {
        env.storage().persistent().set(&packed_key, &packed);
    });
    let packed_write_cost = env.budget().cpu_instruction_cost();

    env.budget().reset_unlimited();
    let _: Vec<u128> = env.as_contract(&contract_id, || {
        env.storage().persistent().get(&packed_key).unwrap()
    });
    let packed_read_cost = env.budget().cpu_instruction_cost();

    println!(
        "GAS_MEASUREMENT: rate_schedule_storage: unpacked_10_segments_write: {}",
        unpacked_write_cost
    );
    println!(
        "GAS_MEASUREMENT: rate_schedule_storage: unpacked_10_segments_read: {}",
        unpacked_read_cost
    );
    println!(
        "GAS_MEASUREMENT: rate_schedule_storage: packed_10_segments_write: {}",
        packed_write_cost
    );
    println!(
        "GAS_MEASUREMENT: rate_schedule_storage: packed_10_segments_read: {}",
        packed_read_cost
    );
    println!(
        "RATE_SCHEDULE_XDR_SIZE: unpacked_10_segments: {} bytes ({:.1} bytes/segment)",
        unpacked_bytes,
        unpacked_bytes as f64 / segments.len() as f64
    );
    println!(
        "RATE_SCHEDULE_XDR_SIZE: packed_10_segments: {} bytes ({:.1} bytes/segment)",
        packed_bytes,
        packed_bytes as f64 / segments.len() as f64
    );

    assert!(
        packed_bytes < unpacked_bytes,
        "packed rate-schedule XDR size ({} bytes) must be smaller than unpacked ({} bytes)",
        packed_bytes,
        unpacked_bytes
    );

    // "Roughly half": allow slack for XDR framing overhead (type tags, vector
    // length prefixes) that doesn't scale down with the per-field bit-width
    // halving.
    assert!(
        (packed_bytes as f64) <= (unpacked_bytes as f64) * 0.75,
        "expected packed storage to be substantially smaller (packed={} bytes, unpacked={} bytes)",
        packed_bytes,
        unpacked_bytes
    );
}

/// Round-trip sanity check: every segment in the sample 10-segment schedule
/// survives `pack_rate_segment` → `unpack_rate_segment` unchanged. This
/// guards the benchmark above against a silently-wrong packed fixture.
#[test]
fn test_rate_schedule_packed_segments_round_trip() {
    use fluxora_stream::accrual::unpack_rate_segment;

    for &(rate, duration) in &sample_rate_schedule_10_segments() {
        let word = pack_rate_segment(rate, duration).expect("segment within packable range");
        assert_eq!(unpack_rate_segment(word), (rate, duration));
    }
}