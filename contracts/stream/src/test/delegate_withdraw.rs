//! `delegate_withdraw` at the edges its direct counterpart `withdraw` already
//! covers.
//!
//! `test::withdraw` pins the direct path exhaustively: the amount domain
//! (`InvalidAmount`, `InsufficientWithdrawable`), the terminal guards, the
//! status transitions, the payout/accounting identity, and the events. Most of
//! that is mirrored for `delegate_withdraw` in `test::delegation`, which pins
//! the replicated amount domain, the terminal guards, the permission bit, the
//! expiry window and same-ledger revocation.
//!
//! This file closes the remaining asymmetry between the two entry points:
//!
//! * **Guard ordering.** `withdraw` loads the stream first and reports
//!   `StreamNotFound` for an unknown id; `delegate_withdraw` validates the
//!   *grant* first, so the same call reports `DelegateNotPermitted`. That is
//!   deliberate — a caller who holds no grant learns nothing about which ids
//!   exist — and it is pinned here so it cannot change silently.
//! * **The delegate's own authorization.** A live grant is necessary but not
//!   sufficient: the delegate named in the call must still authorise it. Without
//!   this the grant table alone would let anyone spend a recipient's claim by
//!   naming themselves a delegate.
//! * **The expiry boundary.** A grant is valid *at* its `expires_at` ledger and
//!   dead one second later, matching the direct path's maturity boundaries.
//! * **The cancelled-with-tail path.** A cancelled stream still owes its accrued
//!   tail, and its delegate can still collect it.
//! * **Payout equivalence.** A delegated withdrawal produces the same payout,
//!   the same resulting record and the same events as the direct call.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error, StreamStatus};

/// The stream ids of every `withdrawn` event observable after the last call, in
/// emission order.
fn withdrawn_event_stream_ids(h: &Harness) -> std::vec::Vec<u64> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .iter()
        .filter_map(|event| {
            let ContractEventBody::V0(v0) = &event.body;
            let [ScVal::Symbol(name), ScVal::U64(stream_id), ..] = v0.topics.as_slice() else {
                return None;
            };
            (name.0.as_slice() == b"withdrawn").then_some(*stream_id)
        })
        .collect()
}

/// Give `agent` the withdraw bit on `id` and mint it nothing: the payout goes to
/// the stream's recipient, so the delegate never needs a balance.
fn grant_withdraw(h: &Harness, id: u64, agent: &Address) {
    h.client
        .grant_delegate(&id, &h.recipient, agent, &op::WITHDRAW, &None);
}

// ---------------------------------------------------------------------------
// Guard ordering: the grant is checked before the stream
// ---------------------------------------------------------------------------

/// An id beyond the count is a *grant* failure on the delegate path, not a
/// `StreamNotFound` — the grant check runs first, and that ordering is the
/// contract's documented behaviour.
///
/// Pinning both halves in one test makes the asymmetry explicit: the same id,
/// the same ledger, two different typed errors depending on which entry point
/// was used.
#[test]
fn an_unissued_id_is_a_grant_failure_on_the_delegate_path() {
    let h = Harness::new();
    h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let unissued = h.client.stream_count();

    assert_eq!(
        h.client
            .try_delegate_withdraw(&unissued, &agent, &None)
            .unwrap_err()
            .unwrap(),
        Error::DelegateNotPermitted,
        "the grant check precedes the stream load on the delegate path",
    );

    // The direct path, which loads the stream first, reports the stream error.
    assert_eq!(
        h.client
            .try_withdraw(&unissued, &None)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound,
    );
}

/// The two layers are reached in order: a valid grant lets the call through the
/// authorization gate, and only then does the stream load decide. Removing the
/// record under a live grant therefore produces `StreamNotFound`, not a grant
/// error — proof that the grant check did not short-circuit the stream read.
#[test]
fn a_live_grant_over_a_missing_record_reports_stream_not_found() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);
    grant_withdraw(&h, id, &agent);

    // Delete the record, leaving the grant in place.
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .remove(&crate::DataKey::Stream(id));
    });

    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &agent, &None)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound,
        "a satisfied grant must not mask a missing stream",
    );
}

/// A stream that exists but has no grant at all reports the grant failure — the
/// stream is never read, so its state cannot leak either way.
#[test]
fn a_stream_with_no_grant_is_rejected_without_being_read() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    let stranger = Address::generate(&h.env);

    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &stranger, &None)
            .unwrap_err()
            .unwrap(),
        Error::DelegateNotPermitted,
    );
    assert_eq!(h.balance(&h.recipient), 0, "nothing may be paid out");
    h.assert_pool_exact();

    // A grant for a *different* stream does not transfer.
    let other = h.create_simple(100 * ONE, DAY);
    grant_withdraw(&h, other, &stranger);
    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &stranger, &None)
            .unwrap_err()
            .unwrap(),
        Error::DelegateNotPermitted,
    );
}

// ---------------------------------------------------------------------------
// The delegate must authorise its own call
// ---------------------------------------------------------------------------

/// A live grant is not a standing permission: the delegate named in the call
/// must authorise it. With every auth revoked, `delegate_withdraw` is rejected
/// by the host's `require_auth` rather than paying out.
#[test]
#[should_panic(expected = "Unauthorized")]
fn a_delegate_without_its_own_authorization_cannot_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);
    grant_withdraw(&h, id, &agent);

    // The grant is live and covers WITHDRAW, so the call reaches
    // `delegate.require_auth()` — and fails there.
    h.env.mock_auths(&[]);
    h.client.delegate_withdraw(&id, &agent, &None);
}

// ---------------------------------------------------------------------------
// The expiry boundary
// ---------------------------------------------------------------------------

/// A grant is valid **at** its `expires_at` ledger (`now > expires` is the
/// rejection test) and dead one second later. Both halves are asserted against
/// the same clock so the boundary cannot drift by an inclusive/exclusive flip.
#[test]
fn a_grant_is_valid_at_its_expiry_ledger_and_dead_one_second_later() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);

    let expires = h.now() + 10 * DAY;
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &Some(expires));

    // Advance to exactly the expiry ledger: still valid.
    h.advance(10 * DAY);
    assert_eq!(h.now(), expires);
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 200 * ONE);
    assert_eq!(h.balance(&h.recipient), 200 * ONE);

    // One second later the grant is dead, and the grant check runs before the
    // stream check, so the error is `DelegateExpired` even though 800 ONE is
    // still claimable.
    h.advance(1);
    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &agent, &None)
            .unwrap_err()
            .unwrap(),
        Error::DelegateExpired,
    );
    assert_eq!(h.balance(&h.recipient), 200 * ONE, "no further payout");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Cancelled-with-tail is still delegable
// ---------------------------------------------------------------------------

/// Cancelling freezes the deposit at what has accrued but leaves the tail owed.
/// That tail is the recipient's claim, so a live withdraw grant still collects
/// it — the same as the direct path — and the stream ends up fully settled.
#[test]
fn a_cancelled_stream_with_a_tail_is_still_withdrawable_by_its_delegate() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let id = h.create_simple(deposit, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(25 * DAY);
    grant_withdraw(&h, id, &agent);

    let sender_before = h.balance(&h.sender);
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert_eq!(h.get(id).withdrawn, 0, "cancel pays out nothing itself");

    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 250 * ONE);
    assert_eq!(h.balance(&h.recipient), 250 * ONE);

    let stream = h.get(id);
    // `Cancelled` is sticky: draining a cancelled stream leaves it visibly
    // cancelled rather than relabelling it as a clean completion.
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.deposited, 250 * ONE);
    assert_eq!(stream.withdrawn, 250 * ONE);
    assert!(
        stream.withdrawn >= stream.deposited,
        "the tail was collected"
    );

    // The refund went to the sender and the pool is empty: the tail was the
    // only claim left.
    assert_eq!(
        h.balance(&h.sender) - sender_before,
        deposit - 250 * ONE,
        "cancel refunded exactly the unaccrued part",
    );
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();

    // A second delegated withdrawal has nothing left to take.
    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &agent, &None)
            .unwrap_err()
            .unwrap(),
        Error::StreamTerminated,
    );
}

// ---------------------------------------------------------------------------
// Payout equivalence with the direct path
// ---------------------------------------------------------------------------

/// The same scenario driven twice — once through `withdraw`, once through
/// `delegate_withdraw` — must produce the same payout, the same resulting
/// record, the same balance deltas, and the same `withdrawn` event stream.
#[test]
fn a_delegated_withdrawal_matches_the_direct_withdrawal() {
    // Direct.
    let direct = Harness::new();
    let direct_id = direct.create_simple(1_000 * ONE, 100 * DAY);
    direct.advance(30 * DAY);
    let direct_pool_before = direct.pool();
    let direct_paid = direct.client.withdraw(&direct_id, &None);
    let direct_events = withdrawn_event_stream_ids(&direct);

    // Delegated, identical inputs.
    let delegated = Harness::new();
    let delegated_id = delegated.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&delegated.env);
    delegated.advance(30 * DAY);
    grant_withdraw(&delegated, delegated_id, &agent);
    let delegated_pool_before = delegated.pool();
    let delegated_paid = delegated
        .client
        .delegate_withdraw(&delegated_id, &agent, &None);
    let delegated_events = withdrawn_event_stream_ids(&delegated);

    assert_eq!(direct_id, delegated_id, "the two runs must line up");
    assert_eq!(direct_paid, delegated_paid);
    assert_eq!(direct_paid, 300 * ONE);
    assert_eq!(direct_pool_before, delegated_pool_before);
    assert_eq!(direct.pool(), delegated.pool());
    assert_eq!(
        direct.get(direct_id),
        delegated.get(delegated_id),
        "the resulting records must be identical",
    );
    assert_eq!(
        direct.balance(&direct.recipient),
        delegated.balance(&delegated.recipient)
    );
    assert_eq!(
        direct.balance(&direct.sender),
        delegated.balance(&delegated.sender)
    );
    assert_eq!(direct_events, delegated_events, "same events, same order");
    assert_eq!(direct_events, std::vec![direct_id]);

    // Both end with an empty pool, and a partial amount behaves the same way.
    direct.assert_pool_exact();
    delegated.assert_pool_exact();

    // Both paths continue identically: time passes, then a partial amount is
    // honoured on each.
    direct.advance(DAY);
    delegated.advance(DAY);
    assert_eq!(
        delegated
            .client
            .delegate_withdraw(&delegated_id, &agent, &Some(1 * ONE)),
        1 * ONE,
    );
    assert_eq!(direct.client.withdraw(&direct_id, &Some(1 * ONE)), 1 * ONE,);
    assert_eq!(direct.get(direct_id), delegated.get(delegated_id));
    direct.assert_pool_exact();
    delegated.assert_pool_exact();
}
