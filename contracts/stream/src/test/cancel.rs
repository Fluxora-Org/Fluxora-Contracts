//! Stage 2 — cancellation.
//!
//! Cancellation rewrites the schedule so the stream looks like one that has
//! fully matured, which is why `withdraw` needs no special case for it. These
//! tests pin that equivalence down.

use super::common::*;
use crate::{Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Assert the three-way balance split and pool invariant after a cancel.
///
/// After cancel `deposited` is rewritten to what vested, so the refund is
/// `original - s.deposited`. The invariant:
///
///   refunded + claimable + already_withdrawn == original_deposit
///
/// No funds may be stranded or double-counted.
fn assert_split(h: &Harness, id: u64, original: i128) {
    let s = h.get(id);
    let refunded = original - s.deposited;
    let claimable = h.client.withdrawable_of(&id);
    assert_eq!(
        refunded + claimable + s.withdrawn,
        original,
        "split: refunded={refunded} + claimable={claimable} + withdrawn={} != original={original}",
        s.withdrawn,
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Existing tests (unchanged)
// ---------------------------------------------------------------------------

#[test]
fn cancel_refunds_the_unvested_remainder_and_leaves_the_rest_claimable() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    // Sender got back the 70% that had not vested.
    assert_eq!(h.balance(&h.sender), sender_before + 700 * ONE);
    // Recipient keeps the 30% they earned, still to be pulled.
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    assert_eq!(h.pool(), 300 * ONE);
    h.assert_pool_exact();

    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();
}

#[test]
fn cancel_accounts_for_what_was_already_withdrawn() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.advance(20 * DAY);
    h.client.withdraw(&id, &None); // 200
    h.advance(20 * DAY);
    h.client.cancel(&id); // vested 400, refund 600

    assert_eq!(h.balance(&h.sender), sender_before + 600 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 200 * ONE);
    h.assert_pool_exact();

    h.client.withdraw(&id, &None);
    assert_eq!(h.balance(&h.recipient), 400 * ONE);
    h.assert_pool_exact();
}

/// A cancelled stream must be frozen. No amount of elapsed time may accrue one
/// more stroop.
#[test]
fn accrual_stops_dead_at_cancellation() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    let frozen = h.client.vested_of(&id);
    assert_eq!(frozen, 300 * ONE);

    for jump in [1u64, DAY, 100 * DAY, 10 * YEAR] {
        h.advance(jump);
        assert_eq!(h.client.vested_of(&id), frozen, "after +{jump}s");
        assert_eq!(h.client.withdrawable_of(&id), frozen);
    }
    h.assert_pool_exact();
}

#[test]
fn cancel_sets_the_cancelled_status_and_collapses_the_schedule() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    let cancel_time = h.now();

    h.client.cancel(&id);
    let s = h.get(id);

    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.deposited, 300 * ONE, "deposit reduced to what vested");
    assert_eq!(s.end_time, cancel_time, "schedule collapsed onto now");
}

/// `Cancelled` is sticky: draining a cancelled stream must not relabel it as a
/// clean completion, or the indexer loses the distinction.
#[test]
fn a_drained_cancelled_stream_stays_cancelled() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.cancel(&id);
    h.client.withdraw(&id, &None);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.withdrawn, s.deposited);
}

// --- Boundaries -----------------------------------------------------------

/// Cancelling one second in leaves a schedule of length one second. The
/// collapsed-schedule trick must not divide by zero or mis-clamp.
#[test]
fn cancel_one_second_after_creation() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.advance(1);
    h.client.cancel(&id);

    let one_second_worth = 1_000 * ONE / (100 * DAY) as i128;
    assert_eq!(h.client.withdrawable_of(&id), one_second_worth);
    assert_eq!(
        h.balance(&h.sender),
        sender_before + 1_000 * ONE - one_second_worth
    );
    h.assert_pool_exact();
}

/// Cancelling at the very instant of creation is the degenerate case: zero
/// elapsed, zero duration after collapse. This is the division-by-zero trap.
#[test]
fn cancel_at_the_instant_of_creation_refunds_everything() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);

    assert_eq!(h.balance(&h.sender), sender_before + 1_000 * ONE);
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.get(id).deposited, 0);
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();

    // The collapsed zero-length schedule must stay readable, not panic.
    // Status is Cancelled with nothing left → StreamTerminated, not the
    // live-stream NothingToWithdraw path.
    h.advance(200 * DAY);
    assert_eq!(h.client.vested_of(&id), 0);
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
}

/// Cancelling before the stream even opens must not produce a negative-length
/// schedule.
#[test]
fn cancel_before_the_start_time() {
    let h = Harness::new();
    let start = h.now() + 30 * DAY;
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        true,
    );
    let sender_before = h.balance(&h.sender);

    h.advance(DAY);
    h.client.cancel(&id);

    let s = h.get(id);
    assert!(s.end_time >= s.start_time, "schedule must not invert");
    assert_eq!(h.balance(&h.sender), sender_before + 1_000 * ONE);
    assert_eq!(h.client.vested_of(&id), 0);
    h.assert_pool_exact();
}

/// Cancelling a fully-vested stream refunds nothing and takes nothing away.
#[test]
fn cancel_after_full_vesting_is_a_no_op_for_balances() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);

    assert_eq!(h.balance(&h.sender), sender_before, "nothing to refund");
    assert_eq!(h.client.withdrawable_of(&id), 1_000 * ONE);
    h.assert_pool_exact();

    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

#[test]
fn cancel_long_after_maturity_still_pays_the_recipient_in_full() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(5 * YEAR);

    h.client.cancel(&id);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

// --- Guards ---------------------------------------------------------------

#[test]
fn a_non_cancellable_stream_cannot_be_cancelled_ever() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        true,
        true,
    );

    for skip in [0u64, DAY, 50 * DAY, 200 * DAY] {
        h.advance(skip);
        let err = h.client.try_cancel(&id).unwrap_err().unwrap();
        assert_eq!(err, Error::NotCancellable);
    }
    assert_eq!(h.pool(), 1_000 * ONE, "funds never left the pool");
}

#[test]
fn cancelling_twice_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.cancel(&id);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    h.assert_pool_exact();
}

#[test]
fn a_depleted_stream_cannot_be_cancelled() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
}

/// Missing stream on cancel must be a decodable contract error, not a trap.
#[test]
fn cancelling_unknown_stream_is_stream_not_found() {
    let h = Harness::new();
    let err = h.client.try_cancel(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound);
}

/// Cancelling a paused stream must settle against the frozen clock, not the
/// wall clock.
#[test]
fn cancel_while_paused_settles_at_the_frozen_clock() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.advance(30 * DAY);
    h.client.pause(&id);
    h.advance(50 * DAY); // no accrual during this
    h.client.cancel(&id);

    // Settlement is at 30 days of accrual, not 80.
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    assert_eq!(h.balance(&h.sender), sender_before + 700 * ONE);
    assert_eq!(h.get(id).paused_at, None, "pause cleared on cancel");
    h.assert_pool_exact();

    // And it stays frozen afterwards.
    h.advance(YEAR);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Issue #1837 — paused cancellation, beyond the simplest case
//
// The base suite already pins the headline scenario: pause, let the wall clock
// run, cancel, and settle at the frozen instant rather than at `now`
// (`cancel_while_paused_settles_at_the_frozen_clock`). The tests below cover
// two complementary edges the simple scenario does not reach — a withdrawal
// taken *while* paused, and several pause/resume cycles before the final pause
// — where the frozen clock must still drive every figure exactly.
// ---------------------------------------------------------------------------

/// Cancel while paused *after* a partial withdrawal from the frozen accrual.
///
/// The withdrawal leaves the pool while the clock is stopped and must be
/// accounted for separately from the refund: the three outgoing quantities
/// (refund, still-claimable tail, already-withdrawn) must partition the original
/// deposit, each measured against the freeze point rather than the wall clock
/// that kept running during the pause.
#[test]
fn cancel_while_paused_after_partial_withdrawal() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    h.advance(30 * DAY);
    h.client.pause(&id); // freeze at 30 days: 300 accrued

    // The wall clock runs on, but the payout stays fixed at the freeze point.
    h.advance(50 * DAY);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);

    // Claim part of the frozen accrual while paused. A withdrawal against
    // already-vested funds is not a clock change.
    assert_eq!(h.client.withdraw(&id, &Some(100 * ONE)), 100 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 200 * ONE);

    // More wall-clock time passes paused; settlement must ignore all of it.
    h.advance(20 * DAY);
    h.client.cancel(&id);

    // Refund is priced at the 30-day freeze point: 1000 - 300 = 700, regardless
    // of the 70 days the ledger clock advanced during the pause.
    assert_eq!(h.balance(&h.sender), sender_before + 700 * ONE);
    assert_eq!(
        h.client.withdrawable_of(&id),
        200 * ONE,
        "the unwithdrawn tail stays claimable"
    );
    let s = h.get(id);
    assert_eq!(s.deposited, 300 * ONE, "deposit rewritten to frozen vested");
    assert_eq!(s.withdrawn, 100 * ONE);
    assert_eq!(s.paused_at, None, "pause cleared on cancel");

    // refunded=700 + claimable=200 + withdrawn=100 == original 1000.
    assert_split(&h, id, 1_000 * ONE);

    // Stays frozen, and the recipient can drain the 200 tail exactly.
    h.advance(YEAR);
    assert_eq!(h.client.withdrawable_of(&id), 200 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 200 * ONE);
    assert_eq!(h.balance(&h.recipient), 300 * ONE, "100 + 200 withdrawn total");
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();
}

/// Cancel while paused after more than one pause/resume cycle.
///
/// Only *accrued* wall-clock time counts: both paused intervals land in
/// `paused_total` and never in vesting, so the refund, the frozen `end_time`,
/// and the claimable tail are all the same figure a single uninterrupted
/// 30-day accrual would produce.
#[test]
fn cancel_while_paused_after_multiple_pause_resume_cycles() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let sender_before = h.balance(&h.sender);

    // First cycle: 20 days of accrual, then 30 days frozen.
    h.advance(20 * DAY);
    h.client.pause(&id);
    h.advance(30 * DAY);
    h.client.resume(&id);
    assert_eq!(h.get(id).paused_total, 30 * DAY);

    // Second accrual window: 10 more days, then pause for good.
    h.advance(10 * DAY);
    h.client.pause(&id);
    assert_eq!(
        h.client.vested_of(&id),
        300 * ONE,
        "30 accrued days, not the 60 wall-clock days since T0"
    );

    // A long final pause elapses; cancel without resuming.
    h.advance(45 * DAY);
    h.client.cancel(&id);

    // Settlement honours only the 30 days of unpaused accrual.
    assert_eq!(h.balance(&h.sender), sender_before + 700 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    let s = h.get(id);
    assert_eq!(s.deposited, 300 * ONE);
    assert_eq!(s.paused_at, None, "pause cleared on cancel");
    assert_eq!(
        s.end_time,
        T0 + 30 * DAY,
        "schedule collapses onto accrued time, not the wall clock"
    );

    assert_split(&h, id, 1_000 * ONE);

    // Still frozen after a further wall-clock jump.
    h.advance(YEAR);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Explicit balance-split invariant tests
//
// Each asserts: refunded + claimable + already_withdrawn == original_deposit
// No funds stranded or double-counted. Cancellation is terminal.
// ---------------------------------------------------------------------------

/// Before start: sender recovers everything, recipient has nothing to claim.
#[test]
fn split_before_start_sender_gets_everything() {
    let h = Harness::new();
    let start = h.now() + 10 * DAY;
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        true,
    );

    h.advance(5 * DAY); // still pre-start
    h.client.cancel(&id);

    // refunded=1000, claimable=0, withdrawn=0
    assert_split(&h, id, 1_000 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
}

/// Mid-accrual, no prior withdrawals: unvested goes to sender, vested stays
/// claimable.
#[test]
fn split_during_accrual_no_prior_withdrawals() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(40 * DAY);
    h.client.cancel(&id);

    // 40% vested → refunded=600, claimable=400, withdrawn=0
    assert_split(&h, id, 1_000 * ONE);

    // Verify the event carried the right settlement figures via stream state.
    let s = h.get(id);
    assert_eq!(s.deposited, 400 * ONE, "deposited rewritten to vested");
    assert_eq!(s.withdrawn, 0);
}

/// Mid-accrual after a partial withdrawal: the split accounts for what already
/// left the pool.
#[test]
fn split_after_partial_withdrawal() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    h.client.withdraw(&id, &Some(300 * ONE)); // 300 of 500 vested pulled

    h.advance(10 * DAY); // now 60% vested
    h.client.cancel(&id);

    // refunded=400, claimable=300, withdrawn=300 → sum=1000
    assert_split(&h, id, 1_000 * ONE);

    let s = h.get(id);
    assert_eq!(s.deposited, 600 * ONE, "deposited rewritten to vested");
    assert_eq!(s.withdrawn, 300 * ONE);
}

/// Second cancel is terminal: error fires, split from the first cancel is
/// unchanged, nothing moves.
#[test]
fn split_repeated_cancel_is_terminal_state_unchanged() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    assert_split(&h, id, 1_000 * ONE);

    let state_after = h.get(id);
    let pool_after = h.pool();

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    assert_eq!(
        h.get(id),
        state_after,
        "state must not change on failed cancel"
    );
    assert_eq!(h.pool(), pool_after);
    h.assert_pool_exact();
}

/// After cancel, recipient drains the tail — pool reaches zero and status stays
/// Cancelled.
#[test]
fn split_holds_after_recipient_drains_the_tail() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(25 * DAY);
    h.client.cancel(&id);
    assert_split(&h, id, 1_000 * ONE);

    h.client.withdraw(&id, &None);

    assert_eq!(h.pool(), 0);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Cancel at exactly `start_time` (Issue #1694)
//
// `accrual::vested` documents that a cancel landing on the schedule's first
// instant collapses the duration to zero and rewrites `deposited` to the
// vested amount, returning it in full rather than dividing by zero. This is
// the path where rounding and the rewrite interact, so exact conservation is
// pinned here across deposits that divide the schedule evenly and ones that
// leave a remainder.
// ---------------------------------------------------------------------------

/// Deposits against a 100-day schedule (8_640_000 seconds).
///
/// `8_640_000` is exactly one stroop per second; `8_640_001` leaves a
/// remainder of one stroop; the rest are deliberately uneven so no test can
/// pass by way of an exact division. All are >= the schedule length, which is
/// the creation-time dust-rate floor.
const START_TIME_DEPOSITS: [i128; 5] = [
    8_640_000,
    8_640_001,
    9_999_999_999,
    1_000 * ONE,
    123_456_789_013,
];

/// Cancelling on the schedule's first instant returns every deposit exactly:
/// the sender is made whole, the recipient receives nothing, and no residue
/// stays attributable to the stream.
#[test]
fn cancel_at_start_time_returns_every_deposit_and_leaves_no_residue() {
    for deposit in START_TIME_DEPOSITS {
        let h = Harness::new();
        let id = h.create_simple(deposit, 100 * DAY);
        let sender_before = h.balance(&h.sender);
        let recipient_before = h.balance(&h.recipient);

        // Nothing has vested on the first instant, so the whole deposit is the
        // refundable amount — the precondition the cancel path relies on.
        assert_eq!(h.client.vested_of(&id), 0, "deposit {deposit}");
        assert_eq!(
            h.client.refundable_of(&id),
            deposit,
            "deposit {deposit}: the entire deposit must be refundable",
        );

        h.client.cancel(&id);

        // The sender receives the entire deposit back, exactly.
        assert_eq!(
            h.balance(&h.sender),
            sender_before + deposit,
            "deposit {deposit}: full refund",
        );
        // The recipient receives nothing.
        assert_eq!(
            h.balance(&h.recipient),
            recipient_before,
            "deposit {deposit}: the recipient must receive nothing",
        );
        assert_eq!(h.client.withdrawable_of(&id), 0, "deposit {deposit}");

        // No residue remains attributable to the stream: the rewritten deposit
        // is zero, the schedule collapsed onto start_time, and the pool is empty.
        let s = h.get(id);
        assert_eq!(s.deposited, 0, "deposit {deposit}: no residue in storage");
        assert_eq!(s.withdrawn, 0, "deposit {deposit}");
        assert_eq!(
            s.end_time, s.start_time,
            "deposit {deposit}: zero-length schedule, not a negative one",
        );
        assert_eq!(h.pool(), 0, "deposit {deposit}: pool must be empty");
        assert_split(&h, id, deposit);

        // And there is nothing left for the recipient to claim afterwards.
        let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
        assert_eq!(err, Error::StreamTerminated, "deposit {deposit}");
    }
}

/// The same instant for a stream whose `start_time` is not the creation time:
/// warping to the first instant and cancelling must refund in full, with no
/// partial accrual creeping in from the wait.
#[test]
fn cancel_exactly_when_a_delayed_stream_opens_refunds_in_full() {
    let h = Harness::new();
    let start = h.now() + 10 * DAY;
    let id = h.create(1_000 * ONE, start, start + 7 * DAY, start, true, true, true);
    let sender_before = h.balance(&h.sender);
    let recipient_before = h.balance(&h.recipient);

    h.warp_to(start);
    assert_eq!(h.now(), start, "the cancel must land exactly on start_time");

    h.client.cancel(&id);

    assert_eq!(h.balance(&h.sender), sender_before + 1_000 * ONE);
    assert_eq!(h.balance(&h.recipient), recipient_before);
    assert_eq!(h.get(id).deposited, 0, "no residue");
    assert_eq!(h.pool(), 0);
    assert_split(&h, id, 1_000 * ONE);
}

// ---------------------------------------------------------------------------
// Issue #1878 — cancellation's end-time rewrite settles the schedule
//
// `cancel` collapses the schedule onto the cancellation instant:
// `deposited` drops to what vested right now and `end_time` is pulled back
// to `settle_at = max(stream_time(now), start_time)`. That collapse is what
// makes every later `vested` call return the final figure, and the
// zero-duration branch in `accrual::vested` exists for the case where the
// collapse lands exactly on `start_time`.
//
// This test cancels at each of the four schedule points and asserts, at the
// same instant before and after the collapse:
//   * `end_time == settle_at` (the rewrite lands on the cancel instant,
//     clamped at `start_time` so a pre-start cancel never inverts);
//   * `deposited == vested_before` and `vested_after == vested_before`
//     (the collapse itself changes nothing the recipient earned);
//   * exact conservation: `refund + vested_before == deposit`.
// ---------------------------------------------------------------------------

/// Cancel at all four schedule points; the collapse settles onto the cancel
/// instant, leaves `vested` unchanged, and conserves exactly.
#[test]
fn cancel_collapse_settles_schedule_at_cancel_instant_and_conserves() {
    const DEPOSIT: i128 = 1_000 * ONE;
    const DURATION: u64 = 100 * DAY;

    // Each case runs in a fresh harness so pool and balance accounting stays
    // isolated. The schedule is identical in all four: no cliff, 100 days.
    struct Case {
        label: &'static str,
        /// Advance from T0 to the cancel instant. `None` means "stay at T0",
        /// which is before `start = T0 + 10 days`.
        cancel_at: Option<u64>,
        /// `start` offset from T0 (always 10 days here).
        start_offset: u64,
        expected_vested: i128,
        /// Where `end_time` must land after the collapse.
        expect_clamped_to_start: bool,
    }

    let start_offset = 10 * DAY;
    let case_defs = [
        Case {
            label: "before start_time",
            cancel_at: None, // T0, still 10 days before start
            start_offset,
            expected_vested: 0,
            expect_clamped_to_start: true,
        },
        Case {
            label: "exactly at start_time",
            cancel_at: Some(start_offset), // warp to start
            start_offset,
            expected_vested: 0,
            expect_clamped_to_start: true,
        },
        Case {
            label: "mid-schedule",
            cancel_at: Some(start_offset + 50 * DAY),
            start_offset,
            expected_vested: 500 * ONE,
            expect_clamped_to_start: false,
        },
        Case {
            label: "after end_time",
            cancel_at: Some(start_offset + DURATION + 10 * DAY),
            start_offset,
            expected_vested: DEPOSIT,
            expect_clamped_to_start: false,
        },
    ];

    for case in case_defs {
        let h = Harness::new();
        let label = case.label;
        let start = T0 + case.start_offset;
        let end = start + DURATION;
        let id = h.create(DEPOSIT, start, end, start, true, true, true);
        let sender_before = h.balance(&h.sender);
        let recipient_before = h.balance(&h.recipient);

        if let Some(offset) = case.cancel_at {
            h.warp_to(T0 + offset);
        }
        let cancel_time = h.now();

        // Pre-cancel figures at the exact cancel instant. `refundable_before`
        // is the amount the cancel path must hand back to the sender.
        let vested_before = h.client.vested_of(&id);
        let refundable_before = h.client.refundable_of(&id);
        assert_eq!(
            vested_before, case.expected_vested,
            "{label}: vested_before"
        );
        assert_eq!(
            refundable_before,
            DEPOSIT - case.expected_vested,
            "{label}: entire unvested remainder must be refundable"
        );

        h.client.cancel(&id);

        // The clock must not have moved under the cancel itself.
        assert_eq!(h.now(), cancel_time, "{label}: cancel moved the clock");

        let s = h.get(id);
        // settle_at = max(stream_time, start_time); unpaused, so stream_time
        // is the cancel instant, clamped at start for pre-start cancels.
        let settle_at = cancel_time.max(start);
        assert_eq!(
            s.end_time, settle_at,
            "{label}: end_time must collapse onto the cancel instant"
        );
        assert!(
            s.end_time >= s.start_time,
            "{label}: schedule must not invert"
        );
        if case.expect_clamped_to_start {
            assert_eq!(
                s.end_time, s.start_time,
                "{label}: pre/at-start cancel clamps to a zero-length schedule"
            );
        }
        assert_eq!(s.status, StreamStatus::Cancelled, "{label}: status");

        // The rewrite drops `deposited` to what vested and nothing else.
        assert_eq!(
            s.deposited, vested_before,
            "{label}: deposited must be rewritten to vested_before"
        );

        // The collapse itself leaves `vested` unchanged at the same instant.
        // At `start_time` this reads through the zero-duration branch in
        // `accrual::vested` (duration 0 returns `deposited` in full rather
        // than dividing), so equality there pins that branch too.
        let vested_after = h.client.vested_of(&id);
        assert_eq!(
            vested_after, vested_before,
            "{label}: collapse must not change vested at the cancel instant"
        );
        if s.end_time == s.start_time {
            assert_eq!(
                vested_after, s.deposited,
                "{label}: zero-duration schedule must vest the settled deposit in full"
            );
        }

        // Exact conservation: refund + vested == original deposit, and the
        // post-cancel stream holds no further refundable remainder.
        let refund = DEPOSIT - vested_before;
        assert_eq!(
            refund + vested_before,
            DEPOSIT,
            "{label}: conservation broken — refund + vested != deposited"
        );
        assert_eq!(
            h.balance(&h.sender),
            sender_before + refund,
            "{label}: sender refund"
        );
        assert_eq!(
            h.balance(&h.recipient),
            recipient_before,
            "{label}: recipient must receive nothing at cancel"
        );
        assert_eq!(
            h.client.refundable_of(&id),
            0,
            "{label}: nothing refundable remains after settlement"
        );
        assert_eq!(
            h.client.withdrawable_of(&id),
            vested_after,
            "{label}: whole settled deposit stays claimable"
        );
        assert_split(&h, id, DEPOSIT);
    }
}
