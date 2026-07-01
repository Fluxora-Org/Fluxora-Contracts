//! Consolidated property-based harness for balance conservation and accrual invariants.
//!
//! This module exercises randomized sequences of mutating operations on both `Linear`
//! and `CliffOnly` streams and asserts the protocol's core financial-safety invariants:
//!
//! 1. **Global balance conservation** — the sum of tokens held by the sender, recipient,
//!    and the contract is constant (no tokens are created or destroyed).
//! 2. **Contract solvency** — the contract balance equals `total_deposited - total_withdrawn
//!    - total_refunded`, so the contract always has exactly enough to cover its obligations.
//! 3. **Accrual boundedness** — `0 <= calculate_accrued <= deposit_amount`.
//! 4. **Accrual monotonicity** — for non-decreasing time, `calculate_accrued` never decreases.
//! 5. **Withdrawal bound** — `0 <= withdrawn_amount <= deposit_amount` and `accrued >= withdrawn`.
//! 6. **Rate-decrease entitlement preservation** — a successful `decrease_rate_per_second`
//!    checkpoint locks in the pre-decrease accrued amount; the same-timestamp withdrawable
//!    value never decreases.
//! 7. **CliffOnly unsupported-operation guard** — `top_up_stream`, `decrease_rate_per_second`,
//!    `update_rate_per_second`, `shorten_stream_end_time`, and `extend_stream_end_time` all
//!    return `ContractError::UnsupportedStreamKind` for `CliffOnly` streams.
//!
//! Run the harness with:
//!
//! ```bash
//! cargo test -p fluxora_stream --features testutils --test balance_conservation
//! ```
//!
//! For deeper local coverage before an audit or release:
//!
//! ```bash
//! PROPTEST_CASES=10000 cargo test -p fluxora_stream --features testutils --test balance_conservation
//! ```
//!
//! # Security notes
//!
//! Balance conservation and accrual monotonicity are the protocol's core financial-safety
//! invariants. A violation means either a recipient can over-withdraw or a sender can be
//! short-refunded. Property-testing the combinatorial operation-sequence space is the most
//! cost-effective way to find `i128` boundary and checkpointing bugs before an audit.

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

/// Total tokens minted into the test ecosystem (sender + recipient).  This is a
/// conservation constant: no operation in this harness should create or destroy
/// tokens, so `sender_balance + recipient_balance + contract_balance` must always
/// equal this value.
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

        // Fund both participants generously so that any generated sequence of
        // top-ups can be satisfied without extra minting.
        StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_000_000_000);
        StellarAssetClient::new(&env, &token_id).mint(&recipient, &1_000_000_000_000);

        // Approve the contract to pull arbitrary top-up amounts from the sender.
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &1_000_000u32);

        env.ledger().set_timestamp(0);

        Self {
            env,
            contract_id,
            token_id,
            sender,
            recipient,
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

    /// Create a stream pinned at `start_time = 0` with the supplied parameters.
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

/// Valid parameters for a `Linear` stream.  The returned tuple is
/// `(deposit_amount, rate_per_second, cliff_time, end_time)` with `start_time = 0`.
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

/// Valid parameters for a `CliffOnly` stream.  The returned tuple is
/// `(deposit_amount, cliff_time, end_time)`; `rate_per_second` is always `0`.
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

/// A single mutating operation in the randomized sequence.
#[derive(Clone, Debug)]
enum Op {
    Withdraw,
    TopUp(i128),
    DecreaseRate(i128),
    IncreaseRate(i128),
    Shorten(u64),
    Extend(u64),
    Pause,
    Resume,
    Cancel,
}

/// Randomized operation sequences interleaved with time jumps.
fn op_sequence() -> impl Strategy<Value = std::vec::Vec<(Op, u64)>> {
    let op = prop_oneof![
        Just(Op::Withdraw),
        (1i128..5_000i128).prop_map(Op::TopUp),
        (1i128..100i128).prop_map(Op::DecreaseRate),
        (1i128..200i128).prop_map(Op::IncreaseRate),
        (1u64..100u64).prop_map(Op::Shorten),
        (1u64..100u64).prop_map(Op::Extend),
        Just(Op::Pause),
        Just(Op::Resume),
        Just(Op::Cancel),
    ];
    prop::collection::vec((op, 0u64..100u64), 0..15)
}

// ---------------------------------------------------------------------------
// Invariant assertions
// ---------------------------------------------------------------------------

/// Assert all global and per-stream invariants after a mutating step.
///
/// Returns the current timestamp, accrued amount, and withdrawn amount so the
/// caller can keep a running history for monotonicity checks.
fn assert_invariants(
    ctx: &TestContext,
    stream_id: u64,
    total_deposited: i128,
    total_withdrawn: i128,
    total_refunded: i128,
    last_time: u64,
    last_accrued: i128,
    last_withdrawn: i128,
    label: &str,
) -> (u64, i128, i128) {
    let current_time = ctx.env.ledger().timestamp();
    let stream = ctx.client().get_stream_state(&stream_id);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    let deposit = stream.deposit_amount;
    let withdrawn = stream.withdrawn_amount;

    // Per-stream bounds.
    assert!(
        withdrawn >= 0 && withdrawn <= deposit,
        "{label}: withdrawn_amount={withdrawn} not in [0, deposit={deposit}]"
    );
    assert!(
        accrued >= 0 && accrued <= deposit,
        "{label}: calculate_accrued={accrued} not in [0, deposit={deposit}]"
    );
    assert!(
        accrued >= withdrawn,
        "{label}: accrued={accrued} < withdrawn={withdrawn}"
    );
    assert!(
        withdrawable >= 0 && withdrawable <= deposit.saturating_sub(withdrawn),
        "{label}: get_withdrawable={withdrawable} not in [0, deposit-withdrawn={}]",
        deposit.saturating_sub(withdrawn)
    );

    // Accrual monotonicity over non-decreasing time.  Status transitions such as
    // cancellation freeze accrual and completion cap it at `deposit`, both of
    // which are non-decreasing relative to the previous active value.
    if current_time >= last_time {
        assert!(
            accrued >= last_accrued,
            "{label}: accrual not monotonic: accrued={accrued} at t={current_time} < last={last_accrued} at t={last_time}"
        );
    }
    assert!(
        withdrawn >= last_withdrawn,
        "{label}: withdrawn_amount decreased from {last_withdrawn} to {withdrawn}"
    );

    // Global token conservation across sender, recipient, and contract.
    let total_outside = ctx
        .sender_balance()
        .saturating_add(ctx.recipient_balance())
        .saturating_add(ctx.contract_balance());
    assert_eq!(
        total_outside,
        INITIAL_MINT,
        "{label}: global token conservation violated: sender={} recipient={} contract={}",
        ctx.sender_balance(),
        ctx.recipient_balance(),
        ctx.contract_balance()
    );

    // Contract balance must exactly match the tracked deposits minus outflows.
    let expected_contract = total_deposited - total_withdrawn - total_refunded;
    assert_eq!(
        ctx.contract_balance(),
        expected_contract,
        "{label}: contract balance {} != expected {} (deposited={} withdrawn={} refunded={})",
        ctx.contract_balance(),
        expected_contract,
        total_deposited,
        total_withdrawn,
        total_refunded
    );

    (current_time, accrued, withdrawn)
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

    /// Randomized operation sequences on `Linear` and `CliffOnly` streams must
    /// preserve balance conservation and accrual monotonicity.
    #[test]
    fn prop_random_op_sequences_preserve_invariants(
        (deposit, rate, cliff, end, kind) in stream_params(),
        ops in op_sequence(),
    ) {
        let ctx = TestContext::new();
        let stream_id = ctx.create_stream(deposit, rate, cliff, end, kind);

        let mut total_deposited = deposit;
        let mut total_withdrawn = 0i128;
        let mut total_refunded = 0i128;
        let mut current_time = 0u64;
        let mut last_time = 0u64;
        let mut last_accrued = 0i128;
        let mut last_withdrawn = 0i128;
        let mut terminal = false;

        // Initial state.
        (last_time, last_accrued, last_withdrawn) = assert_invariants(
            &ctx, stream_id, total_deposited, total_withdrawn, total_refunded,
            last_time, last_accrued, last_withdrawn, "initial",
        );

        for (idx, (op, advance)) in ops.iter().enumerate() {
            if terminal {
                break;
            }

            current_time = current_time.saturating_add(*advance);
            ctx.env.ledger().set_timestamp(current_time);
            // Keep ledger sequence advancing so pause/resume and withdrawal
            // cooldowns are satisfied on the happy path; failed attempts due to
            // cooldown simply leave state unchanged and invariants still hold.
            ctx.env.ledger().set_sequence_number((current_time / 5 + 1).max(1) as u32);

            let stream = ctx.client().get_stream_state(&stream_id);
            let label = std::format!("step {idx} op={op:?} kind={kind:?} t={current_time}");

            match op {
                Op::Withdraw => {
                    let result = ctx.client().try_withdraw(&stream_id);
                    if let Ok(Ok(amount)) = result {
                        total_withdrawn = total_withdrawn.saturating_add(amount);
                    }
                }

                Op::TopUp(amount) => {
                    let result = ctx.client().try_top_up_stream(&stream_id, &ctx.sender, amount);
                    if stream.kind == StreamKind::CliffOnly {
                        assert!(
                            matches!(result, Err(Ok(ContractError::UnsupportedStreamKind))),
                            "{label}: CliffOnly top_up must be UnsupportedStreamKind, got {result:?}"
                        );
                    } else if let Ok(Ok(())) = result {
                        total_deposited = total_deposited.saturating_add(*amount);
                    }
                }

                Op::DecreaseRate(new_rate) => {
                    let accrued_before = ctx.client().calculate_accrued(&stream_id);
                    let withdrawable_before = ctx.client().get_withdrawable(&stream_id);
                    let deposit_before = stream.deposit_amount;
                    let sender_before = ctx.sender_balance();

                    let result = ctx.client().try_decrease_rate_per_second(&stream_id, new_rate);

                    if stream.kind == StreamKind::CliffOnly {
                        assert!(
                            matches!(result, Err(Ok(ContractError::UnsupportedStreamKind))),
                            "{label}: CliffOnly decrease_rate must be UnsupportedStreamKind, got {result:?}"
                        );
                    } else if let Ok(Ok(())) = result {
                        let stream_after = ctx.client().get_stream_state(&stream_id);
                        // Refunds are sent *to* the sender, so the sender balance increases.
                        let refund = ctx.sender_balance().saturating_sub(sender_before);
                        total_refunded = total_refunded.saturating_add(refund);

                        assert_eq!(
                            stream_after.deposit_amount,
                            deposit_before.saturating_sub(refund),
                            "{label}: deposit did not decrease by refund"
                        );
                        assert_eq!(
                            ctx.client().calculate_accrued(&stream_id),
                            accrued_before,
                            "{label}: decrease_rate changed same-timestamp accrued"
                        );
                        assert!(
                            ctx.client().get_withdrawable(&stream_id) >= withdrawable_before,
                            "{label}: decrease_rate reduced same-timestamp withdrawable"
                        );
                    }
                }

                Op::IncreaseRate(new_rate) => {
                    let result = ctx.client().try_update_rate_per_second(&stream_id, new_rate);
                    if stream.kind == StreamKind::CliffOnly {
                        assert!(
                            matches!(result, Err(Ok(ContractError::UnsupportedStreamKind))),
                            "{label}: CliffOnly increase_rate must be UnsupportedStreamKind, got {result:?}"
                        );
                    }
                    // No token flow on a successful rate increase; invariants catch any
                    // unexpected state change via the generic checks below.
                }

                Op::Shorten(delta) => {
                    let new_end = current_time.saturating_add(*delta);
                    let deposit_before = stream.deposit_amount;
                    let sender_before = ctx.sender_balance();

                    let result = ctx.client().try_shorten_stream_end_time(&stream_id, &new_end);

                    if stream.kind == StreamKind::CliffOnly {
                        assert!(
                            matches!(result, Err(Ok(ContractError::UnsupportedStreamKind))),
                            "{label}: CliffOnly shorten must be UnsupportedStreamKind, got {result:?}"
                        );
                    } else if let Ok(Ok(())) = result {
                        let stream_after = ctx.client().get_stream_state(&stream_id);
                        let refund = ctx.sender_balance().saturating_sub(sender_before);
                        total_refunded = total_refunded.saturating_add(refund);
                        assert_eq!(
                            stream_after.deposit_amount,
                            deposit_before.saturating_sub(refund),
                            "{label}: shorten deposit mismatch"
                        );
                    }
                }

                Op::Extend(delta) => {
                    let new_end = stream.end_time.saturating_add(*delta);
                    let result = ctx.client().try_extend_stream_end_time(&stream_id, &new_end);

                    if stream.kind == StreamKind::CliffOnly {
                        assert!(
                            matches!(result, Err(Ok(ContractError::UnsupportedStreamKind))),
                            "{label}: CliffOnly extend must be UnsupportedStreamKind, got {result:?}"
                        );
                    }
                    // No token flow on a successful extend.
                }

                Op::Pause => {
                    let _ = ctx.client().try_pause_stream(&stream_id, &PauseReason::Operational);
                }

                Op::Resume => {
                    let _ = ctx.client().try_resume_stream(&stream_id);
                }

                Op::Cancel => {
                    let sender_before = ctx.sender_balance();
                    let result = ctx.client().try_cancel_stream(&stream_id);
                    if let Ok(Ok(())) = result {
                        // Refund is sent *to* the sender, increasing its balance.
                        total_refunded = total_refunded.saturating_add(
                            ctx.sender_balance().saturating_sub(sender_before),
                        );
                        terminal = true;
                    }
                }
            }

            (last_time, last_accrued, last_withdrawn) = assert_invariants(
                &ctx, stream_id, total_deposited, total_withdrawn, total_refunded,
                last_time, last_accrued, last_withdrawn, &label,
            );

            // Stop once the stream reaches a terminal state; any further mutating
            // operations would be rejected by the contract anyway.
            let status = ctx.client().get_stream_state(&stream_id).status;
            if status == StreamStatus::Completed || status == StreamStatus::Cancelled {
                terminal = true;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic regression tests
// ---------------------------------------------------------------------------

/// A successful rate decrease on a `Linear` stream must preserve the recipient's
/// already-accrued entitlement at the checkpoint timestamp.
#[test]
fn regression_rate_decrease_preserves_entitlement() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(1000, 10, 0, 100, StreamKind::Linear);

    ctx.env.ledger().set_timestamp(50);
    ctx.env.ledger().set_sequence_number(20);

    let accrued_before = ctx.client().calculate_accrued(&id);
    let withdrawable_before = ctx.client().get_withdrawable(&id);
    assert_eq!(accrued_before, 500); // 10 * 50

    ctx.client().decrease_rate_per_second(&id, &5);

    let stream = ctx.client().get_stream_state(&id);
    assert_eq!(stream.rate_per_second, 5);
    assert_eq!(stream.checkpointed_at, 50);
    assert_eq!(stream.checkpointed_amount, 500);
    // New deposit = 500 + 5 * 50 = 750, so refund = 250.
    assert_eq!(stream.deposit_amount, 750);

    assert_eq!(
        ctx.client().calculate_accrued(&id),
        accrued_before,
        "same-timestamp accrued must be preserved"
    );
    assert!(
        ctx.client().get_withdrawable(&id) >= withdrawable_before,
        "same-timestamp withdrawable must not decrease"
    );
}

/// `CliffOnly` streams reject all schedule/rate/top-up mutations with
/// `UnsupportedStreamKind` while still allowing creation, cliff withdrawal, and
/// cancellation.
#[test]
fn regression_cliff_only_unsupported_mutations() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(1000, 0, 50, 100, StreamKind::CliffOnly);

    // Before cliff: no accrual.
    ctx.env.ledger().set_timestamp(25);
    assert_eq!(ctx.client().calculate_accrued(&id), 0);

    // All of these must be unsupported.
    let unsupported = [
        ctx.client().try_top_up_stream(&id, &ctx.sender, &100),
        ctx.client().try_decrease_rate_per_second(&id, &1),
        ctx.client().try_update_rate_per_second(&id, &1),
        ctx.client().try_shorten_stream_end_time(&id, &75),
        ctx.client().try_extend_stream_end_time(&id, &150),
    ];
    for result in unsupported {
        assert!(
            matches!(result, Err(Ok(ContractError::UnsupportedStreamKind))),
            "CliffOnly mutation must return UnsupportedStreamKind, got {result:?}"
        );
    }

    // After cliff: full deposit is accrued and withdrawable.
    ctx.env.ledger().set_timestamp(50);
    ctx.env.ledger().set_sequence_number(100);
    assert_eq!(ctx.client().calculate_accrued(&id), 1000);
    assert_eq!(ctx.client().get_withdrawable(&id), 1000);
    let withdrawn = ctx.client().withdraw(&id);
    assert_eq!(withdrawn, 1000);
    assert_eq!(
        ctx.client().get_stream_state(&id).status,
        StreamStatus::Completed
    );
}

/// Completed streams must report a deterministic `deposit_amount` accrual
/// regardless of the timestamp passed to `calculate_accrued`.
#[test]
fn regression_completed_stream_accrual_is_deterministic() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(1000, 1, 0, 1000, StreamKind::Linear);

    ctx.env.ledger().set_timestamp(1000);
    ctx.env.ledger().set_sequence_number(1000);
    ctx.client().withdraw(&id);
    assert_eq!(
        ctx.client().get_stream_state(&id).status,
        StreamStatus::Completed
    );

    for t in [0u64, 500, 1000, 10_000, u64::MAX] {
        ctx.env.ledger().set_timestamp(t);
        assert_eq!(ctx.client().calculate_accrued(&id), 1000);
    }
}

/// Immediate cancellation (before any accrual) refunds the entire deposit to
/// the sender while preserving global balance conservation.
#[test]
fn regression_immediate_cancel_refunds_full_deposit() {
    let ctx = TestContext::new();
    let deposit = 1234i128;
    let id = ctx.create_stream(deposit, 1, 100, 500, StreamKind::Linear);

    let sender_before = ctx.sender_balance();
    let contract_before = ctx.contract_balance();

    ctx.env.ledger().set_timestamp(0);
    ctx.client().cancel_stream(&id);

    assert_eq!(ctx.sender_balance(), sender_before + deposit);
    assert_eq!(ctx.contract_balance(), contract_before - deposit);
    assert_eq!(
        ctx.sender_balance() + ctx.recipient_balance() + ctx.contract_balance(),
        INITIAL_MINT
    );
}
