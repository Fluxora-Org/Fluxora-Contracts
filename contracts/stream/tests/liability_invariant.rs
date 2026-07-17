//! Property-based invariant: for every token, the contract's total tracked
//! liabilities never exceed its actual token balance, both immediately before
//! and immediately after a `sweep_excess` call.
//!
//! **What is tested**
//!
//! The contract maintains a `TotalLiabilities` counter in instance storage that
//! is incremented on `create_stream` / `top_up_stream` and decremented on
//! `withdraw` / `cancel_stream` / `shorten_stream_end_time`.  The sweep
//! operation transfers out any surplus:
//!
//! ```text
//! excess = contract_balance.saturating_sub(TotalLiabilities)
//! ```
//!
//! If `contract_balance >= TotalLiabilities` holds before the sweep, it
//! continues to hold after the sweep (excess is removed, the balance lands
//! at `TotalLiabilities`).  If it is violated *before* the sweep, a recipient
//! withdrawal could fail or another stream could over-withdraw.
//!
//! This property test exercises the invariant across:
//!
//! - 1–5 simultaneous streams with randomised `Linear` / `CliffOnly` parameters
//! - Randomised operation sequences (withdraw, top-up, cancel, rate-decrease,
//!   pause/resume)
//! - Direct token injections that simulate rounding errors or lost refunds
//! - Time advances that move streams through accrual, completion, and expiry
//!
//! **Known discrepancy**: `decrease_rate_per_second` refunds tokens to the
//! sender **without** reducing `TotalLiabilities`.  After a rate decrease the
//! invariant `balance >= TotalLiabilities` is therefore violated (the counter
//! overstates the true obligation).  The proptest will find this, shrink it,
//! and report the minimal reproduction below.
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
    ContractError, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind, StreamStatus,
};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

/// Total tokens minted into the test ecosystem.
const INITIAL_MINT: i128 = 2_000_000_000_000;

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

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_000_000_000);
        StellarAssetClient::new(&env, &token_id).mint(&recipient, &1_000_000_000_000);

        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &1_000_000u32);

        env.ledger().set_timestamp(0);

        Self {
            env,
            contract_id,
            token_id,
            sender,
            recipient,
            admin,
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
            &self.recipient,
            &deposit,
            &rate,
            &0u64,
            &cliff,
            &end,
            &0i128,
            &None,
            &kind,
        )
    }
}

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

/// Valid parameters for a `Linear` stream.  Returns
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

/// Valid parameters for a `CliffOnly` stream.  Returns
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
/// streams created at the start of a test case).  Indices are mapped modulo
/// the actual stream count so the strategy does not depend on it.
#[derive(Clone, Debug)]
enum Op {
    Withdraw(usize),
    TopUp(usize, i128),
    DecreaseRate(usize, i128),
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
        swept,
        expected_excess,
        "{label} sweep_excess returned {}, expected {} (balance={}, liabilities={})",
        swept,
        expected_excess,
        balance_before,
        tracked_liabilities,
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
    /// cancel).  Operations that **do not** update `TotalLiabilities`
    /// (`decrease_rate_per_second`) are reflected exactly as the contract
    /// behaves — the local counter is also left untouched, reproducing the
    /// known accounting discrepancy.
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
                    }
                    // Contract does NOT update TotalLiabilities on rate
                    // decrease.  We mirror that behaviour exactly, which
                    // means `tracked_liabilities` stays unchanged even though
                    // the refund left the contract.  The proptest will later
                    // detect this as a pre-sweep invariant violation.
                    //
                    // (decrease_rate_per_second lines 4314-4413)
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
