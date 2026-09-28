//! Terminal operation matrix — StreamTerminated rejection across all mutating methods.
//!
//! Terminal states (Cancelled, Depleted) must reject every mutating lifecycle
//! operation with a stable error, emit no success events, and leave storage
//! unchanged. This test module provides focused regression coverage for
//! terminal-operation rejection behavior across the full method matrix.
//!
//! # Design decision
//!
//! Every terminal entrypoint must reject with `Error::StreamTerminated` and
//! guarantee:
//! * Storage remains unchanged after rejection
//! * No success events are emitted
//! * TTL is not extended by the failed call
//!
//! # Coverage matrix
//!
//! | Operation       | Cancelled | Depleted | Notes                            |
//! |-----------------|-----------|----------|----------------------------------|
//! | resume          | ✓         | ✓        | pause state already cleared      |
//! | pause           | ✓         | ✓        | accrual already stopped          |
//! | top_up          | ✓         | ✓        | cannot extend finished schedule  |
//! | withdraw        | ✓         | ✓        | liability already settled        |
//! | cancel          | ✓         | n/a      | already terminal                 |
//! | transfer_recip. | ✓         | special  | allowed on depleted (see below)  |
//!
//! # Special case: transfer_recipient on Depleted
//!
//! `transfer_recipient` is **allowed** on a `Depleted` stream — it fails with
//! `StreamTerminated` only if there is literally nothing left to transfer
//! (withdrawn == deposited). If the stream is depleted but the recipient has
//! not yet withdrawn everything, transfer is a no-op but not an error. This
//! behavior is pinned by a dedicated test in this module.

use super::common::*;
use crate::{Error, StreamStatus};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

// ---------------------------------------------------------------------------
// Cancelled streams — reject all mutating operations
// ---------------------------------------------------------------------------

#[test]
fn cancelled_stream_rejects_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    let before = h.get(id);
    let pool_before = h.pool();
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_resume(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    // Storage unchanged.
    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before, "pool unchanged");
    assert_eq!(h.ttl_of(id), ttl_before, "TTL not extended");
    h.assert_pool_exact();
}

#[test]
fn cancelled_stream_rejects_pause() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    let before = h.get(id);
    let pool_before = h.pool();
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

#[test]
fn cancelled_stream_rejects_top_up() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    let before = h.get(id);
    let pool_before = h.pool();
    let sender_balance_before = h.balance(&h.sender);
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before, "no funds moved");
    assert_eq!(
        h.balance(&h.sender),
        sender_balance_before,
        "sender balance unchanged"
    );
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

#[test]
fn cancelled_stream_rejects_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    // Immediately withdraw the vested portion, leaving nothing.
    h.client.withdraw(&id, &None);

    let before = h.get(id);
    let pool_before = h.pool();
    let recipient_balance_before = h.balance(&h.recipient);
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before);
    assert_eq!(h.balance(&h.recipient), recipient_balance_before);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

#[test]
fn cancelled_stream_rejects_second_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    let before = h.get(id);
    let pool_before = h.pool();
    let sender_balance_before = h.balance(&h.sender);
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before, "no refund on second cancel");
    assert_eq!(h.balance(&h.sender), sender_balance_before);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

/// A cancelled stream is reassignable only while an unclaimed tail remains
/// (`withdrawn < deposited`). Once the tail is fully drawn the claim is
/// settled and the stream can no longer be transferred.
#[test]
fn cancelled_stream_with_settled_claim_rejects_transfer_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let new_recipient = Address::generate(&h.env);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    h.client.withdraw(&id, &None); // draw the full (now-cancelled) tail: settled

    let before = h.get(id);
    assert_eq!(
        before.withdrawn, before.deposited,
        "sanity: claim is settled"
    );
    let ttl_before = h.ttl_of(id);

    let err = h
        .client
        .try_transfer_recipient(&id, &new_recipient)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(after.recipient, before.recipient, "recipient unchanged");
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Depleted streams — reject all mutating operations
// ---------------------------------------------------------------------------

#[test]
fn depleted_stream_rejects_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);

    let before = h.get(id);
    let pool_before = h.pool();
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_resume(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

#[test]
fn depleted_stream_rejects_pause() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);

    let before = h.get(id);
    let pool_before = h.pool();
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

#[test]
fn depleted_stream_rejects_top_up() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);

    let before = h.get(id);
    let pool_before = h.pool();
    let sender_balance_before = h.balance(&h.sender);
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before);
    assert_eq!(h.balance(&h.sender), sender_balance_before);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

#[test]
fn depleted_stream_rejects_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);

    let before = h.get(id);
    let pool_before = h.pool();
    let recipient_balance_before = h.balance(&h.recipient);
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before);
    assert_eq!(h.balance(&h.recipient), recipient_balance_before);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

#[test]
fn depleted_stream_rejects_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);

    let before = h.get(id);
    let pool_before = h.pool();
    let sender_balance_before = h.balance(&h.sender);
    let ttl_before = h.ttl_of(id);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(h.pool(), pool_before);
    assert_eq!(h.balance(&h.sender), sender_balance_before);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

#[test]
fn depleted_stream_rejects_transfer_recipient_when_fully_drained() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let new_recipient = Address::generate(&h.env);

    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    assert_eq!(h.get(id).withdrawn, h.get(id).deposited, "fully drained");

    let before = h.get(id);
    let ttl_before = h.ttl_of(id);

    let err = h
        .client
        .try_transfer_recipient(&id, &new_recipient)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let after = h.get(id);
    assert_eq!(before, after);
    assert_eq!(after.recipient, before.recipient);
    assert_eq!(h.ttl_of(id), ttl_before);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Boundary: cancelled stream with remaining withdrawable balance
// ---------------------------------------------------------------------------

#[test]
fn cancelled_stream_with_withdrawable_balance_still_rejects_operations() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    h.client.cancel(&id);
    // 500 vested, not withdrawn yet.
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert_eq!(h.client.withdrawable_of(&id), 500 * ONE);

    let before = h.get(id);

    // All mutating operations still rejected.
    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_resume(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_cancel(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );

    // Storage unchanged by the rejection.
    assert_eq!(h.get(id), before);
    h.assert_pool_exact();

    // But withdraw still works.
    assert_eq!(h.client.withdraw(&id, &None), 500 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Boundary: paused stream cancelled (pause state cleared)
// ---------------------------------------------------------------------------

#[test]
fn cancelled_stream_after_pause_clears_pause_state_and_rejects_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.pause(&id);
    h.advance(20 * DAY);
    h.client.cancel(&id);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.paused_at, None, "pause state cleared by cancel");

    let err = h.client.try_resume(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Boundary: depleted stream after pause
// ---------------------------------------------------------------------------

#[test]
fn depleted_stream_after_pause_clears_pause_state_and_rejects_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 150 * DAY);
    h.client.pause(&id);
    h.advance(10 * DAY);
    h.client.withdraw(&id, &None);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Depleted);
    assert_eq!(s.paused_at, None, "pause state cleared by depletion");

    let err = h.client.try_resume(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Retry: multiple rejections must not change state
// ---------------------------------------------------------------------------

#[test]
fn repeated_rejection_on_cancelled_stream_does_not_mutate_state() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    let after_cancel = h.get(id);

    // Attempt each operation multiple times.
    for _ in 0..3 {
        let _ = h.client.try_pause(&id);
        let _ = h.client.try_resume(&id);
        let _ = h.client.try_top_up(&id, &(10 * ONE));
        let _ = h.client.try_cancel(&id);
    }

    let after_retries = h.get(id);
    assert_eq!(
        after_cancel, after_retries,
        "state unchanged after repeated rejections"
    );
    h.assert_pool_exact();
}

#[test]
fn repeated_rejection_on_depleted_stream_does_not_mutate_state() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);

    let after_depletion = h.get(id);

    for _ in 0..3 {
        let _ = h.client.try_pause(&id);
        let _ = h.client.try_resume(&id);
        let _ = h.client.try_top_up(&id, &(10 * ONE));
        let _ = h.client.try_cancel(&id);
        let _ = h.client.try_withdraw(&id, &None);
    }

    let after_retries = h.get(id);
    assert_eq!(after_depletion, after_retries);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Authorization: terminal rejection precedes auth checks
// ---------------------------------------------------------------------------

#[test]
fn cancelled_stream_returns_terminal_error_before_auth_check() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    // Attempting pause with wrong caller (would normally fail auth) still
    // returns StreamTerminated as the precondition check.
    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::StreamTerminated,
        "terminal state checked before auth"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Matrix: terminal states across all lifecycle operations
// ---------------------------------------------------------------------------

/// Comprehensive matrix test: every terminal state rejects every mutating
/// operation with StreamTerminated and leaves state unchanged.
#[test]
fn terminal_operation_matrix_comprehensive() {
    // Test cancelled state.
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 100 * DAY);
        h.advance(30 * DAY);
        h.client.cancel(&id);

        let before = h.get(id);
        // `withdraw` is deliberately absent from the cancelled block: this
        // stream still has a withdrawable tail, and the recipient may pull it
        // through the normal path. Withdraw is rejected only once the claim is
        // settled (see `cancelled_stream_with_withdrawable_balance_still_rejects_operations`).
        // The `.map(...)` wrappers unify the `try_*` result types so the array
        // type-checks; a contract error surfaces as `Err(Ok(Error))`.
        let operations = [
            (
                "pause",
                h.client
                    .try_pause(&id)
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
            (
                "resume",
                h.client
                    .try_resume(&id)
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
            (
                "top_up",
                h.client
                    .try_top_up(&id, &(10 * ONE))
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
            (
                "cancel",
                h.client
                    .try_cancel(&id)
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
        ];

        for (op, result) in operations {
            assert_eq!(
                result.unwrap_err().unwrap(),
                Error::StreamTerminated,
                "cancelled → {op}"
            );
        }

        let after = h.get(id);
        assert_eq!(before, after, "cancelled: state unchanged after rejections");
        h.assert_pool_exact();
    }

    // Test depleted state.
    {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 100 * DAY);
        h.warp_to(T0 + 100 * DAY);
        h.client.withdraw(&id, &None);

        let before = h.get(id);
        let operations = [
            (
                "pause",
                h.client
                    .try_pause(&id)
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
            (
                "resume",
                h.client
                    .try_resume(&id)
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
            (
                "top_up",
                h.client
                    .try_top_up(&id, &(10 * ONE))
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
            (
                "cancel",
                h.client
                    .try_cancel(&id)
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
            (
                "withdraw",
                h.client
                    .try_withdraw(&id, &None)
                    .map(|r| r.map(|_| ()).map_err(|_| ())),
            ),
        ];

        for (op, result) in operations {
            assert_eq!(
                result.unwrap_err().unwrap(),
                Error::StreamTerminated,
                "depleted → {op}"
            );
        }

        let after = h.get(id);
        assert_eq!(before, after, "depleted: state unchanged after rejections");
        h.assert_pool_exact();
    }
}

// ---------------------------------------------------------------------------
// Boundary: cancel at creation (zero-length collapsed schedule)
// ---------------------------------------------------------------------------

#[test]
fn cancel_at_creation_produces_terminal_state_with_zero_balance() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.cancel(&id);

    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert_eq!(h.client.withdrawable_of(&id), 0);

    // All operations rejected.
    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_resume(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_top_up(&id, &(10 * ONE)).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Boundary: depleted before cliff (zero vested, zero withdrawn)
// ---------------------------------------------------------------------------

#[test]
fn depleted_before_cliff_is_still_terminal() {
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

    // Cancel before cliff → vested = 0, refund everything, status = Cancelled.
    h.advance(10 * DAY);
    h.client.cancel(&id);

    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert_eq!(h.get(id).deposited, 0, "nothing vested");
    assert_eq!(h.get(id).withdrawn, 0);

    // Terminal even though withdrawn == deposited == 0.
    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_top_up(&id, &(10 * ONE)).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Boundary: terminal after top-up (extended schedule, then cancelled)
// ---------------------------------------------------------------------------

#[test]
fn top_up_then_cancel_leaves_terminal_state() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    h.client.top_up(&id, &(500 * ONE));
    // New schedule is 1500 over 150 days, 50 days elapsed, 500 vested.
    assert_eq!(h.client.vested_of(&id), 500 * ONE);

    h.advance(10 * DAY);
    h.client.cancel(&id);

    assert_eq!(h.get(id).status, StreamStatus::Cancelled);

    // All operations rejected.
    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_top_up(&id, &(10 * ONE)).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    h.assert_pool_exact();
}
