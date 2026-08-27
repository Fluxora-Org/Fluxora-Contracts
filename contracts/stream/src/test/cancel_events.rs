//! Stage 2 — the cancellation event's accounting contract (issue #1584).
//!
//! `Cancelled` publishes `refunded`, `vested` and `withdrawn`, which makes it a
//! public accounting statement: an indexer, a treasury reconciliation job and a
//! recipient's wallet all read those three numbers and expect them to agree
//! with the chain. This module pins the ruling down.
//!
//! # The ruling
//!
//! **`vested` is the total vested at the cancellation instant** — cumulative
//! over the life of the stream and *inclusive* of anything the recipient had
//! already withdrawn. It is exactly the post-cancel `deposited`. The amount
//! still claimable is `vested - withdrawn`, which the event also lets you
//! compute because it carries `withdrawn`.
//!
//! The alternative reading (publish the withdrawable remainder) was rejected:
//! it breaks conservation for any partially withdrawn stream, and it destroys
//! information, since total vested cannot be recovered from the remainder.
//! See the [`Cancelled`] docs for the full rationale.
//!
//! # What these tests assert
//!
//! For every reachable cancellation state, the published event is rebuilt from
//! *independent ground truth* — the stream in storage and the token ledger —
//! and compared byte-for-byte against what the contract emitted. Nothing is
//! compared against the values the contract passed to its own emitter, because
//! that would only prove the contract agrees with itself.
//!
//! Two identities are checked alongside every cancellation:
//!
//! ```text
//! refunded + vested == deposited_before_cancel        (conservation, I4)
//! vested - withdrawn == withdrawable == pooled tokens (nothing stranded)
//! ```

use soroban_sdk::testutils::Events as _;
use soroban_sdk::{xdr, Event as _};

use super::common::*;
use crate::events::Cancelled;
use crate::{Error, StreamStatus};

/// The events the *stream* contract published during the last invocation.
///
/// `Events::all()` only reports the most recent contract invocation, so this
/// has to be the first thing a test does after `cancel` — any other client call
/// (even a read-only `get_stream`) replaces the snapshot. Filtering by the
/// stream contract drops the SAC's own `transfer` event, which belongs to the
/// token contract.
fn published_by_stream(h: &Harness) -> std::vec::Vec<xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// The settled figures, returned so a caller can assert exact amounts on top of
/// the structural checks done here.
struct Settlement {
    refunded: i128,
    vested: i128,
    withdrawn: i128,
    end_time: u64,
}

/// **The core assertion of this module.**
///
/// Call immediately after `cancel`. Rebuilds the expected [`Cancelled`] event
/// out of storage (`deposited`, `withdrawn`, `end_time`) and the token ledger
/// (the sender's balance delta), then requires the contract to have published
/// exactly that, and nothing else.
///
/// Assumes a single-stream harness with no loose tokens donated to the
/// contract, so the pooled balance can be asserted exactly.
fn assert_cancel_settlement(
    h: &Harness,
    id: u64,
    sender_before: i128,
    deposited_before: i128,
) -> Settlement {
    // 1. Snapshot first — before any other contract call resets the buffer.
    let published = published_by_stream(h);

    // 2. Ground truth, read back independently of anything the emitter saw.
    let stream = h.get(id);
    let refunded_on_chain = h.balance(&h.sender) - sender_before;
    let pooled = h.pool();
    let withdrawable = h.client.withdrawable_of(&id);

    // 3. Exactly one event, and it must equal the state.
    let expected = Cancelled {
        stream_id: id,
        sender: h.sender.clone(),
        recipient: h.recipient.clone(),
        // From the token ledger.
        refunded: refunded_on_chain,
        // From storage: cancellation rewrites `deposited` to the total vested.
        vested: stream.deposited,
        withdrawn: stream.withdrawn,
        end_time: stream.end_time,
    };
    assert_eq!(
        published,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the Cancelled event must be the only stream event and must match \
         storage + token balances exactly",
    );

    // 4. The published identities.
    assert_eq!(
        expected.refunded + expected.vested,
        deposited_before,
        "conservation (I4): refunded + vested must partition the pre-cancel deposit",
    );
    assert!(
        expected.vested >= expected.withdrawn,
        "vested {} must cover what was already withdrawn {}",
        expected.vested,
        expected.withdrawn,
    );
    assert_eq!(
        withdrawable,
        expected.vested - expected.withdrawn,
        "still claimable must be vested - withdrawn",
    );
    assert_eq!(
        pooled,
        expected.vested - expected.withdrawn,
        "the pool must hold exactly the unclaimed remainder — no stranded tokens",
    );
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(
        stream.end_time, expected.end_time,
        "event end_time must be the collapsed schedule in storage",
    );
    h.assert_pool_exact();

    Settlement {
        refunded: expected.refunded,
        vested: expected.vested,
        withdrawn: expected.withdrawn,
        end_time: expected.end_time,
    }
}

// --- The disambiguation ---------------------------------------------------

/// The case the ruling exists for. With 400 vested and 200 already withdrawn,
/// "total vested" and "currently withdrawable" disagree — the event must carry
/// the total, and `withdrawn` alongside it so the remainder is derivable.
#[test]
fn vested_is_the_cumulative_total_not_the_withdrawable_remainder() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let deposited_before = 1_000 * ONE;

    h.advance(20 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 200 * ONE);
    h.advance(20 * DAY);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, deposited_before);

    assert_eq!(s.vested, 400 * ONE, "total vested, not the 200 remainder");
    assert_eq!(s.withdrawn, 200 * ONE);
    assert_eq!(s.refunded, 600 * ONE);
    // And the derived figure an indexer would compute.
    assert_eq!(s.vested - s.withdrawn, 200 * ONE);

    // The remainder really is claimable, so the event did not over-report.
    assert_eq!(h.client.withdraw(&id, &None), 200 * ONE);
    assert_eq!(h.balance(&h.recipient), 400 * ONE);
    h.assert_pool_exact();
}

// --- Every cancellation state --------------------------------------------

#[test]
fn cancel_of_an_untouched_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    let cancel_time = h.now();
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);

    assert_eq!(s.vested, 300 * ONE);
    assert_eq!(s.refunded, 700 * ONE);
    assert_eq!(s.withdrawn, 0);
    assert_eq!(s.end_time, cancel_time, "schedule collapsed onto now");
}

#[test]
fn cancel_at_the_instant_of_creation() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let start = h.now();
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);

    assert_eq!(s.vested, 0);
    assert_eq!(s.refunded, 1_000 * ONE);
    assert_eq!(
        s.end_time, start,
        "zero-length schedule, not a negative one"
    );
}

/// Pre-cliff entitlement is zero, so the event must publish zero vested and a
/// full refund — not the amount that would have accrued had the cliff passed.
#[test]
fn cancel_before_the_cliff() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start + 50 * DAY,
        true,
        true,
        true,
    );
    let sender_before = h.balance(&h.sender);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);

    assert_eq!(s.vested, 0, "the cliff had not opened");
    assert_eq!(s.refunded, 1_000 * ONE);
}

/// A stream cancelled before it opens must not publish a negative-length
/// schedule or a negative refund.
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
    let s = assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);

    assert_eq!(s.vested, 0);
    assert_eq!(s.refunded, 1_000 * ONE);
    assert_eq!(s.end_time, start, "clamped at start_time");
}

/// Cancelling while paused must settle against the frozen stream clock, and the
/// event must publish those frozen figures — 30 days of accrual, not 80.
#[test]
fn cancel_while_paused_publishes_the_frozen_figures() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    let froze_at = h.now();
    h.client.pause(&id);
    h.advance(50 * DAY);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);

    assert_eq!(s.vested, 300 * ONE, "the paused interval accrued nothing");
    assert_eq!(s.refunded, 700 * ONE);
    assert_eq!(s.end_time, froze_at, "collapsed onto the frozen clock");
    assert_eq!(h.get(id).paused_at, None, "pause cleared by the cancel");
}

/// Pause, resume and a withdrawal in between: the event must still reconcile,
/// with `vested` measured on the stream clock (40 days of accrual across a
/// 50-day wall-clock window).
#[test]
fn cancel_after_a_pause_resume_and_partial_withdrawal() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(20 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 200 * ONE);
    h.client.pause(&id);
    h.advance(10 * DAY);
    h.client.resume(&id);
    h.advance(20 * DAY);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);

    assert_eq!(s.vested, 400 * ONE, "40 days on the stream clock");
    assert_eq!(s.withdrawn, 200 * ONE);
    assert_eq!(s.refunded, 600 * ONE);
}

/// Nothing left to claw back: `refunded` must be exactly zero, and `vested` the
/// whole deposit — including the part already withdrawn.
#[test]
fn cancel_after_full_vesting_with_a_partial_withdrawal() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.warp_to(T0 + 100 * DAY);
    assert_eq!(h.client.withdraw(&id, &Some(400 * ONE)), 400 * ONE);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);

    assert_eq!(s.refunded, 0);
    assert_eq!(s.vested, 1_000 * ONE);
    assert_eq!(s.withdrawn, 400 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 600 * ONE);
    h.assert_pool_exact();
}

/// A top-up moves `deposited` and `end_time`, which are both inputs to the
/// settlement. The event must reconcile against the post-top-up schedule.
#[test]
fn cancel_after_a_top_up() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(10 * DAY);
    h.client.top_up(&id, &(500 * ONE));
    let deposited_before = h.get(id).deposited;
    assert_eq!(deposited_before, 1_500 * ONE);

    h.advance(30 * DAY);
    let vested_at_cancel = h.client.vested_of(&id);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, deposited_before);

    assert_eq!(
        s.vested, vested_at_cancel,
        "event vested must equal what the view reported an instant earlier",
    );
    assert_eq!(s.refunded, deposited_before - vested_at_cancel);
}

/// Long past maturity the stream is fully vested; the event must not report
/// more than was deposited, however far the clock has run.
#[test]
fn cancel_long_after_maturity() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(5 * YEAR);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);
    let s = assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);

    assert_eq!(s.vested, 1_000 * ONE);
    assert_eq!(s.refunded, 0);
}

// --- Boundaries -----------------------------------------------------------

/// Sweep the whole schedule, with and without a prior withdrawal. Truncation
/// makes the exact numbers awkward to hand-write, so the identities are
/// asserted instead — which is the actual contract.
#[test]
fn the_identities_hold_at_every_cancellation_point() {
    for offset in [0u64, 1, DAY, 33 * DAY, 99 * DAY, 100 * DAY, 3 * YEAR] {
        for withdraw_first in [false, true] {
            let h = Harness::new();
            let id = h.create_simple(1_000 * ONE, 100 * DAY);

            if withdraw_first {
                // Draw down early so `withdrawn` is non-zero at the cancel.
                h.advance(7 * DAY);
                h.client.withdraw(&id, &None);
                if offset > 7 * DAY {
                    h.advance(offset - 7 * DAY);
                }
            } else if offset > 0 {
                h.advance(offset);
            }

            let deposited_before = h.get(id).deposited;
            let sender_before = h.balance(&h.sender);

            h.client.cancel(&id);
            let s = assert_cancel_settlement(&h, id, sender_before, deposited_before);

            assert!(
                s.refunded >= 0 && s.vested >= 0 && s.withdrawn >= 0,
                "offset {offset}, withdraw_first {withdraw_first}: negative amount published",
            );
            assert!(
                s.vested <= deposited_before,
                "offset {offset}: vested exceeded the deposit",
            );
        }
    }
}

/// One cancel, one event — the recipient must not be able to make the contract
/// restate the settlement by draining afterwards.
#[test]
fn withdrawing_after_a_cancel_publishes_no_second_cancelled_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.cancel(&id);
    let cancel_events = published_by_stream(&h);
    assert_eq!(cancel_events.len(), 1, "exactly one cancel event");

    h.client.withdraw(&id, &None);
    let after = published_by_stream(&h);
    assert_eq!(after.len(), 1, "the withdraw publishes exactly one event");
    assert_ne!(
        after[0], cancel_events[0],
        "the withdraw must not republish the cancellation",
    );
    h.assert_pool_exact();
}

// --- Failure and authorization -------------------------------------------

/// A rejected cancel is a no-op: no event, no token movement, no state change.
/// Checked for both guard errors.
#[test]
fn a_rejected_cancel_publishes_nothing_and_moves_nothing() {
    // Not cancellable at all.
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
    h.advance(30 * DAY);
    let sender_before = h.balance(&h.sender);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotCancellable);
    assert!(
        published_by_stream(&h).is_empty(),
        "a failed cancel must publish no event",
    );
    assert_eq!(h.balance(&h.sender), sender_before, "no refund on failure");
    assert_eq!(h.pool(), 1_000 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Active);
    h.assert_pool_exact();

    // Already cancelled — the retry must not restate the settlement.
    let h2 = Harness::new();
    let id2 = h2.create_simple(1_000 * ONE, 100 * DAY);
    h2.advance(30 * DAY);
    h2.client.cancel(&id2);
    let settled = h2.get(id2);
    let sender_after_cancel = h2.balance(&h2.sender);

    let err = h2.client.try_cancel(&id2).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    assert!(
        published_by_stream(&h2).is_empty(),
        "a rejected retry must publish no second event",
    );
    assert_eq!(
        h2.balance(&h2.sender),
        sender_after_cancel,
        "no double refund"
    );
    assert_eq!(
        h2.get(id2),
        settled,
        "state untouched by the rejected retry"
    );
    h2.assert_pool_exact();
}

/// The settlement is the sender's to trigger: the successful cancel must have
/// demanded the sender's authorization, not the recipient's.
#[test]
fn the_published_settlement_required_the_senders_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    let sender_before = h.balance(&h.sender);

    h.client.cancel(&id);

    let auths = h.env.auths();
    assert!(!auths.is_empty(), "cancel required no authorization at all");
    assert_eq!(auths[0].0, h.sender, "cancel must be sender-authorized");

    assert_cancel_settlement(&h, id, sender_before, 1_000 * ONE);
}
