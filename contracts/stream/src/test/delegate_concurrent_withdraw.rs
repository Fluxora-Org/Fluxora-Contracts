//! Issue #1854 — two delegates withdrawing concurrently in the same ledger.
//!
//! `WITHDRAW` grants are stored per `(stream_id, delegate)`, so a recipient can
//! issue two independent delegates the permission to withdraw from the *same*
//! stream — a payroll service and a treasury keeper, say. Both may transact in
//! the same ledger, and each call reads the stream fresh from storage before it
//! pays, so settlement is **serialised by storage**: whichever call executes
//! first pays only what was available at its read, and the second sees the
//! updated `withdrawn` and is paid only the remainder. Double settlement would
//! be a second full payout against a stale balance.
//!
//! # What these tests assert
//!
//! Both orderings are exercised end to end through the public ABI:
//!
//! 1. **Delegate A first** — A drains the full balance; B, ordered after it,
//!    is rejected with `NothingToWithdraw` (17) (the stream is still `Active`,
//!    never terminal, so the empty-balance distinction applies — see
//!    `docs/ABI.md`, the `withdraw` error table).
//! 2. **Delegate B first** — B takes a partial payout; A, ordered after it,
//!    drains exactly the remainder. The two payouts sum to the vested amount
//!    with nothing paid twice.
//!
//! In both orderings:
//!
//! * **Funds conservation** holds exactly: the recipient's token gain sums to
//!   the vested amount at the shared ledger instant, the pool drops by the
//!   same figure, and the pre-settlement `vested + refundable == deposited`
//!   identity holds.
//! * **Accounting and events** match: the `withdrawn` events are the only
//!   stream events, each publishes its own payout, and the second event's
//!   cumulative `withdrawn` equals the first's payout plus its own. Topics
//!   carry the stream's *current* recipient — both delegates pay the holder of
//!   the recipient slot, never the delegate itself.
//! * **Final stream state** matches the documented one: `withdrawn == vested`,
//!   `status == Active` while a positive liability remains (flipping to
//!   `Depleted` exactly once when the deposit settles), and the pool holds
//!   exactly the outstanding liability (`Harness::assert_pool_exact`).

use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::xdr::ContractEvent;
use soroban_sdk::Address;
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::Withdrawn;
use crate::{accrual, op, Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Grant `op::WITHDRAW` on `id` to both delegates, all in the shared ledger.
fn grant_both(h: &Harness, id: u64, a: &Address, b: &Address) {
    h.client
        .grant_delegate(&id, &h.recipient, a, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.recipient, b, &op::WITHDRAW, &None);
}

/// The events the *stream* contract published during the last invocation.
///
/// `Events::all()` only reports the most recent contract invocation, so this
/// must be called immediately after the delegate call — before any other
/// client call replaces the snapshot. The token contract's own `transfer`
/// event is filtered out by the contract filter.
fn published_by_stream(h: &Harness) -> std::vec::Vec<ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// Assert that exactly one `Withdrawn` event was published and that it matches
/// the delegate payout byte-for-byte, built from ground truth (post-call
/// storage state) rather than from the values the contract passed to its own
/// emitter. Same approach as `withdraw_events::assert_withdrawn_event`.
fn assert_single_withdrawn_event(h: &Harness, stream_id: u64, payout: i128) {
    let published = published_by_stream(h);
    let stream = h.get(stream_id);

    let expected = Withdrawn {
        stream_id,
        recipient: h.recipient.clone(),
        amount: payout,
        withdrawn: stream.withdrawn,
        deposited: stream.deposited,
        status: stream.status,
        sender: stream.sender.clone(),
        paused_at: stream.paused_at,
        paused_total: stream.paused_total,
    };

    assert_eq!(
        published,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the Withdrawn event must be the only stream event and must match \
         storage state exactly",
    );
}

// ---------------------------------------------------------------------------
// Order 1: delegate A drains fully, delegate B is ordered after it
// ---------------------------------------------------------------------------

/// Two delegates settle in one ledger, A ordered before B, draining fully.
#[test]
fn concurrent_delegates_withdraw_in_same_ledger_a_then_b() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    let agent_a = Address::generate(&h.env);
    let agent_b = Address::generate(&h.env);
    grant_both(&h, id, &agent_a, &agent_b);

    // One ledger advance only — grants, both calls and the final reads share
    // this ledger, which is what pins the same-ledger case.
    h.advance(30 * DAY);
    let now = h.now();

    // At this shared instant exactly 300 ONE is vested and the stream is
    // still live, so a second full payout would be double settlement.
    assert_eq!(h.get(id).status, StreamStatus::Active);
    let vested = accrual::vested(&h.get(id), now).expect("vested must not overflow");
    assert_eq!(vested, 300 * ONE);

    let recipient_before = h.balance(&h.recipient);
    let pool_before = h.pool();

    // Delegate A, ordered first, drains the full available balance.
    let paid_a = h.client.delegate_withdraw(&id, &agent_a, &None);
    assert_eq!(paid_a, 300 * ONE);
    assert_single_withdrawn_event(&h, id, paid_a);

    // Delegate B, ordered after A in the same ledger, finds the balance
    // already drawn: rejected, and the stream is live, so the typed error is
    // `NothingToWithdraw`, never `StreamTerminated`.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent_b, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NothingToWithdraw);

    // A rejected withdrawal emits nothing (`withdraw_events` pins this for the
    // owner path; here the same rule holds for the delegate path).
    assert!(
        published_by_stream(&h).is_empty(),
        "the rejected second call must emit no event"
    );

    // Accounting: one payout, exactly once.
    let stream = h.get(id);
    assert_eq!(stream.withdrawn, 300 * ONE);
    assert_eq!(stream.deposited, deposit);
    assert_eq!(stream.status, StreamStatus::Active);

    // Funds conservation: the recipient gained exactly the payout, the pool
    // dropped by it, and the settled figures partition the deposit.
    assert_eq!(h.balance(&h.recipient) - recipient_before, 300 * ONE);
    assert_eq!(pool_before - h.pool(), 300 * ONE);
    let refundable = accrual::refundable(&stream, now).expect("refundable must not overflow");
    assert_eq!(refundable, 700 * ONE);
    assert_eq!(vested + refundable, stream.deposited);
    assert_eq!(h.pool(), refundable, "pool holds only the refundable claim");

    // Final stream state matches the documented one.
    h.assert_pool_exact();
    h.assert_invariants();
}

// ---------------------------------------------------------------------------
// Order 2: delegate B takes a partial payout, delegate A drains the remainder
// ---------------------------------------------------------------------------

/// Two delegates settle in one ledger, B ordered before A, splitting the pay.
#[test]
fn concurrent_delegates_withdraw_in_same_ledger_b_then_a() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    let agent_a = Address::generate(&h.env);
    let agent_b = Address::generate(&h.env);
    grant_both(&h, id, &agent_a, &agent_b);

    // Same single-ledger discipline as the A-then-B run.
    h.advance(30 * DAY);
    let now = h.now();

    let recipient_before = h.balance(&h.recipient);
    let pool_before = h.pool();

    // Delegate B, ordered first, draws an explicit partial payout.
    let partial = 100 * ONE;
    let paid_b = h.client.delegate_withdraw(&id, &agent_b, &Some(partial));
    assert_eq!(paid_b, partial);
    assert_single_withdrawn_event(&h, id, paid_b);

    // Delegate A, ordered after B in the same ledger, drains exactly the
    // remainder — the second read sees B's settlement, not a stale balance.
    let paid_a = h.client.delegate_withdraw(&id, &agent_a, &None);
    assert_eq!(paid_a, 200 * ONE);
    assert_single_withdrawn_event(&h, id, paid_a);

    // The two events, taken together, publish serialised settlement: the
    // second's cumulative `withdrawn` is the first's payout plus its own.
    assert_eq!(paid_a + paid_b, 300 * ONE);
    assert_eq!(h.get(id).withdrawn, 300 * ONE);

    // Accounting: nothing paid twice, and the stream is still live — a
    // positive liability remains, so depletion has not happened.
    let stream = h.get(id);
    assert_eq!(stream.withdrawn, 300 * ONE);
    assert_eq!(stream.deposited, deposit);
    assert_eq!(stream.status, StreamStatus::Active);

    // Funds conservation across the shared ledger: the recipient's combined
    // gain is the vested amount (both delegate payouts land in the stream's
    // recipient slot — the same address), and the pool dropped by the same
    // total, holding only the sender's refundable claim afterwards.
    let recipient_gain = h.balance(&h.recipient) - recipient_before;
    assert_eq!(recipient_gain, 300 * ONE);
    assert_eq!(pool_before - h.pool(), 300 * ONE);
    let refundable = accrual::refundable(&stream, now).expect("refundable must not overflow");
    assert_eq!(refundable, 700 * ONE);
    assert_eq!(h.pool(), refundable);

    h.assert_pool_exact();
    h.assert_invariants();
}

// ---------------------------------------------------------------------------
// Full drain: both delegates empty the stream across the same ledger
// ---------------------------------------------------------------------------

/// The stream ends the same ledger fully drained and terminal, exactly once.
#[test]
fn concurrent_delegates_drain_to_depleted_in_one_ledger() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 10 * DAY;
    let id = h.create_simple(deposit, duration);

    let agent_a = Address::generate(&h.env);
    let agent_b = Address::generate(&h.env);
    grant_both(&h, id, &agent_a, &agent_b);

    // Mature the stream: the full deposit is vested and withdrawable.
    h.advance(10 * DAY);

    let recipient_before = h.balance(&h.recipient);
    let pool_before = h.pool();

    // B, ordered first, takes half.
    let paid_b = h.client.delegate_withdraw(&id, &agent_b, &Some(500 * ONE));
    assert_eq!(paid_b, 500 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Active);

    // A, ordered after, drains the rest — and this settles the deposit, so
    // the stream flips to `Depleted` exactly once, on the settling call.
    let paid_a = h.client.delegate_withdraw(&id, &agent_a, &None);
    assert_eq!(paid_a, 500 * ONE);

    // Capture the events immediately — before any read-only client call, which
    // replaces the last-invocation event buffer (see `withdraw_events`).
    let settling = Withdrawn {
        stream_id: id,
        recipient: h.recipient.clone(),
        amount: 500 * ONE,
        withdrawn: deposit,
        deposited: deposit,
        status: StreamStatus::Depleted,
        sender: h.sender.clone(),
        paused_at: None,
        paused_total: 0,
    };
    let published = published_by_stream(&h);
    assert_eq!(
        published,
        std::vec![settling.to_xdr(&h.env, &h.contract_id)]
    );

    let stream = h.get(id);
    assert_eq!(stream.withdrawn, deposit);
    assert_eq!(stream.status, StreamStatus::Depleted);

    // Conservation: everything deposited left the pool, and only the
    // recipient holds it.
    assert_eq!(h.balance(&h.recipient) - recipient_before, deposit);
    assert_eq!(pool_before - h.pool(), deposit);
    assert_eq!(h.pool(), 0);

    // The settlement is terminal: a further delegate call on either grant is
    // rejected with `StreamTerminated` — never a second full payout.
    for agent in [&agent_a, &agent_b] {
        let err = h
            .client
            .try_delegate_withdraw(&id, agent, &None)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::StreamTerminated);
    }
    assert!(
        published_by_stream(&h).is_empty(),
        "the rejected terminal calls must emit no events"
    );

    h.assert_pool_exact();
    h.assert_invariants();
}

// ---------------------------------------------------------------------------
// Both delegates attempt full drains: no payout may be issued twice
// ---------------------------------------------------------------------------

/// Two full-drain attempts in one ledger cannot pay the balance twice.
#[test]
fn concurrent_delegates_both_asking_full_drain_pay_exactly_once() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    let agent_a = Address::generate(&h.env);
    let agent_b = Address::generate(&h.env);
    grant_both(&h, id, &agent_a, &agent_b);

    h.advance(50 * DAY); // 500 ONE vested at the shared instant

    let recipient_before = h.balance(&h.recipient);
    let pool_before = h.pool();

    // A drains the full balance; B, ordered after, finds nothing left.
    let paid_a = h.client.delegate_withdraw(&id, &agent_a, &None);
    assert_eq!(paid_a, 500 * ONE);
    assert_single_withdrawn_event(&h, id, paid_a);
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent_b, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NothingToWithdraw);

    // No payout was issued twice: the recipient holds exactly one payout and
    // the pool dropped by exactly one payout.
    assert_eq!(h.balance(&h.recipient) - recipient_before, 500 * ONE);
    assert_eq!(pool_before - h.pool(), 500 * ONE);
    assert_eq!(h.get(id).withdrawn, 500 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Active);

    h.assert_pool_exact();
    h.assert_invariants();
}

// ---------------------------------------------------------------------------
// Explicit-amount domain: a request larger than the shared balance is rejected
// ---------------------------------------------------------------------------

/// An explicit request exceeding the *shared* available balance is rejected —
/// the availability check runs against the live, already-decayed balance, not
/// a snapshot taken when the grant was issued.
#[test]
fn concurrent_delegates_explicit_over_request_is_rejected() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    let agent_a = Address::generate(&h.env);
    let agent_b = Address::generate(&h.env);
    grant_both(&h, id, &agent_a, &agent_b);

    h.advance(30 * DAY); // 300 ONE vested

    // B draws first, 100 ONE.
    let paid_b = h.client.delegate_withdraw(&id, &agent_b, &Some(100 * ONE));
    assert_eq!(paid_b, 100 * ONE);
    assert_single_withdrawn_event(&h, id, paid_b);

    // A requests 300 ONE — more than the 200 ONE now available. The request is
    // rejected with `InsufficientWithdrawable` and nothing changes.
    let before = h.get(id);
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent_a, &Some(300 * ONE))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable);
    assert_eq!(h.get(id), before, "rejected over-request must not mutate");
    assert!(published_by_stream(&h).is_empty());

    // The exact remainder is accepted, which closes the ledger at 300 paid.
    let paid_a = h.client.delegate_withdraw(&id, &agent_a, &Some(200 * ONE));
    assert_eq!(paid_a, 200 * ONE);
    assert_eq!(h.get(id).withdrawn, 300 * ONE);

    h.assert_pool_exact();
    h.assert_invariants();
}

// ---------------------------------------------------------------------------
// Grants are per (stream, delegate): the two delegates cannot act cross-stream
// ---------------------------------------------------------------------------

/// Two delegates granted on one stream cannot withdraw from a second stream.
/// Concurrency in one ledger must not widen a grant's scope.
#[test]
fn concurrent_delegates_cannot_withdraw_from_another_stream() {
    let h = Harness::new();
    let shared = h.create_simple(1_000 * ONE, 100 * DAY);
    let other = h.create_simple(1_000 * ONE, 100 * DAY);

    let agent_a = Address::generate(&h.env);
    let agent_b = Address::generate(&h.env);
    grant_both(&h, shared, &agent_a, &agent_b);

    h.advance(10 * DAY); // 100 ONE vested on both streams

    // A, authorised on `shared` only, drains it fully.
    let paid = h.client.delegate_withdraw(&shared, &agent_a, &None);
    assert_eq!(paid, 100 * ONE);
    assert_single_withdrawn_event(&h, shared, paid);

    // B's grant covers `shared` too, but A got there first: rejected.
    let err = h
        .client
        .try_delegate_withdraw(&shared, &agent_b, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NothingToWithdraw);

    // Neither grant authorises `other` — a withdrawal there is rejected and
    // leaves the stream untouched.
    for agent in [&agent_a, &agent_b] {
        let err = h
            .client
            .try_delegate_withdraw(&other, agent, &None)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::DelegateNotPermitted);
    }
    assert_eq!(h.get(other).withdrawn, 0);
    assert_eq!(h.get(other).status, StreamStatus::Active);

    h.assert_pool_exact();
    h.assert_invariants();
}

// ---------------------------------------------------------------------------
// Delegation moves the authority, not the amount
// ---------------------------------------------------------------------------

/// A delegate's payout for a given instant equals what the recipient could
/// have drawn itself: running both delegate calls in either order pays out
/// exactly the balance a single owner call would have, and no more.
#[test]
fn delegate_payouts_in_either_order_sum_to_the_owner_balance() {
    for order in 0..2 {
        let h = Harness::new();
        let deposit = 1_000 * ONE;
        let duration = 100 * DAY;
        let id = h.create_simple(deposit, duration);

        let agent_a = Address::generate(&h.env);
        let agent_b = Address::generate(&h.env);
        grant_both(&h, id, &agent_a, &agent_b);

        h.advance(30 * DAY);

        let recipient_before = h.balance(&h.recipient);
        let reference = h.client.withdrawable_of(&id);
        assert_eq!(reference, 300 * ONE);

        // Run both delegate calls in the order given; the total paid must
        // equal the balance the recipient could have drawn in one call.
        let (first, second) = if order == 0 {
            (&agent_a, &agent_b)
        } else {
            (&agent_b, &agent_a)
        };

        let paid_first = h.client.delegate_withdraw(&id, first, &None);
        let paid_second = if reference > paid_first {
            h.client.delegate_withdraw(&id, second, &None)
        } else {
            // Nothing left for the second delegate: the typed error is
            // `NothingToWithdraw` on a live stream, and it pays nothing.
            let err = h
                .client
                .try_delegate_withdraw(&id, second, &None)
                .unwrap_err()
                .unwrap();
            assert_eq!(err, Error::NothingToWithdraw);
            0
        };

        assert_eq!(paid_first + paid_second, reference);
        assert_eq!(h.balance(&h.recipient) - recipient_before, reference);
        assert_eq!(h.get(id).withdrawn, reference);

        h.assert_pool_exact();
        h.assert_invariants();
    }
}
