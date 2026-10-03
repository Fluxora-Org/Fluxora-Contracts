//! Issue #1817 — the recipient acceptance gate.
//!
//! `create_stream_pending` creates a stream in `Pending`: the sender's deposit
//! is escrowed, but nothing accrues and nothing can be paid until the recipient
//! either accepts (`accept_stream`) or declines (`decline_stream`).
//!
//! The contract is designed so the pending path *is* the default creation path
//! plus a gate. These tests pin both halves of that claim:
//!
//! * `create_stream` is unchanged — still `Active`, still accruing;
//! * a pending stream behaves exactly like a live one except that
//!   `accrual::vested` returns zero, which is why `vested_of`, `withdrawable_of`
//!   and `refundable_of` all report the pre-start picture.
//!
//! The acceptance criterion from the issue is
//! [`declining_a_pending_stream_refunds_the_sender_in_full_with_no_accrual`]:
//! create a pending stream, decline it, and assert a full refund with no
//! accrual.

use super::common::*;
use crate::{Error, StreamStatus};

// ---------------------------------------------------------------------------
// Creation: opt-in, and the default is untouched
// ---------------------------------------------------------------------------

/// `create_stream_pending` escrows the deposit immediately but accrues nothing.
#[test]
fn create_stream_pending_escrows_but_does_not_accrue() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_pending(deposit, duration);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Pending, "created pending");
    assert_eq!(s.deposited, deposit, "deposit escrowed at creation");
    assert_eq!(s.withdrawn, 0, "nothing withdrawn yet");

    // The deposit really left the sender and is pooled: a pending stream is
    // funded, not merely promised.
    assert_eq!(h.balance(&h.sender), 1_000_000 * ONE - deposit);
    assert_eq!(h.pool(), deposit);
    h.assert_pool_exact();

    // Wall-clock time passes. A pending stream's clock does not run.
    h.advance(50 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "a pending stream must not accrue"
    );
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(
        h.client.refundable_of(&id),
        deposit,
        "the whole deposit stays refundable while pending"
    );
    h.assert_pool_exact();
}

/// The default creation path is unchanged: `create_stream` still yields an
/// `Active`, accruing stream.
#[test]
fn create_stream_default_is_still_active_and_accrues() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    assert_eq!(h.get(id).status, StreamStatus::Active);
    h.advance(50 * DAY);
    assert_eq!(h.client.vested_of(&id), 500 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 500 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Decline — the acceptance criterion from the issue
// ---------------------------------------------------------------------------

/// **Issue #1817 validation.** Create a pending stream, decline it, and assert
/// a full refund with no accrual.
#[test]
fn declining_a_pending_stream_refunds_the_sender_in_full_with_no_accrual() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let sender_before = h.balance(&h.sender);
    let recipient_before = h.balance(&h.recipient);
    let id = h.create_pending(deposit, 100 * DAY);

    // Wait well past several accrual points. A pending stream vests nothing, so
    // the decline must refund the *whole* deposit, not merely the unvested part.
    h.advance(50 * DAY);
    assert_eq!(h.client.vested_of(&id), 0, "no accrual while pending");

    h.client.decline_stream(&id);

    let s = h.get(id);
    assert_eq!(
        s.status,
        StreamStatus::Declined,
        "declined is its own state"
    );
    assert_eq!(s.withdrawn, 0, "the recipient kept nothing");
    assert_eq!(s.deposited, 0, "the stream is settled");

    assert_eq!(
        h.balance(&h.sender),
        sender_before,
        "sender refunded in full"
    );
    assert_eq!(
        h.balance(&h.recipient),
        recipient_before,
        "recipient was paid nothing"
    );
    assert_eq!(h.pool(), 0, "the pool is drained");
    h.assert_pool_exact();
}

/// A declined stream is terminal: no lifecycle operation may revive it.
#[test]
fn declined_stream_is_terminal() {
    let h = Harness::new();
    let id = h.create_pending(1_000 * ONE, 100 * DAY);
    h.client.decline_stream(&id);

    assert_eq!(
        h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_cancel(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_top_up(&id, &(10 * ONE)).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    // Acceptance is spent; the gate cannot be re-entered.
    assert_eq!(
        h.client.try_accept_stream(&id).unwrap_err().unwrap(),
        Error::StreamNotPending
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Accept — the clock starts at acceptance
// ---------------------------------------------------------------------------

/// Accepting rebases the schedule onto the acceptance instant while preserving
/// the authored duration and cliff offset.
#[test]
fn accepting_starts_the_clock_and_preserves_duration_and_cliff_offset() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let start = h.now();
    let duration = 100 * DAY;
    let cliff_offset = 10 * DAY;
    let id = h.create_pending_full(
        deposit,
        start,
        start + duration,
        start + cliff_offset,
        true,
        true,
        true,
    );

    // A week passes before the recipient answers.
    h.advance(7 * DAY);
    h.client.accept_stream(&id);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Active, "accepted");
    assert_eq!(
        s.start_time,
        start + 7 * DAY,
        "the clock starts at the acceptance instant"
    );
    assert_eq!(s.end_time, s.start_time + duration, "duration preserved");
    assert_eq!(
        s.cliff_time,
        s.start_time + cliff_offset,
        "cliff offset preserved"
    );

    // Nothing was credited for the week spent pending.
    assert_eq!(h.client.vested_of(&id), 0);

    // Half the (rebased) schedule later, half the deposit is vested. The cliff
    // sits at +10 days, well before the +50-day midpoint.
    h.advance(duration / 2);
    assert_eq!(h.client.vested_of(&id), deposit / 2);
    assert_eq!(h.client.withdrawable_of(&id), deposit / 2);
    h.assert_pool_exact();

    // The recipient can withdraw normally once accepted.
    assert_eq!(h.client.withdraw(&id, &None), deposit / 2);
    h.assert_pool_exact();
}

/// A future-dated `start_time` survives acceptance: the stream is scheduled,
/// not started early.
#[test]
fn accepting_a_future_dated_pending_stream_keeps_the_scheduled_start() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let start = h.now() + 30 * DAY;
    let id = h.create_pending_full(deposit, start, start + 10 * DAY, start, true, true, true);

    h.client.accept_stream(&id);

    let s = h.get(id);
    assert_eq!(s.start_time, start, "a future start is honoured as-is");
    assert_eq!(s.end_time, start + 10 * DAY, "duration preserved");
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "nothing before the scheduled start"
    );

    h.advance(40 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        deposit,
        "fully vested after the schedule"
    );
}

/// Accept and decline are one-shot: once answered, the window is closed.
#[test]
fn accept_and_decline_are_one_shot() {
    let h = Harness::new();

    let accepted = h.create_pending(1_000 * ONE, 100 * DAY);
    h.client.accept_stream(&accepted);
    assert_eq!(
        h.client.try_accept_stream(&accepted).unwrap_err().unwrap(),
        Error::StreamNotPending
    );
    assert_eq!(
        h.client.try_decline_stream(&accepted).unwrap_err().unwrap(),
        Error::StreamNotPending
    );

    let declined = h.create_pending(1_000 * ONE, 100 * DAY);
    h.client.decline_stream(&declined);
    assert_eq!(
        h.client.try_decline_stream(&declined).unwrap_err().unwrap(),
        Error::StreamNotPending
    );
    h.assert_pool_exact();
}

/// Accepting a stream that was never pending is rejected.
#[test]
fn answering_a_non_pending_stream_is_rejected() {
    let h = Harness::new();
    let active = h.create_simple(1_000 * ONE, 100 * DAY);
    assert_eq!(
        h.client.try_accept_stream(&active).unwrap_err().unwrap(),
        Error::StreamNotPending
    );
    assert_eq!(
        h.client.try_decline_stream(&active).unwrap_err().unwrap(),
        Error::StreamNotPending
    );
    // And an unknown id is StreamNotFound, not a panic.
    assert_eq!(
        h.client.try_accept_stream(&999).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
}

// ---------------------------------------------------------------------------
// Pending streams reject accrual-dependent operations
// ---------------------------------------------------------------------------

/// Every operation that assumes a started stream is refused with
/// `StreamPending`, and refuses without mutating anything.
#[test]
fn pending_stream_rejects_every_accrual_dependent_operation() {
    let h = Harness::new();
    let id = h.create_pending(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    let before = h.get(id);
    let pool_before = h.pool();
    let ttl_before = h.ttl_of(id);

    assert_eq!(
        h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
        Error::StreamPending
    );
    assert_eq!(
        h.client.try_withdraw(&id, &Some(1)).unwrap_err().unwrap(),
        Error::StreamPending
    );
    assert_eq!(
        h.client.try_top_up(&id, &(10 * ONE)).unwrap_err().unwrap(),
        Error::StreamPending
    );
    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamPending
    );
    assert_eq!(
        h.client.try_resume(&id).unwrap_err().unwrap(),
        Error::StreamPending
    );
    assert_eq!(
        h.client
            .try_transfer_recipient(&id, &h.other)
            .unwrap_err()
            .unwrap(),
        Error::StreamPending
    );

    assert_eq!(h.get(id), before, "state unchanged by the rejections");
    assert_eq!(h.pool(), pool_before, "no funds moved");
    assert_eq!(
        h.ttl_of(id),
        ttl_before,
        "TTL not extended by a rejected call"
    );
    h.assert_pool_exact();
}

/// The sender is never stuck: a pending stream is refundable in full with
/// `cancel`, even when it was created `cancellable == false`.
#[test]
fn sender_can_reclaim_a_pending_stream_even_when_not_cancellable() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let sender_before = h.balance(&h.sender);
    let now = h.now();
    let id = h.create_pending_full(deposit, now, now + 100 * DAY, now, false, true, true);
    h.advance(3 * DAY);

    h.client.cancel(&id);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.withdrawn, 0);
    assert_eq!(
        h.balance(&h.sender),
        sender_before,
        "full refund despite cancellable == false"
    );
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

#[test]
fn accept_requires_the_recipient_authorization() {
    let h = Harness::new();
    let id = h.create_pending(1_000 * ONE, 100 * DAY);

    h.env.mock_auths(&[]); // no valid auth is offered
    assert!(
        h.client.try_accept_stream(&id).is_err(),
        "accept without auth must fail"
    );
    assert_eq!(h.get(id).status, StreamStatus::Pending, "state unchanged");

    h.env.mock_all_auths();
    h.client.accept_stream(&id);
    assert_eq!(h.get(id).status, StreamStatus::Active);
}

#[test]
fn decline_requires_the_recipient_authorization() {
    let h = Harness::new();
    let id = h.create_pending(1_000 * ONE, 100 * DAY);

    h.env.mock_auths(&[]);
    assert!(
        h.client.try_decline_stream(&id).is_err(),
        "decline without auth must fail"
    );
    assert_eq!(h.get(id).status, StreamStatus::Pending, "state unchanged");
    assert_eq!(h.pool(), 1_000 * ONE, "no refund without auth");

    h.env.mock_all_auths();
    h.client.decline_stream(&id);
    assert_eq!(h.get(id).status, StreamStatus::Declined);
}

/// A declined-then-recreated stream keeps the id sequence contiguous (no id is
/// consumed or reused by the acceptance gate).
#[test]
fn ids_stay_contiguous_across_accept_and_decline() {
    let h = Harness::new();
    let a = h.create_pending(100 * ONE, 10 * DAY);
    let b = h.create_pending(100 * ONE, 10 * DAY);
    let c = h.create_pending(100 * ONE, 10 * DAY);
    assert_eq!((a, b, c), (0, 1, 2));

    h.client.accept_stream(&b);
    h.client.decline_stream(&c);

    // The next create gets the next id, never a freed one.
    let d = h.create_simple(100 * ONE, 10 * DAY);
    assert_eq!(d, 3);
    assert_eq!(h.client.stream_count(), 4);
    h.assert_stream_count_consistent();
}
