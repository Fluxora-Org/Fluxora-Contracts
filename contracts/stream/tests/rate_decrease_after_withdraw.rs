//! `decrease_rate_per_second` after partial withdraw — checkpoint arithmetic regression.
//!
//! Targeted regression suite for the sequence
//!
//! ```text
//! create stream → advance → withdraw → decrease_rate_per_second → later withdraw
//! ```
//!
//! that closes the gap between (a) the proptest in
//! `src/test_withdrawable_props.rs::prop_decrease_rate_per_second_checkpoints_withdrawable_monotonicity`
//! (which only randomises *rate* decreases) and (b) the deterministic
//! `regression_rate_decrease_preserves_entitlement` test in
//! `tests/balance_conservation.rs` (which never issues any intervening
//! `withdraw`). Combining withdrawals with a rate decrease is the realistic
//! treasury path where checkpoint math errors leak funds.
//!
//! # Security invariants under test
//!
//! 1. **Entitlement preservation** — A rate decrease never lowers the
//!    recipient's already-earned entitlement. After the call,
//!    `calculate_accrued(t) == checkpointed_amount` for any `t` in
//!    `[checkpointed_at, checkpointed_at]`, and `withdrawn_amount` is preserved.
//! 2. **No double-count** — `withdrawable == accrued − withdrawn_amount`.
//!    The post-decrease accrued value at any later `t` is
//!    `min(checkpointed_amount + new_rate * (t − checkpointed_at), deposit_amount)`
//!    and `withdrawn_amount` is monotonically non-decreasing across the whole
//!    sequence.
//! 3. **Boundary correctness** — A decrease called at exactly the same ledger
//!    timestamp as a prior `withdraw` still preserves that `withdrawn_amount`
//!    and the per-call `withdrawable` value at the boundary.
//! 4. **Refund math** — `new_deposit = checkpointed_amount + new_rate * (end − now)`
//!    and `refund = old_deposit − new_deposit` exactly balances the books without
//!    re-pricing the past. The contract enforces `refund >= 0` (`checked_sub`):
//!    any rate change that would *grow* `new_deposit` past `old_deposit` is
//!    rejected with `ArithmeticOverflow`.
//!
//! # Determinism
//!
//! All tests are deterministic. Expected values are hand-computed and asserted
//! with `assert_eq!` so a regression that breaks the checkpoint math produces
//! a precise failure message, not just an inequality.

extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// Total tokens minted into the test ecosystem for the optional balance check.
const INITIAL_MINT: i128 = 1_000_000_000_000;

// ---------------------------------------------------------------------------
// Canonical fixture (test #1 baseline math)
// ---------------------------------------------------------------------------
//   deposit = 1000, rate = 10/s, end = 100  → max_streamable = 10 * 100 = 1000
//   t = 10:   accrued = 10 * 10 = 100     (genuinely partial of the 1000 ceiling)
//             withdraw returns 100        → withdrawn_amount = 100
//   t = 10:   decrease_rate_per_second(10, 5)
//             checkpointed = 100,  checkpointed_at = 10
//             new_deposit = 100 + 5 * (100 − 10) = 550
//             refund = 1000 − 550 = 450
//   t = 80:   accrued = 100 + 5 * 70 = 450
//             withdrawable = 450 − 100 = 350 → withdraw returns 350
//   t = 100:  accrued = 100 + 5 * 90 = 550
//             withdrawable = 550 − 450 = 100 → withdraw returns 100 → Completed
// ---------------------------------------------------------------------------
const CANONICAL_DEPOSIT: i128 = 1_000;
const CANONICAL_OLD_RATE: i128 = 10;
const CANONICAL_NEW_RATE: i128 = 5;
const CANONICAL_END: u64 = 100;
const CANONICAL_DECREASE_AT: u64 = 10;
const CANONICAL_SECOND_WITHDRAW_AT: u64 = 80;

// ===========================================================================
// Test harness (mirrors `tests/balance_conservation.rs`).
// ===========================================================================

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

        // Mint enough tokens for the sender to fund any test fixture used here.
        StellarAssetClient::new(&env, &token_id).mint(&sender, &INITIAL_MINT);
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

    fn recipient_balance(&self) -> i128 {
        self.token().balance(&self.recipient)
    }

    fn sender_balance(&self) -> i128 {
        self.token().balance(&self.sender)
    }

    /// Build a stream pinned at `start_time = 0` with the supplied parameters.
    fn create_stream(&self, deposit: i128, rate: i128, cliff: u64, end: u64) -> u64 {
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
            &StreamKind::Linear,
            &None,
        )
    }

    /// Advance the ledger timestamp and sequence number past the withdrawal
    /// cooldown (`MIN_WITHDRAW_INTERVAL_LEDGERS == 1`) so the next `withdraw`
    /// is allowed.
    fn advance(&self, timestamp: u64, sequence: u64) {
        self.env.ledger().set_timestamp(timestamp);
        self.env.ledger().set_sequence_number(sequence as u32);
    }
}

// ===========================================================================
// Core scenario: partial withdraw → decrease → later withdraw
// ===========================================================================

/// Hand-computed baseline for the canonical scenario. Every subsequent test
/// in this file uses the same numbers (or close variants) so a regression
/// that breaks the checkpoint math produces a precise, easy-to-read failure.
#[test]
fn rate_decrease_after_partial_withdraw_baseline() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(CANONICAL_DEPOSIT, CANONICAL_OLD_RATE, 0, CANONICAL_END);

    // ---- t = 10: genuinely-partial withdraw (100 of the eventual 1000 ceiling) ----
    ctx.advance(CANONICAL_DECREASE_AT, CANONICAL_DECREASE_AT + 1);
    assert_eq!(ctx.client().calculate_accrued(&id), 100); // 10 * 10
    assert_eq!(ctx.client().get_withdrawable(&id), 100); // accrued 100 - withdrawn 0

    let first_withdrawn = ctx.client().withdraw(&id);
    assert_eq!(
        first_withdrawn, 100,
        "`withdraw` returns the full withdrawable at the current ledger second"
    );
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 100);

    // ---- t = 10 (same second): decrease to 5/s ----
    let sender_balance_before = ctx.sender_balance();
    ctx.client()
        .decrease_rate_per_second(&id, &CANONICAL_NEW_RATE);
    let s = ctx.client().get_stream_state(&id);
    assert_eq!(s.rate_per_second, CANONICAL_NEW_RATE);
    assert_eq!(s.checkpointed_at, 10);
    assert_eq!(s.checkpointed_amount, 100);
    assert_eq!(s.deposit_amount, 550); // 100 + 5 * (100 - 10)
    assert_eq!(
        s.withdrawn_amount, 100,
        "prior withdraw must persist (no double-count)"
    );

    // Refund: 1000 (old) - 550 (new) = 450
    let sender_balance_after = ctx.sender_balance();
    assert_eq!(
        sender_balance_after - sender_balance_before,
        450,
        "sender must receive the difference back"
    );

    // ---- At t = 10 still: accrued equals checkpointed_amount (no time elapsed);
    //       withdrawable = checkpointed_amount − prior withdrawn_amount = 0 ----
    assert_eq!(
        ctx.client().calculate_accrued(&id),
        100,
        "same-timestamp accrued must equal checkpointed_amount"
    );
    assert_eq!(
        ctx.client().get_withdrawable(&id),
        0,
        "withdrawable at boundary must be checkpointed_amount minus prior withdrawn_amount"
    );

    // ---- t = 80: slow accrual at 5/s for 70 seconds ----
    ctx.advance(
        CANONICAL_SECOND_WITHDRAW_AT,
        CANONICAL_SECOND_WITHDRAW_AT + 1,
    );
    assert_eq!(ctx.client().calculate_accrued(&id), 450); // 100 + 5 * 70
    assert_eq!(ctx.client().get_withdrawable(&id), 350); // 450 - 100

    let second_withdrawn = ctx.client().withdraw(&id);
    assert_eq!(
        second_withdrawn, 350,
        "second withdraw must pull exactly the slowed-accrual delta, no double-count"
    );
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 450);

    // ---- t = 100: end of stream, full new_deposit payable ----
    ctx.advance(CANONICAL_END, CANONICAL_END + 1);
    assert_eq!(ctx.client().calculate_accrued(&id), 550);
    assert_eq!(ctx.client().get_withdrawable(&id), 100); // 550 - 450

    let third_withdrawn = ctx.client().withdraw(&id);
    assert_eq!(
        third_withdrawn, 100,
        "final withdrawal completes the stream by paying the last 100 tokens"
    );
    let final_state = ctx.client().get_stream_state(&id);
    assert_eq!(final_state.withdrawn_amount, 550);
    assert_eq!(final_state.deposit_amount, 550);
    assert_eq!(
        final_state.status,
        StreamStatus::Completed,
        "withdrawing the full new_deposit must transition to Completed"
    );

    // Balance-conservation sanity check across sender + recipient + contract.
    let total = ctx.sender_balance() + ctx.recipient_balance() + ctx.contract_balance();
    assert_eq!(total, INITIAL_MINT);
}

/// Variant with the decrease happening *first* (before any withdraw) to prove
/// the math is symmetric: at the same ledger second, the order of
/// `decrease_rate_per_second` and `withdraw` is interchangeable when no time
/// elapses between them.
#[test]
fn rate_decrease_before_withdraw_yields_same_baseline() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(CANONICAL_DEPOSIT, CANONICAL_OLD_RATE, 0, CANONICAL_END);

    // t = 10: decrease at the same moment the canonical test begins to withdraw.
    ctx.advance(CANONICAL_DECREASE_AT, CANONICAL_DECREASE_AT + 1);
    ctx.client()
        .decrease_rate_per_second(&id, &CANONICAL_NEW_RATE);

    // Same-timestamp withdrawable equals checkpointed_amount when no prior
    // withdrawal has occurred.
    assert_eq!(ctx.client().get_withdrawable(&id), 100);

    // Advance one extra ledger second so the cooldown gate is open.
    ctx.advance(CANONICAL_DECREASE_AT + 1, CANONICAL_DECREASE_AT + 2);
    let first = ctx.client().withdraw(&id);
    assert_eq!(
        first, 105,
        "withdraw one second past checkpoint = 100 + 5 * 1"
    );
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 105);

    // t = 80: the remaining entitlement is exactly `accrued − withdrawn`.
    ctx.advance(
        CANONICAL_SECOND_WITHDRAW_AT,
        CANONICAL_SECOND_WITHDRAW_AT + 2,
    );
    assert_eq!(ctx.client().calculate_accrued(&id), 450);
    assert_eq!(ctx.client().get_withdrawable(&id), 450 - 105);
    let second = ctx.client().withdraw(&id);
    assert_eq!(second, 345);
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 450);
}

/// Boundary: `withdraw` and `decrease_rate_per_second` called at the exact
/// same ledger timestamp must produce the same checkpoint state as in the
/// canonical baseline. The prior `withdrawn_amount` is included in the
/// post-decrease `withdrawable` math.
#[test]
fn rate_decrease_at_withdrawal_boundary_preserves_state() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(CANONICAL_DEPOSIT, CANONICAL_OLD_RATE, 0, CANONICAL_END);

    ctx.advance(CANONICAL_DECREASE_AT, CANONICAL_DECREASE_AT + 1);
    ctx.client().withdraw(&id); // pulls the entire 100 at t=10

    // No accrual between t=10 and t=10.
    assert_eq!(ctx.client().get_withdrawable(&id), 0);

    // Both operations stamped at t = 10. Post-decrease state must include
    // the prior withdraw in its withdrawable math.
    ctx.client()
        .decrease_rate_per_second(&id, &CANONICAL_NEW_RATE);
    let s = ctx.client().get_stream_state(&id);
    assert_eq!(s.rate_per_second, CANONICAL_NEW_RATE);
    assert_eq!(s.checkpointed_at, 10);
    assert_eq!(s.checkpointed_amount, 100);
    assert_eq!(s.deposit_amount, 550);
    assert_eq!(s.withdrawn_amount, 100, "prior withdraw must persist");
    assert_eq!(ctx.client().get_withdrawable(&id), 0);

    // t = 11: exactly one second of new-rate accrual (5 tokens) becomes
    // withdrawable on top of the prior 100.
    ctx.advance(CANONICAL_DECREASE_AT + 1, CANONICAL_DECREASE_AT + 2);
    assert_eq!(ctx.client().calculate_accrued(&id), 105);
    assert_eq!(ctx.client().get_withdrawable(&id), 5);
    assert_eq!(ctx.client().withdraw(&id), 5);
}

/// Boundary scenario: withdraw → decrease in the same ledger second, then
/// three later withdrawals drain the stream. Verifies that the post-decrease
/// `withdrawn_amount` progression is monotone and the stream transitions to
/// `Completed` only when the full `new_deposit_amount` has been paid.
#[test]
fn rate_decrease_boundary_full_drain_three_withdrawals() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(CANONICAL_DEPOSIT, CANONICAL_OLD_RATE, 0, CANONICAL_END);

    // t = 10: pre-decrease withdraw of all 100 (partial of the 1000 ceiling).
    ctx.advance(CANONICAL_DECREASE_AT, CANONICAL_DECREASE_AT + 1);
    assert_eq!(ctx.client().withdraw(&id), 100);
    assert_eq!(ctx.client().get_withdrawable(&id), 0);

    // t = 10 -> 5/s decrease. After: deposit=550, checkpoint=100, withdrawn=100.
    ctx.client()
        .decrease_rate_per_second(&id, &CANONICAL_NEW_RATE);
    assert_eq!(ctx.client().get_withdrawable(&id), 100 - 100);

    // Three further withdrawals over the slowed 5/s schedule.
    // t = 40: 30 seconds of new accrual = 150. Withdrawable 150.
    ctx.advance(40, 41);
    assert_eq!(ctx.client().calculate_accrued(&id), 250); // 100 + 5 * 30
    assert_eq!(ctx.client().get_withdrawable(&id), 150); // 250 - 100
    assert_eq!(ctx.client().withdraw(&id), 150);

    // t = 70: 60 seconds of new accrual. Withdrawable still 150.
    ctx.advance(70, 71);
    assert_eq!(ctx.client().calculate_accrued(&id), 400);
    assert_eq!(ctx.client().get_withdrawable(&id), 150);
    assert_eq!(ctx.client().withdraw(&id), 150);

    // t = 100: end-of-stream payout.
    ctx.advance(CANONICAL_END, CANONICAL_END + 1);
    assert_eq!(ctx.client().calculate_accrued(&id), 550);
    assert_eq!(ctx.client().get_withdrawable(&id), 150); // 550 - 400
    assert_eq!(ctx.client().withdraw(&id), 150); // final drain

    let final_state = ctx.client().get_stream_state(&id);
    assert_eq!(final_state.withdrawn_amount, 550);
    assert_eq!(final_state.deposit_amount, 550);
    assert_eq!(final_state.status, StreamStatus::Completed);
}

/// Multiple rate decreases interleaved with withdrawals. Each decrease must
/// preserve its own already-earned entitlement and never grow `new_deposit`
/// past `old_deposit` (enforced by the contract via `checked_sub`).
///
/// Concrete math:
///   deposit = 2000, rate = 10, end = 200 → max_streamable = 10 * 200 = 2000
///   t = 100:  accrued = 1000.   Withdraw returns 1000.   withdrawn = 1000
///             decrease 10 → 5:
///             checkpointed = 1000,  new_deposit = 1000 + 5 * 100 = 1500
///             refund = 500
///   t = 150:  accrued = 1500 + 5 * 50 = 1250.  wait, recalc:
///             checkpointed (1000) + new_rate (5) * (150 - 100) = 1250
///             withdrawable = 1250 - 1000 = 250.    Withdraw returns 250.
///             decrease 5 → 1:
///             checkpointed = 1250,  new_deposit = 1250 + 1 * 50 = 1300
///             refund = 1500 - 1300 = 200
///   t = 200:  accrued = 1300.     withdrawable = 1300 - 1250 = 50.  Withdraw returns 50.
#[test]
fn multiple_rate_decreases_with_interleaved_withdrawals() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(2_000, 10, 0, 200);

    // ---- t = 100: withdraw all 1000 (half of the streamable ceiling) ----
    ctx.advance(100, 101);
    assert_eq!(ctx.client().calculate_accrued(&id), 1_000);
    assert_eq!(ctx.client().withdraw(&id), 1_000);
    assert_eq!(ctx.client().get_withdrawable(&id), 0);

    // ---- t = 100: rate 10 → 5. new_deposit = 1500 (≤ 2000 ✓) ----
    ctx.client().decrease_rate_per_second(&id, &5);
    let s = ctx.client().get_stream_state(&id);
    assert_eq!(s.deposit_amount, 1_500);
    assert_eq!(s.checkpointed_amount, 1_000);
    assert_eq!(s.checkpointed_at, 100);
    assert_eq!(s.rate_per_second, 5);
    assert_eq!(s.withdrawn_amount, 1_000);

    // ---- t = 150: 50 seconds of 5/s accrual (+250). Withdrawable = 250 ----
    ctx.advance(150, 151);
    assert_eq!(ctx.client().calculate_accrued(&id), 1_250);
    assert_eq!(ctx.client().get_withdrawable(&id), 1_250 - 1_000);
    assert_eq!(ctx.client().withdraw(&id), 250);
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 1_250);

    // ---- t = 150: rate 5 → 1. new_deposit = 1300 (≤ 1500 ✓) ----
    ctx.client().decrease_rate_per_second(&id, &1);
    let s = ctx.client().get_stream_state(&id);
    assert_eq!(s.deposit_amount, 1_300);
    assert_eq!(s.checkpointed_amount, 1_250);
    assert_eq!(s.checkpointed_at, 150);
    assert_eq!(s.rate_per_second, 1);
    assert_eq!(s.withdrawn_amount, 1_250);

    // ---- t = 200: end of stream ----
    ctx.advance(200, 201);
    assert_eq!(ctx.client().calculate_accrued(&id), 1_300);
    assert_eq!(ctx.client().get_withdrawable(&id), 1_300 - 1_250);
    assert_eq!(ctx.client().withdraw(&id), 50);

    let final_state = ctx.client().get_stream_state(&id);
    assert_eq!(final_state.withdrawn_amount, 1_300);
    assert_eq!(final_state.status, StreamStatus::Completed);
    assert_eq!(final_state.deposit_amount, 1_300);
}

/// Edge case: the entire earned entitlement is already withdrawn before the
/// decrease. The decrease must still checkpoint the earned amount under the
/// OLD rate and refund the unstreamed portion — `withdrawn_amount` is
/// preserved verbatim.
#[test]
fn rate_decrease_preserves_withdraw_amount_at_full_drain() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(CANONICAL_DEPOSIT, CANONICAL_OLD_RATE, 0, CANONICAL_END);

    // t = 80: accrued = 800. Withdraw all 800 (still partial of the 1000 total).
    ctx.advance(80, 81);
    assert_eq!(ctx.client().calculate_accrued(&id), 800);
    assert_eq!(ctx.client().withdraw(&id), 800);
    assert_eq!(ctx.client().get_withdrawable(&id), 0);

    // t = 80: decrease to 5/s. new_deposit = 800 + 5*(100-80) = 900 ≤ 1000 ✓.
    ctx.client()
        .decrease_rate_per_second(&id, &CANONICAL_NEW_RATE);
    let s = ctx.client().get_stream_state(&id);
    assert_eq!(
        s.withdrawn_amount, 800,
        "no double-count: prior withdrawn_amount preserved"
    );
    assert_eq!(s.checkpointed_amount, 800);
    assert_eq!(s.deposit_amount, 900);
    assert_eq!(s.rate_per_second, CANONICAL_NEW_RATE);
    assert_eq!(s.checkpointed_at, 80);

    // No withdrawable at the same timestamp.
    assert_eq!(ctx.client().get_withdrawable(&id), 0);

    // t = 100: full end-of-stream payout of 100 (5/s × 20s + 800 checkpointed).
    ctx.advance(CANONICAL_END, CANONICAL_END + 1);
    assert_eq!(ctx.client().calculate_accrued(&id), 900);
    assert_eq!(ctx.client().get_withdrawable(&id), 100); // 900 - 800
    let post = ctx.client().withdraw(&id);
    assert_eq!(post, 100);
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 900);
    // 900 == new_deposit, so the stream is now drained: Completed.
    assert_eq!(
        ctx.client().get_stream_state(&id).status,
        StreamStatus::Completed
    );
}

/// `decrease_rate_per_second` enforces two hard preconditions: the new rate
/// must be strictly less than the current rate, and it must be positive.
/// This guards against accidental future flip-flopping of the contract's
/// directionality policy.
#[test]
fn rate_decrease_rejects_non_strict_decrease() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(1_000, 10, 0, 100);

    // Sanity: stream is freshly created.
    let s0 = ctx.client().get_stream_state(&id);
    assert_eq!(s0.rate_per_second, 10);

    // Each of the four invalid scenarios must reject. The exact error
    // envelope differs across contract versions, so we accept either a
    // ContractError variant or a host trap-class error.
    let invalid_attempts: [(&str, i128); 4] = [
        ("equal rate", 10),
        ("higher rate", 20),
        ("zero rate", 0),
        ("negative rate", -1),
    ];
    for (label, new_rate) in invalid_attempts {
        let result = ctx.client().try_decrease_rate_per_second(&id, &new_rate);
        assert!(
            result.is_err(),
            "decrease_rate_per_second with {label} ({new_rate}) must reject, got {result:?}"
        );
    }

    // Sanity check: rejected mutations must not mutate the stream.
    let s = ctx.client().get_stream_state(&id);
    assert_eq!(
        s.rate_per_second, 10,
        "stream rate must still be 10 after rejections"
    );
    assert_eq!(s.checkpointed_amount, 0);
    assert_eq!(s.checkpointed_at, 0);
    assert_eq!(s.deposit_amount, 1_000);
    assert_eq!(s.withdrawn_amount, 0);
}

#[test]
fn test_rate_decrease_immediately_after_withdraw_boundary_exact_accrual() {
    let ctx = TestContext::new();
    // Start rate: 100/s, total duration 1000s, deposit 100,000
    let id = ctx.create_stream(100_000, 100, 0, 1000);

    // t = 250: perform withdrawal
    ctx.advance(250, 251);
    assert_eq!(ctx.client().calculate_accrued(&id), 25_000); // 250 * 100
    assert_eq!(ctx.client().withdraw(&id), 25_000);
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 25_000);

    // t = 251 (immediately after): decrease rate to 50/s
    ctx.advance(251, 252);
    // before decrease: accrued is 251 * 100 = 25,100. withdrawable is 100.
    assert_eq!(ctx.client().calculate_accrued(&id), 25_100);
    assert_eq!(ctx.client().get_withdrawable(&id), 100);

    ctx.client().decrease_rate_per_second(&id, &50);

    // after decrease:
    // checkpointed_amount = 25,100, checkpointed_at = 251, deposit_amount = 25,100 + 50 * 749 = 62,550
    let s = ctx.client().get_stream_state(&id);
    assert_eq!(s.rate_per_second, 50);
    assert_eq!(s.checkpointed_at, 251);
    assert_eq!(s.checkpointed_amount, 25_100);
    assert_eq!(s.deposit_amount, 62_550);
    assert_eq!(s.withdrawn_amount, 25_000);

    // Already accrued but unwithdrawn amount is preserved exactly
    assert_eq!(ctx.client().calculate_accrued(&id), 25_100);
    assert_eq!(ctx.client().get_withdrawable(&id), 100);

    // Withdraw the preserved amount
    assert_eq!(ctx.client().withdraw(&id), 100);
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 25_100);

    // t = 300: subsequent withdrawal uses new rate
    ctx.advance(300, 301);
    // accrued = 25,100 + 50 * (300 - 251) = 25,100 + 50 * 49 = 27,550
    assert_eq!(ctx.client().calculate_accrued(&id), 27_550);
    assert_eq!(ctx.client().get_withdrawable(&id), 2_450); // 27,550 - 25,100

    assert_eq!(ctx.client().withdraw(&id), 2_450);
    assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 27_550);
}

#[test]
fn test_rate_decrease_same_ledger_as_withdraw_boundary_exact_accrual() {
    let ctx = TestContext::new();
    let id = ctx.create_stream(100_000, 100, 0, 1000);

    // t = 250: perform withdrawal
    ctx.advance(250, 251);
    assert_eq!(ctx.client().withdraw(&id), 25_000);

    // t = 250 (same ledger second): decrease rate to 50/s
    ctx.client().decrease_rate_per_second(&id, &50);

    // after decrease:
    // checkpointed_amount = 25,000, checkpointed_at = 250, deposit_amount = 25,000 + 50 * 750 = 62,500
    let s = ctx.client().get_stream_state(&id);
    assert_eq!(s.rate_per_second, 50);
    assert_eq!(s.checkpointed_at, 250);
    assert_eq!(s.checkpointed_amount, 25_000);
    assert_eq!(s.deposit_amount, 62_500);
    assert_eq!(s.withdrawn_amount, 25_000);
    assert_eq!(ctx.client().get_withdrawable(&id), 0);

    // t = 251 (1 second later under new rate)
    ctx.advance(251, 252);
    assert_eq!(ctx.client().calculate_accrued(&id), 25_050); // 25,000 + 50 * 1
    assert_eq!(ctx.client().get_withdrawable(&id), 50);
    assert_eq!(ctx.client().withdraw(&id), 50);
}

