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

    assert_eq!(h.get(id), state_after, "state must not change on failed cancel");
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
