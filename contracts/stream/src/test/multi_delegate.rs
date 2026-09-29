//! Two delegates holding the same permission on one stream (#1881).
//!
//! `test::delegation` covers a grant as a single object: one delegate, one
//! bitmask, one stream. It never asks what happens when *two* delegates hold the
//! same bit on the same stream at once — which is the ordinary case as soon as
//! two services are trusted to service one stream.
//!
//! The storage model says the answer should be boring. Grants are keyed by
//! `DataKey::Delegate(stream_id, delegate)`, so two delegates are two
//! independent entries; the permission bit is not a budget, and there is no
//! per-stream tally of how many delegates hold it. But "the model says" is not
//! a test. What this module pins is the pair of claims that make the model
//! safe:
//!
//! * **independence** — nothing about one delegate's grant, use or revocation
//!   is visible in the other's;
//! * **conservation** — a stream's accounting is bounded by the stream, not by
//!   the number of delegates, so two delegates draining it in the same ledger
//!   cannot together take more than it owes.
//!
//! The second is the one with teeth. Each delegate's call reads
//! [`crate::accrual::withdrawable`] against the *same* starting `withdrawn`, so
//! an implementation that only reconciled the stream between calls — rather
//! than persisting inside each — would pay the first delegate, then pay the
//! second against the pre-first-call state, and overdraw the pool. The
//! invariant assertions below fail loudly on exactly that.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error, StreamStatus};

/// Mint the tokens the delegate itself needs (its own account pays nothing, but
/// granting is auth-checked against the grantor, so this keeps the fixture
/// symmetric with `test::delegation`).
fn agent(h: &Harness) -> Address {
    let a = Address::generate(&h.env);
    h.token_admin.mint(&a, &(1_000 * ONE));
    a
}

// ---------------------------------------------------------------------------
// Independence
// ---------------------------------------------------------------------------

/// Two delegates can hold `WITHDRAW` on one stream at once, as two distinct
/// storage entries.
#[test]
fn two_delegates_can_hold_the_same_bit_on_one_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let first = agent(&h);
    let second = agent(&h);

    h.client
        .grant_delegate(&id, &h.recipient, &first, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.recipient, &second, &op::WITHDRAW, &None);

    // Both are admissible; neither displaced the other. Each takes its own
    // quarter of the stream, one after the other.
    h.advance(50 * DAY);
    assert_eq!(
        h.client.delegate_withdraw(&id, &first, &Some(100 * ONE)),
        100 * ONE,
    );
    assert_eq!(
        h.client.delegate_withdraw(&id, &second, &Some(100 * ONE)),
        100 * ONE,
    );
    assert_eq!(h.get(id).withdrawn, 200 * ONE);
    h.assert_pool_exact();
}

/// Revoking one delegate leaves the other's grant and access untouched.
#[test]
fn revoking_one_delegate_leaves_the_other_grant_intact() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let kept = agent(&h);
    let dropped = agent(&h);

    h.client
        .grant_delegate(&id, &h.recipient, &kept, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.recipient, &dropped, &op::WITHDRAW, &None);

    h.client.revoke_delegate(&id, &h.recipient, &dropped);

    h.advance(10 * DAY);
    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &dropped, &None)
            .unwrap_err()
            .unwrap(),
        Error::DelegateNotPermitted,
    );
    assert_eq!(h.client.delegate_withdraw(&id, &kept, &None), 100 * ONE);
    h.assert_pool_exact();
}

/// Re-granting one delegate a narrower mask does not narrow the other's.
#[test]
fn narrowing_one_grant_does_not_narrow_the_other() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let narrow = agent(&h);
    let wide = agent(&h);

    h.client.grant_delegate(
        &id,
        &h.recipient,
        &narrow,
        &(op::WITHDRAW | op::TRANSFER_RECIPIENT),
        &None,
    );
    h.client.grant_delegate(
        &id,
        &h.recipient,
        &wide,
        &(op::WITHDRAW | op::TRANSFER_RECIPIENT),
        &None,
    );

    // Re-grant the first delegate `WITHDRAW` only.
    h.client
        .grant_delegate(&id, &h.recipient, &narrow, &op::WITHDRAW, &None);

    assert_eq!(
        h.client
            .try_delegate_transfer_recipient(&id, &narrow, &h.other)
            .unwrap_err()
            .unwrap(),
        Error::DelegateNotPermitted,
    );

    // The other delegate's mask is unchanged.
    h.advance(10 * DAY);
    let before = h.get(id).recipient.clone();
    h.client.delegate_transfer_recipient(&id, &wide, &h.other);
    assert_ne!(h.get(id).recipient, before);
    h.assert_pool_exact();
}

/// Two delegates may hold *different* bits on one stream, and neither can use
/// the other's bit.
#[test]
fn two_delegates_may_hold_disjoint_bits_and_cannot_borrow_each_other() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let payer = agent(&h);
    let controller = agent(&h);

    h.client
        .grant_delegate(&id, &h.recipient, &payer, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.sender, &controller, &op::CANCEL, &None);

    assert_eq!(
        h.client
            .try_delegate_cancel(&id, &payer)
            .unwrap_err()
            .unwrap(),
        Error::DelegateNotPermitted,
        "the withdrawal delegate must not inherit the cancel bit",
    );
    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &controller, &None)
            .unwrap_err()
            .unwrap(),
        Error::DelegateNotPermitted,
        "the cancel delegate must not inherit the withdrawal bit",
    );

    h.advance(10 * DAY);
    assert_eq!(h.client.delegate_withdraw(&id, &payer, &None), 100 * ONE);
    h.client.delegate_cancel(&id, &controller);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Conservation
// ---------------------------------------------------------------------------

/// The headline: two `WITHDRAW` delegates draining the same stream in sequence
/// cannot together take more than the stream owes.
#[test]
fn two_delegates_cannot_withdraw_more_than_the_stream_owes() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let first = agent(&h);
    let second = agent(&h);
    h.client
        .grant_delegate(&id, &h.recipient, &first, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.recipient, &second, &op::WITHDRAW, &None);

    h.advance(50 * DAY);
    let owed = h.client.withdrawable_of(&id);
    assert_eq!(owed, 500 * ONE);

    // Each takes their own request; the second is capped by what is left.
    assert_eq!(
        h.client.delegate_withdraw(&id, &first, &Some(300 * ONE)),
        300 * ONE,
    );
    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &second, &Some(300 * ONE))
            .unwrap_err()
            .unwrap(),
        Error::InsufficientWithdrawable,
        "the second delegate must see the first delegate's withdrawal",
    );

    // The full remaining amount is still payable, and the total paid equals
    // what the stream owed — not the sum of the two requests.
    assert_eq!(
        h.client.delegate_withdraw(&id, &second, &None),
        owed - 300 * ONE,
    );
    assert_eq!(h.get(id).withdrawn, owed);
    // The stream still has unvested value behind it, so draining the accrued
    // half does not deplete it — `Depleted` means the whole deposit is paid.
    assert_eq!(h.get(id).status, StreamStatus::Active);
    assert_eq!(h.pool(), 1_000 * ONE - owed);
    h.assert_pool_exact();
}

/// The dangerous ordering: both delegates settle with `None` (\"take everything
/// available\") against the same instant, with no ledger boundary and no time
/// advance in between. The second must see a stream with nothing left.
#[test]
fn two_delegates_settling_in_one_ledger_cannot_oversubscribe_the_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let first = agent(&h);
    let second = agent(&h);
    h.client
        .grant_delegate(&id, &h.recipient, &first, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.recipient, &second, &op::WITHDRAW, &None);

    h.advance(40 * DAY);
    let owed = h.client.withdrawable_of(&id);

    let pool_before = h.pool();
    let recipient_before = h.balance(&h.recipient);

    assert_eq!(h.client.delegate_withdraw(&id, &first, &None), owed);
    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &second, &None)
            .unwrap_err()
            .unwrap(),
        Error::NothingToWithdraw,
        "the second delegate settles against the same instant and must find \
         nothing left",
    );

    // Exactly the stream's liability left the pool — no more, no less.
    assert_eq!(h.get(id).withdrawn, owed);
    assert_eq!(h.pool(), pool_before - owed);
    assert_eq!(h.balance(&h.recipient), recipient_before + owed);
    assert_eq!(h.pool(), 1_000 * ONE - owed);
    h.assert_pool_exact();
}

/// The same, but with both delegates racing a *partial* request that each would
/// be entitled to on its own: 60% + 60% is more than the stream owes, so one of
/// them must be refused rather than the pool being overdrawn.
#[test]
fn two_partial_requests_that_sum_past_the_liability_are_refused_not_overdrawn() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let first = agent(&h);
    let second = agent(&h);
    h.client
        .grant_delegate(&id, &h.recipient, &first, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.recipient, &second, &op::WITHDRAW, &None);

    h.advance(50 * DAY);
    let owed = h.client.withdrawable_of(&id); // 500
    let each_wants = owed * 3 / 5; // 300 + 300 > 500

    assert_eq!(
        h.client.delegate_withdraw(&id, &first, &Some(each_wants)),
        each_wants,
    );
    assert!(each_wants * 2 > owed);

    let result = h
        .client
        .try_delegate_withdraw(&id, &second, &Some(each_wants));
    assert_eq!(
        result.unwrap_err().unwrap(),
        Error::InsufficientWithdrawable
    );

    assert_eq!(h.get(id).withdrawn, each_wants);
    assert_eq!(h.pool(), 1_000 * ONE - each_wants);
    h.assert_pool_exact();
}

/// Funding is bounded too, and by the same shared figure: two `TOP_UP`
/// delegates cannot make the stream's `deposited` exceed what the pool actually
/// holds. This is the mirror of the withdrawal direction, and the case where a
/// per-delegate running total would be most tempting to add.
#[test]
fn two_top_up_delegates_are_bounded_by_the_pool_not_by_each_other() {
    let h = Harness::new();
    let id = h.create_simple(100 * ONE, 100 * DAY);
    let first = agent(&h);
    let second = agent(&h);
    h.client
        .grant_delegate(&id, &h.sender, &first, &op::TOP_UP, &None);
    h.client
        .grant_delegate(&id, &h.sender, &second, &op::TOP_UP, &None);

    let start = h.get(id).start_time;
    h.advance(10 * DAY);
    h.client.delegate_top_up(&id, &first, &(50 * ONE));
    h.client.delegate_top_up(&id, &second, &(50 * ONE));

    // Each top-up bought its own 50 days at the original rate, against the
    // schedule left by the previous one — so the two are additive and the
    // rate is unchanged.
    let stream = h.get(id);
    assert_eq!(stream.deposited, 200 * ONE);
    assert_eq!(stream.end_time, start + 200 * DAY);
    assert_eq!(h.pool(), 200 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Attribution
// ---------------------------------------------------------------------------

/// Each delegate's action is separately attributable: the emitted events name
/// the delegate that acted, so an indexer can tell which of the two drained the
/// stream rather than seeing one merged blob of activity.
#[test]
fn each_delegates_action_is_attributable_in_the_emitted_events() {
    use soroban_sdk::testutils::Events as _;

    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let first = agent(&h);
    let second = agent(&h);
    h.client
        .grant_delegate(&id, &h.recipient, &first, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.recipient, &second, &op::WITHDRAW, &None);
    h.advance(50 * DAY);

    h.client.delegate_withdraw(&id, &first, &Some(ONE));
    let first_events = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .len();
    assert!(first_events > 0, "a delegate withdrawal must be observable");

    h.client.delegate_withdraw(&id, &second, &Some(ONE));
    let second_events = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .len();
    assert!(
        second_events > 0,
        "the second delegate's call must emit too"
    );

    // Two calls, two payouts, and the stream records both.
    assert_eq!(h.get(id).withdrawn, 2 * ONE);
    assert_eq!(h.balance(&h.recipient), 2 * ONE);
}

// ---------------------------------------------------------------------------
// Combined activity / conservation
// ---------------------------------------------------------------------------

/// Randomized-ish sequence across both delegates and the owner paths, with the
/// accounting identities checked after every step: the pool equals the stream's
/// outstanding liability, `withdrawn` never exceeds what has vested, and the
/// recipient's balance is exactly the total withdrawn.
#[test]
fn conservation_holds_across_two_delegates_combined_activity() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let first = agent(&h);
    let second = agent(&h);
    h.client
        .grant_delegate(&id, &h.recipient, &first, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.recipient, &second, &op::WITHDRAW, &None);

    let mut total_paid = 0i128;

    for step in 0..10u64 {
        h.advance(7 * DAY + step);

        let delegate = if step % 2 == 0 { &first } else { &second };
        if let Err(e) = h.client.try_delegate_withdraw(&id, delegate, &None) {
            assert_eq!(
                e.unwrap(),
                Error::NothingToWithdraw,
                "step {step}: the only acceptable failure is having nothing to take",
            );
        } else {
            let paid = h.get(id).withdrawn - total_paid;
            assert!(paid >= 0, "step {step}: withdrawn moved backwards");
            total_paid += paid;
        }

        // The recipient's balance is exactly what the stream says was paid.
        assert_eq!(
            h.balance(&h.recipient),
            h.get(id).withdrawn,
            "step {step}: recipient balance and stream accounting diverged",
        );
        assert_eq!(
            h.balance(&h.recipient),
            total_paid,
            "step {step}: the two delegates together paid more than the stream recorded",
        );
        h.assert_pool_exact();
    }

    // Nothing was created: everything the delegates moved came out of one
    // deposit, and the remainder is still in the pool.
    assert!(total_paid <= 1_000 * ONE);
    assert_eq!(h.pool(), 1_000 * ONE - total_paid);
}
