//! [`CliffMode`] — schedule-relative vs wall-clock cliff gates.
//!
//! # What is being pinned
//!
//! The cliff *gates* the payout; it does not delay accrual, and that is true in
//! both modes. The only thing the mode changes is **which clock the gate is read
//! against**, and therefore what pausing does to it:
//!
//! | | gate opens at | paused across the cliff |
//! |---|---|---|
//! | [`CliffMode::Schedule`] | `stream_time(now) >= cliff_time`, i.e. `cliff_time + paused_total` in wall-clock terms | the gate freezes; nothing vests |
//! | [`CliffMode::WallClock`] | `now >= cliff_time` | the gate opens on its date; only what accrued before the pause is claimable |
//!
//! Neither mode changes the schedule, the rate, or the total value delivered.
//! A wall-clock stream paused before its cliff is not "kept accruing" — it is
//! accruing nothing, and what it exposes at the cliff is the backlog it had
//! accumulated before being frozen.
//!
//! `test::cliff::pause_across_cliff_delays_the_wall_clock_cliff` (issue #1688)
//! already pins the `Schedule` half and `docs/KNOWN-LIMITATIONS.md` §7 documents
//! it as by-design. This file is the other half, plus the paired tests that
//! show the two modes actually diverging under identical operations.
//!
//! # The fixture
//!
//! Every test uses one schedule — 10,000 seconds long with a 1,000-second cliff
//! and a `10_000 * ONE` deposit — so the rate is exactly `ONE` per second and
//! every expected figure is just "elapsed seconds", readable by eye.

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, Env, IntoVal, Symbol, TryFromVal, TryIntoVal, Val};

use super::common::*;
use crate::{accrual, CliffMode, Error};

/// 10,000 USDC over 10,000 seconds is exactly `ONE` per second, so
/// `vested == elapsed_seconds * ONE` for every stream built by [`scheduled`].
const DEPOSIT: i128 = 10_000 * ONE;
const DURATION: u64 = 10_000;
const CLIFF_OFFSET: u64 = 1_000;

/// Create a stream on the shared fixture schedule in the given cliff mode.
fn scheduled(h: &Harness, mode: CliffMode) -> u64 {
    let start = T0;
    h.create_with_cliff_mode(
        DEPOSIT,
        start,
        start + DURATION,
        start + CLIFF_OFFSET,
        mode,
        true,
        true,
        true,
    )
}

/// The `stream_created` payload emitted by the invocation that just ran,
/// decoded as a field map.
///
/// **Must be called immediately after the `create` it is inspecting.** The test
/// host exposes only the most recent invocation's events, so capturing after a
/// second `create` would return that one's payload and silently find nothing
/// for the first stream. `test::events::test_golden_events` accumulates across a
/// whole script for the same reason.
fn last_created_payload(h: &Harness) -> soroban_sdk::Map<Symbol, Val> {
    let events = h.env.events().all().filter_by_contract(&h.contract_id);
    for event in events.events().iter() {
        let soroban_sdk::xdr::ContractEventBody::V0(ref body) = event.body;
        let name: Symbol = Val::try_from_val(&h.env, &body.topics[0])
            .unwrap()
            .try_into_val(&h.env)
            .unwrap();
        if name != Symbol::new(&h.env, "stream_created") {
            continue;
        }
        let data = Val::try_from_val(&h.env, &body.data).unwrap();
        return data.try_into_val(&h.env).unwrap();
    }
    panic!("the last invocation emitted no stream_created event");
}

/// Read `field` out of a captured `stream_created` payload, typed.
fn created_field<T: TryFromVal<Env, Val>>(
    payload: &soroban_sdk::Map<Symbol, Val>,
    h: &Harness,
    field: &str,
) -> T {
    let val = payload
        .get(Symbol::new(&h.env, field))
        .unwrap_or_else(|| panic!("stream_created payload has no `{field}` field"));
    val.try_into_val(&h.env).unwrap()
}

// ---------------------------------------------------------------------------
// The default
// ---------------------------------------------------------------------------

/// `create_stream` — the v1 entry point — records `Schedule`, so a caller that
/// knows nothing about this feature gets exactly the behaviour it had before it
/// existed.
#[test]
fn create_stream_records_schedule_mode_by_default() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        DEPOSIT,
        start,
        start + DURATION,
        start + CLIFF_OFFSET,
        true,
        true,
        true,
    );

    assert_eq!(h.get(id).cliff_mode, CliffMode::Schedule);
    assert_eq!(CliffMode::DEFAULT, CliffMode::Schedule);
    h.assert_pool_exact();
}

/// Asking for `Schedule` explicitly is indistinguishable from not asking. Two
/// streams, one schedule each, must agree on every figure forever.
#[test]
fn an_explicit_schedule_mode_stream_is_indistinguishable_from_the_default() {
    let h = Harness::new();
    let start = h.now();
    let default_id = h.create(
        DEPOSIT,
        start,
        start + DURATION,
        start + CLIFF_OFFSET,
        true,
        true,
        true,
    );
    let explicit_id = scheduled(&h, CliffMode::Schedule);

    assert_eq!(h.get(default_id).cliff_mode, h.get(explicit_id).cliff_mode);

    for offset in [0u64, 1, CLIFF_OFFSET - 1, CLIFF_OFFSET, DURATION] {
        h.warp_to(start + offset);
        assert_eq!(
            h.client.vested_of(&default_id),
            h.client.vested_of(&explicit_id),
            "vested disagrees at +{offset}"
        );
        assert_eq!(
            h.client.withdrawable_of(&default_id),
            h.client.withdrawable_of(&explicit_id),
            "withdrawable disagrees at +{offset}"
        );
    }
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Visibility: get_stream and the event
// ---------------------------------------------------------------------------

#[test]
fn cliff_mode_is_readable_from_get_stream_for_both_modes() {
    let h = Harness::new();
    let schedule_id = scheduled(&h, CliffMode::Schedule);
    let wall_id = scheduled(&h, CliffMode::WallClock);

    assert_eq!(h.get(schedule_id).cliff_mode, CliffMode::Schedule);
    assert_eq!(h.get(wall_id).cliff_mode, CliffMode::WallClock);
    h.assert_pool_exact();
}

/// A recipient's whole guarantee is that these terms will not change later, so
/// the mode has to be visible in the event that carries the rest of the terms.
#[test]
fn the_created_event_publishes_the_cliff_mode() {
    let h = Harness::new();

    let schedule_id = scheduled(&h, CliffMode::Schedule);
    let schedule_payload = last_created_payload(&h);
    let schedule_mode: CliffMode = created_field(&schedule_payload, &h, "cliff_mode");
    assert_eq!(schedule_mode, CliffMode::Schedule);

    let wall_id = scheduled(&h, CliffMode::WallClock);
    let wall_payload = last_created_payload(&h);
    let wall_mode: CliffMode = created_field(&wall_payload, &h, "cliff_mode");
    assert_eq!(wall_mode, CliffMode::WallClock);

    // The event and storage cannot disagree: both are read off the same stream.
    assert_eq!(schedule_mode, h.get(schedule_id).cliff_mode);
    assert_eq!(wall_mode, h.get(wall_id).cliff_mode);
    h.assert_pool_exact();
}

/// The mode is a *field*, not just a value: it must sit in the payload so an
/// indexer decoding the event finds it, and the rest of the schedule must still
/// be there unchanged.
#[test]
fn the_created_event_field_sits_beside_the_unchanged_schedule() {
    let h = Harness::new();
    let id = scheduled(&h, CliffMode::WallClock);
    let payload = last_created_payload(&h);

    assert_eq!(created_field::<u64>(&payload, &h, "start_time"), T0);
    assert_eq!(
        created_field::<u64>(&payload, &h, "cliff_time"),
        T0 + CLIFF_OFFSET
    );
    assert!(created_field::<bool>(&payload, &h, "pausable"));
    assert!(created_field::<bool>(&payload, &h, "transferable"));
    assert_eq!(
        created_field::<CliffMode>(&payload, &h, "cliff_mode"),
        CliffMode::WallClock
    );

    // A `stream_created` payload is the indexer's whole picture of the stream, so
    // it must carry every field `get_stream` does.
    for field in [
        "cancellable",
        "cliff_mode",
        "cliff_time",
        "deposited",
        "end_time",
        "pausable",
        "start_time",
        "token",
        "transferable",
    ] {
        assert!(
            payload.contains_key(Symbol::new(&h.env, field)),
            "stream_created payload is missing `{field}`"
        );
    }
    let _ = id;
}

// ---------------------------------------------------------------------------
// The headline: the two modes diverge under a pause across the cliff
// ---------------------------------------------------------------------------

/// The issue's validation case, end to end.
///
/// One stream per mode on an identical schedule, both paused 100 seconds before
/// the cliff and held across it. Asserted at each step, because the interesting
/// difference is not a final number — it is *when* each gate opens and *how much*
/// is behind it at that moment.
#[test]
fn pausing_across_the_cliff_moves_a_schedule_cliff_but_not_a_wall_clock_one() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;

    let schedule_id = scheduled(&h, CliffMode::Schedule);
    let wall_id = scheduled(&h, CliffMode::WallClock);

    // 900 of 10,000 seconds elapsed, still 100 short of the cliff. Neither
    // mode has opened: `now < cliff_time` in both, and the stream clock is
    // running normally in both.
    h.warp_to(cliff - 100);
    assert_eq!(h.client.vested_of(&schedule_id), 0, "schedule pre-cliff");
    assert_eq!(h.client.vested_of(&wall_id), 0, "wall-clock pre-cliff");

    // The sender pauses both, then holds them across the cliff.
    h.client.pause(&schedule_id);
    h.client.pause(&wall_id);
    h.warp_to(cliff + 500);

    // Schedule: the gate is frozen below the cliff, so the recipient still has
    // nothing — the whole point of the mode, and the exposure §7 describes.
    assert_eq!(
        h.client.vested_of(&schedule_id),
        0,
        "schedule cliff must not open while frozen"
    );
    assert_eq!(h.client.withdrawable_of(&schedule_id), 0);

    // Wall-clock: the gate opened on its own date at T0 + 1000. The 900 seconds
    // that had accrued before the pause are now claimable, and no more.
    assert_eq!(
        h.client.vested_of(&wall_id),
        900 * ONE,
        "wall-clock cliff opens on its date, releasing the pre-pause backlog"
    );
    assert_eq!(h.client.withdrawable_of(&wall_id), 900 * ONE);

    // The recipient can actually take it while frozen: pausing stops accrual,
    // not access.
    assert_eq!(h.client.withdraw(&wall_id, &None), 900 * ONE);
    assert_eq!(h.client.withdrawable_of(&wall_id), 0);
    h.assert_pool_exact();

    // Resume both. `paused_total` absorbs the whole interval for each.
    h.client.resume(&schedule_id);
    h.client.resume(&wall_id);
    assert_eq!(h.get(schedule_id).paused_total, 600);
    assert_eq!(h.get(wall_id).paused_total, 600);

    // The schedule gate is still shut one second before `cliff + paused_total`,
    // and opens exactly at it. The wall-clock stream opened 600 seconds earlier.
    let moved_cliff = cliff + h.get(schedule_id).paused_total;
    h.warp_to(moved_cliff - 1);
    assert_eq!(
        h.client.vested_of(&schedule_id),
        0,
        "schedule gate must still be shut one second before the moved instant"
    );
    assert_eq!(
        h.client.vested_of(&wall_id),
        999 * ONE,
        "wall-clock stream keeps accruing normally once resumed"
    );

    h.warp_to(moved_cliff);
    assert_eq!(h.client.vested_of(&schedule_id), 1_000 * ONE);
    assert_eq!(h.client.vested_of(&wall_id), 1_000 * ONE);

    h.assert_pool_exact();
}

/// The complementary half of the same claim, stated on a wall-clock stream
/// alone: at exactly `cliff_time` the gate is open, even though a pause sits
/// inside the window.
#[test]
fn a_wall_clock_cliff_is_open_at_its_stored_second_despite_a_pause() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);

    h.warp_to(cliff - 100);
    h.client.pause(&id);

    h.warp_to(cliff - 1);
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "one second before the stored cliff the gate is still shut"
    );

    h.warp_to(cliff);
    assert_eq!(
        h.client.vested_of(&id),
        900 * ONE,
        "at the stored cliff the gate opens, releasing the pre-pause backlog"
    );
    h.assert_pool_exact();
}

/// And on a schedule stream it is not: the same instants, the opposite outcome.
/// Kept adjacent to the test above so the pair reads as one statement.
#[test]
fn a_schedule_cliff_is_not_open_at_its_stored_second_after_a_pause() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::Schedule);

    h.warp_to(cliff - 100);
    h.client.pause(&id);

    h.warp_to(cliff);
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "the stored cliff_time is a lower bound, not the opening instant"
    );
    assert_eq!(h.client.withdrawable_of(&id), 0);
    h.assert_pool_exact();
}

/// Pausing a wall-clock stream does not stop the gate from opening, and does not
/// restart accrual either: `vested` is flat across the cliff instant.
#[test]
fn a_wall_clock_pause_freezes_the_amount_not_the_gate() {
    let h = Harness::new();
    let cliff = T0 + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);

    h.warp_to(cliff - 100);
    h.client.pause(&id);
    let before = h.client.vested_of(&id);
    assert_eq!(before, 0);

    // Flat before, at, and after the cliff: the gate opens but accrual is frozen.
    h.warp_to(cliff);
    assert_eq!(h.client.vested_of(&id), 900 * ONE);
    h.warp_to(cliff + DAY);
    assert_eq!(
        h.client.vested_of(&id),
        900 * ONE,
        "a pause stops accrual; the cliff opening does not restart it"
    );

    h.client.resume(&id);
    h.advance(100);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// What the mode must NOT change
// ---------------------------------------------------------------------------

/// The headline safety claim: identical operations, identical totals. The mode
/// moves *when funds become claimable*, never *how much there is*.
#[test]
fn the_mode_never_changes_the_total_value_delivered() {
    let h = Harness::new();
    let start = T0;
    let schedule_id = scheduled(&h, CliffMode::Schedule);
    let wall_id = scheduled(&h, CliffMode::WallClock);

    // Same pause schedule applied to both, at matching stream-clock instants.
    h.warp_to(start + 500);
    h.client.pause(&schedule_id);
    h.client.pause(&wall_id);
    h.warp_to(start + 1_500);
    h.client.resume(&schedule_id);
    h.client.resume(&wall_id);
    let paused = h.get(schedule_id).paused_total;
    assert_eq!(paused, 1_000, "both absorbed the same interval");
    assert_eq!(h.get(wall_id).paused_total, paused);

    // Let both run past their (identically stretched) end. A 1,000-second pause
    // slides the effective end forward by exactly 1,000, which is the property
    // that must be mode-independent.
    h.warp_to(start + DURATION + paused);

    assert_eq!(h.get(schedule_id).end_time, h.get(wall_id).end_time);
    assert_eq!(h.client.vested_of(&schedule_id), DEPOSIT);
    assert_eq!(h.client.vested_of(&wall_id), DEPOSIT);
    assert_eq!(h.client.withdraw(&schedule_id, &None), DEPOSIT);
    assert_eq!(h.client.withdraw(&wall_id, &None), DEPOSIT);
    h.assert_pool_exact();
    h.assert_stream_count_consistent();
}

/// With no pause the two modes are pure duplicates — the mode only bites when
/// `pause` is reachable, which is a useful thing for an integrator to be able to
/// state.
#[test]
fn the_modes_agree_when_the_stream_cannot_be_paused() {
    let h = Harness::new();
    let start = T0;
    let end = start + DURATION;
    let cliff = start + CLIFF_OFFSET;
    let schedule_id = h.create_with_cliff_mode(
        DEPOSIT,
        start,
        end,
        cliff,
        CliffMode::Schedule,
        true,
        false,
        true,
    );
    let wall_id = h.create_with_cliff_mode(
        DEPOSIT,
        start,
        end,
        cliff,
        CliffMode::WallClock,
        true,
        false,
        true,
    );

    for offset in [CLIFF_OFFSET - 1, CLIFF_OFFSET, DURATION] {
        h.warp_to(start + offset);
        assert_eq!(
            h.client.vested_of(&schedule_id),
            h.client.vested_of(&wall_id),
            "modes must coincide at +{offset} on an unpausable stream"
        );
    }

    // And `pause` really is unavailable, so the equivalence is not vacuous.
    h.warp_to(start + 100);
    assert_eq!(
        h.client.try_pause(&wall_id).unwrap_err().unwrap(),
        Error::NotPausable
    );
    h.assert_pool_exact();
}

/// A top-up extends `end_time` but must not move a wall-clock cliff: the date the
/// recipient agreed to is the date, and lengthening the schedule afterwards does
/// not renegotiate it.
#[test]
fn a_top_up_does_not_move_a_wall_clock_cliff() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);

    h.warp_to(start + 100);
    h.client.top_up(&id, &DEPOSIT);

    let stream = h.get(id);
    assert_eq!(stream.deposited, 2 * DEPOSIT);
    assert_eq!(stream.end_time, start + 2 * DURATION, "duration doubled");
    assert_eq!(
        stream.cliff_time, cliff,
        "the cliff date is not renegotiated by a top-up"
    );

    // The gate still opens at the original second, now releasing twice the
    // backlog: 1,000 seconds of a 20,000-second schedule at 2x the deposit.
    h.warp_to(cliff - 1);
    assert_eq!(h.client.vested_of(&id), 0);
    h.warp_to(cliff);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.assert_pool_exact();
}

/// The mode is part of the terms, so no entry point may change it.
#[test]
fn cliff_mode_is_immutable_across_the_whole_lifecycle() {
    let h = Harness::new();
    let start = T0;
    let id = scheduled(&h, CliffMode::WallClock);
    let other = Address::generate(&h.env);

    let modes = [CliffMode::Schedule, CliffMode::WallClock];
    for expected in modes {
        let other_id = h.create_with_cliff_mode(
            DEPOSIT,
            start,
            start + DURATION,
            start + CLIFF_OFFSET,
            expected,
            true,
            true,
            true,
        );
        assert_eq!(h.get(other_id).cliff_mode, expected);
    }
    assert_eq!(
        h.get(id).cliff_mode,
        CliffMode::WallClock,
        "the stream under test must be a wall-clock one"
    );

    h.warp_to(start + 100);
    h.client.pause(&id);
    h.warp_to(start + 200);
    h.client.resume(&id);
    h.client.top_up(&id, &(DEPOSIT / DURATION as i128));
    h.client.transfer_recipient(&id, &other);
    h.warp_to(start + 2_000);
    h.client.withdraw(&id, &None);
    h.client.cancel(&id);

    assert_eq!(
        h.get(id).cliff_mode,
        CliffMode::WallClock,
        "cliff_mode must survive pause, resume, top-up, transfer, withdraw and cancel"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// `cliff_time` is validated against `[start_time, end_time]` identically in
/// both modes — the mode selects a clock, it does not widen the domain.
#[test]
fn cliff_range_validation_is_identical_in_both_modes() {
    let h = Harness::new();
    let start = T0;
    let end = start + DURATION;

    for mode in [CliffMode::Schedule, CliffMode::WallClock] {
        // Below start.
        assert_eq!(
            h.client
                .try_create_stream_with_cliff_mode(
                    &h.sender,
                    &h.recipient,
                    &h.token,
                    &DEPOSIT,
                    &start,
                    &end,
                    &(start - 1),
                    &mode,
                    &true,
                    &true,
                    &true,
                )
                .unwrap_err()
                .unwrap(),
            Error::InvalidCliff,
            "{mode:?} accepted a cliff before start"
        );
        // Above end.
        assert_eq!(
            h.client
                .try_create_stream_with_cliff_mode(
                    &h.sender,
                    &h.recipient,
                    &h.token,
                    &DEPOSIT,
                    &start,
                    &end,
                    &(end + 1),
                    &mode,
                    &true,
                    &true,
                    &true,
                )
                .unwrap_err()
                .unwrap(),
            Error::InvalidCliff,
            "{mode:?} accepted a cliff after end"
        );
        // Both boundary values stay legal.
        h.create_with_cliff_mode(DEPOSIT, start, end, start, mode, true, true, true);
        h.create_with_cliff_mode(DEPOSIT, start, end, end, mode, true, true, true);
    }

    assert_eq!(h.client.stream_count(), 4);
    h.assert_pool_exact();
}

/// The shared validation errors are unchanged by the new entry point.
#[test]
fn the_new_entry_point_keeps_every_creation_guard() {
    let h = Harness::new();
    let start = T0;
    let end = start + DURATION;
    let cliff = start + CLIFF_OFFSET;
    let wall = CliffMode::WallClock;

    assert_eq!(
        h.client
            .try_create_stream_with_cliff_mode(
                &h.sender, &h.sender, &h.token, &DEPOSIT, &start, &end, &cliff, &wall, &true,
                &true, &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::SelfStream
    );
    assert_eq!(
        h.client
            .try_create_stream_with_cliff_mode(
                &h.sender,
                &h.recipient,
                &h.token,
                &0,
                &start,
                &end,
                &cliff,
                &wall,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::InvalidDeposit
    );
    assert_eq!(
        h.client
            .try_create_stream_with_cliff_mode(
                &h.sender,
                &h.recipient,
                &h.token,
                &DEPOSIT,
                &start,
                &start,
                &start,
                &wall,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::InvalidTimeRange
    );
    assert_eq!(
        h.client
            .try_create_stream_with_cliff_mode(
                &h.sender,
                &h.recipient,
                &h.token,
                &1,
                &start,
                &end,
                &cliff,
                &wall,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::DepositRateTooLow
    );

    // No id was consumed by any of the rejected calls.
    assert_eq!(h.client.stream_count(), 0);
    h.assert_stream_count_consistent();
}

/// `cliff_time == start_time` is the encoding of "no cliff" and means the same
/// thing in both modes: accrual from the first second, with no gate.
#[test]
fn no_cliff_behaves_identically_in_both_modes() {
    let h = Harness::new();
    let start = T0;
    let schedule_id = h.create_with_cliff_mode(
        DEPOSIT,
        start,
        start + DURATION,
        start,
        CliffMode::Schedule,
        true,
        true,
        true,
    );
    let wall_id = h.create_with_cliff_mode(
        DEPOSIT,
        start,
        start + DURATION,
        start,
        CliffMode::WallClock,
        true,
        true,
        true,
    );

    for offset in [0u64, 1, 100, DURATION] {
        h.warp_to(start + offset);
        assert_eq!(
            h.client.vested_of(&schedule_id),
            offset as i128 * ONE,
            "schedule, +{offset}"
        );
        assert_eq!(
            h.client.vested_of(&wall_id),
            offset as i128 * ONE,
            "wall-clock, +{offset}"
        );
    }
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Fund safety: cancel, conservation, monotonicity
// ---------------------------------------------------------------------------

/// The case where a wall-clock gate is open while the stream is still frozen, and
/// the sender then cancels.
///
/// `cancel` sets `deposited = vested(now)` and pulls `end_time` back onto the
/// stream clock — and for a paused stream the stream clock is *behind* the
/// wall-clock cliff, so the collapsed schedule can end up shorter than
/// `cliff_time`. That is the shape that would strand an unwithdrawn tail behind a
/// shut gate if the gate were allowed to re-close. It cannot: a wall-clock gate
/// reads only `now` and the immutable `cliff_time`, so once `deposited > 0` the
/// gate is open for every future instant and the tail stays claimable. Asserted
/// rather than argued.
#[test]
fn cancelling_while_paused_past_a_wall_clock_cliff_leaves_the_tail_claimable() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);

    h.warp_to(cliff - 100);
    h.client.pause(&id);
    h.warp_to(cliff + 500);

    // Gate open, 900 seconds of backlog, schedule frozen at T0 + 900.
    assert_eq!(h.client.vested_of(&id), 900 * ONE);
    assert!(h.get(id).end_time > cliff, "end_time has not collapsed yet");

    let sender_before = h.balance(&h.sender);
    h.client.cancel(&id);

    // The backlog vested; everything after it is the sender's.
    let stream = h.get(id);
    assert_eq!(stream.status, crate::StreamStatus::Cancelled);
    assert_eq!(stream.deposited, 900 * ONE, "deposited collapsed to vested");
    assert_eq!(
        h.balance(&h.sender),
        sender_before + (DEPOSIT - 900 * ONE),
        "refund is the unvested remainder"
    );

    // `end_time` is now behind `cliff_time` — the dangerous shape.
    assert!(
        stream.end_time < cliff,
        "collapsed schedule ends before the cliff: {:?}",
        stream
    );

    // ...and the tail is still claimable, at the cancel instant and forever
    // after. I1 (`withdrawn <= vested`) is checked by every pool assertion.
    assert_eq!(h.client.withdrawable_of(&id), 900 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 900 * ONE);
    h.warp_to(cliff + DAY);
    assert_eq!(h.client.withdrawable_of(&id), 0, "nothing new accrues");
    h.assert_pool_exact();
}

/// The mirror image: cancelling *before* the wall-clock cliff refunds everything,
/// and the gate opening later does not resurrect the entitlement.
#[test]
fn cancelling_before_a_wall_clock_cliff_refunds_everything_permanently() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);

    let sender_before = h.balance(&h.sender);
    h.warp_to(cliff - 1);
    h.client.cancel(&id);

    assert_eq!(h.balance(&h.sender), sender_before + DEPOSIT);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.pool(), 0);

    // The cliff date arrives after the stream is gone. The settled schedule is a
    // fully matured one, so `vested` clamps to the reduced deposit — zero.
    h.warp_to(cliff + 10 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "a cancelled stream cannot re-vest when its cliff date passes"
    );
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();
}

/// Cancelling exactly at the wall-clock cliff: the whole backlog is the
/// recipient's and the rest is the sender's, with no residue.
#[test]
fn cancelling_exactly_at_a_wall_clock_cliff_splits_the_deposit_exactly() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);

    h.warp_to(cliff);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    let sender_before = h.balance(&h.sender);
    h.client.cancel(&id);

    assert_eq!(
        h.balance(&h.sender),
        sender_before + (DEPOSIT - 1_000 * ONE)
    );
    assert_eq!(h.client.withdrawable_of(&id), 1_000 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

/// I3 — no operation may reduce `vested` at a fixed instant, in wall-clock mode.
/// Checked with the clock frozen, which is the only way the property is
/// observable (see the `accrual` module docs).
#[test]
fn no_operation_reduces_vested_on_a_wall_clock_stream() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);
    let other = Address::generate(&h.env);

    // Before the cliff: the gate is shut, so `vested` is 0 and a regression
    // would be invisible. Advance past it so the gate is open and the figure
    // is non-trivial.
    h.warp_to(cliff);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    let before = h.vested_snapshot();

    h.client.pause(&id);
    h.assert_no_vested_regression(&before, "pause");
    let before = h.vested_snapshot();

    h.client.resume(&id);
    h.assert_no_vested_regression(&before, "resume");
    let before = h.vested_snapshot();

    h.client.top_up(&id, &(DEPOSIT / DURATION as i128));
    h.assert_no_vested_regression(&before, "top_up");
    let before = h.vested_snapshot();

    h.client.transfer_recipient(&id, &other);
    h.assert_no_vested_regression(&before, "transfer_recipient");
    let before = h.vested_snapshot();

    h.client.withdraw(&id, &None);
    h.assert_no_vested_regression(&before, "withdraw");

    h.assert_pool_exact();
}

/// A wall-clock stream's gate is monotone in time and invariant across calls,
/// because it reads only `now` and the immutable `cliff_time`. Asserted directly
/// against the pure predicate so the property is stated, not just implied.
#[test]
fn the_wall_clock_gate_is_monotone_in_time_and_unchanged_by_pauses() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);

    h.warp_to(cliff - 100);
    h.client.pause(&id);
    h.warp_to(cliff + 100);

    let mut opened = None;
    for offset in (CLIFF_OFFSET - 100)..=(CLIFF_OFFSET + 100) {
        let t = start + offset;
        let stream = h.get(id);
        let reached = accrual::cliff_reached(&stream, t);
        if let Some(first) = opened {
            assert!(reached, "wall-clock gate re-closed at +{offset}");
            assert!(offset >= first, "gate opened non-monotonically");
        } else if reached {
            opened = Some(offset);
        }
    }
    assert_eq!(
        opened,
        Some(CLIFF_OFFSET),
        "gate opens exactly at cliff_time"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Interaction with the rest of the surface
// ---------------------------------------------------------------------------

/// The delegate withdrawal path sees the same gate the recipient does.
#[test]
fn a_delegate_can_withdraw_at_a_wall_clock_cliff() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let id = scheduled(&h, CliffMode::WallClock);
    let delegate = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &delegate, &crate::op::WITHDRAW, &None);

    h.warp_to(cliff - 1);
    assert_eq!(
        h.client
            .try_delegate_withdraw(&id, &delegate, &None)
            .unwrap_err()
            .unwrap(),
        Error::NothingToWithdraw,
        "the gate is shut one second before the wall-clock cliff"
    );

    h.warp_to(cliff);
    assert_eq!(
        h.client.delegate_withdraw(&id, &delegate, &None),
        1_000 * ONE
    );
    h.assert_pool_exact();
}

/// Batch withdrawal reads the same gate per stream, so a mixed batch splits
/// cleanly at the wall-clock cliff.
#[test]
fn batch_withdraw_splits_at_a_wall_clock_cliff() {
    let h = Harness::new();
    let start = T0;
    let cliff = start + CLIFF_OFFSET;
    let end = start + DURATION;
    let late = h.create_with_cliff_mode(
        DEPOSIT,
        start,
        end,
        cliff + 100,
        CliffMode::WallClock,
        true,
        true,
        true,
    );
    let due = scheduled(&h, CliffMode::WallClock);

    h.warp_to(cliff);
    // `late` is 100 seconds short of its own cliff; `due` is exactly on it.
    assert_eq!(h.client.vested_of(&late), 0);
    assert_eq!(h.client.vested_of(&due), 1_000 * ONE);

    assert_eq!(
        h.client.batch_withdraw(&h.recipient, &h.ids(&[late, due])),
        1_000 * ONE
    );
    assert_eq!(h.client.withdrawable_of(&due), 0);
    assert_eq!(h.client.withdrawable_of(&late), 0);
    h.assert_pool_exact();
}

/// A mode the contract does not define must be refused by the host while the
/// arguments are still being decoded, so it cannot reach the validation order,
/// consume a stream id, or pull a deposit. There is deliberately no
/// `InvalidCliffMode` error: the rejection is a decode failure, not a
/// contract-level outcome, and `docs/ABI.md` documents it that way.
///
/// The generated client types the parameter as `&CliffMode`, so the only way to
/// put a bad value on the wire is to call the entry point directly. The wire
/// encoding of a `CliffMode` is a bare `u32`, which this test also pins.
#[test]
fn an_undefined_cliff_mode_is_refused_before_any_state_is_touched() {
    let h = Harness::new();
    let count_before = h.client.stream_count();
    let sender_before = h.balance(&h.sender);

    let args: soroban_sdk::Vec<soroban_sdk::Val> = soroban_sdk::vec![
        &h.env,
        h.sender.clone().into_val(&h.env),
        h.recipient.clone().into_val(&h.env),
        h.token.clone().into_val(&h.env),
        DEPOSIT.into_val(&h.env),
        T0.into_val(&h.env),
        (T0 + DURATION).into_val(&h.env),
        (T0 + CLIFF_OFFSET).into_val(&h.env),
        soroban_sdk::Val::from(7u32),
        true.into_val(&h.env),
        true.into_val(&h.env),
        true.into_val(&h.env),
    ];
    let outcome: Result<
        Result<u64, soroban_sdk::Error>,
        Result<crate::Error, soroban_sdk::InvokeError>,
    > = h.env.try_invoke_contract(
        &h.contract_id,
        &soroban_sdk::Symbol::new(&h.env, "create_stream_with_cliff_mode"),
        args,
    );

    // The host refused the call. The decode happens in the contract's argument
    // unwrapping, which panics on the ConversionError, so the whole invocation
    // is rolled back — that is what makes the no-state-changed claim hold. Note
    // the failure is *not* one of this contract's `Error` values: no
    // `InvalidCliffMode` discriminant exists, and adding one would mean
    // unreachable code.
    assert!(
        outcome.is_err(),
        "a discriminant that is neither 0 nor 1 must not create a stream: {outcome:?}"
    );

    assert_eq!(h.client.stream_count(), count_before, "no id was consumed");
    assert_eq!(h.balance(&h.sender), sender_before, "no deposit was pulled");
    h.assert_pool_exact();
}
