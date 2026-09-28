//! Stage 2 — top-up.
//!
//! Chosen semantics: **extend the duration, keep the rate**. The per-second
//! rate the recipient agreed to at creation never changes; `end_time` moves
//! forward instead. These tests pin that down, because the alternative
//! (hold `end_time`, raise the rate) is retroactive and would silently re-vest
//! elapsed time.

use super::common::*;
use crate::{Error, StreamStatus};
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::Events;

#[test]
fn top_up_extends_the_end_date_at_the_same_rate() {
    let h = Harness::new();
    // 1000 tokens over 100 days = 10/day.
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let original_end = h.get(id).end_time;

    h.client.top_up(&id, &(100 * ONE));
    let s = h.get(id);

    assert_eq!(s.deposited, 1_100 * ONE);
    assert_eq!(
        s.end_time,
        original_end + 10 * DAY,
        "100 tokens at 10/day = 10 days"
    );
    assert_eq!(h.pool(), 1_100 * ONE);
    h.assert_pool_exact();
}

/// The defining property: a top-up must not change what is already withdrawable.
#[test]
fn top_up_does_not_retroactively_vest_elapsed_time() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    let before = h.client.vested_of(&id);
    assert_eq!(before, 500 * ONE);

    h.client.top_up(&id, &(1_000 * ONE));

    assert_eq!(
        h.client.vested_of(&id),
        before,
        "topping up must not move already-vested funds",
    );
}

/// Regression for #1589: adding funds at a fixed timestamp must leave the
/// already-earned amount unchanged, even when the original rate is fractional.
#[test]
fn top_up_preserves_the_vesting_curve_at_the_top_up_timestamp() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(1_000, start, start + 300, start, true, true, true);

    h.advance(137);
    let before = h.client.vested_of(&id);
    h.client.top_up(&id, &7);

    assert_eq!(
        h.client.vested_of(&id),
        before,
        "top-up must not retroactively revalue elapsed time",
    );
    assert_eq!(h.get(id).withdrawn, 0);
    h.assert_pool_exact();
}

#[test]
fn the_per_second_rate_survives_a_top_up() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    h.client.top_up(&id, &(500 * ONE));

    // Still 10 tokens/day.
    let before = h.client.vested_of(&id);
    h.advance(10 * DAY);
    assert_eq!(h.client.vested_of(&id) - before, 100 * ONE);
}

#[test]
fn a_topped_up_stream_eventually_delivers_everything() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);
    h.client.top_up(&id, &(500 * ONE));

    let end = h.get(id).end_time;
    h.warp_to(end);

    assert_eq!(h.client.vested_of(&id), 1_500 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 1_500 * ONE);
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();
}

#[test]
fn repeated_top_ups_compound_correctly() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    for _ in 0..5 {
        h.advance(5 * DAY);
        h.client.top_up(&id, &(100 * ONE));
    }

    let s = h.get(id);
    assert_eq!(s.deposited, 1_500 * ONE);
    assert_eq!(s.end_time, T0 + 150 * DAY, "5 x 10 days of extension");

    h.warp_to(s.end_time);
    assert_eq!(h.client.withdraw(&id, &None), 1_500 * ONE);
    h.assert_pool_exact();
}

#[test]
fn top_up_works_after_a_partial_withdrawal() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.withdraw(&id, &None);

    h.client.top_up(&id, &(200 * ONE));
    assert_eq!(h.get(id).deposited, 1_200 * ONE);
    assert_eq!(h.pool(), 900 * ONE, "700 unvested + 200 new");
    h.assert_pool_exact();

    h.warp_to(h.get(id).end_time);
    assert_eq!(h.client.withdraw(&id, &None), 900 * ONE);
    assert_eq!(h.balance(&h.recipient), 1_200 * ONE);
    h.assert_pool_exact();
}

#[test]
fn top_up_is_allowed_while_paused_and_does_not_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.pause(&id);

    h.client.top_up(&id, &(100 * ONE));

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Paused);
    assert_eq!(s.deposited, 1_100 * ONE);
    assert_eq!(h.client.vested_of(&id), 300 * ONE, "still frozen");
    h.assert_pool_exact();
}

/// **Regression.** The duration extension must round **down**, because rounding
/// up lowers the rate and therefore retroactively *reduces* already-vested
/// value — letting `withdrawn` exceed `vested`.
///
/// Found by `test::invariants` at seed 11694633084171541224 step 27, where a
/// recipient ended up holding 93 stroops more than `vested_of` reported. Left
/// unfixed, a subsequent `cancel` (which sets `deposited = vested`) would drive
/// the stream's liability negative and refund the sender funds the recipient
/// had already withdrawn.
#[test]
fn a_top_up_never_reduces_what_is_already_vested() {
    let h = Harness::new();
    // Deliberately inexact: 1000 stroops over 300 seconds is 3.33/sec.
    let start = h.now();
    let id = h.create(1_000, start, start + 300, start, true, true, true);

    h.advance(150);
    let before = h.client.vested_of(&id);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).withdrawn, before);

    // Top up by amounts chosen to land on awkward remainders.
    for amount in [7i128, 13, 101, 17] {
        h.client.top_up(&id, &amount);
        let after = h.client.vested_of(&id);
        let s = h.get(id);
        assert!(
            after >= before,
            "vested went backwards across top_up({amount}): {before} -> {after}",
        );
        assert!(
            s.withdrawn <= after,
            "withdrawn {} exceeded vested {after} after top_up({amount})",
            s.withdrawn,
        );
        h.advance(1);
    }
    h.assert_pool_exact();
}

/// The same case carried through to settlement: cancelling after a top-up must
/// never produce a deposit below what was already withdrawn.
#[test]
fn cancelling_after_a_top_up_cannot_refund_withdrawn_funds() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(1_000, start, start + 300, start, true, true, true);

    h.advance(150);
    h.client.withdraw(&id, &None);
    h.client.top_up(&id, &7);
    h.client.cancel(&id);

    let s = h.get(id);
    assert!(
        s.deposited >= s.withdrawn,
        "cancel left deposited {} below withdrawn {}",
        s.deposited,
        s.withdrawn,
    );
    h.assert_pool_exact();
}

/// A top-up too small to buy one second of schedule is rejected: absorbing it
/// would mean raising the rate, which re-vests elapsed time retroactively.
#[test]
fn a_sub_second_top_up_is_rejected() {
    let h = Harness::new();
    let start = h.now();

    // 1 stroop/sec: one stroop buys exactly one second, so it is accepted.
    let sparse = h.create(1_000, start, start + 1_000, start, true, true, true);
    h.client.top_up(&sparse, &1);
    assert_eq!(h.get(sparse).end_time, start + 1_001);

    // 100 stroops/sec: one stroop buys nothing, so it must be rejected rather
    // than absorbed by raising the rate.
    let dense = h.create(10_000, start, start + 100, start, true, true, true);
    let err = h.client.try_top_up(&dense, &1).unwrap_err().unwrap();
    assert_eq!(err, Error::TopUpTooSmall);
    assert_eq!(
        h.get(dense).deposited,
        10_000,
        "rejected top-up changed nothing"
    );
    h.assert_pool_exact();
}

// --- Guards ---------------------------------------------------------------

/// Topping up a matured stream would make the new funds instantly withdrawable,
/// which is never what the sender means.
#[test]
fn topping_up_a_matured_stream_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);

    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamMatured);

    h.advance(YEAR);
    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamMatured);

    assert_eq!(
        h.pool(),
        1_000 * ONE,
        "no funds pulled by a rejected top-up"
    );
    h.assert_pool_exact();
}

/// One second before maturity is still fine — the boundary is exact.
#[test]
fn topping_up_one_second_before_maturity_is_allowed() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY - 1);

    h.client.top_up(&id, &(100 * ONE));
    assert_eq!(h.get(id).deposited, 1_100 * ONE);
    h.assert_pool_exact();
}

#[test]
fn topping_up_a_cancelled_stream_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.cancel(&id);

    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    h.assert_pool_exact();
}

#[test]
fn topping_up_a_depleted_stream_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);
    h.advance(10 * DAY);
    h.client.withdraw(&id, &None);

    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
}

#[test]
fn non_positive_top_up_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    for amount in [0i128, -1, -100 * ONE] {
        let err = h.client.try_top_up(&id, &amount).unwrap_err().unwrap();
        assert_eq!(err, Error::InvalidAmount, "amount {amount}");
    }
    assert_eq!(h.pool(), 1_000 * ONE);
}

#[test]
fn a_top_up_that_would_overflow_accrual_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let err = h
        .client
        .try_top_up(&id, &(i128::MAX / 2))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Overflow);
    h.assert_pool_exact();
}

// --- Boundary condition tests for end-time extension at delta computation ---

/// A top-up that computes delta == 0 is rejected with TopUpTooSmall.
///
/// This happens when the amount is too small to buy even one second at the
/// current rate: `amount * duration / deposited = 0`. Absorbing such a
/// top-up would require raising the rate, which retroactively re-vests
/// elapsed time — the exact thing the fixed-rate design prevents.
#[test]
fn top_up_computing_zero_delta_is_rejected_as_too_small() {
    let h = Harness::new();
    let start = h.now();

    // Create a stream with a high rate: 100_000 stroops over 100 seconds = 1000/sec.
    // To compute delta = 0, we need: amount * 100 / 100_000 = 0
    // This requires amount < 1000.
    let id = h.create(100_000, start, start + 100, start, true, true, true);

    // Top-up with 999 stroops: 999 * 100 / 100_000 = 99_900 / 100_000 = 0 (floor division).
    let err = h.client.try_top_up(&id, &999).unwrap_err().unwrap();
    assert_eq!(err, Error::TopUpTooSmall);

    // The stream is unchanged.
    let s = h.get(id);
    assert_eq!(s.deposited, 100_000);
    assert_eq!(s.end_time, start + 100);

    // No funds were pulled.
    assert_eq!(h.pool(), 100_000);
    h.assert_pool_exact();
}

/// At the boundary: a top-up that buys exactly one second is accepted.
///
/// Regression: ensure the boundary check is `delta < 0` or `delta > MAX`,
/// not `delta <= 0`. The zero-delta case is special-cased after the
/// overflow check and must not catch delta == 1.
#[test]
fn top_up_computing_one_second_delta_is_accepted() {
    let h = Harness::new();
    let start = h.now();

    // 1_000_000 stroops over 1_000_000 seconds = 1 stroop/sec.
    // To compute delta = 1, we need: amount * 1_000_000 / 1_000_000 = 1
    // So amount = 1.
    let id = h.create(1_000_000, start, start + 1_000_000, start, true, true, true);

    let old_end = h.get(id).end_time;
    h.client.top_up(&id, &1);

    let s = h.get(id);
    assert_eq!(s.deposited, 1_000_001);
    assert_eq!(s.end_time, old_end + 1, "delta should be exactly 1 second");
    h.assert_pool_exact();
}

/// A top-up that would overflow end_time is rejected with Overflow.
///
/// This tests the boundary where `end_time + delta > u64::MAX`.
#[test]
fn top_up_overflow_on_end_time_addition_is_rejected() {
    let h = Harness::new();

    // Create a stream ending near u64::MAX.
    let start = 1_000_000u64;
    let end = u64::MAX - 100; // Leave room for delta
    let id = h.create(100_000, start, end, start, true, true, true);

    // Top-up with an amount that would compute a huge delta.
    // delta = amount * duration / deposited
    // duration = u64::MAX - 100 - 1_000_000 = u64::MAX - 1_000_100 (large)
    // If amount is large enough, delta could exceed u64::MAX - end.
    let err = h
        .client
        .try_top_up(&id, &(i128::MAX / 2))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Overflow);

    // Stream is unchanged.
    let s = h.get(id);
    assert_eq!(s.end_time, end);
    h.assert_pool_exact();
}

/// A top-up on a paused stream preserves the frozen clock and does not move vested.
///
/// This test verifies that topping up while paused:
/// 1. Does not change `paused_at` (the freeze point).
/// 2. Does not change `paused_total` (cumulative pause time).
/// 3. Does not advance `vested` beyond what it was before the top-up,
///    since the clock is frozen.
///
/// Invariant I3 requires that `vested(t)` never decreases for a fixed timestamp `t`.
/// For a paused stream, the stream clock is frozen, so `vested` should not change.
#[test]
fn top_up_on_paused_stream_preserves_frozen_clock_and_vested() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Advance partway through the stream and pause.
    h.advance(30 * DAY);
    h.client.pause(&id);

    let s_paused = h.get(id);
    let paused_at_before = s_paused.paused_at;
    let paused_total_before = s_paused.paused_total;
    let vested_before = h.client.vested_of(&id);

    // Top-up while paused.
    h.client.top_up(&id, &(100 * ONE));

    let s_after = h.get(id);

    // Clock freeze point and cumulative pause time must be unchanged.
    assert_eq!(
        s_after.paused_at, paused_at_before,
        "paused_at changed across top-up"
    );
    assert_eq!(
        s_after.paused_total, paused_total_before,
        "paused_total changed across top-up"
    );

    // Status remains Paused.
    assert_eq!(s_after.status, StreamStatus::Paused);

    // Vested must not move forward (frozen clock means no accrual).
    let vested_after = h.client.vested_of(&id);
    assert_eq!(
        vested_after, vested_before,
        "vested moved while clock is frozen"
    );

    // Deposited increased by the top-up amount.
    assert_eq!(s_after.deposited, 1_100 * ONE);

    h.assert_pool_exact();
}

/// A top-up on a paused stream extends end_time correctly without causing vested regression.
///
/// This is a specific case of the frozen-clock preservation test, but it also
/// verifies the rate computation works correctly even while paused.
#[test]
fn top_up_on_paused_stream_extends_end_time_while_preserving_rate() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    let vested_before_pause = h.client.vested_of(&id);

    h.client.pause(&id);

    // Top-up: 100 tokens at 10/day should extend by 10 days.
    h.client.top_up(&id, &(100 * ONE));

    let s = h.get(id);
    assert_eq!(s.deposited, 1_100 * ONE);
    assert_eq!(s.end_time, T0 + 110 * DAY, "rate is 10/day, so 100 new = 10 days");

    // Vested must not have moved while paused.
    let vested_after = h.client.vested_of(&id);
    assert_eq!(
        vested_after, vested_before_pause,
        "vested moved while stream is paused"
    );

    // After resume, the clock should not have skipped ahead.
    h.client.resume(&id);
    assert_eq!(
        h.client.vested_of(&id),
        vested_before_pause,
        "vested changed across resume"
    );

    h.assert_pool_exact();
}

/// The VestedDecreased guard is evaluated at the right point in the computation.
///
/// This test verifies that after updating `deposited` and `end_time`, we check
/// that `vested(t)` for the current timestamp `t` has not moved backwards.
/// This is a defensive guard: the math of top_up (scaling numerator and
/// denominator together while keeping `elapsed` constant) means `vested` cannot
/// decrease, so this error is classified as reserved in error_reachability.
/// But the guard itself is load-bearing — it catches logic errors in future
/// maintenance, so it must not be removed.
#[test]
fn top_up_vested_decreased_guard_is_checked() {
    let h = Harness::new();
    let start = h.now();

    // Create a stream and advance partway through.
    let id = h.create(1_000, start, start + 1_000, start, true, true, true);
    h.advance(500);

    let vested_before = h.client.vested_of(&id);
    assert!(vested_before > 0, "must be past the cliff and have accrued");

    // Normal top-up should succeed and preserve vested.
    h.client.top_up(&id, &100);
    let vested_after = h.client.vested_of(&id);
    assert_eq!(vested_after, vested_before);

    h.assert_pool_exact();
}

/// Multiple top-ups at fixed time (frozen clock, like the monotonicity test)
/// all preserve invariant I3 (vested does not decrease).
///
/// This is a stress test of the VestedDecreased guard using the same fixed-clock
/// pattern as `test::monotonicity`, applied specifically to boundary cases.
#[test]
fn multiple_top_ups_at_fixed_time_preserve_invariant_i3() {
    let h = Harness::new();
    let start = h.now();

    // Create a stream and advance partway.
    let id = h.create(10_000, start, start + 1_000, start, true, true, true);
    h.advance(500);

    // Capture vested at a fixed instant (do not advance the clock further).
    let vested_before = h.client.vested_of(&id);

    // Top-up several times, each checking that vested does not move backwards.
    for amount in &[100i128, 250, 50, 1_000] {
        h.client.top_up(&id, amount);

        let vested_after = h.client.vested_of(&id);
        assert_eq!(
            vested_after, vested_before,
            "I3 violated: vested moved across top_up({amount}) at fixed time"
        );
    }

    h.assert_pool_exact();
}

use crate::DataKey;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[contract]
struct FaultyToken;

#[contractimpl]
impl FaultyToken {
    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().instance().get(&id).unwrap_or(0)
    }

    pub fn transfer(env: Env, _from: Address, to: Address, amount: i128) {
        if amount == 999 {
            panic!("Mock token transfer failed");
        }
        let balance = Self::balance(env.clone(), to.clone());
        env.storage().instance().set(&to, &(balance + amount));
    }
}

#[test]
fn failed_transfer_reverts_state_and_ttl_changes() {
    let h = Harness::new();
    let mock_token = h.env.register(FaultyToken, ());

    let start = h.now();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &mock_token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );

    let before = h.get(id);
    let ttl_before = h.env.as_contract(&h.contract_id, || {
        h.env.storage().persistent().get_ttl(&DataKey::Stream(id))
    });

    let res = h.client.try_top_up(&id, &999);
    assert!(res.is_err());

    let after = h.get(id);
    let ttl_after = h.env.as_contract(&h.contract_id, || {
        h.env.storage().persistent().get_ttl(&DataKey::Stream(id))
    });

    assert_eq!(before.deposited, after.deposited);
    assert_eq!(before.end_time, after.end_time);
    assert_eq!(ttl_before, ttl_after);
    // `Events::all()` reports only the most recent invocation; a reverted
    // frame publishes nothing, so the observable log must be empty.
    assert!(
        h.env.events().all().events().is_empty(),
        "a reverted top-up publishes no events",
    );
}
