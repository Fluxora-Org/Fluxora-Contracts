//! Stage 2 — pause and resume.
//!
//! The model: pausing freezes the stream's clock and pushes the effective end
//! forward by the paused duration. Total value delivered stays constant; the
//! schedule stretches.

use super::common::*;
use crate::{Error, StreamStatus};

#[test]
fn pausing_freezes_accrual() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.pause(&id);
    let frozen = h.client.vested_of(&id);
    assert_eq!(frozen, 300 * ONE);

    for jump in [1u64, DAY, 30 * DAY, YEAR] {
        h.advance(jump);
        assert_eq!(
            h.client.vested_of(&id),
            frozen,
            "accrued while paused (+{jump}s)"
        );
    }
}

#[test]
fn resuming_picks_up_exactly_where_the_clock_stopped() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.pause(&id);
    h.advance(50 * DAY);
    h.client.resume(&id);

    assert_eq!(h.client.vested_of(&id), 300 * ONE, "no jump on resume");
    assert_eq!(h.get(id).paused_total, 50 * DAY);
    assert_eq!(h.get(id).paused_at, None);
    assert_eq!(h.get(id).status, StreamStatus::Active);

    h.advance(10 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        400 * ONE,
        "rate unchanged after resume"
    );
}

/// The stretch is exact: the stream completes `paused_total` seconds later than
/// originally scheduled, and delivers exactly the same total.
#[test]
fn the_schedule_stretches_by_exactly_the_paused_duration() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let original_end = T0 + 100 * DAY;

    h.advance(30 * DAY);
    h.client.pause(&id);
    h.advance(50 * DAY);
    h.client.resume(&id);

    // At the original end date the stream is 50 days short of complete.
    h.warp_to(original_end);
    assert_eq!(h.client.vested_of(&id), 500 * ONE);

    // At the stretched end date it is exactly complete — not a stroop more.
    h.warp_to(original_end + 50 * DAY);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    h.warp_to(original_end + 50 * DAY + YEAR);
    assert_eq!(
        h.client.vested_of(&id),
        1_000 * ONE,
        "clamped after stretched end"
    );

    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

/// Pausing stops *accrual*, not access. Freezing funds the recipient already
/// earned would make pausable streams unacceptable to any serious recipient.
#[test]
fn the_recipient_can_still_withdraw_while_paused() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.pause(&id);

    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    assert_eq!(h.balance(&h.recipient), 300 * ONE);
    assert_eq!(
        h.get(id).status,
        StreamStatus::Paused,
        "still paused after withdrawal"
    );
    h.assert_pool_exact();

    // Nothing further accrues while frozen.
    h.advance(20 * DAY);
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);
}

#[test]
fn repeated_pause_cycles_accumulate_correctly() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let mut expected_paused = 0u64;
    for _ in 0..4 {
        h.advance(10 * DAY);
        h.client.pause(&id);
        h.advance(5 * DAY);
        h.client.resume(&id);
        expected_paused += 5 * DAY;
    }

    assert_eq!(h.get(id).paused_total, expected_paused);
    assert_eq!(
        h.client.vested_of(&id),
        400 * ONE,
        "40 days of real accrual"
    );

    h.warp_to(T0 + 100 * DAY + expected_paused);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.assert_pool_exact();
}

/// A pause that starts before the cliff and ends after it must not let the
/// cliff pass on the wall clock while the stream is frozen.
#[test]
fn pausing_across_the_cliff_defers_the_cliff_too() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 50 * DAY;
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        cliff,
        true,
        true,
        true,
    );

    h.advance(30 * DAY);
    h.client.pause(&id);

    // Wall clock crosses the cliff, but the stream clock has not.
    h.warp_to(cliff + 5 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "cliff must not open while frozen"
    );
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);

    h.client.resume(&id);
    assert_eq!(h.client.vested_of(&id), 0, "still 30 days of stream time");

    // 20 more days of real accrual reaches the 50-day cliff.
    h.advance(20 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        500 * ONE,
        "cliff opens on stream time"
    );
    h.assert_pool_exact();
}

#[test]
fn pausing_a_matured_stream_changes_nothing() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 150 * DAY);

    h.client.pause(&id);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.advance(50 * DAY);
    h.client.resume(&id);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

// --- Guards ---------------------------------------------------------------

#[test]
fn a_non_pausable_stream_cannot_be_paused_ever() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        false,
        true,
    );

    for skip in [0u64, DAY, 50 * DAY] {
        h.advance(skip);
        let err = h.client.try_pause(&id).unwrap_err().unwrap();
        assert_eq!(err, Error::NotPausable);
    }
    assert_eq!(h.get(id).status, StreamStatus::Active);
}

#[test]
fn double_pause_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.pause(&id);

    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamAlreadyPaused);
}

#[test]
fn resuming_a_stream_that_is_not_paused_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let err = h.client.try_resume(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotPaused);

    h.advance(10 * DAY);
    h.client.pause(&id);
    h.client.resume(&id);
    let err = h.client.try_resume(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotPaused);
}

#[test]
fn terminated_streams_cannot_be_paused_or_resumed() {
    let h = Harness::new();

    let cancelled = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.cancel(&cancelled);
    assert_eq!(
        h.client.try_pause(&cancelled).unwrap_err().unwrap(),
        Error::StreamTerminated,
    );
    assert_eq!(
        h.client.try_resume(&cancelled).unwrap_err().unwrap(),
        Error::StreamTerminated,
    );

    let depleted = h.create_simple(1_000 * ONE, 10 * DAY);
    h.advance(10 * DAY);
    h.client.withdraw(&depleted, &None);
    assert_eq!(
        h.client.try_pause(&depleted).unwrap_err().unwrap(),
        Error::StreamTerminated,
    );
}

/// Pausing an already-paused stream must not overwrite `paused_at` and thereby
/// silently erase paused time.
#[test]
fn a_rejected_double_pause_does_not_move_the_freeze_point() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.pause(&id);
    let freeze_point = h.get(id).paused_at;

    h.advance(20 * DAY);
    let _ = h.client.try_pause(&id);

    assert_eq!(h.get(id).paused_at, freeze_point);
    h.client.resume(&id);
    assert_eq!(h.get(id).paused_total, 20 * DAY, "no paused time lost");
}

/// Regression: a stream paused *after* maturity and then fully drained becomes
/// `Depleted`, and depletion is terminal — `resume` is rejected. If depletion
/// left `paused_at` set, the stream would be permanently stuck reporting both
/// "Depleted" and "frozen", with nothing able to clear it.
///
/// Found by the randomized sequence test in `test::invariants`.
#[test]
fn depleting_a_paused_stream_closes_out_the_pause() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 150 * DAY);
    h.client.pause(&id);
    h.advance(10 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Depleted);
    assert_eq!(s.paused_at, None, "a terminal stream must not stay frozen");
    assert_eq!(
        s.paused_total,
        10 * DAY,
        "the pause is recorded, not discarded"
    );

    // And the terminal state is coherent: no further transitions are possible.
    assert_eq!(
        h.client.try_resume(&id).unwrap_err().unwrap(),
        Error::StreamTerminated,
    );
    h.assert_pool_exact();
}
