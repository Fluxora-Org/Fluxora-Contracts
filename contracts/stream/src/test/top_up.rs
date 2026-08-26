//! Stage 2 — top-up.
//!
//! Chosen semantics: **extend the duration, keep the rate**. The per-second
//! rate the recipient agreed to at creation never changes; `end_time` moves
//! forward instead. These tests pin that down, because the alternative
//! (hold `end_time`, raise the rate) is retroactive and would silently re-vest
//! elapsed time.

use super::common::*;
use crate::{Error, StreamStatus};

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
