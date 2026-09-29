//! Delegation expiry boundary — a delegate acting in the very ledger their
//! grant expires.
//!
//! `expires_at` is an instant, not a duration, so the only question the
//! boundary raises is whether the **expiry ledger itself** is inside the
//! grant's lifetime. `docs/ABI.md` answers it for `grant_delegate`: the grant
//! is valid while `ledger.timestamp() <= expires_at`, and `check_delegate`
//! rejects a grant only once `ledger.timestamp() > expires_at`. The expiry
//! ledger is therefore *live* — a grant whose `expires_at` equals the current
//! ledger timestamp still authorises the delegate.
//!
//! That boundary is load-bearing. Rewriting the comparison as `>=` would
//! silently drop the whole expiry ledger, and no existing test would notice:
//! `test::delegation::expired_grant_is_rejected` only samples one ledger
//! comfortably before expiry and one two ledgers past it, never the instant
//! itself. The tests below pin the exact instant.
//!
//! Every scenario runs end to end through the public ABI, asserts the emitted
//! events against independent ground truth (storage plus the token ledger), and
//! closes with [`Harness::assert_pool_exact`] so funds conservation is checked
//! rather than assumed.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{xdr, Address, Event as _};

use super::common::*;
use crate::events::{DelegateGranted, Withdrawn};
use crate::{op, Error, StreamStatus};

/// The events the *stream* contract published during the last invocation.
///
/// `Events::all()` only reports the most recent contract invocation, so this
/// has to be the first thing a test does after a call — any other client call,
/// including a read-only view, replaces the snapshot. Filtering by the stream
/// contract drops the SAC's own `transfer` event, which belongs to the token
/// contract.
fn published_by_stream(h: &Harness) -> std::vec::Vec<xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// The headline scenario: grant a delegate `WITHDRAW` that expires at `t`, then
/// have the delegate act at exactly `t`.
///
/// The payment must land, the `Withdrawn` event must agree with storage and the
/// token ledger, the stream must retain every other field, and the pool must
/// still hold exactly the outstanding liability at the end.
#[test]
fn delegate_withdraw_in_the_expiry_ledger_pays_and_conserves_funds() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    // The grant expires in the ledger whose timestamp is `expires`.
    let expires = h.now() + 10 * DAY;
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &Some(expires));

    // The grant event states the boundary exactly: `expires_at == t`.
    let granted = published_by_stream(&h);
    let expected_grant = DelegateGranted {
        stream_id: id,
        grantor: h.recipient.clone(),
        delegate: agent.clone(),
        ops: op::WITHDRAW,
        expires_at: Some(expires),
    };
    assert_eq!(
        granted,
        std::vec![expected_grant.to_xdr(&h.env, &h.contract_id)],
        "grant_delegate must emit exactly one DelegateGranted carrying expires_at = t",
    );

    // Move time to the expiry instant itself — the ledger the grant expires in.
    // Nothing else changes, so the boundary is the only variable under test.
    h.warp_to(expires);
    assert_eq!(
        h.now(),
        expires,
        "pre-condition: we are on the expiry ledger"
    );

    // Conservation bookkeeping, captured before the delegate acts.
    let sender_before = h.balance(&h.sender);
    let recipient_before = h.balance(&h.recipient);
    let pool_before = h.pool();
    let total_before = sender_before + recipient_before + pool_before;
    let stream_before = h.get(id);

    // 10 of 100 days have elapsed at `t`, so 100 of 1_000 ONE has vested.
    let expected_payout = 100 * ONE;
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(
        paid, expected_payout,
        "the delegate must be able to act on its expiry ledger",
    );

    // Event == storage == token delta.
    let published = published_by_stream(&h);
    let after = h.get(id);
    let expected_event = Withdrawn {
        stream_id: id,
        recipient: h.recipient.clone(),
        amount: paid,
        withdrawn: after.withdrawn,
        deposited: after.deposited,
        status: after.status,
    };
    assert_eq!(
        published,
        std::vec![expected_event.to_xdr(&h.env, &h.contract_id)],
        "the Withdrawn event must match storage exactly",
    );

    assert_eq!(h.balance(&h.recipient), recipient_before + paid);
    assert_eq!(h.pool(), pool_before - paid);
    assert_eq!(
        h.balance(&h.sender),
        sender_before,
        "the sender is untouched"
    );

    // Stream accounting: only `withdrawn` moves at the boundary.
    assert_eq!(after.withdrawn, expected_payout);
    assert_eq!(after.deposited, stream_before.deposited);
    assert_eq!(after.status, StreamStatus::Active);
    assert_eq!(
        h.client.withdrawable_of(&id),
        0,
        "the full accrued 100 ONE was drawn at `t`",
    );

    // Conservation across the whole system, plus the exactness check that no
    // token is stranded in the contract.
    assert_eq!(
        h.balance(&h.sender) + h.balance(&h.recipient) + h.pool(),
        total_before,
        "tokens were created or destroyed at the delegation boundary",
    );
    h.assert_pool_exact();
}

/// The tightest reading of "the same ledger": the grant is issued *and* expires
/// at the current ledger timestamp, and the delegate acts in that same ledger
/// with no clock movement at all.
///
/// `t <= t` holds, so the grant is live. Moving either comparison by one would
/// turn the grant into a dead entry at the instant it was written.
#[test]
fn grant_issued_at_its_own_expiry_instant_is_live_in_that_ledger() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Give the stream 10 days of accrual so the delegated op has something to
    // do, then use the current instant as the expiry.
    h.advance(10 * DAY);

    let agent = Address::generate(&h.env);
    let now = h.now();
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &Some(now));

    // No `advance`/`warp_to` between the grant and the call: grant and action
    // share one ledger, and that ledger *is* `expires_at`.
    assert_eq!(h.now(), now, "pre-condition: no time has passed");
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 100 * ONE);
    h.assert_pool_exact();
}

/// The expiry ledger is the *last* live ledger, and the very next ledger is
/// already past the boundary.
///
/// This pins the shape of the comparison: at `t` the same grant succeeds, at
/// `t + 1` it fails with `DelegateExpired` even though the stream still has
/// funds to draw. A one-second window cannot be produced by either `>` or `>=`
/// alone — it is exactly the inclusive reading `docs/ABI.md` documents.
#[test]
fn the_expiry_ledger_is_live_and_the_next_ledger_is_not() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    let expires = h.now() + 10 * DAY;
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &Some(expires));

    // On the expiry ledger the grant is live.
    h.warp_to(expires);
    let paid = h.client.delegate_withdraw(&id, &agent, &Some(ONE));
    assert_eq!(paid, ONE, "the expiry ledger is live");

    // One ledger later the grant is gone, with accrual still outstanding.
    h.advance(1);
    assert_eq!(h.now(), expires + 1);
    let before = h.get(id);
    assert!(
        h.client.withdrawable_of(&id) > 0,
        "pre-condition: the stream still owes funds, so only expiry can reject",
    );

    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateExpired);

    // A pure authorization failure: no event, no state change, funds untouched.
    assert!(
        published_by_stream(&h).is_empty(),
        "a rejected delegate call must not emit an event",
    );
    assert_eq!(h.get(id), before);
    h.assert_pool_invariant();
    h.assert_pool_exact();
}
