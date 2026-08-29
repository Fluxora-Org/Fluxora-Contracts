//! Issue #1583 — Guarantee withdrawal return values match emitted amounts.
//!
//! The `withdraw` function returns an `i128` (the payout), and the `Withdrawn`
//! event carries an `amount` field. Both derive from the same `payout` variable
//! in `apply_withdrawal`, but clients depend on this contract: an indexer reads
//! the event, a wallet displays the return value, and the token ledger records
//! the transfer. If any two of these three disagree, reconciliation breaks.
//!
//! # What these tests assert
//!
//! For every reachable withdrawal scenario, three quantities are compared:
//!
//! 1. **Return value** — what the contract call returns to the caller.
//! 2. **Event `amount`** — the `Withdrawn.amount` field published on chain.
//! 3. **Token delta** — the actual balance change for the recipient.
//!
//! All three must be identical. Additionally, the event's cumulative fields
//! (`withdrawn`, `deposited`, `status`) are checked against post-call storage
//! state to catch accounting drift.

use soroban_sdk::testutils::Events as _;
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::Withdrawn;
use crate::{Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The events the *stream* contract published during the last invocation.
///
/// `Events::all()` only reports the most recent contract invocation, so this
/// must be called immediately after the withdrawal — before any other client
/// call (including read-only views) replaces the snapshot.
fn published_by_stream(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// Assert that exactly one `Withdrawn` event was published and that it matches
/// the expected event byte-for-byte. Returns the `Withdrawn` struct built from
/// ground truth so the caller can inspect individual fields.
///
/// Ground truth is reconstructed from:
/// - Post-call stream state (storage)
/// - Token ledger delta (recipient balance change)
///
/// This is the same approach as `cancel_events::assert_cancel_settlement`:
/// the expected event is built from independent sources, not from the values
/// the contract passed to its own emitter.
fn assert_withdrawn_event(
    h: &Harness,
    stream_id: u64,
    payout: i128,
    recipient_before: i128,
) -> Withdrawn {
    let published = published_by_stream(h);

    let stream = h.get(stream_id);
    let recipient_after = h.balance(&h.recipient);
    let token_delta = recipient_after - recipient_before;

    assert_eq!(
        token_delta, payout,
        "token delta must equal payout (pre-condition)"
    );

    let expected = Withdrawn {
        stream_id,
        recipient: h.recipient.clone(),
        amount: payout,
        withdrawn: stream.withdrawn,
        deposited: stream.deposited,
        status: stream.status,
    };

    assert_eq!(
        published,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the Withdrawn event must be the only stream event and must match \
         storage + token balances exactly",
    );

    expected
}

// ---------------------------------------------------------------------------
// Core assertions: return value == event amount == token delta
// ---------------------------------------------------------------------------

/// Partial withdrawal: the recipient draws a specific amount from a larger
/// available balance. All three quantities must agree.
#[test]
fn partial_withdraw_return_matches_event_amount() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let recipient_before = h.balance(&h.recipient);
    let requested = 100 * ONE;

    let returned = h.client.withdraw(&id, &Some(requested));
    let expected = assert_withdrawn_event(&h, id, requested, recipient_before);

    assert_eq!(
        returned, requested,
        "return value must equal requested amount"
    );
    assert_eq!(
        expected.amount, requested,
        "event amount must equal requested"
    );
    assert_eq!(
        expected.amount, returned,
        "event amount must equal return value"
    );
    h.assert_pool_exact();
}

/// Full withdrawal (`None`): the recipient drains the entire available balance.
#[test]
fn full_withdraw_return_matches_event_amount() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let recipient_before = h.balance(&h.recipient);
    let available = h.client.withdrawable_of(&id);

    let returned = h.client.withdraw(&id, &None);
    let expected = assert_withdrawn_event(&h, id, available, recipient_before);

    assert_eq!(
        returned, available,
        "return value must equal available balance"
    );
    assert_eq!(
        expected.amount, available,
        "event amount must equal available"
    );
    assert_eq!(
        expected.amount, returned,
        "event amount must equal return value"
    );
    h.assert_pool_exact();
}

/// Withdrawal at exactly `end_time`: the full deposit is vested and withdrawable.
#[test]
fn exact_boundary_withdraw_at_end_time() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);

    let recipient_before = h.balance(&h.recipient);

    let returned = h.client.withdraw(&id, &None);
    let expected = assert_withdrawn_event(&h, id, 1_000 * ONE, recipient_before);

    assert_eq!(returned, 1_000 * ONE, "full deposit at end_time");
    assert_eq!(expected.amount, 1_000 * ONE);
    assert_eq!(expected.amount, returned);
    assert_eq!(
        expected.status,
        StreamStatus::Depleted,
        "stream is depleted"
    );
    h.assert_pool_exact();
}

/// Zero available balance before the stream starts: typed error, no event.
#[test]
fn zero_available_returns_error_no_event() {
    let h = Harness::new();
    let start = h.now() + 10 * DAY;
    let id = h.create(100 * ONE, start, start + 100 * DAY, start, true, true, true);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);

    let events = published_by_stream(&h);
    assert!(events.is_empty(), "no event on NothingToWithdraw");
    h.assert_pool_exact();
}

/// Pre-cliff withdrawal: nothing is vested yet, so no event.
#[test]
fn pre_cliff_withdraw_returns_error_no_event() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 30 * DAY;
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        cliff,
        true,
        true,
        true,
    );
    h.advance(15 * DAY); // before cliff

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);

    let events = published_by_stream(&h);
    assert!(events.is_empty(), "no event pre-cliff");
    h.assert_pool_exact();
}

/// Depleted stream: already fully drained, returns error, no new event.
#[test]
fn depleted_stream_returns_error_no_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None); // drain

    let events_after_drain = published_by_stream(&h);
    assert_eq!(events_after_drain.len(), 1, "one event from the drain");

    // Second attempt must fail with no new event.
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);

    let events = published_by_stream(&h);
    assert!(events.is_empty(), "no event from failed withdrawal");
    h.assert_pool_exact();
}

/// Multiple sequential withdrawals: each call's return value must match its
/// event, and the cumulative `withdrawn` in the final event must equal the
/// sum of all payouts.
#[test]
fn multiple_withdrawals_event_consistency() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY); // 500 ONE available

    let mut cumulative_payout = 0i128;

    // First partial withdrawal: 100.
    let before1 = h.balance(&h.recipient);
    let r1 = h.client.withdraw(&id, &Some(100 * ONE));
    assert_eq!(r1, 100 * ONE);
    let e1 = assert_withdrawn_event(&h, id, 100 * ONE, before1);
    assert_eq!(e1.amount, 100 * ONE);
    assert_eq!(e1.amount, r1);
    cumulative_payout += r1;

    // Second partial withdrawal: 200.
    let before2 = h.balance(&h.recipient);
    let r2 = h.client.withdraw(&id, &Some(200 * ONE));
    assert_eq!(r2, 200 * ONE);
    let e2 = assert_withdrawn_event(&h, id, 200 * ONE, before2);
    assert_eq!(e2.amount, 200 * ONE);
    assert_eq!(e2.amount, r2);
    cumulative_payout += r2;

    // Final full withdrawal of remainder.
    let before3 = h.balance(&h.recipient);
    let r3 = h.client.withdraw(&id, &None);
    let e3 = assert_withdrawn_event(&h, id, r3, before3);
    assert_eq!(e3.amount, r3);
    assert_eq!(e3.amount, r3);
    cumulative_payout += r3;

    // Cumulative withdrawn in the last event must equal total payouts.
    assert_eq!(
        e3.withdrawn, cumulative_payout,
        "event withdrawn must equal sum of all payouts"
    );
    assert_eq!(
        e3.withdrawn,
        h.get(id).withdrawn,
        "event withdrawn must match storage"
    );
    assert_eq!(cumulative_payout, 500 * ONE);
    h.assert_pool_exact();
}

/// Withdrawal after a cancel: the event must reflect the cancelled status and
/// the correct vested amount.
#[test]
fn cancelled_stream_withdraw_matches_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY); // 300 ONE vested
    h.client.cancel(&id);

    let recipient_before = h.balance(&h.recipient);

    let returned = h.client.withdraw(&id, &None);
    let expected = assert_withdrawn_event(&h, id, 300 * ONE, recipient_before);

    assert_eq!(returned, 300 * ONE, "vested amount at cancel time");
    assert_eq!(expected.amount, 300 * ONE);
    assert_eq!(expected.amount, returned);
    assert_eq!(expected.status, StreamStatus::Cancelled, "cancel is sticky");
    h.assert_pool_exact();
}

/// Batch withdrawal: each stream's event amount must match its individual
/// payout, and the batch return value must equal the sum of all event amounts.
#[test]
fn batch_withdraw_per_stream_event_amounts() {
    let h = Harness::new();
    // Three streams with distinct deposits so each payout is unique.
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(200 * ONE, 100 * DAY);
    let c = h.create_simple(300 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let expected_a = h.client.withdrawable_of(&a);
    let expected_b = h.client.withdrawable_of(&b);
    let expected_c = h.client.withdrawable_of(&c);
    let expected_total = expected_a + expected_b + expected_c;

    let recipient_before = h.balance(&h.recipient);

    let total = h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b, c]));
    let published = published_by_stream(&h);

    assert_eq!(
        total, expected_total,
        "batch return must equal sum of payouts"
    );
    assert_eq!(published.len(), 3, "one event per stream");

    // Build expected events from ground truth.
    let stream_a = h.get(a);
    let stream_b = h.get(b);
    let stream_c = h.get(c);

    let expected_a_event = Withdrawn {
        stream_id: a,
        recipient: h.recipient.clone(),
        amount: expected_a,
        withdrawn: stream_a.withdrawn,
        deposited: stream_a.deposited,
        status: stream_a.status,
    };
    let expected_b_event = Withdrawn {
        stream_id: b,
        recipient: h.recipient.clone(),
        amount: expected_b,
        withdrawn: stream_b.withdrawn,
        deposited: stream_b.deposited,
        status: stream_b.status,
    };
    let expected_c_event = Withdrawn {
        stream_id: c,
        recipient: h.recipient.clone(),
        amount: expected_c,
        withdrawn: stream_c.withdrawn,
        deposited: stream_c.deposited,
        status: stream_c.status,
    };

    let expected_xdr: std::vec::Vec<_> = [expected_a_event, expected_b_event, expected_c_event]
        .iter()
        .map(|e| e.to_xdr(&h.env, &h.contract_id))
        .collect();

    assert_eq!(
        published, expected_xdr,
        "each stream's event must match its individual payout exactly"
    );

    // Token delta matches total.
    let token_delta = h.balance(&h.recipient) - recipient_before;
    assert_eq!(token_delta, expected_total);
    h.assert_pool_exact();
}

/// Batch with a zero-available stream: the zero stream is skipped (no event),
/// and the non-zero stream's event still matches its payout.
#[test]
fn batch_withdraw_zero_stream_skipped_no_event() {
    let h = Harness::new();
    let drained = h.create_simple(100 * ONE, 10 * DAY);
    let live = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.withdraw(&drained, &None); // fully drain

    let recipient_before = h.balance(&h.recipient);

    let total = h
        .client
        .batch_withdraw(&h.recipient, &h.ids(&[drained, live]));
    let published = published_by_stream(&h);

    assert_eq!(total, 10 * ONE, "only the live stream pays");
    assert_eq!(published.len(), 1, "no event for the drained stream");

    let expected = Withdrawn {
        stream_id: live,
        recipient: h.recipient.clone(),
        amount: 10 * ONE,
        withdrawn: h.get(live).withdrawn,
        deposited: h.get(live).deposited,
        status: h.get(live).status,
    };
    assert_eq!(
        published,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the single event must match the live stream's payout"
    );

    let token_delta = h.balance(&h.recipient) - recipient_before;
    assert_eq!(token_delta, 10 * ONE);
    h.assert_pool_exact();
}

/// Explicit zero amount is rejected: returns `InvalidAmount`, no event.
#[test]
fn explicit_zero_amount_rejected_no_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let err = h.client.try_withdraw(&id, &Some(0)).unwrap_err().unwrap();
    assert_eq!(err, Error::InvalidAmount);

    let events = published_by_stream(&h);
    assert!(events.is_empty(), "no event on InvalidAmount");
    h.assert_pool_exact();
}

/// Negative amount is rejected: returns `InvalidAmount`, no event.
#[test]
fn negative_amount_rejected_no_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let err = h.client.try_withdraw(&id, &Some(-1)).unwrap_err().unwrap();
    assert_eq!(err, Error::InvalidAmount);

    let events = published_by_stream(&h);
    assert!(events.is_empty(), "no event on negative amount");
    h.assert_pool_exact();
}

/// Over-request is rejected: returns `InsufficientWithdrawable`, no event.
#[test]
fn over_request_rejected_no_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let err = h
        .client
        .try_withdraw(&id, &Some(300 * ONE + 1))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable);

    let events = published_by_stream(&h);
    assert!(events.is_empty(), "no event on over-request");
    h.assert_pool_exact();
}

/// Unknown stream: returns `StreamNotFound`, no event.
#[test]
fn unknown_stream_returns_error_no_event() {
    let h = Harness::new();

    let err = h.client.try_withdraw(&999, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound);

    let events = published_by_stream(&h);
    assert!(events.is_empty(), "no event for unknown stream");
}

/// Rate-floor stream: one stroop per second, withdrawal returns exactly one
/// stroop per second elapsed, and the event matches.
#[test]
fn rate_floor_stream_event_matches_return() {
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

    let recipient_before = h.balance(&h.recipient);

    h.warp_to(start + 500);
    let returned = h.client.withdraw(&id, &None);
    let expected = assert_withdrawn_event(&h, id, 500, recipient_before);

    assert_eq!(returned, 500, "500 stroops at 1/s over 500s");
    assert_eq!(expected.amount, 500);
    assert_eq!(expected.amount, returned);
    h.assert_pool_exact();
}

/// Minimum viable stream (1 second, 1 stroop): withdraw at end returns 1,
/// event amount is 1, token delta is 1.
#[test]
fn minimum_stream_event_matches_return() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(1, start, start + 1, start, true, true, true);

    let recipient_before = h.balance(&h.recipient);

    h.advance(1);
    let returned = h.client.withdraw(&id, &None);
    let expected = assert_withdrawn_event(&h, id, 1, recipient_before);

    assert_eq!(returned, 1);
    assert_eq!(expected.amount, 1);
    assert_eq!(expected.amount, returned);
    h.assert_pool_exact();
}

/// Stream with a cliff: before cliff nothing is available, at cliff the full
/// accrued amount is withdrawable and the event matches.
#[test]
fn cliff_withdraw_event_matches_return() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 30 * DAY;
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        cliff,
        true,
        true,
        true,
    );

    // Before cliff: error, no event.
    h.advance(15 * DAY);
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);
    assert!(published_by_stream(&h).is_empty());

    // At cliff: 30 days vested out of 100.
    h.warp_to(cliff);
    let recipient_before = h.balance(&h.recipient);
    let returned = h.client.withdraw(&id, &None);
    let expected = assert_withdrawn_event(&h, id, 300 * ONE, recipient_before);

    assert_eq!(returned, 300 * ONE);
    assert_eq!(expected.amount, 300 * ONE);
    assert_eq!(expected.amount, returned);
    assert_eq!(expected.status, StreamStatus::Active);
    h.assert_pool_exact();
}

/// Paused stream: accrual is frozen while paused, and withdrawal after resume
/// produces an event matching the return value.
#[test]
fn paused_stream_withdraw_event_matches_return() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(20 * DAY); // 200 ONE vested

    h.client.pause(&id);
    let paused_vested = h.client.vested_of(&id);
    assert_eq!(paused_vested, 200 * ONE);

    // Time passes but accrual is frozen.
    h.advance(30 * DAY);
    assert_eq!(h.client.vested_of(&id), 200 * ONE, "paused: no accrual");

    h.client.resume(&id);
    h.advance(10 * DAY); // 10 more days = 100 ONE

    let recipient_before = h.balance(&h.recipient);
    let returned = h.client.withdraw(&id, &None);
    let expected = assert_withdrawn_event(&h, id, 300 * ONE, recipient_before);

    // 30 days total (20 before pause + 10 after resume) = 300 ONE.
    assert_eq!(returned, 300 * ONE);
    assert_eq!(expected.amount, 300 * ONE);
    assert_eq!(expected.amount, returned);
    h.assert_pool_exact();
}
