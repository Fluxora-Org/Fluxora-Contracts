//! Property-based tests for withdrawable arithmetic invariants.
//!
//! Proves that across every status transition (Active → Paused → Resumed →
//! Cancelled / Completed) the following invariants always hold:
//!
//! 1. **Non-negativity**:  `accrued - withdrawn_amount >= 0`
//! 2. **Deposit bound**:   `accrued <= deposit_amount`
//! 3. **Withdrawal bound**: `withdrawn_amount <= deposit_amount`
//! 4. **Withdrawable bound**: `get_withdrawable() <= deposit_amount`
//!
//! Run with: `cargo test -p fluxora_stream`
//! Deeper coverage: `PROPTEST_CASES=10000 cargo test -p fluxora_stream`

extern crate std;

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

use crate::{FluxoraStream, FluxoraStreamClient, StreamStatus};

// ---------------------------------------------------------------------------
// Minimal isolated test harness
// ---------------------------------------------------------------------------

struct PropCtx {
    env: Env,
    client_id: Address,
    sender: Address,
    recipient: Address,
}

impl PropCtx {
    fn new(deposit: i128) -> Self {
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

        FluxoraStreamClient::new(&env, &contract_id).init(&token_id, &admin);
        StellarAssetClient::new(&env, &token_id).mint(&sender, &deposit);
        soroban_sdk::token::Client::new(&env, &token_id).approve(
            &sender,
            &contract_id,
            &deposit,
            &100_000,
        );

        PropCtx {
            env,
            client_id: contract_id,
            sender,
            recipient,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.client_id)
    }
}

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

/// Generates (deposit, rate, duration) satisfying `deposit >= rate * duration`.
/// Keeps values small to avoid i128 overflow.
fn valid_stream_config() -> impl Strategy<Value = (i128, i128, u64)> {
    (1_i128..=1_000_i128, 1_u64..=1_000_u64).prop_flat_map(|(rate, duration)| {
        let min_deposit = rate * duration as i128;
        let max_deposit = min_deposit + 500;
        (Just(rate), Just(duration), min_deposit..=max_deposit).prop_map(|(r, d, dep)| (dep, r, d))
    })
}

/// Sorted sequence of up to 6 timestamps in [0, end_time + 100].
fn time_sequence(end_time: u64) -> impl Strategy<Value = std::vec::Vec<u64>> {
    proptest::collection::vec(0_u64..=(end_time + 100), 1..=6).prop_map(|mut v| {
        v.sort();
        v
    })
}

// ---------------------------------------------------------------------------
// Core invariant checker
// ---------------------------------------------------------------------------

fn assert_invariants(ctx: &PropCtx, stream_id: u64, label: &str) {
    let state = ctx.client().get_stream_state(&stream_id);
    let deposit = state.deposit_amount;
    let withdrawn = state.withdrawn_amount;

    // withdrawn_amount in [0, deposit]
    assert!(withdrawn >= 0, "{label}: withdrawn negative: {withdrawn}");
    assert!(
        withdrawn <= deposit,
        "{label}: withdrawn={withdrawn} > deposit={deposit}"
    );

    // accrued in [0, deposit]
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued >= 0, "{label}: accrued negative: {accrued}");
    assert!(
        accrued <= deposit,
        "{label}: accrued={accrued} > deposit={deposit}"
    );

    // accrued - withdrawn >= 0  (the core non-negativity invariant)
    assert!(
        accrued >= withdrawn,
        "{label}: accrued={accrued} < withdrawn={withdrawn} — would underflow"
    );

    // get_withdrawable in [0, deposit]
    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    assert!(
        withdrawable >= 0,
        "{label}: get_withdrawable negative: {withdrawable}"
    );
    assert!(
        withdrawable <= deposit,
        "{label}: get_withdrawable={withdrawable} > deposit={deposit}"
    );
}

// ---------------------------------------------------------------------------
// Proptest suite
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Invariants hold at arbitrary timestamps on an active stream.
    #[test]
    fn prop_active_stream_invariants_at_arbitrary_times(
        (deposit, rate, duration) in valid_stream_config(),
        times in time_sequence(1_000),
    ) {
        let ctx = PropCtx::new(deposit);
        ctx.env.ledger().set_timestamp(0);
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &deposit,
            &rate,
            &0u64,
            &0u64,
            &duration,
            &None,
        );
        for t in &times {
            ctx.env.ledger().set_timestamp(*t);
            assert_invariants(&ctx, id, &std::format!("active t={t}"));
        }
    }

    /// Invariants hold after each withdrawal in a sequence.
    #[test]
    fn prop_invariants_hold_after_withdrawals(
        (deposit, rate, duration) in valid_stream_config(),
        times in time_sequence(1_000),
    ) {
        let ctx = PropCtx::new(deposit);
        ctx.env.ledger().set_timestamp(0);
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &deposit,
            &rate,
            &0u64,
            &0u64,
            &duration,
            &None,
        );
        for t in &times {
            ctx.env.ledger().set_timestamp(*t);
            let _ = ctx.client().try_withdraw(&id);
            assert_invariants(&ctx, id, &std::format!("post-withdraw t={t}"));
        }
    }

    /// Invariants hold across pause/resume cycles.
    #[test]
    fn prop_invariants_hold_across_pause_resume(
        (deposit, rate, duration) in valid_stream_config(),
        times in time_sequence(1_000),
    ) {
        let ctx = PropCtx::new(deposit);
        ctx.env.ledger().set_timestamp(0);
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &deposit,
            &rate,
            &0u64,
            &0u64,
            &duration,
            &None,
        );
        let mut paused = false;
        for t in &times {
            ctx.env.ledger().set_timestamp(*t);
            let state = ctx.client().get_stream_state(&id);
            if state.status == StreamStatus::Active || state.status == StreamStatus::Paused {
                if paused {
                    let _ = ctx.client().try_resume_stream(&id);
                    paused = false;
                } else {
                    let _ = ctx.client().try_pause_stream(&id, &crate::PauseReason::Operational);
                    paused = true;
                }
            }
            assert_invariants(&ctx, id, &std::format!("pause/resume t={t}"));
        }
    }

    /// Invariants hold after cancellation at any point in time.
    #[test]
    fn prop_invariants_hold_after_cancel(
        (deposit, rate, duration) in valid_stream_config(),
        cancel_at in 0_u64..=1_100_u64,
    ) {
        let ctx = PropCtx::new(deposit);
        ctx.env.ledger().set_timestamp(0);
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &deposit,
            &rate,
            &0u64,
            &0u64,
            &duration,
            &None,
        );
        ctx.env.ledger().set_timestamp(cancel_at);
        ctx.client().cancel_stream(&id);
        assert_invariants(&ctx, id, "post-cancel");
        let _ = ctx.client().withdraw(&id);
        assert_invariants(&ctx, id, "post-cancel-withdraw");
    }

    /// withdrawn_amount is monotonically non-decreasing across sequential withdrawals.
    #[test]
    fn prop_withdrawn_amount_monotonically_increases(
        (deposit, rate, duration) in valid_stream_config(),
        times in time_sequence(1_000),
    ) {
        let ctx = PropCtx::new(deposit);
        ctx.env.ledger().set_timestamp(0);
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &deposit,
            &rate,
            &0u64,
            &0u64,
            &duration,
            &None,
        );
        let mut prev = 0_i128;
        for t in &times {
            ctx.env.ledger().set_timestamp(*t);
            let _ = ctx.client().try_withdraw(&id);
            let state = ctx.client().get_stream_state(&id);
            assert!(
                state.withdrawn_amount >= prev,
                "withdrawn_amount decreased at t={t}: {} < {prev}",
                state.withdrawn_amount
            );
            prev = state.withdrawn_amount;
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic regression tests — one per status transition path
// ---------------------------------------------------------------------------

fn setup_standard(deposit: i128) -> (PropCtx, u64) {
    let ctx = PropCtx::new(deposit);
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    (ctx, id)
}

#[test]
fn invariants_active_at_start() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(0);
    assert_invariants(&ctx, id, "active t=0");
}

#[test]
fn invariants_active_midstream() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(500);
    assert_invariants(&ctx, id, "active t=500");
}

#[test]
fn invariants_active_at_end() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(1000);
    assert_invariants(&ctx, id, "active t=1000");
}

#[test]
fn invariants_after_partial_withdrawal() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&id);
    assert_invariants(&ctx, id, "after partial withdraw t=300");
}

#[test]
fn invariants_after_multiple_withdrawals() {
    let (ctx, id) = setup_standard(1000);
    for t in [100u64, 300, 600, 900, 1000] {
        ctx.env.ledger().set_timestamp(t);
        ctx.client().withdraw(&id);
        assert_invariants(&ctx, id, &std::format!("multi-withdraw t={t}"));
    }
}

#[test]
fn invariants_completed_stream() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id);
    assert_eq!(
        ctx.client().get_stream_state(&id).status,
        StreamStatus::Completed
    );
    assert_invariants(&ctx, id, "completed");
}

#[test]
fn invariants_paused_stream() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(400);
    ctx.client()
        .pause_stream(&id, &crate::PauseReason::Operational);
    assert_invariants(&ctx, id, "paused t=400");
}

#[test]
fn invariants_paused_then_resumed() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(400);
    ctx.client()
        .pause_stream(&id, &crate::PauseReason::Operational);
    ctx.env.ledger().set_timestamp(600);
    ctx.client().resume_stream(&id);
    assert_invariants(&ctx, id, "resumed t=600");
}

#[test]
fn invariants_paused_withdraw_then_resume() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(400);
    ctx.client()
        .pause_stream(&id, &crate::PauseReason::Operational);
    assert_invariants(&ctx, id, "paused before resume");
    ctx.env.ledger().set_timestamp(600);
    ctx.client().resume_stream(&id);
    ctx.client().withdraw(&id);
    assert_invariants(&ctx, id, "post-resume withdraw");
}

#[test]
fn invariants_cancelled_before_cliff() {
    let ctx = PropCtx::new(1000);
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &500u64,
        &1000u64,
        &None,
    );
    ctx.env.ledger().set_timestamp(200);
    ctx.client().cancel_stream(&id);
    assert_invariants(&ctx, id, "cancelled before cliff");
}

#[test]
fn invariants_cancelled_after_partial_accrual() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&id);
    ctx.env.ledger().set_timestamp(600);
    ctx.client().cancel_stream(&id);
    assert_invariants(&ctx, id, "cancelled after partial accrual");
    ctx.client().withdraw(&id);
    assert_invariants(&ctx, id, "post-cancel final withdraw");
}

#[test]
fn invariants_cancelled_fully_accrued() {
    let (ctx, id) = setup_standard(1000);
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&id);
    assert_invariants(&ctx, id, "cancelled fully accrued");
}

#[test]
fn invariants_high_rate_deposit_capped() {
    // rate=10/s, duration=100s, deposit=1000 (exact minimum)
    let ctx = PropCtx::new(1000);
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &10_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    for t in [0u64, 10, 50, 99, 100, 200] {
        ctx.env.ledger().set_timestamp(t);
        assert_invariants(&ctx, id, &std::format!("high-rate t={t}"));
    }
}

#[test]
fn invariants_excess_deposit_stream() {
    // deposit=2000 > rate*duration=1000: excess stays in contract
    let ctx = PropCtx::new(2000);
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    for t in [0u64, 500, 1000, 1500] {
        ctx.env.ledger().set_timestamp(t);
        assert_invariants(&ctx, id, &std::format!("excess-deposit t={t}"));
    }
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id);
    assert_invariants(&ctx, id, "excess-deposit post-withdraw");
}

#[test]
fn invariants_multiple_pause_resume_cycles() {
    let (ctx, id) = setup_standard(1000);
    for (t, pause) in [
        (100u64, true),
        (200, false),
        (300, true),
        (500, false),
        (700, true),
        (800, false),
    ] {
        ctx.env.ledger().set_timestamp(t);
        if pause {
            ctx.client()
                .pause_stream(&id, &crate::PauseReason::Operational);
        } else {
            ctx.client().resume_stream(&id);
        }
        assert_invariants(&ctx, id, &std::format!("cycle pause={pause} t={t}"));
    }
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id);
    assert_invariants(&ctx, id, "post-cycles final withdraw");
}
