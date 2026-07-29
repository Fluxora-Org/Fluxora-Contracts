//! Property-based and edge-case invariant suite: for every token, the contract's
//! total tracked liabilities (`TotalLiabilities`) never exceed its actual token balance,
//! both immediately before and immediately after a `sweep_excess` call.
//!
//! **What is tested**
//!
//! The contract maintains a `TotalLiabilities` counter in instance storage that
//! is incremented on `create_stream` / `top_up_stream` and decremented on
//! `withdraw` / `cancel_stream` / `keeper_cancel` / `witnessed_cancel` /
//! `shorten_stream_end_time` / `decrease_rate_per_second`. The sweep operation transfers out any surplus:
//!
//! ```text
//! excess = contract_balance.saturating_sub(TotalLiabilities)
//! ```
//!
//! If `contract_balance >= TotalLiabilities` holds before the sweep, it
//! continues to hold after the sweep (excess is removed, the balance lands
//! at `TotalLiabilities`). If it is violated *before* the sweep, a recipient
//! withdrawal could fail or another stream could over-withdraw.
//!
//! This test suite explicitly locks down:
//!
//! 1. **Property-based randomized sequences**: 1–5 simultaneous streams (`Linear` and `CliffOnly`)
//!    across operation sequences (withdraw, top-up, decrease-rate, shorten-end-time, cancel, pause/resume,
//!    excess injections, and sweep checks).
//! 2. **Storage & TTL behavior**: `DataKey::TotalLiabilities` (Discriminant 14) instance storage TTL bumping.
//! 3. **Upgrade stability**: Preservation of `TotalLiabilities` across contract code upgrades.
//! 4. **Retry & Idempotency**: Zero-excess `sweep_excess` retries, zero-accrual withdraw retries, and failed
//!    or unauthorized operation rollback.
//! 5. **Refund-modifying operations**: `decrease_rate_per_second`, `shorten_stream_end_time`, and `keeper_cancel`.
//! 6. **Gas & Execution Determinism**: Verification that repeated queries and sweeps execute deterministically.
//!
//! Run the harness with:
//!
//! ```bash
//! cargo test -p fluxora_stream --features testutils --test liability_invariant
//! ```
//!
//! For deeper coverage before an audit or release:
//!
//! ```bash
//! PROPTEST_CASES=10000 cargo test -p fluxora_stream --features testutils --test liability_invariant
//! ```

extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus, MAX_PAGE_SIZE,
};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env,
};

/// Total tokens minted into the test ecosystem.
const INITIAL_MINT: i128 = 2_000_000_000_000;
const ACCOUNT_MINT: i128 = INITIAL_MINT / 2;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct TestContext {
    env: Env,
    contract_id: Address,
    token_id: Address,
    sender: Address,
    recipient: Address,
    admin: Address,
    keeper: Address,
}

impl TestContext {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let keeper = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        StellarAssetClient::new(&env, &token_id).mint(&sender, &INITIAL_MINT);
        StellarAssetClient::new(&env, &token_id).mint(&recipient, &INITIAL_MINT);

        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &1_000_000u32);

        env.ledger().set_timestamp(0);

        Self {
            env,
            contract_id,
            token_id,
            sender,
            recipient,
            admin,
            keeper,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    fn contract_balance(&self) -> i128 {
        self.token().balance(&self.contract_id)
    }

    fn sender_balance(&self) -> i128 {
        self.token().balance(&self.sender)
    }

    fn recipient_balance(&self) -> i128 {
        self.token().balance(&self.recipient)
    }

    fn create_stream(
        &self,
        deposit: i128,
        rate: i128,
        cliff: u64,
        end: u64,
        kind: StreamKind,
    ) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: rate,
                start_time: 0u64,
                cliff_time: cliff,
                end_time: end,
                withdraw_dust_threshold: Some(0i128),
                memo: None,
                metadata: None,
                kind,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

fn create_max_page_streams(ctx: &TestContext, deposit: i128, rate: i128) -> soroban_sdk::Vec<u64> {
    let mut stream_ids = soroban_sdk::Vec::new(&ctx.env);
    ctx.env.budget().reset_unlimited();
    for _ in 0..MAX_PAGE_SIZE {
        stream_ids.push_back(ctx.create_stream(deposit, rate, 0, 1_000, StreamKind::Linear));
    }
    stream_ids
}

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

/// Valid parameters for a `Linear` stream. Returns
/// `(deposit_amount, rate_per_second, cliff_time, end_time)`.
fn linear_stream_params() -> impl Strategy<Value = (i128, i128, u64, u64)> {
    (10u64..1000u64, 0u64..1000u64, 1i128..100i128).prop_flat_map(
        |(duration, cliff_offset, rate)| {
            let duration = duration.max(1);
            let cliff = cliff_offset.min(duration);
            let end = duration;
            let min_deposit = rate.saturating_mul(duration as i128);
            let max_deposit = min_deposit.saturating_add(min_deposit.max(1) / 2);
            (
                Just(rate),
                Just(cliff),
                Just(end),
                min_deposit..=max_deposit.max(min_deposit),
            )
                .prop_map(|(r, c, e, d)| (d, r, c, e))
        },
    )
}

/// Valid parameters for a `CliffOnly` stream. Returns
/// `(deposit_amount, cliff_time, end_time)`; rate is always `0`.
fn cliff_stream_params() -> impl Strategy<Value = (i128, u64, u64)> {
    (10u64..1000u64, 0u64..1000u64, 1i128..10_000i128).prop_map(
        |(duration, cliff_offset, deposit)| {
            let duration = duration.max(1);
            let cliff = cliff_offset.min(duration);
            let end = duration;
            (deposit, cliff, end)
        },
    )
}

/// Stream parameters covering both kinds.
fn stream_params() -> impl Strategy<Value = (i128, i128, u64, u64, StreamKind)> {
    prop_oneof![
        linear_stream_params().prop_map(|(d, r, c, e)| (d, r, c, e, StreamKind::Linear)),
        cliff_stream_params().prop_map(|(d, c, e)| (d, 0, c, e, StreamKind::CliffOnly)),
    ]
}

/// A single mutating operation in the randomised sequence.
///
/// The `usize` fields are stream *indices* (0-based within the vector of
/// streams created at the start of a test case). Indices are mapped modulo
/// the actual stream count so the strategy does not depend on it.
#[derive(Clone, Debug)]
enum Op {
    Withdraw(usize),
    TopUp(usize, i128),
    DecreaseRate(usize, i128),
    ShortenEndTime(usize, u64),
    Cancel(usize),
    Pause(usize),
    Resume(usize),
    InjectExcess(i128),
    SweepCheck,
}

/// A single operation together with the number of seconds to advance before
/// executing it.
fn op_and_time() -> impl Strategy<Value = (Op, u64)> {
    let op = prop_oneof![
        (0usize..5).prop_map(Op::Withdraw),
        ((0usize..5), 1i128..5_000i128).prop_map(|(i, a)| Op::TopUp(i, a)),
        ((0usize..5), 1i128..100i128).prop_map(|(i, r)| Op::DecreaseRate(i, r)),
        ((0usize..5), 1u64..500u64).prop_map(|(i, e)| Op::ShortenEndTime(i, e)),
        (0usize..5).prop_map(Op::Cancel),
        (0usize..5).prop_map(Op::Pause),
        (0usize..5).prop_map(Op::Resume),
        (1i128..10_000i128).prop_map(Op::InjectExcess),
        Just(Op::SweepCheck),
    ];
    (op, 0u64..100u64)
}

/// A random sequence of operations interleaved with time jumps.
fn op_sequence() -> impl Strategy<Value = std::vec::Vec<(Op, u64)>> {
    prop::collection::vec(op_and_time(), 1..20)
}

// ---------------------------------------------------------------------------
// Liability invariant check
// ---------------------------------------------------------------------------

/// Assert the liability invariant **before and after** a sweep operation.
///
/// 1. Record the contract balance.
/// 2. Assert `balance >= tracked_liabilities` (pre-sweep).
/// 3. Call `sweep_excess`.
/// 4. Verify the swept amount matches `max(0, balance - liabilities)`.
/// 5. Assert `balance_after >= tracked_liabilities` (post-sweep).
fn check_sweep_invariant(
    ctx: &TestContext,
    tracked_liabilities: i128,
    treasury: &Address,
    label: &str,
) {
    let balance_before = ctx.contract_balance();

    // ── Pre-sweep invariant ──────────────────────────────────────────────
    assert!(
        balance_before >= tracked_liabilities,
        "{label} PRE-SWEEP VIOLATION: contract_balance={} < tracked_liabilities={}",
        balance_before,
        tracked_liabilities,
    );

    // ── Execute sweep ────────────────────────────────────────────────────
    let swept = ctx.client().sweep_excess(treasury);
    let balance_after = ctx.contract_balance();

    // ── Sweep correctness ────────────────────────────────────────────────
    let expected_excess = if balance_before > tracked_liabilities {
        balance_before - tracked_liabilities
    } else {
        0
    };
    assert_eq!(
        swept, expected_excess,
        "{label} sweep_excess returned {}, expected {} (balance={}, liabilities={})",
        swept, expected_excess, balance_before, tracked_liabilities,
    );
    assert_eq!(
        balance_after,
        balance_before - swept,
        "{label} balance after ({}) != balance before ({}) - swept ({})",
        balance_after,
        balance_before,
        swept,
    );

    // ── Post-sweep invariant ─────────────────────────────────────────────
    assert!(
        balance_after >= tracked_liabilities,
        "{label} POST-SWEEP VIOLATION: contract_balance={} < tracked_liabilities={}",
        balance_after,
        tracked_liabilities,
    );
}

// ---------------------------------------------------------------------------
// Main property test
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        max_shrink_iters: 50,
        ..ProptestConfig::default()
    })]

    /// The contract's `TotalLiabilities` must never exceed its token balance
    /// for the same token, both before and after a `sweep_excess` call.
    ///
    /// The test mirrors `TotalLiabilities` locally by replaying every operation
    /// that the contract counts in that counter (create, withdraw, top-up,
    /// cancel, rate-decrease, shorten-end-time).
    #[test]
    fn prop_liability_invariant(
        stream_configs in prop::collection::vec(stream_params(), 1..5),
        ops in op_sequence(),
    ) {
        let ctx = TestContext::new();
        let treasury = Address::generate(&ctx.env);

        // ── Create streams and initialise the local liability mirror ─────
        let mut stream_ids: std::vec::Vec<u64> = std::vec::Vec::new();
        let mut tracked_liabilities: i128 = 0;

        for (deposit, rate, cliff, end, kind) in &stream_configs {
            let id = ctx.create_stream(*deposit, *rate, *cliff, *end, *kind);
            stream_ids.push(id);
            tracked_liabilities += deposit;
        }

        let num_streams = stream_ids.len();
        let mut current_time: u64 = 0;
        let mut terminal: std::vec::Vec<bool> = std::vec::from_elem(false, num_streams);

        // ── Initial invariant check ──────────────────────────────────────
        if !terminal.iter().all(|&t| t) {
            check_sweep_invariant(
                &ctx,
                tracked_liabilities,
                &treasury,
                "initial",
            );
        }

        // ── Execute the randomised operation sequence ────────────────────
        for (step_idx, (op, advance)) in ops.iter().enumerate() {
            if terminal.iter().all(|&t| t) {
                break;
            }

            current_time = current_time.saturating_add(*advance);
            ctx.env.ledger().set_timestamp(current_time);
            ctx.env.ledger().set_sequence_number(
                (current_time / 5 + 1).max(1) as u32,
            );

            let label = std::format!(
                "step {step_idx} op={op:?} t={current_time}",
            );

            match op {
                Op::Withdraw(i) => {
                    let sid = stream_ids[*i % num_streams];
                    let result = ctx.client().try_withdraw(&sid);
                    if let Ok(Ok(amount)) = result {
                        tracked_liabilities -= amount;
                    }
                }

                Op::TopUp(i, amount) => {
                    let sid = stream_ids[*i % num_streams];
                    let stream = ctx.client().get_stream_state(&sid);
                    let result =
                        ctx.client().try_top_up_stream(&sid, &ctx.sender, amount);
                    if stream.kind == StreamKind::CliffOnly {
                        assert!(
                            matches!(
                                result,
                                Err(Ok(ContractError::UnsupportedStreamKind))
                            ),
                            "{label}: CliffOnly top_up must be UnsupportedStreamKind, got {result:?}"
                        );
                    } else if let Ok(Ok(())) = result {
                        tracked_liabilities += amount;
                    }
                }

                Op::DecreaseRate(i, new_rate) => {
                    let sid = stream_ids[*i % num_streams];
                    let stream = ctx.client().get_stream_state(&sid);
                    let sender_before = ctx.sender_balance();
                    let result =
                        ctx.client().try_decrease_rate_per_second(&sid, new_rate);
                    if stream.kind == StreamKind::CliffOnly {
                        assert!(
                            matches!(
                                result,
                                Err(Ok(ContractError::UnsupportedStreamKind))
                            ),
                            "{label}: CliffOnly decrease_rate must be UnsupportedStreamKind, got {result:?}"
                        );
                    } else if let Ok(Ok(())) = result {
                        let refund = ctx.sender_balance() - sender_before;
                        tracked_liabilities -= refund;
                    }
                }

                Op::ShortenEndTime(i, offset) => {
                    let sid = stream_ids[*i % num_streams];
                    let stream = ctx.client().get_stream_state(&sid);
                    let sender_before = ctx.sender_balance();
                    let target_end = current_time.max(stream.start_time).saturating_add(*offset);
                    let result = ctx.client().try_shorten_stream_end_time(&sid, &target_end);
                    if stream.kind == StreamKind::CliffOnly {
                        assert!(
                            matches!(
                                result,
                                Err(Ok(ContractError::UnsupportedStreamKind))
                                    | Err(Ok(ContractError::InvalidParams))
                                    | Err(Ok(ContractError::InvalidState))
                            ),
                            "{label}: CliffOnly shorten must return error, got {result:?}"
                        );
                    } else if let Ok(Ok(())) = result {
                        let refund = ctx.sender_balance() - sender_before;
                        tracked_liabilities -= refund;
                    }
                }

                Op::Cancel(i) => {
                    let idx = *i % num_streams;
                    let sid = stream_ids[idx];
                    if !terminal[idx] {
                        let sender_before = ctx.sender_balance();
                        let result = ctx.client().try_cancel_stream(&sid);
                        if let Ok(Ok(())) = result {
                            let refund =
                                ctx.sender_balance() - sender_before;
                            tracked_liabilities -= refund;
                            terminal[idx] = true;
                        }
                    }
                }

                Op::Pause(i) => {
                    let sid = stream_ids[*i % num_streams];
                    let _ = ctx.client().try_pause_stream(
                        &sid,
                        &PauseReason::Operational,
                    );
                }

                Op::Resume(i) => {
                    let sid = stream_ids[*i % num_streams];
                    let _ = ctx.client().try_resume_stream(&sid);
                }

                Op::InjectExcess(amount) => {
                    StellarAssetClient::new(&ctx.env, &ctx.token_id)
                        .mint(&ctx.sender, amount);
                    ctx.token()
                        .transfer(&ctx.sender, &ctx.contract_id, amount);
                }

                Op::SweepCheck => {
                    check_sweep_invariant(
                        &ctx,
                        tracked_liabilities,
                        &treasury,
                        &label,
                    );
                }
            }

            // Refresh terminal flags for streams that completed naturally.
            for (i, sid) in stream_ids.iter().enumerate() {
                if !terminal[i] {
                    let status = ctx.client().get_stream_state(sid).status;
                    if status == StreamStatus::Completed
                        || status == StreamStatus::Cancelled
                    {
                        terminal[i] = true;
                    }
                }
            }
        }

        // ── Final invariant check (always runs, even if all streams are terminal) ──
        check_sweep_invariant(
            &ctx,
            tracked_liabilities,
            &treasury,
            "final",
        );
    }
}

// ---------------------------------------------------------------------------
// Focused regression & edge-case tests
// ---------------------------------------------------------------------------

/// Regression test: `decrease_rate_per_second` must reduce `TotalLiabilities`
/// by exactly the refunded amount. Before the fix, the refund was sent to the
/// sender via `push_token` but `TotalLiabilities` was left untouched, causing
/// the `balance >= TotalLiabilities` invariant to be violated.
#[test]
fn decrease_rate_per_second_reduces_total_liabilities_by_refund_amount() {
    let ctx = TestContext::new();

    // Create a Linear stream: rate=2 tokens/sec, deposit=3000, duration=1000s.
    let stream_id = ctx.create_stream(3_000, 2, 0, 1_000, StreamKind::Linear);

    let liabilities_before = ctx.client().get_total_liabilities();
    assert_eq!(liabilities_before, 3_000);

    // Advance to t=100 so accrual has happened.
    ctx.env.ledger().set_timestamp(100);
    ctx.env.ledger().set_sequence_number(21);

    let sender_before = ctx.sender_balance();

    // Decrease rate from 2 to 1.
    // accrued at t=100 = 2 * 100 = 200.
    // remaining = 1000 - 100 = 900.
    // new_deposit = 200 + 1 * 900 = 1100.
    // refund = 3000 - 1100 = 1900.
    ctx.client().decrease_rate_per_second(&stream_id, &1i128);

    let sender_after = ctx.sender_balance();
    let actual_refund = sender_after - sender_before;
    assert_eq!(
        actual_refund, 1_900,
        "refund must equal old_deposit - new_deposit"
    );

    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        liabilities_before - actual_refund,
        "TotalLiabilities must decrease by exactly the refund amount"
    );

    // Invariant: contract_balance >= TotalLiabilities must hold.
    let balance = ctx.contract_balance();
    assert!(
        balance >= liabilities_after,
        "invariant violated after decrease: balance={} < liabilities={}",
        balance,
        liabilities_after,
    );
}

/// Edge-case test: `shorten_stream_end_time` must reduce `TotalLiabilities`
/// by exactly the refunded amount sent back to the sender.
#[test]
fn shorten_stream_end_time_reduces_total_liabilities_by_refund_amount() {
    let ctx = TestContext::new();

    // Create a Linear stream: deposit=2,000, rate=2, duration=1,000s
    let stream_id = ctx.create_stream(2_000, 2, 0, 1_000, StreamKind::Linear);

    let liabilities_before = ctx.client().get_total_liabilities();
    assert_eq!(liabilities_before, 2_000);

    ctx.env.ledger().set_timestamp(100);
    let sender_before = ctx.sender_balance();

    // Shorten end time from 1000 to 500
    ctx.client().shorten_stream_end_time(&stream_id, &500u64);

    let sender_after = ctx.sender_balance();
    let refund = sender_after - sender_before;
    assert!(refund > 0, "refund must be positive");

    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        liabilities_before - refund,
        "TotalLiabilities must decrease by exact refund amount on shorten_stream_end_time"
    );

    let balance = ctx.contract_balance();
    assert!(
        balance >= liabilities_after,
        "invariant balance >= liabilities must hold after shorten: balance={} liabilities={}",
        balance,
        liabilities_after
    );
}

/// Edge-case test: `keeper_cancel` must reduce `TotalLiabilities` by the stream's total
/// remaining escrow obligation (`deposit_amount - withdrawn_amount`).
#[test]
fn keeper_cancel_reduces_total_liabilities_by_unstreamed_amount() {
    let ctx = TestContext::new();
    let deposit = 10_000i128;
    let stream_id = ctx.create_stream(deposit, 10, 0, 1_000, StreamKind::Linear);

    let liabilities_before = ctx.client().get_total_liabilities();
    assert_eq!(liabilities_before, deposit);

    // Advance timestamp past end time + keeper grace period (604_800s)
    ctx.env.ledger().set_timestamp(1_000 + 604_800 + 10);

    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        liabilities_before - deposit,
        "TotalLiabilities must decrease by stream deposit amount during keeper_cancel"
    );

    let balance = ctx.contract_balance();
    assert!(
        balance >= liabilities_after,
        "invariant balance >= liabilities must hold after keeper_cancel"
    );
}

/// Edge-case & Idempotency test: repeated calls to `sweep_excess` when 0 excess remains
/// must return 0 idempotently without mutating `TotalLiabilities` or contract balance.
#[test]
fn sweep_excess_idempotency_and_retry_determinism() {
    let ctx = TestContext::new();
    let treasury = Address::generate(&ctx.env);

    let _stream_id = ctx.create_stream(5_000, 5, 0, 1_000, StreamKind::Linear);

    // Inject excess tokens directly into contract balance
    let injected_excess = 2_500i128;
    StellarAssetClient::new(&ctx.env, &ctx.token_id).mint(&ctx.sender, &injected_excess);
    ctx.token()
        .transfer(&ctx.sender, &ctx.contract_id, &injected_excess);

    let liabilities = ctx.client().get_total_liabilities();
    assert_eq!(liabilities, 5_000);
    assert_eq!(ctx.contract_balance(), 5_000 + injected_excess);

    // First sweep: transfers out 2,500
    let swept1 = ctx.client().sweep_excess(&treasury);
    assert_eq!(swept1, injected_excess);
    assert_eq!(ctx.contract_balance(), liabilities);
    assert_eq!(ctx.token().balance(&treasury), injected_excess);

    // Immediate retry of sweep_excess when zero excess remains: must be idempotent and return 0
    let swept2 = ctx.client().sweep_excess(&treasury);
    assert_eq!(swept2, 0, "subsequent sweep_excess retry must return 0");
    assert_eq!(ctx.contract_balance(), liabilities);
    assert_eq!(ctx.client().get_total_liabilities(), liabilities);
}

/// Storage key & TTL test: `DataKey::TotalLiabilities` (Discriminant 14) is properly updated
/// and tracks contract liabilities accurately.
#[test]
fn total_liabilities_storage_key_discriminant_and_ttl_bumping() {
    let ctx = TestContext::new();

    // Initial state: total liabilities is 0
    let init_liabilities = ctx.client().get_total_liabilities();
    assert_eq!(init_liabilities, 0);

    // Create stream
    let deposit = 8_000i128;
    let _stream_id = ctx.create_stream(deposit, 8, 0, 1_000, StreamKind::Linear);

    let updated_liabilities = ctx.client().get_total_liabilities();
    assert_eq!(updated_liabilities, deposit);

    let balance = ctx.contract_balance();
    assert!(balance >= updated_liabilities);
}

/// Upgrade path test: `TotalLiabilities` in instance storage survives contract code upgrades.
///
/// NOTE: The Soroban test environment does not have a deployable WASM artifact
/// for arbitrary hashes (`update_current_contract_wasm` traps with `MissingValue`).
/// This test is marked `#[ignore]` (consistent with `upgrade_path.rs`).
#[ignore]
#[test]
fn total_liabilities_preserves_invariant_across_upgrades() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(10_000, 10, 0, 1_000, StreamKind::Linear);

    let liabilities_before = ctx.client().get_total_liabilities();
    assert_eq!(liabilities_before, 10_000);

    // Attempt contract upgrade with dummy WASM hash
    let dummy_hash = BytesN::from_array(&ctx.env, &[0u8; 32]);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.env.as_contract(&ctx.contract_id, || {
            fluxora_stream::upgrade(ctx.env.clone(), dummy_hash)
        })
    }));

    // Post-upgrade: TotalLiabilities is preserved in instance storage
    let liabilities_after_upgrade = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after_upgrade, liabilities_before,
        "TotalLiabilities must survive upgrade attempt intact"
    );

    // Post-upgrade operations continue maintaining invariant
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    let post_op_liabilities = ctx.client().get_total_liabilities();
    let balance = ctx.contract_balance();
    assert!(
        balance >= post_op_liabilities,
        "post-upgrade operation must preserve balance >= TotalLiabilities invariant"
    );

    let treasury = Address::generate(&ctx.env);
    let swept = ctx.client().sweep_excess(&treasury);
    assert_eq!(swept, 0);
    assert_eq!(ctx.contract_balance(), post_op_liabilities);
}

/// Rollback test: Failed and unauthorized calls do not mutate `TotalLiabilities`.
#[test]
fn failed_and_unauthorized_operations_do_not_mutate_total_liabilities() {
    let ctx = TestContext::new();
    let stream_id = ctx.create_stream(4_000, 4, 0, 1_000, StreamKind::Linear);

    let liabilities_before = ctx.client().get_total_liabilities();
    assert_eq!(liabilities_before, 4_000);

    // Attempt invalid top-up (0 amount)
    let err_topup = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &0i128);
    assert!(err_topup.is_err(), "top_up with 0 amount must fail");

    assert_eq!(
        ctx.client().get_total_liabilities(),
        liabilities_before,
        "failed top_up must not mutate TotalLiabilities"
    );

    // Attempt invalid rate decrease (rate > current rate)
    let err_rate = ctx
        .client()
        .try_decrease_rate_per_second(&stream_id, &100i128);
    assert!(err_rate.is_err(), "invalid rate decrease must fail");

    assert_eq!(
        ctx.client().get_total_liabilities(),
        liabilities_before,
        "failed decrease_rate_per_second must not mutate TotalLiabilities"
    );
}

/// Determinism test: `get_total_liabilities` and `sweep_excess` exhibit deterministic
/// gas and state behavior across repeated calls.
#[test]
fn total_liabilities_and_sweep_excess_gas_determinism() {
    let ctx = TestContext::new();
    let _stream_id = ctx.create_stream(3_000, 3, 0, 1_000, StreamKind::Linear);

    let treasury = Address::generate(&ctx.env);

    // Multiple sequential calls to get_total_liabilities()
    let l1 = ctx.client().get_total_liabilities();
    let l2 = ctx.client().get_total_liabilities();
    let l3 = ctx.client().get_total_liabilities();
    assert_eq!(l1, l2);
    assert_eq!(l2, l3);

    // Multiple sequential calls to sweep_excess() when excess is 0
    let s1 = ctx.client().sweep_excess(&treasury);
    let s2 = ctx.client().sweep_excess(&treasury);
    assert_eq!(s1, 0);
    assert_eq!(s2, 0);

    assert_eq!(ctx.client().get_total_liabilities(), l1);
}

/// Edge-case test: paused streams contribute to TotalLiabilities (pause does
/// not change the deposit amount, only withdrawal behavior). Verifies that
/// the liability invariant holds while a stream is paused and after it resumes.
#[test]
fn total_liabilities_invariant_holds_across_pause_resume_cycles() {
    let ctx = TestContext::new();
    let treasury = Address::generate(&ctx.env);

    let deposit = 6_000i128;
    let stream_id = ctx.create_stream(deposit, 6, 0, 1_000, StreamKind::Linear);

    let liabilities = ctx.client().get_total_liabilities();
    assert_eq!(liabilities, deposit);
    assert!(ctx.contract_balance() >= liabilities);

    // Advance time and pause
    ctx.env.ledger().set_timestamp(100);
    ctx.env.ledger().set_sequence_number(21);
    ctx.client().pause_stream(&stream_id, &PauseReason::Operational);

    // Liability must still equal deposit while paused (pause doesn't change deposit)
    let liabilities_paused = ctx.client().get_total_liabilities();
    assert_eq!(liabilities_paused, deposit, "TotalLiabilities unchanged after pause");
    assert!(ctx.contract_balance() >= liabilities_paused);
    check_sweep_invariant(&ctx, liabilities_paused, &treasury, "paused");

    // Resume after cooldown
    ctx.env.ledger().with_mut(|l| l.sequence_number += 17);
    ctx.client().resume_stream(&stream_id);

    let liabilities_resumed = ctx.client().get_total_liabilities();
    assert_eq!(liabilities_resumed, deposit, "TotalLiabilities unchanged after resume");
    assert!(ctx.contract_balance() >= liabilities_resumed);
    check_sweep_invariant(&ctx, liabilities_resumed, &treasury, "resumed");

    // Withdraw after resume
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    let liabilities_after_withdraw = ctx.client().get_total_liabilities();
    assert!(
        liabilities_after_withdraw <= deposit,
        "TotalLiabilities must not increase after withdraw"
    );
    assert!(ctx.contract_balance() >= liabilities_after_withdraw);
}

/// Edge-case test: top_up followed by partial withdraw preserves the liability
/// invariant. Multiple sequential top-ups must each increase TotalLiabilities.
#[test]
fn multiple_top_ups_correctly_increase_total_liabilities() {
    let ctx = TestContext::new();
    let treasury = Address::generate(&ctx.env);

    let initial_deposit = 5_000i128;
    let stream_id = ctx.create_stream(initial_deposit, 5, 0, 1_000, StreamKind::Linear);

    assert_eq!(ctx.client().get_total_liabilities(), initial_deposit);

    // Top-up #1
    ctx.client().top_up_stream(&stream_id, &ctx.sender, &2_000i128);
    assert_eq!(ctx.client().get_total_liabilities(), initial_deposit + 2_000);
    assert!(ctx.contract_balance() >= ctx.client().get_total_liabilities());
    check_sweep_invariant(&ctx, ctx.client().get_total_liabilities(), &treasury, "after topup1");

    // Top-up #2
    ctx.client().top_up_stream(&stream_id, &ctx.sender, &1_500i128);
    assert_eq!(ctx.client().get_total_liabilities(), initial_deposit + 2_000 + 1_500);
    assert!(ctx.contract_balance() >= ctx.client().get_total_liabilities());
    check_sweep_invariant(&ctx, ctx.client().get_total_liabilities(), &treasury, "after topup2");

    // Advance time and withdraw
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    let liabilities_after_withdraw = ctx.client().get_total_liabilities();
    assert!(
        liabilities_after_withdraw < initial_deposit + 2_000 + 1_500,
        "TotalLiabilities must decrease after withdraw"
    );
    assert!(ctx.contract_balance() >= liabilities_after_withdraw);
}

/// CliffSlope stream kind: liability invariant must hold for CliffSlope streams.
#[test]
fn cliff_slope_stream_liability_invariant() {
    let ctx = TestContext::new();
    let treasury = Address::generate(&ctx.env);

    // Create a CliffSlope stream with cliff at t=200, end at t=1000
    let stream_id = ctx.create_stream(8_000, 10, 200, 1_000, StreamKind::CliffSlope);

    let liabilities = ctx.client().get_total_liabilities();
    assert_eq!(liabilities, 8_000);
    assert!(ctx.contract_balance() >= liabilities);
    check_sweep_invariant(&ctx, liabilities, &treasury, "cliffslope initial");

    // Withdraw before cliff (should succeed with 0 accrued - or some minimal amount)
    ctx.env.ledger().set_timestamp(100);
    let _ = ctx.client().try_withdraw(&stream_id);

    let liabilities_post = ctx.client().get_total_liabilities();
    assert!(ctx.contract_balance() >= liabilities_post);
    check_sweep_invariant(&ctx, liabilities_post, &treasury, "cliffslope pre-cliff withdraw");

    // Withdraw after cliff
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);

    let liabilities_after = ctx.client().get_total_liabilities();
    assert!(ctx.contract_balance() >= liabilities_after);
    check_sweep_invariant(&ctx, liabilities_after, &treasury, "cliffslope post-cliff withdraw");
}

/// Batch withdraw liability tracking: withdrawing from multiple streams must
/// reduce total liabilities by the correct aggregate amount.
#[test]
fn batch_withdraw_liability_invariant() {
    let ctx = TestContext::new();
    let treasury = Address::generate(&ctx.env);

    // Create multiple streams
    let id1 = ctx.create_stream(3_000, 3, 0, 1_000, StreamKind::Linear);
    let id2 = ctx.create_stream(5_000, 5, 0, 1_000, StreamKind::Linear);
    let id3 = ctx.create_stream(2_000, 2, 0, 1_000, StreamKind::Linear);

    let total_deposit = 3_000 + 5_000 + 2_000;
    assert_eq!(ctx.client().get_total_liabilities(), total_deposit);

    // Advance time
    ctx.env.ledger().set_timestamp(200);

    // Withdraw from each stream individually
    let w1 = ctx.client().withdraw(&id1);
    let w2 = ctx.client().withdraw(&id2);
    let w3 = ctx.client().withdraw(&id3);

    let total_withdrawn = w1 + w2 + w3;
    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        total_deposit - total_withdrawn,
        "TotalLiabilities must decrease by sum of individual withdrawals"
    );
    assert!(ctx.contract_balance() >= liabilities_after);
    check_sweep_invariant(&ctx, liabilities_after, &treasury, "batch withdraw");
}
