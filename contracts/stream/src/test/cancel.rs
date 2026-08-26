//! Stage 2 -- cancellation.
//
/// Cancellation rewrites the schedule so the stream looks like one that has
/// fully matured, which is why `withdraw` needs no special case for it. These
/// tests pin that equivalence down.

use super::common::*;
use crate::{Error, StreamStatus};

#[test]
fn cancel_refunds_the_unvested_remainder_and_leaves_the_rest_claimable() {
    let h = Harness::new();
    let id = h.create_simple(1,000 * ONE, 100 * DAY);
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
    let id = h.create_simple(1,000 * ONE, 100 * DAY);
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
    let id = h.create_simple(1,000 * ONE, 100 * DAY);

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
    let id = h.create_simple(1,000 * ONE, 100 * DAY);
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
    let id = h.create_simple(1,000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.cancel(&id);
    h.client.withdraw(&id, &None);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.withdrawn, s.deposited);
}

//--- Boundaries -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------