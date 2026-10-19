//! Issue #1841 — a stream whose duration is exactly one second.
//!
//! `end_time - start_time == 1` is the smallest non-degenerate schedule the
//! contract accepts, and it is the one duration where the two things that
//! normally make a schedule interesting both disappear:
//!
//! * **No rounding.** `vested` is `deposited * elapsed / duration`, so with
//!   `duration == 1` and `elapsed in {0, 1}` every result is exact — there is
//!   no residue, and `vested + refundable == deposited` holds in stroops
//!   rather than approximately. A one-second stream therefore pins the rounding
//!   *direction* without any of the arithmetic hiding behind truncation.
//! * **No reachable dust-rate floor.** `create_stream` rejects
//!   `deposit < duration` ([`Error::DepositRateTooLow`]); at a one-second
//!   duration that floor collapses to `deposit >= 1`, which
//!   [`Error::InvalidDeposit`] already enforces. The floor is unreachable here,
//!   and this module says so rather than leaving it implied.
//!
//! The rest of the module checks that the shortest schedule still behaves like
//! a schedule: it can be paused, cancelled before it opens and after it
//! completes, topped up, and withdrawn from — each at the second boundaries
//! where an off-by-one would show up.
//!
//! Boundary instants are probed explicitly rather than by advancing in
//! bulk, because at this duration the whole vesting curve is two points.

use super::common::*;
use crate::{Error, StreamStatus};
use soroban_sdk::{testutils::Events, Map, Symbol, TryFromVal, TryIntoVal, Val};

/// Minimal one-second stream. One stroop per second is exactly the rate floor.
fn one_second_stream(h: &Harness, deposit: i128) -> u64 {
    let start = h.now();
    h.create(deposit, start, start + 1, start, true, true, true)
}

/// The `Map` payload of the most recent event from the stream contract.
fn last_event_payload(h: &Harness) -> Map<Symbol, Val> {
    let raw = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();
    let ev = raw
        .last()
        .expect("the last invocation must have emitted an event");
    let soroban_sdk::xdr::ContractEventBody::V0(body) = &ev.body;
    let data: Val = Val::try_from_val(&h.env, &body.data).unwrap();
    data.try_into_val(&h.env).unwrap()
}

/// Read an integer field out of a decoded event payload.
fn payload_u64(h: &Harness, map: &Map<Symbol, Val>, field: &str) -> u64 {
    map.get(Symbol::new(&h.env, field))
        .unwrap_or_else(|| panic!("event payload has no field {field}"))
        .try_into_val(&h.env)
        .unwrap()
}

// ---------------------------------------------------------------------------
// The schedule itself
// ---------------------------------------------------------------------------

/// A one-second schedule is accepted, and it is a *two-point* vesting curve:
/// nothing before `end_time`, everything at it.
#[test]
fn a_one_second_stream_is_accepted_and_vests_fully_at_its_end_time() {
    let h = Harness::new();
    let id = one_second_stream(&h, ONE);

    let s = h.get(id);
    assert_eq!(s.start_time, T0);
    assert_eq!(s.end_time, T0 + 1, "duration must be exactly one second");
    assert_eq!(s.end_time - s.start_time, 1);
    assert_eq!(s.status, StreamStatus::Active);

    assert_eq!(h.client.vested_of(&id), 0, "nothing vests at start_time");
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), ONE, "all of it still locked");

    h.advance(1);
    assert_eq!(
        h.client.vested_of(&id),
        ONE,
        "the whole deposit vests at start_time + 1s",
    );
    assert_eq!(h.client.withdrawable_of(&id), ONE);
    assert_eq!(h.client.refundable_of(&id), 0);

    h.assert_pool_exact();
}

/// The second boundary is a hard edge, not a ramp: probing at the start instant
/// and one second later yields only the two endpoints, never an intermediate
/// value that would betray a rounding or scaling mistake.
#[test]
fn a_one_second_stream_is_never_partially_vested() {
    let h = Harness::new();
    // A deposit that is not a round number of seconds of anything, so a stray
    // scale factor would show up immediately.
    let deposit = 7 * ONE + 13;
    let id = one_second_stream(&h, deposit);

    let mut seen = std::vec::Vec::new();
    for _ in 0..4 {
        seen.push(h.client.vested_of(&id));
        h.advance(1);
        seen.push(h.client.vested_of(&id));
        h.advance(DAY); // far past the end; must stay clamped
    }

    assert!(
        seen.iter().all(|&v| v == 0 || v == deposit),
        "a one-second schedule may only ever report 0 or the full deposit, saw {seen:?}",
    );
    assert_eq!(h.client.vested_of(&id), deposit);
}

/// The minimum viable stream: one stroop, one second. It is accepted, it is
/// whole, and it survives a withdrawal that empties it.
#[test]
fn the_minimum_viable_stream_is_one_stroop_over_one_second() {
    let h = Harness::new();
    let sender_before = h.balance(&h.sender);
    let id = one_second_stream(&h, 1);

    assert_eq!(h.get(id).deposited, 1);
    assert_eq!(h.balance(&h.sender), sender_before - 1, "one stroop pulled");

    h.advance(1);
    assert_eq!(h.client.withdrawable_of(&id), 1);
    assert_eq!(h.client.withdraw(&id, &None), 1);
    assert_eq!(h.balance(&h.recipient), 1, "exactly one stroop, not zero");
    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    h.assert_pool_exact();
}

/// `DepositRateTooLow` is unreachable at a one-second duration: the floor is
/// `deposit >= 1`, which `create_stream` has already enforced as
/// `deposit > 0`. The only rejectable deposit is therefore a non-positive one,
/// and it is rejected as `InvalidDeposit`.
#[test]
fn the_dust_rate_floor_is_unreachable_at_a_one_second_duration() {
    let h = Harness::new();
    let start = h.now();

    for deposit in [0i128, -1, i128::MIN] {
        let err = h
            .client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &deposit,
                &start,
                &(start + 1),
                &start,
                &true,
                &true,
                &true,
                &None,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(
            err,
            Error::InvalidDeposit,
            "deposit {deposit} must fail as InvalidDeposit, never DepositRateTooLow",
        );
    }

    // Every positive deposit clears the rate floor, so the boundary is exactly
    // between 0 and 1 stroop.
    let id = one_second_stream(&h, 1);
    assert_eq!(h.get(id).deposited, 1);
}

/// There is no rounding residue to strand: at every instant the two halves of
/// the deposit partition it exactly, and the pooled balance matches the
/// outstanding liability to the stroop.
#[test]
fn a_one_second_stream_settles_with_no_rounding_residue() {
    let h = Harness::new();
    // Deliberately not a divisor of anything: 1 second, 1_000_003 stroops.
    let deposit = 1_000_003i128;
    let sender_before = h.balance(&h.sender);
    let id = one_second_stream(&h, deposit);

    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), deposit);
    h.assert_pool_exact();

    h.advance(1);
    assert_eq!(h.client.vested_of(&id), deposit);
    assert_eq!(h.client.refundable_of(&id), 0);
    assert_eq!(
        h.client.vested_of(&id) + h.client.refundable_of(&id),
        deposit,
        "I4 must hold stroop-exactly",
    );

    assert_eq!(h.client.withdraw(&id, &None), deposit);
    assert_eq!(h.balance(&h.recipient), deposit);
    assert_eq!(h.balance(&h.sender), sender_before - deposit);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Cancellation at both ends of the second
// ---------------------------------------------------------------------------

/// Cancelling before the one second has elapsed refunds everything and leaves a
/// **zero-length** schedule — `end_time == start_time`, never one second
/// *before* the start, which is what a naive `now`-based collapse would produce.
#[test]
fn cancelling_a_one_second_stream_before_it_opens_refunds_everything() {
    let h = Harness::new();
    let sender_before = h.balance(&h.sender);
    let id = one_second_stream(&h, 100 * ONE);

    h.client.cancel(&id);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(
        s.deposited, 0,
        "nothing had vested, so nothing was claimable"
    );
    assert_eq!(
        s.end_time, s.start_time,
        "the collapsed schedule must be zero-length, not inverted",
    );
    assert_eq!(h.balance(&h.sender), sender_before, "fully refunded");

    // A zero-length cancelled schedule must still answer every view without
    // dividing by zero.
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), 0);
    h.assert_pool_exact();
}

/// Cancelling *after* the second completes refunds nothing and leaves the whole
/// deposit as the recipient's claim — and the collapsed schedule keeps its
/// one-second length, so the record still explains how that claim arose.
#[test]
fn cancelling_a_one_second_stream_after_it_completes_pays_the_recipient_in_full() {
    let h = Harness::new();
    let sender_before = h.balance(&h.sender);
    let id = one_second_stream(&h, 100 * ONE);

    h.advance(1);
    h.client.cancel(&id);

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Cancelled);
    assert_eq!(s.deposited, 100 * ONE);
    assert_eq!(
        s.end_time,
        s.start_time + 1,
        "the schedule keeps its original one-second length"
    );
    assert_eq!(s.paused_at, None);
    assert_eq!(
        h.balance(&h.sender),
        sender_before - 100 * ONE,
        "nothing was refunded",
    );

    assert_eq!(h.client.withdrawable_of(&id), 100 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 100 * ONE);
    assert_eq!(h.balance(&h.recipient), 100 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Pausing and topping up the shortest schedule
// ---------------------------------------------------------------------------

/// A one-second stream can be paused for longer than it was ever scheduled to
/// last, and the stretch is still exact: one second of clock, no value created
/// or destroyed.
#[test]
fn pausing_a_one_second_stream_stretches_it_by_exactly_the_pause() {
    let h = Harness::new();
    let id = one_second_stream(&h, 50 * ONE);

    h.client.pause(&id);
    assert_eq!(h.get(id).paused_at, Some(T0));

    // Frozen for far longer than the whole schedule.
    h.advance(30 * DAY);
    assert_eq!(h.client.vested_of(&id), 0, "frozen means frozen");

    h.client.resume(&id);
    assert_eq!(h.get(id).paused_total, 30 * DAY);
    assert_eq!(h.client.vested_of(&id), 0, "no jump on resume");

    // The single scheduled second now sits 30 days out.
    h.warp_to(T0 + 30 * DAY);
    assert_eq!(h.client.vested_of(&id), 0);
    h.warp_to(T0 + 30 * DAY + 1);
    assert_eq!(
        h.client.vested_of(&id),
        50 * ONE,
        "exactly one second of stream clock, stretched by exactly the pause",
    );
    h.assert_pool_exact();
}

/// The smallest legal top-up on a one-second schedule buys exactly one more
/// second: `delta = floor(amount * 1 / deposited)`, so topping up the whole
/// deposit doubles the duration to two seconds.
#[test]
fn a_one_second_stream_can_be_topped_up_into_a_two_second_schedule() {
    let h = Harness::new();
    let id = one_second_stream(&h, 10 * ONE);

    h.client.top_up(&id, &(10 * ONE));

    let s = h.get(id);
    assert_eq!(s.deposited, 20 * ONE);
    assert_eq!(s.end_time, T0 + 2, "one more second bought");
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "I3: nothing re-vested by the top-up"
    );

    h.advance(1);
    assert_eq!(
        h.client.vested_of(&id),
        10 * ONE,
        "one of the two seconds has elapsed",
    );
    h.advance(1);
    assert_eq!(h.client.vested_of(&id), 20 * ONE);
    h.assert_pool_exact();
}

/// A top-up too small to buy a whole second is refused rather than silently
/// absorbed into a rate change, which is what keeps a one-second schedule's
/// rate invariant.
#[test]
fn a_sub_second_top_up_is_refused_on_a_one_second_schedule() {
    let h = Harness::new();
    let id = one_second_stream(&h, 10 * ONE);

    // delta = floor(9 * 1 / 10) = 0.
    let err = h.client.try_top_up(&id, &9).unwrap_err().unwrap();
    assert_eq!(err, Error::TopUpTooSmall);

    let s = h.get(id);
    assert_eq!(s.deposited, 10 * ONE, "a refused top-up changes nothing");
    assert_eq!(s.end_time, T0 + 1);
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// `StreamCreated` for a one-second stream carries the exact one-second
/// schedule — an off-by-one here would be invisible in storage but wrong for
/// every indexer rebuilding the schedule from the event alone.
#[test]
fn the_created_event_carries_the_exact_one_second_schedule() {
    let h = Harness::new();
    let id = one_second_stream(&h, 42 * ONE);

    // `stream_id`, `sender` and `recipient` are topics on this event, not data
    // fields, so the payload map carries only the schedule itself.
    let payload = last_event_payload(&h);
    assert_eq!(payload_u64(&h, &payload, "start_time"), T0);
    assert_eq!(payload_u64(&h, &payload, "end_time"), T0 + 1);
    assert_eq!(
        payload_u64(&h, &payload, "end_time") - payload_u64(&h, &payload, "start_time"),
        1,
    );

    // And the event agrees with storage, not merely with itself.
    let s = h.get(id);
    assert_eq!(payload_u64(&h, &payload, "end_time"), s.end_time);
    assert_eq!(payload_u64(&h, &payload, "start_time"), s.start_time);
}
