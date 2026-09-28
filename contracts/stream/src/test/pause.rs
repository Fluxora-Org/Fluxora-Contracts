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

        assert_eq!(
            h.client.withdrawable_of(&id),
            h.client.vested_of(&id),
            "withdrawable balance must track vested balance after each resume"
        );
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

// --- State-machine: repeated transitions ------------------------------------

/// Pause/resume with an explicit state check after every transition.
/// Verifies that each step in the lifecycle leaves the stream in exactly the
/// expected state.
#[test]
fn state_machine_pause_resume_cycle_checks_every_step() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // After creation: Active, no pause state.
    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Active);
    assert_eq!(s.paused_at, None);
    assert_eq!(s.paused_total, 0);
    assert_eq!(h.client.vested_of(&id), 0);

    // Advance 30 days of stream time.
    h.advance(30 * DAY);
    assert_eq!(h.client.vested_of(&id), 300 * ONE);

    // Pause: status becomes Paused, clock freezes.
    h.client.pause(&id);
    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Paused);
    assert!(s.paused_at.is_some(), "paused_at must be set");
    assert_eq!(s.paused_total, 0);
    assert_eq!(h.client.vested_of(&id), 300 * ONE, "frozen at pause");

    // Wait 10 days while paused — accrual must not advance.
    h.advance(10 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        300 * ONE,
        "no accrual while paused"
    );
    assert_eq!(h.get(id).paused_at, s.paused_at, "freeze point unchanged");

    // Resume: status returns to Active, paused_total absorbs the interval.
    h.client.resume(&id);
    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Active);
    assert_eq!(s.paused_at, None);
    assert_eq!(s.paused_total, 10 * DAY);
    assert_eq!(h.client.vested_of(&id), 300 * ONE, "no jump on resume");

    // Accrual resumes at the same rate.
    h.advance(20 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        500 * ONE,
        "50 days of real accrual"
    );
    h.assert_pool_exact();
}

/// Multiple pause/resume cycles with an explicit state check after every
/// transition. Verifies paused_total accumulates correctly and the stream clock
/// is conserved.
#[test]
fn state_machine_repeated_cycles_accumulate_paused_total_step_by_step() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let mut total_paused = 0u64;
    let mut real_days = 0u64;

    for (cycle, pause_len) in [5u64, 10, 3, 22].into_iter().enumerate() {
        let advance_days = 10;
        h.advance(advance_days * DAY);
        real_days += advance_days;

        h.client.pause(&id);
        let s = h.get(id);
        assert_eq!(s.status, StreamStatus::Paused);
        assert_eq!(s.paused_total, total_paused);

        h.advance(pause_len * DAY);

        h.client.resume(&id);
        total_paused += pause_len * DAY;
        let s = h.get(id);
        assert_eq!(s.status, StreamStatus::Active);
        assert_eq!(s.paused_total, total_paused);
        assert_eq!(
            h.client.vested_of(&id),
            real_days as i128 * 10 * ONE,
            "cycle {cycle}: vested must equal {real_days} days of real accrual"
        );
    }

    assert_eq!(total_paused, 40 * DAY);
    h.assert_pool_exact();
}

/// Withdraw between pause/resume cycles: the recipient can claim earned funds
/// while paused and the invariant is maintained across the full sequence.
#[test]
fn state_machine_withdraw_between_pause_resume_cycles() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Cycle 1: accrue, pause, withdraw while paused, resume.
    h.advance(20 * DAY);
    h.client.pause(&id);
    assert_eq!(h.client.vested_of(&id), 200 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 200 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Paused);
    assert_eq!(h.get(id).paused_total, 0);
    h.advance(5 * DAY);
    h.client.resume(&id);
    assert_eq!(h.get(id).paused_total, 5 * DAY);
    assert_eq!(h.balance(&h.recipient), 200 * ONE);
    h.assert_pool_exact();

    // Cycle 2: accrue more, pause, withdraw again, resume.
    h.advance(30 * DAY);
    assert_eq!(h.client.vested_of(&id), 500 * ONE);
    h.client.pause(&id);
    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    assert_eq!(h.get(id).paused_total, 5 * DAY);
    h.advance(10 * DAY);
    h.client.resume(&id);
    assert_eq!(h.get(id).paused_total, 15 * DAY);
    assert_eq!(h.balance(&h.recipient), 500 * ONE);
    h.assert_pool_exact();

    // Total withdrawn matches what we pulled out.
    assert_eq!(h.get(id).withdrawn, 500 * ONE);
}

/// Cancel immediately after resume: the frozen clock is still at the pause
/// instant, so cancellation must settle at the same amount as pausing.
#[test]
fn state_machine_cancel_immediately_after_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.advance(30 * DAY);
    h.client.pause(&id);
    h.advance(50 * DAY);
    h.client.resume(&id);

    // Cancel immediately — stream clock is at day 30 (30 real + 0 after resume).
    h.client.cancel(&id);
    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.paused_at, None);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    assert_eq!(h.balance(&h.sender), sender_before + 700 * ONE);
    h.assert_pool_exact();
}

// --- State-machine: boundary timestamps -------------------------------------

/// Pause at the exact creation instant (T0): the clock freezes immediately
/// with zero accrued, and the full deposit remains refundable.
#[test]
fn state_machine_pause_at_creation_instant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.client.pause(&id);
    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Paused);
    assert_eq!(s.paused_at, Some(T0));
    assert_eq!(s.paused_total, 0);
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), 1_000 * ONE);

    // Advance a full year — nothing accrues.
    h.advance(YEAR);
    assert_eq!(h.client.vested_of(&id), 0);

    // Resume, then accrue normally from the frozen point.
    h.client.resume(&id);
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.get(id).paused_total, YEAR);

    h.advance(10 * DAY);
    assert_eq!(h.client.vested_of(&id), 100 * ONE);
    h.assert_pool_exact();
}

/// Resume after a zero-duration pause: `paused_duration == 0` must not affect
/// `paused_total` or the vesting schedule.
#[test]
fn state_machine_resume_with_zero_duration_pause() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.pause(&id);
    let vested_at_pause = h.client.vested_of(&id);
    assert_eq!(vested_at_pause, 300 * ONE);

    // Resume immediately — no wall clock elapsed.
    h.client.resume(&id);
    assert_eq!(h.client.vested_of(&id), vested_at_pause, "no jump");
    assert_eq!(h.get(id).paused_total, 0, "zero-length pause not recorded");

    // Accrual resumes from the exact frozen point.
    h.advance(10 * DAY);
    assert_eq!(h.client.vested_of(&id), 400 * ONE);
    h.assert_pool_exact();
}

/// Pause at the exact maturity boundary: vested already equals deposited.
/// The clock freezes at maturity, and the stream remains fully matured after
/// resume.
#[test]
fn state_machine_pause_at_exact_maturity_boundary() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    h.client.pause(&id);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    h.advance(YEAR);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    h.client.resume(&id);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    assert_eq!(h.get(id).paused_total, YEAR);

    // Still fully matured after resume.
    h.advance(30 * DAY);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

/// Resume at the original end_time: during the pause, `stream_time` is frozen
/// before the end, but wall clock is at or past the end. After resume the
/// clock advances through the remaining schedule.
#[test]
fn state_machine_resume_at_original_end_time() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let original_end = T0 + 100 * DAY;

    h.advance(30 * DAY);
    h.client.pause(&id);

    // Warp to the original end time while paused.
    h.warp_to(original_end);
    assert_eq!(
        h.client.vested_of(&id),
        300 * ONE,
        "must not accrue during pause even at the original end"
    );

    h.client.resume(&id);
    assert_eq!(h.client.vested_of(&id), 300 * ONE, "no jump on resume");
    assert_eq!(h.get(id).paused_total, 70 * DAY);

    // 70 more days of real accrual reaches the full deposit.
    h.warp_to(original_end + 70 * DAY);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.assert_pool_exact();
}

/// Pause across the original end time: the stream is still running when
/// paused, but the pause pushes the effective end forward. After resume the
/// remaining schedule completes.
#[test]
fn state_machine_pause_spans_original_end_time() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let original_end = T0 + 100 * DAY;

    // At day 80, pause — stream has 20 days left.
    h.advance(80 * DAY);
    h.client.pause(&id);
    assert_eq!(h.client.vested_of(&id), 800 * ONE);

    // Warp past the original end while paused.
    h.warp_to(original_end + 30 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        800 * ONE,
        "must not accrue past original end while paused"
    );

    h.client.resume(&id);
    assert_eq!(h.get(id).paused_total, 50 * DAY);

    // 20 more days completes the remaining schedule.
    h.warp_to(original_end + 50 * DAY);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.assert_pool_exact();
}

/// Pause after the stream has already matured and been partially withdrawn.
/// The clock freezes at a point where the remaining balance is less than the
/// full deposit.
#[test]
fn state_machine_pause_after_partial_withdrawal() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    assert_eq!(h.client.vested_of(&id), 500 * ONE);
    h.client.withdraw(&id, &None); // 500 withdrawn
    assert_eq!(h.get(id).withdrawn, 500 * ONE);

    h.client.pause(&id);
    assert_eq!(h.client.vested_of(&id), 500 * ONE);

    h.advance(20 * DAY);
    assert_eq!(h.client.vested_of(&id), 500 * ONE);

    h.client.resume(&id);
    assert_eq!(h.get(id).paused_total, 20 * DAY);

    // Remaining 50 days of accrual completes the deposit.
    h.advance(50 * DAY);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 500 * ONE);
    h.assert_pool_exact();
}

// --- State-machine: invalid transition sequences ---------------------------

/// Every non-Active state must reject `pause`.
#[test]
fn state_machine_pause_rejects_all_invalid_targets() {
    // Already paused → StreamAlreadyPaused
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 100 * DAY);
        h.advance(10 * DAY);
        h.client.pause(&id);
        assert_eq!(
            h.client.try_pause(&id).unwrap_err().unwrap(),
            Error::StreamAlreadyPaused,
        );
    }
    // Cancelled → StreamTerminated
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 100 * DAY);
        h.advance(10 * DAY);
        h.client.cancel(&id);
        assert_eq!(
            h.client.try_pause(&id).unwrap_err().unwrap(),
            Error::StreamTerminated,
        );
    }
    // Depleted → StreamTerminated
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 10 * DAY);
        h.advance(10 * DAY);
        h.client.withdraw(&id, &None);
        assert_eq!(
            h.client.try_pause(&id).unwrap_err().unwrap(),
            Error::StreamTerminated,
        );
    }
    // Non-pausable → NotPausable
    {
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
        h.advance(10 * DAY);
        assert_eq!(
            h.client.try_pause(&id).unwrap_err().unwrap(),
            Error::NotPausable,
        );
    }
}

/// Every non-Paused state must reject `resume`.
#[test]
fn state_machine_resume_rejects_all_invalid_targets() {
    // Active (fresh) → StreamNotPaused
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 100 * DAY);
        assert_eq!(
            h.client.try_resume(&id).unwrap_err().unwrap(),
            Error::StreamNotPaused,
        );
    }
    // Active (after previous pause/resume) → StreamNotPaused
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 100 * DAY);
        h.advance(10 * DAY);
        h.client.pause(&id);
        h.client.resume(&id);
        assert_eq!(
            h.client.try_resume(&id).unwrap_err().unwrap(),
            Error::StreamNotPaused,
        );
    }
    // Cancelled → StreamTerminated
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 100 * DAY);
        h.advance(10 * DAY);
        h.client.cancel(&id);
        assert_eq!(
            h.client.try_resume(&id).unwrap_err().unwrap(),
            Error::StreamTerminated,
        );
    }
    // Depleted → StreamTerminated
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 10 * DAY);
        h.advance(10 * DAY);
        h.client.withdraw(&id, &None);
        assert_eq!(
            h.client.try_resume(&id).unwrap_err().unwrap(),
            Error::StreamTerminated,
        );
    }
}

/// Pause-after-cancel must fail and leave the stream settled. A cancelled
/// stream with funds already returned to the sender must not become pausable.
#[test]
fn state_machine_pause_after_cancel_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamTerminated,
    );

    // Stream is still Cancelled, balances unchanged.
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    assert_eq!(h.balance(&h.sender), sender_before + 700 * ONE);
    h.assert_pool_exact();
}

/// Cancel-after-pause must settle against the frozen clock (not wall clock)
/// and clear the pause state. This covers the transition Paused → Cancelled.
#[test]
fn state_machine_cancel_after_pause_clears_pause_state() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.advance(30 * DAY);
    h.client.pause(&id);
    h.advance(100 * DAY);

    // Cancel while paused — settles at frozen clock (day 30), not day 130.
    h.client.cancel(&id);
    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.paused_at, None, "pause state cleared by cancel");
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    assert_eq!(h.balance(&h.sender), sender_before + 700 * ONE);

    // Subsequent pause attempt must fail — stream is terminal.
    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamTerminated,
    );
    h.assert_pool_exact();
}

/// Paused duration is exactly conserved across resume: the total wall-clock
/// time spent paused is recorded in `paused_total`, not accumulated error.
#[test]
fn state_machine_paused_duration_is_exactly_conserved() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Run many short cycles with varying pause lengths.
    let pause_lengths: [u64; 8] = [1, 3600, DAY, 3 * DAY, 7 * DAY, 30 * DAY, 90 * DAY, YEAR];
    let mut expected_total = 0u64;

    for pause_len in pause_lengths {
        h.advance(DAY);
        h.client.pause(&id);
        h.advance(pause_len);
        h.client.resume(&id);
        expected_total += pause_len;
        assert_eq!(
            h.get(id).paused_total,
            expected_total,
            "paused_total must be exactly {expected_total} after pausing for {pause_len}"
        );
    }

    // The schedule stretches by exactly the cumulative paused duration.
    h.warp_to(T0 + 100 * DAY + expected_total);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.assert_pool_exact();
}

/// A stream paused for the entirety of its lifetime still has the correct
/// accrual after resume: zero real accrual while paused, then normal accrual
/// from the frozen point.
#[test]
fn state_machine_paused_for_full_lifetime_resumes_normally() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.client.pause(&id);

    // Warp far past the original end — no accrual while paused.
    h.advance(YEAR);
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.get(id).paused_at, Some(T0));

    h.client.resume(&id);
    assert_eq!(h.client.vested_of(&id), 0, "no jump on resume");
    assert_eq!(h.get(id).paused_total, YEAR);

    // Accrue normally from the frozen point.
    h.advance(50 * DAY);
    assert_eq!(h.client.vested_of(&id), 500 * ONE);
    h.assert_pool_exact();
}

// --- Issue #1832 — resume exactly at end_time ----------------------------------

/// Resuming at exactly the wall-clock `end_time` is the specific case where
/// `paused_total` arithmetic could push `elapsed` past `duration` if the
/// stream-clock formula is wrong.
///
/// Scenario: a 100-day stream paused at day 30 (300 tokens vested, 700 still
/// locked). The pause holds until the wall clock reaches `end_time`. At that
/// point, `paused_total = 70 days`, so:
///
/// ```text
/// stream_time(end_time) = end_time - paused_total
///                       = start + 100d - 70d
///                       = start + 30d
/// elapsed               = 30 days   (must not overflow to > duration)
/// vested                = 300 ONE
/// refundable            = 700 ONE   (conservation exact)
/// ```
///
/// The stream is still `Active` after resume — it has 70 days of stream-time
/// left to run. Only after another 70 days of wall-clock time does it reach
/// full maturity.
///
/// Acceptance criteria (issue #1832):
///
/// * Scenario exercised end-to-end through the public ABI.
/// * Funds conservation `vested + refundable == deposited` asserted at the
///   resume instant.
/// * `resumed` event carries the correct `paused_duration` and `paused_total`.
/// * Final stream state (`Depleted`) verified after full withdrawal.
/// * `assert_pool_exact` called at the end.
/// * The test fails if the behaviour it pins is changed.
#[test]
fn resume_exactly_at_end_time_conservation_and_final_state() {
    use crate::events::Resumed;
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::Event as _;

    let h = Harness::new();
    let duration = 100 * DAY;
    let deposit = 1_000 * ONE;

    // Create a 100-day pausable stream. start_time == T0.
    let start = h.now();
    let end = start + duration;
    let id = h.create(deposit, start, end, start, true, true, true);

    // Advance 30 days — 300 ONE vested, 700 ONE locked.
    h.advance(30 * DAY);
    assert_eq!(h.client.vested_of(&id), 300 * ONE);

    // Pause here. Stream clock freezes at day 30.
    h.client.pause(&id);
    let pause_instant = h.now();
    assert_eq!(pause_instant, start + 30 * DAY);
    assert_eq!(h.get(id).paused_at, Some(pause_instant));

    // Advance the wall clock to exactly the original end_time while paused.
    // Accrual must not move — stream clock is frozen.
    h.warp_to(end);
    assert_eq!(h.now(), end, "wall clock must be exactly at end_time");
    assert_eq!(
        h.client.vested_of(&id),
        300 * ONE,
        "no accrual while paused even at original end_time"
    );

    // Resume at exactly end_time. This is the arithmetic boundary:
    //   paused_duration = end_time - pause_instant = 70 days
    //   paused_total    = 70 days (was 0 before)
    //   stream_time     = end_time - paused_total = start + 30d (not beyond end_time)
    //   elapsed         = 30 days  (not >= duration)
    h.client.resume(&id);

    // --- Event assertion ---------------------------------------------------
    // Build the expected Resumed event from ground truth (post-call stream
    // state) and compare byte-for-byte, matching the pattern in
    // withdraw_events.rs / cancel_events.rs.
    {
        let s_after = h.get(id);
        let expected_event = Resumed {
            stream_id: id,
            sender: h.sender.clone(),
            // paused_duration = wall-clock delta from pause_instant to now
            paused_duration: 70 * DAY,
            // paused_total is the post-resume cumulative value stored in stream
            paused_total: s_after.paused_total,
        };

        let published: std::vec::Vec<_> = h
            .env
            .events()
            .all()
            .filter_by_contract(&h.contract_id)
            .events()
            .to_vec();

        assert_eq!(
            published,
            std::vec![expected_event.to_xdr(&h.env, &h.contract_id)],
            "resume must emit exactly one Resumed event with the correct payload"
        );

        // Also verify the individual field against the expected value.
        assert_eq!(
            s_after.paused_total,
            70 * DAY,
            "paused_total must equal the 70-day pause duration"
        );
    }

    // --- Post-resume stream state ------------------------------------------
    let s = h.get(id);
    assert_eq!(
        s.status,
        StreamStatus::Active,
        "stream must be Active after resume"
    );
    assert_eq!(s.paused_at, None, "paused_at must be cleared");
    assert_eq!(
        s.paused_total,
        70 * DAY,
        "paused_total must absorb the 70-day pause"
    );

    // --- Conservation at the resume instant --------------------------------
    // Invariant I4: vested + refundable == deposited, exactly.
    let vested_now = h.client.vested_of(&id);
    let refundable_now = h.client.refundable_of(&id);
    assert_eq!(
        vested_now,
        300 * ONE,
        "vested must equal the amount accrued before the pause"
    );
    assert_eq!(
        refundable_now,
        700 * ONE,
        "refundable must be the remainder"
    );
    assert_eq!(
        vested_now + refundable_now,
        deposit,
        "vested + refundable must equal deposited (conservation I4)"
    );

    // withdrawable == vested - withdrawn == 300 ONE (nothing withdrawn yet)
    assert_eq!(
        h.client.withdrawable_of(&id),
        300 * ONE,
        "withdrawable must equal vested at this point"
    );

    // --- Partial withdrawal while the stream still has time left -----------
    // The stream is Active but the clock is at stream-day 30. Withdraw what
    // is available now to prove the stream is not stuck in a terminal state.
    let paid = h.client.withdraw(&id, &None);
    assert_eq!(
        paid,
        300 * ONE,
        "first withdrawal must be the pre-pause accrual"
    );
    assert_eq!(h.balance(&h.recipient), 300 * ONE);
    assert_eq!(
        h.get(id).status,
        StreamStatus::Active,
        "stream remains Active — 70 days of stream-time remain"
    );
    h.assert_pool_exact();

    // --- Accrual resumes correctly after the resume call -------------------
    // Advancing 35 more wall-clock days adds 35 days of stream time (no more
    // pauses), so vested goes from 300 to 650 ONE.
    h.advance(35 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        650 * ONE,
        "vested must advance at the original rate after resume"
    );

    // --- Stream fully matures at end_time + paused_total -------------------
    // The stretched end is end_time + paused_total = start + 100d + 70d = start + 170d.
    // Current wall clock: end_time + 35d = start + 135d.  35d remain.
    h.advance(35 * DAY);
    // Wall clock is now start + 170d.  Stream clock = 170d - 70d = 100d == duration.
    assert_eq!(
        h.client.vested_of(&id),
        1_000 * ONE,
        "stream must be fully vested at the stretched end time"
    );
    assert_eq!(
        h.client.withdrawable_of(&id),
        700 * ONE,
        "withdrawable must be the remaining 700 ONE"
    );
    h.assert_pool_exact();

    // --- Final withdrawal drains the stream to Depleted --------------------
    let final_paid = h.client.withdraw(&id, &None);
    assert_eq!(
        final_paid,
        700 * ONE,
        "final withdrawal must collect the remaining deposit"
    );
    assert_eq!(
        h.balance(&h.recipient),
        1_000 * ONE,
        "recipient must hold the full deposit"
    );

    let s_final = h.get(id);
    assert_eq!(
        s_final.status,
        StreamStatus::Depleted,
        "stream must become Depleted after the last withdrawal"
    );
    assert_eq!(s_final.withdrawn, deposit, "withdrawn must equal deposited");
    assert_eq!(
        s_final.paused_at,
        None,
        "no open pause on a terminal stream"
    );

    // Pool must be empty — every token has been paid to the recipient.
    h.assert_pool_exact();
}

/// Sharper variant of the end_time resume: the stream is paused at
/// `start_time` itself (zero stream-time elapsed) and the resume call lands
/// exactly at `end_time`. This maximises `paused_total` to `duration`, so
/// `stream_time(end_time) == start_time` and `elapsed == 0`.
///
/// Guards the saturating-subtraction boundary: `paused_total == duration`
/// must yield `stream_time == start_time`, not an underflow or wrap.
#[test]
fn resume_at_end_time_with_full_duration_paused() {
    let h = Harness::new();
    let duration = 100 * DAY;
    let deposit = 1_000 * ONE;

    let start = h.now();
    let end = start + duration;
    let id = h.create(deposit, start, end, start, true, true, true);

    // Pause immediately at creation — zero stream-time elapsed.
    h.client.pause(&id);
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.get(id).paused_at, Some(start));

    // Advance exactly to end_time.
    h.warp_to(end);
    assert_eq!(h.client.vested_of(&id), 0, "no accrual during full pause");

    // Resume. paused_total = end_time - start = duration = 100 days.
    // stream_time(end) = end - paused_total = start. elapsed = 0.
    h.client.resume(&id);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Active);
    assert_eq!(s.paused_at, None);
    assert_eq!(s.paused_total, duration);

    // Conservation: vested(0) + refundable(deposited) == deposited.
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), deposit);
    assert_eq!(
        h.client.vested_of(&id) + h.client.refundable_of(&id),
        deposit,
        "conservation must hold immediately after resume"
    );

    // The stream must run its full 100 days from the resumed point.
    h.advance(duration);
    assert_eq!(
        h.client.vested_of(&id),
        deposit,
        "stream fully vests after the complete duration from the resume point"
    );
    assert_eq!(h.client.withdraw(&id, &None), deposit);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    h.assert_pool_exact();
}
