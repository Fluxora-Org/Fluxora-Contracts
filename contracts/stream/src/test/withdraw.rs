//! Stage 1 — `withdraw`: linear accrual, partial draws, boundaries, depletion.

use super::common::*;
use crate::{Error, StreamStatus};

/// A schedule that collapses to zero duration after an at-start cancel must
/// stay readable and writable: reads return the settled (zero) amounts without
/// dividing by zero, and withdraw reports `NothingToWithdraw` rather than
/// trapping.
#[test]
fn zero_duration_collapsed_stream_is_safe_to_read_and_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Cancel at the instant of creation collapses the schedule to zero length.
    h.client.cancel(&id);

    // Reads must not divide by zero.
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), 0);

    // Write path is a typed error, not a panic. Cancelled status returns StreamTerminated.
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    h.assert_pool_exact();
}

#[test]
fn nothing_is_withdrawable_before_the_stream_starts() {
    let h = Harness::new();
    let start = h.now() + 10 * DAY;
    let id = h.create(100 * ONE, start, start + 100 * DAY, start, true, true, true);

    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);
    h.assert_pool_exact();
}

#[test]
fn accrual_is_linear_across_the_schedule() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    for (days, expected_pct) in [(0u64, 0i128), (1, 1), (25, 25), (50, 50), (99, 99)] {
        h.warp_to(T0 + days * DAY);
        assert_eq!(
            h.client.vested_of(&id),
            10 * ONE * expected_pct,
            "at day {days}",
        );
    }
}

#[test]
fn withdraw_max_transfers_everything_accrued() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let paid = h.client.withdraw(&id, &None);

    assert_eq!(paid, 300 * ONE);
    assert_eq!(h.balance(&h.recipient), 300 * ONE);
    assert_eq!(h.pool(), 700 * ONE);
    assert_eq!(h.get(id).withdrawn, 300 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    h.assert_pool_exact();
}

#[test]
fn partial_withdrawals_leave_the_remainder_claimable() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    assert_eq!(h.client.withdraw(&id, &Some(100 * ONE)), 100 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 200 * ONE);

    assert_eq!(h.client.withdraw(&id, &Some(200 * ONE)), 200 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.balance(&h.recipient), 300 * ONE);
    h.assert_pool_exact();
}

/// Truncation must not accumulate: `vested` is always recomputed from the
/// cumulative formula, never summed from per-interval deltas.
#[test]
fn many_small_withdrawals_total_the_same_as_one_large_one() {
    let h = Harness::new();
    let drip = h.create_simple(1_000 * ONE, 100 * DAY);
    let lump = h.create_simple(1_000 * ONE, 100 * DAY);

    let mut drip_total = 0i128;
    for _ in 0..100 {
        h.advance(DAY);
        drip_total += h.client.withdraw(&drip, &None);
    }
    let lump_total = h.client.withdraw(&lump, &None);

    assert_eq!(drip_total, lump_total);
    assert_eq!(drip_total, 1_000 * ONE);
    h.assert_pool_exact();
}

#[test]
fn requesting_more_than_available_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let err = h
        .client
        .try_withdraw(&id, &Some(300 * ONE + 1))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable);

    // The failed attempt changed nothing.
    assert_eq!(h.get(id).withdrawn, 0);
    assert_eq!(h.balance(&h.recipient), 0);
    h.assert_pool_exact();
}

#[test]
fn explicit_zero_or_negative_amount_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    for amount in [0i128, -1, -100 * ONE] {
        let err = h
            .client
            .try_withdraw(&id, &Some(amount))
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::InvalidAmount, "amount {amount}");
    }
}

// --- Boundaries -----------------------------------------------------------

/// Exactly at `end_time` the whole deposit must be vested — not one stroop
/// less, which is what a naive `elapsed < duration` comparison would produce.
#[test]
fn withdraw_at_exactly_end_time_yields_the_full_deposit() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);

    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();
}

#[test]
fn accrual_stops_at_end_time_and_never_exceeds_the_deposit() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY - 1);
    let just_before = h.client.vested_of(&id);
    assert!(just_before < 1_000 * ONE);

    for extra in [0u64, 1, DAY, 10 * YEAR] {
        h.warp_to(T0 + 100 * DAY + extra);
        assert_eq!(
            h.client.vested_of(&id),
            1_000 * ONE,
            "clamped at end + {extra}s",
        );
    }
}

#[test]
fn draining_a_stream_marks_it_depleted() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);

    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    assert_eq!(h.get(id).withdrawn, 1_000 * ONE);
}

/// A depleted stream must return `StreamTerminated`, never panic and never
/// pay twice. Distinct from `NothingToWithdraw` so a client does not confuse
/// "not accrued yet" with "this stream is over".
#[test]
fn withdrawing_from_a_depleted_stream_is_a_typed_error() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    assert_eq!(h.balance(&h.recipient), 1_000 * ONE);

    h.advance(10 * YEAR);
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    h.assert_pool_exact();
}

/// Missing stream on a fund-moving path must be a decodable contract error,
/// not a host trap from an unwrap on storage.
#[test]
fn withdrawing_unknown_stream_is_stream_not_found() {
    let h = Harness::new();
    let err = h.client.try_withdraw(&999, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound);
}

/// A one-second stream is the tightest possible schedule and must still behave.
#[test]
fn minimum_viable_stream_of_one_second() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(1, start, start + 1, start, true, true, true);

    assert_eq!(h.client.vested_of(&id), 0);
    h.advance(1);
    assert_eq!(h.client.vested_of(&id), 1);
    assert_eq!(h.client.withdraw(&id, &None), 1);
    h.assert_pool_exact();
}

/// At one stroop per second the recipient accrues exactly one stroop per
/// second — the rate floor's whole purpose.
#[test]
fn rate_floor_stream_accrues_one_stroop_per_second() {
    let h = Harness::new();
    let start = h.now();
    let duration = 1_000u64;
    let id = h.create(
        duration as i128,
        start,
        start + duration,
        start,
        true,
        true,
        true,
    );

    for t in [1u64, 10, 500, 999] {
        h.warp_to(start + t);
        assert_eq!(h.client.vested_of(&id), t as i128, "at t+{t}");
    }
}

// --- Views ----------------------------------------------------------------

#[test]
fn views_agree_with_what_withdraw_actually_pays() {
    let h = Harness::new();
    let id = h.create_simple(777 * ONE, 37 * DAY);

    for step in 1..=37u64 {
        h.warp_to(T0 + step * DAY);

        let vested = h.client.vested_of(&id);
        let available = h.client.withdrawable_of(&id);
        let refundable = h.client.refundable_of(&id);
        let before = h.get(id).withdrawn;

        assert_eq!(available, vested - before, "withdrawable == vested - drawn");
        assert_eq!(
            vested + refundable,
            777 * ONE,
            "vested + refundable == deposited"
        );

        if available > 0 {
            assert_eq!(h.client.withdraw(&id, &None), available);
        }
    }
    h.assert_pool_exact();
}

/// Views must not mutate. Two identical reads at the same instant must return
/// identical results and leave state untouched.
#[test]
fn views_are_side_effect_free() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(33 * DAY);

    let before = h.get(id);
    let a = h.client.withdrawable_of(&id);
    let b = h.client.withdrawable_of(&id);
    let after = h.get(id);

    assert_eq!(a, b);
    assert_eq!(before, after);
}
