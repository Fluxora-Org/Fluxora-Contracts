//! Issue #1851 — `get_stream` for a cancelled stream.
//!
//! `cancel` does not delete the record, it *settles* it in place: `deposited`
//! is rewritten to the amount vested at the cancellation instant, `end_time` is
//! collapsed onto that instant, `paused_at` is cleared, and the status becomes
//! `Cancelled`. `get_stream` is therefore the entry point that has to report a
//! schedule whose two ends have been moved, and an indexer that replayed the
//! pre-cancel schedule from the record alone would compute the wrong vesting.
//!
//! `cancel.rs` covers the *effects* of cancellation (refunds, balances,
//! replayed splits). This module pins what `get_stream` *returns* once a stream
//! is cancelled:
//!
//! | property | test |
//! |---|---|
//! | the settled record: status, rewritten deposit, collapsed `end_time`, cleared freeze point | `..._returns_the_settled_record` |
//! | a cancel before `start_time` leaves a **zero-length** schedule, never an inverted one | `..._reports_a_zero_length_schedule_...` |
//! | the record survives the claim being drained, transitioning to `Depleted` | `..._keeps_returning_the_record_after_...` |
//! | the returned record and the derived views (`vested_of`, `withdrawable_of`, `refundable_of`) agree | `..._agrees_with_the_derived_views` |
//! | the collapse is exact for every cancel instant | `..._reports_the_collapsed_end_time_for_every_cancel_instant` |
//! | the read is idempotent and inert (no TTL bump, no state change) | `..._is_read_only_and_idempotent` |
//! | an unknown id is still `StreamNotFound` after other streams are cancelled | `..._still_reports_stream_not_found_for_unknown_ids` |
//!
//! # The one asymmetry worth knowing
//!
//! Cancelling *while paused* does not add the in-flight pause to `paused_total`
//! (unlike draining a paused stream to `Depleted`, which does). It does not
//! need to: the collapse moves `end_time` onto the frozen clock, so `elapsed`
//! is immediately capped at the full (now shorter) duration and the record is
//! fully vested with or without the pause on the books.
//! `..._cancelled_while_paused_...` asserts both halves of that difference so
//! neither drifts silently.

use super::common::*;
use crate::{Error, StreamStatus};
use soroban_sdk::testutils::Events;

/// One cancellable stream over `100 * DAY`, created at `T0`.
fn hundred_day_stream(h: &Harness) -> u64 {
    h.create_simple(1_000 * ONE, 100 * DAY)
}

// ---------------------------------------------------------------------------
// The settled record
// ---------------------------------------------------------------------------

/// `get_stream` on a cancelled stream returns the settled record: the deposit
/// rewritten to what had vested, the schedule collapsed onto the cancellation
/// instant, the freeze point cleared, and everything already withdrawn intact.
#[test]
fn get_stream_on_a_cancelled_stream_returns_the_settled_record() {
    let h = Harness::new();
    let id = hundred_day_stream(&h);

    // Withdraw a slice first, so `deposited` and `withdrawn` differ and the
    // record has something to preserve beyond the collapse.
    h.advance(30 * DAY);
    assert_eq!(h.client.withdraw(&id, &Some(100 * ONE)), 100 * ONE);

    h.advance(10 * DAY);
    h.client.cancel(&id);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(
        s.start_time, T0,
        "cancellation must not move the schedule's start",
    );
    assert_eq!(
        s.end_time,
        T0 + 40 * DAY,
        "end_time collapses onto the cancellation instant (day 40)",
    );
    assert_eq!(
        s.deposited,
        400 * ONE,
        "deposited is rewritten to the amount vested at cancellation",
    );
    assert_eq!(s.withdrawn, 100 * ONE, "what was already paid stays paid");
    assert_eq!(s.paused_at, None, "a terminal stream is never frozen");
    assert_eq!(s.cliff_time, T0, "the cliff gate is left as it was");

    // The record still explains its own numbers: the rewritten deposit is the
    // full vesting of the collapsed schedule.
    assert_eq!(h.client.vested_of(&id), 400 * ONE);
    assert_eq!(
        s.deposited,
        400 * ONE,
        "vested_of and the stored deposit must not disagree",
    );
}

/// Cancelling before `start_time` collapses onto `start_time`, giving a
/// zero-length schedule. The clamp in `cancel` is what stops `end_time` from
/// landing *before* `start_time`, which would make `duration` a
/// `saturating_sub` to zero at best and an inverted range at worst.
#[test]
fn get_stream_reports_a_zero_length_schedule_when_cancelled_before_the_start() {
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

    // Cancel while the schedule has not opened yet.
    h.client.cancel(&id);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(
        s.end_time, s.start_time,
        "a pre-start cancel must collapse to a zero-length schedule, never an inverted one",
    );
    assert_eq!(
        s.deposited, 0,
        "nothing had vested, so nothing is claimable"
    );
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), 0);
}

/// The collapse is exact at every cancellation instant, including inside the
/// first second and long past maturity. `end_time` always lands on
/// `max(stream clock at cancel, start_time)`.
#[test]
fn get_stream_reports_the_collapsed_end_time_for_every_cancel_instant() {
    for advance in [
        0u64,
        1,
        12 * 3600,
        DAY,
        30 * DAY,
        99 * DAY,
        100 * DAY,
        150 * DAY,
    ] {
        let h = Harness::new();
        let sender_before = h.balance(&h.sender);
        let id = hundred_day_stream(&h);
        if advance > 0 {
            h.advance(advance);
        }
        h.client.cancel(&id);

        let s = h.get(id);
        assert_eq!(
            s.end_time,
            T0 + advance,
            "cancel {advance}s in must collapse end_time onto T0+{advance}",
        );
        assert_eq!(s.status, StreamStatus::Cancelled);

        // Conservation across the settle: the sender paid `deposit` up front and
        // was handed the unvested remainder back, so they must end up out of
        // pocket exactly the settled claim — no more, no less.
        assert_eq!(
            h.balance(&h.sender),
            sender_before - s.deposited,
            "cancel {advance}s in: the sender must be refunded exactly the unvested remainder",
        );
        assert_eq!(
            h.client.refundable_of(&id),
            0,
            "cancel {advance}s in: a settled stream has nothing left refundable",
        );
        assert_eq!(
            h.client.vested_of(&id),
            s.deposited,
            "cancel {advance}s in: a settled stream is fully vested at its own end_time",
        );
    }
}

// ---------------------------------------------------------------------------
// The record survives its own settlement
// ---------------------------------------------------------------------------

/// Draining a cancelled stream does not delete it: `get_stream` keeps returning
/// the settled record, now `Depleted`, with the claim fully accounted for.
#[test]
fn get_stream_keeps_returning_the_record_after_the_cancelled_claim_is_drained() {
    let h = Harness::new();
    let id = hundred_day_stream(&h);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);

    let claim = h.client.withdrawable_of(&id);
    assert_eq!(claim, 300 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), claim);

    let s = h.get(id);
    // `Cancelled` is sticky — see the `withdraw` implementation: draining a
    // cancelled stream leaves it visibly cancelled rather than relabelling it
    // as a clean completion.
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.deposited, claim, "the settled deposit is unchanged");
    assert_eq!(s.withdrawn, claim, "the claim moved to `withdrawn`");
    assert_eq!(s.end_time, T0 + 30 * DAY, "the schedule is still collapsed");

    assert!(h.client.stream_exists(&id), "the record itself survives");
    assert_eq!(h.client.stream_count(), 1, "and so does the counter");
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), 0);
    h.assert_pool_exact();
}

/// A cancelled stream cannot be cancelled twice, and the second attempt leaves
/// the returned record byte-identical — the terminal state is absorbing.
#[test]
fn get_stream_returns_an_unchanged_record_after_a_rejected_re_cancel() {
    let h = Harness::new();
    let id = hundred_day_stream(&h);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    let before = h.get(id);

    assert_eq!(
        h.client.try_cancel(&id).unwrap_err().unwrap(),
        Error::StreamTerminated,
    );

    assert_eq!(h.get(id), before, "a rejected cancel must change nothing");
}

// ---------------------------------------------------------------------------
// Agreement with the derived views
// ---------------------------------------------------------------------------

/// The settled record and every derived view tell the same story: the stream is
/// fully vested at its collapsed end, the remainder is exactly what has not
/// been withdrawn, and nothing is left refundable.
#[test]
fn get_stream_on_a_cancelled_stream_agrees_with_the_derived_views() {
    for withdrew_before_cancel in [false, true] {
        let h = Harness::new();
        let id = hundred_day_stream(&h);

        h.advance(25 * DAY);
        if withdrew_before_cancel {
            h.client.withdraw(&id, &None);
        }
        h.advance(5 * DAY);
        h.client.cancel(&id);

        let s = h.get(id);
        let label = if withdrew_before_cancel {
            "after a pre-cancel withdrawal"
        } else {
            "with nothing withdrawn"
        };

        assert_eq!(
            h.client.vested_of(&id),
            s.deposited,
            "{label}: a settled stream is fully vested",
        );
        assert_eq!(
            h.client.withdrawable_of(&id),
            s.deposited - s.withdrawn,
            "{label}: the withdrawable remainder is exactly deposit minus withdrawn",
        );
        assert_eq!(
            h.client.refundable_of(&id),
            0,
            "{label}: nothing remains for the sender to claw back",
        );
        assert!(
            s.withdrawn <= s.deposited,
            "{label}: I1 must hold on the returned record",
        );
        h.assert_pool_exact();
    }
}

/// `get_stream` is a read: repeated calls return equal values, advance no
/// ledger, and leave the cancelled entry's TTL exactly where it was.
///
/// `read_methods_no_side_effects` asserts this for active and archived streams;
/// the cancelled state is the third case, and the one where a careless
/// implementation might "repair" the settled record on read.
#[test]
fn get_stream_on_a_cancelled_stream_is_read_only_and_idempotent() {
    let h = Harness::new();
    let id = hundred_day_stream(&h);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    let first = h.get(id);
    let ttl_before = h.ttl_of(id);
    let sequence_before = h.env.ledger().sequence();

    for _ in 0..6 {
        assert_eq!(h.get(id), first, "get_stream must be idempotent");
    }

    assert_eq!(
        h.ttl_of(id),
        ttl_before,
        "get_stream must not extend a cancelled stream's TTL",
    );
    assert_eq!(
        h.env.ledger().sequence(),
        sequence_before,
        "get_stream must not advance the ledger",
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Paused streams
// ---------------------------------------------------------------------------

/// Cancelling a paused stream settles against the *frozen* clock, not the wall
/// clock, and reports a cleared freeze point.
///
/// It also pins the one way the two terminal paths differ: `cancel` rewrites
/// `end_time` onto the freeze point, so the in-flight pause needs no entry in
/// `paused_total`; a depleting `withdraw` keeps the original schedule, so it
/// does record one. Both records are internally consistent, and this test
/// asserts both so the difference is deliberate rather than accidental.
#[test]
fn get_stream_on_a_stream_cancelled_while_paused_reports_the_frozen_settlement() {
    // Cancel while paused: settled at the freeze point, paused_total untouched.
    {
        let h = Harness::new();
        let id = hundred_day_stream(&h);

        h.advance(30 * DAY);
        h.client.pause(&id);
        h.advance(10 * DAY);
        h.client.cancel(&id);

        let s = h.get(id);
        assert_eq!(s.status, StreamStatus::Cancelled);
        assert_eq!(
            s.end_time,
            T0 + 30 * DAY,
            "settlement must use the frozen clock, not the 40 days of wall clock",
        );
        assert_eq!(s.deposited, 300 * ONE, "vested at the freeze point");
        assert_eq!(s.paused_at, None, "the freeze point is cleared");
        assert_eq!(
            s.paused_total, 0,
            "the collapse moves end_time onto the freeze point, so the in-flight \
             pause needs no separate record — and the record is fully vested \
             without one",
        );
        assert_eq!(
            h.client.vested_of(&id),
            s.deposited,
            "the settled record is fully vested despite the unrecorded pause",
        );
        assert_eq!(h.client.refundable_of(&id), 0);
        h.assert_pool_exact();
    }

    // Drain while paused: the schedule is preserved, so the in-flight pause
    // *is* recorded.
    {
        let h = Harness::new();
        let id = hundred_day_stream(&h);

        h.warp_to(T0 + 150 * DAY); // matured, so a full withdrawal depletes it
        h.client.pause(&id);
        h.advance(10 * DAY);
        h.client.withdraw(&id, &None);

        let s = h.get(id);
        assert_eq!(s.status, StreamStatus::Depleted);
        assert_eq!(s.paused_at, None, "depletion must not leave it frozen");
        assert_eq!(
            s.paused_total,
            10 * DAY,
            "depletion preserves the schedule, so the pause is recorded",
        );
    }
}

// ---------------------------------------------------------------------------
// Unknown ids
// ---------------------------------------------------------------------------

/// Cancelling one stream must not change how `get_stream` answers for an id
/// that never existed — including ids either side of the cancelled one.
#[test]
fn get_stream_still_reports_stream_not_found_for_unknown_ids_after_a_cancel() {
    let h = Harness::new();
    let a = hundred_day_stream(&h);
    let b = hundred_day_stream(&h);

    h.advance(30 * DAY);
    h.client.cancel(&a);

    // One settle, one event — captured while the settle is still the most
    // recent invocation.
    let cancels_emitted = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();
    assert_eq!(cancels_emitted.len(), 1, "one settle must emit one event");

    for unknown in [b + 1, b + 100, 10_000, u64::MAX] {
        assert_eq!(
            h.client.try_get_stream(&unknown).unwrap_err().unwrap(),
            Error::StreamNotFound,
            "id {unknown} never existed and must stay unreachable",
        );
        assert!(!h.client.stream_exists(&unknown));
    }

    // And the surviving stream still reads normally next to the cancelled one.
    assert_eq!(h.get(b).status, StreamStatus::Active);
    assert_eq!(h.get(a).status, StreamStatus::Cancelled);
}
