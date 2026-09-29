//! Issue #1850 — `stream_exists` for an id beyond `stream_count()`.
//!
//! `stream_exists(id)` answers a boolean question, but two distinct facts sit
//! behind it:
//!
//! * the **id counter** (`stream_count()`), which only moves forward and records
//!   every id ever issued; and
//! * the **persistent population** (`DataKey::Stream(id)`), which an entry can
//!   leave — by archiving once its TTL runs out — without the counter moving.
//!
//! An id at or beyond the counter has never been issued; an id *below* the
//! counter that is missing from storage was issued and is now archived. Both
//! make `stream_exists` return `false` and both make every `Result` view return
//! `StreamNotFound`, so the only discriminator is the comparison against
//! `stream_count()` that `docs/ABI.md` (Client requirements) prescribes:
//! `id < stream_count()` means "archived, needs restoring", while
//! `id >= stream_count()` means "never issued".
//!
//! This module pins that boundary — that it tracks the population as it grows,
//! that a failed create does not move it, that probing it is free of side
//! effects, and that it is distinguishable from an archived id — end to end
//! through the public ABI.
//!
//! | acceptance criterion | test |
//! |---|---|
//! | exercised end to end through the public ABI | `the_boundary_scenario_conserves_funds_end_to_end` |
//! | funds conservation asserted | `probing_beyond_the_count_moves_no_funds`, `the_boundary_scenario_conserves_funds_end_to_end` |
//! | matches `docs/ABI.md` | `an_id_beyond_the_count_is_distinguishable_from_an_archived_id` |
//! | fails if the behaviour it pins is changed | every assertion below |

use soroban_sdk::testutils::Events as _;
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::{Cancelled, StreamCreated, Withdrawn};
use crate::{DataKey, Error, StreamStatus};

/// The events the *stream* contract published during the last invocation.
///
/// `Events::all()` is invocation-scoped, so this must be captured immediately
/// after the call under test — before any other client call, read-only views
/// included, replaces the snapshot. Same shape as `test::withdraw_events`.
fn published_by_stream(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// Ids are contiguous from zero, so the first id that has never been issued is
/// exactly `stream_count()`. On a contract that has issued nothing, that is id
/// `0` — the "first id" is already beyond the count.
#[test]
fn the_first_id_is_beyond_the_count_of_an_empty_contract() {
    let h = Harness::new();

    assert_eq!(
        h.client.stream_count(),
        0,
        "a fresh contract has issued no ids"
    );
    // The counter is zero, so id 0 is already at the boundary.

    // No id whatsoever exists, up to and including the extreme — the probe
    // must be total and must not wrap or panic.
    for id in [0u64, 1, 2, 7, 1_000, u64::MAX] {
        assert!(
            !h.client.stream_exists(&id),
            "id {id} was never issued and must not exist",
        );
    }

    // Probing cannot have moved the boundary, the pool, or any balance.
    assert_eq!(
        h.client.stream_count(),
        0,
        "probing must not advance the counter"
    );
    assert_eq!(h.pool(), 0, "an empty contract holds nothing");
    h.assert_pool_exact();
}

/// The boundary is not a constant: it moves with the population, staying
/// exactly one past the newest id after every successful create.
#[test]
fn the_boundary_tracks_the_population_as_it_grows() {
    let h = Harness::new();
    let deposits = [1_000i128 * ONE, 250 * ONE, 3 * ONE];

    for (expected_id, deposit) in deposits.iter().enumerate() {
        let id = h.create_simple(*deposit, 10 * DAY);
        // Capture the invocation-scoped event buffer before any read replaces
        // it — `stream_count()` below is itself a contract call.
        let events = published_by_stream(&h);
        assert_eq!(id as usize, expected_id, "ids run 0.. without gaps");

        let count = h.client.stream_count();
        assert_eq!(
            count,
            id + 1,
            "the boundary sits exactly one past the newest id",
        );

        // Every issued id is inside the boundary and readable; the boundary
        // itself is the first id that has never been issued.
        for issued in 0..count {
            assert!(
                h.client.stream_exists(&issued),
                "issued id {issued} must exist below the count {count}",
            );
        }
        assert!(
            !h.client.stream_exists(&count),
            "id {count} is beyond the count and must not exist",
        );

        // The create event is the only witness that the id was issued: it must
        // name the new id and carry the full initial state.
        let stream = h.get(id);
        let expected = StreamCreated {
            stream_id: id,
            sender: h.sender.clone(),
            recipient: h.recipient.clone(),
            token: h.token.clone(),
            deposited: *deposit,
            start_time: stream.start_time,
            end_time: stream.end_time,
            cliff_time: stream.cliff_time,
            cancellable: true,
            pausable: true,
            transferable: true,
        };
        assert_eq!(
            events,
            std::vec![expected.to_xdr(&h.env, &h.contract_id)],
            "create must publish exactly one StreamCreated naming id {id}",
        );
    }

    assert_eq!(h.client.stream_count(), 3);
    h.assert_pool_exact();
}

/// For an id at or beyond the count, `stream_exists` is `false` and every
/// `Result` view reports `StreamNotFound` — for the boundary id, an id just
/// past it, a far-off id, and `u64::MAX` alike.
#[test]
fn an_id_beyond_stream_count_does_not_exist() {
    let h = Harness::new();
    let first = h.create_simple(100 * ONE, 10 * DAY);
    let second = h.create_simple(200 * ONE, 20 * DAY);
    let third = h.create_simple(300 * ONE, 30 * DAY);
    let count = h.client.stream_count();
    assert_eq!([first, second, third], [0, 1, 2]);
    assert_eq!(count, 3);

    for id in [count, count + 1, count + 2, count + 97, u64::MAX] {
        assert!(id >= count, "the probe must be at or beyond the boundary");
        assert!(
            !h.client.stream_exists(&id),
            "id {id} is beyond the count and was never issued",
        );
        assert_eq!(
            h.client.try_get_stream(&id).unwrap_err().unwrap(),
            Error::StreamNotFound,
            "get_stream({id}) must report the missing entry",
        );
    }

    // The probe left the boundary, the population, and the funds untouched.
    assert_eq!(
        h.client.stream_count(),
        count,
        "probing must not move the boundary"
    );
    for issued in 0..count {
        assert!(
            h.client.stream_exists(&issued),
            "issued id {issued} must survive"
        );
    }
    h.assert_pool_exact();
}

/// The heart of the issue: `false` from `stream_exists` is ambiguous on its own,
/// and the count is what disambiguates it.
///
/// `docs/ABI.md` (Client requirements) states that `stream_exists(id) == false`
/// with `id < stream_count()` means the entry was archived and needs restoring,
/// not that it never existed. An id at or beyond the count has no such history.
#[test]
fn an_id_beyond_the_count_is_distinguishable_from_an_archived_id() {
    let h = Harness::new();
    let archived = h.create_simple(100 * ONE, 10 * DAY);
    let live = h.create_simple(200 * ONE, 20 * DAY);
    assert_eq!([archived, live], [0, 1]);

    // Archive id 0 the way a TTL expiry removes it from the live ledger: the
    // record is gone, but the counter still counts the id as issued.
    let archived_stream = h.get(archived);
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .remove(&DataKey::Stream(archived));
    });
    assert_eq!(h.client.stream_count(), 2, "archiving must not free the id");

    // Both ids report `false`, and a read gives the *same* error for both — so
    // the boolean and the error alone cannot tell them apart.
    assert!(!h.client.stream_exists(&archived));
    assert!(!h.client.stream_exists(&2));
    assert_eq!(
        h.client.try_get_stream(&archived).unwrap_err().unwrap(),
        Error::StreamNotFound,
    );
    assert_eq!(
        h.client.try_get_stream(&2).unwrap_err().unwrap(),
        Error::StreamNotFound,
    );

    // The count is the discriminator, exactly as documented.
    let count = h.client.stream_count();
    assert!(
        archived < count,
        "archived id {archived} stays below the count {count} and needs restoring",
    );
    assert!(
        2 >= count,
        "id 2 is at or beyond the count {count} and was never issued",
    );
    assert!(
        h.client.stream_exists(&live),
        "the live record is unaffected"
    );

    // Restore the archived entry — the client action the docs prescribe — and
    // the same id is readable again without the counter having moved.
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .set(&DataKey::Stream(archived), &archived_stream);
    });
    assert!(h.client.stream_exists(&archived));
    assert_eq!(
        h.get(archived).deposited,
        100 * ONE,
        "the restored record is intact"
    );
    assert!(
        !h.client.stream_exists(&2),
        "restoring a record does not create id 2"
    );
    assert_eq!(h.client.stream_count(), count, "the counter is unchanged");
    h.assert_pool_exact();
}

/// A create that fails — at a validation gate, or partway at the deposit
/// transfer — must not consume the boundary. The next successful create gets
/// the exact id the failed attempt would have used.
#[test]
fn a_failed_create_does_not_move_the_boundary() {
    let h = Harness::new();
    let sender_before = h.balance(&h.sender);
    let start = h.now();

    // A validation-gate rejection: fails before the id counter is consulted.
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.sender,
            &h.token,
            &(10 * ONE),
            &start,
            &(start + DAY),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::SelfStream);

    // A partway failure: clears every gate, then fails at the deposit transfer
    // after the id has been read off the counter.
    let too_much = h.balance(&h.sender) + 1;
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &too_much,
            &start,
            &(start + DAY),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // Neither attempt moved the boundary: id 0 is still beyond the count and
    // still unissued, no funds moved, and no create event was published.
    assert_eq!(
        h.client.stream_count(),
        0,
        "a failed create must not consume an id"
    );
    assert!(!h.client.stream_exists(&0));
    assert!(!h.client.stream_exists(&1));
    assert_eq!(
        h.balance(&h.sender),
        sender_before,
        "a failed create must move no funds"
    );
    assert_eq!(h.pool(), 0);
    assert!(
        published_by_stream(&h).is_empty(),
        "a failed create must publish no StreamCreated event",
    );

    // The retried create receives the id the failed attempt would have used.
    let id = h.create_simple(100 * ONE, DAY);
    assert_eq!(id, 0, "a failed create must not consume an id");
    assert_eq!(h.client.stream_count(), 1);
    assert!(h.client.stream_exists(&0));
    assert!(!h.client.stream_exists(&1), "id 1 is beyond the new count");
    assert_eq!(
        h.balance(&h.sender),
        sender_before - 100 * ONE,
        "exactly the successful deposit left the sender",
    );
    h.assert_pool_exact();
}

/// Every read-only view rejects an id beyond the count: the boolean view says
/// `false`, and the four `Result` views return `StreamNotFound` — with no event
/// and no state change, exactly as the views section of `docs/ABI.md` promises.
#[test]
fn every_view_rejects_an_id_beyond_the_count() {
    let h = Harness::new();
    let first = h.create_simple(100 * ONE, 10 * DAY);
    let second = h.create_simple(200 * ONE, 20 * DAY);
    let count = h.client.stream_count();
    assert_eq!([first, second], [0, 1]);
    assert_eq!(count, 2);

    for id in [count, count + 1, u64::MAX] {
        assert!(id >= count);
        assert!(!h.client.stream_exists(&id), "stream_exists({id})");
        assert_eq!(
            h.client.try_get_stream(&id).unwrap_err().unwrap(),
            Error::StreamNotFound,
            "get_stream({id})",
        );
        assert_eq!(
            h.client.try_withdrawable_of(&id).unwrap_err().unwrap(),
            Error::StreamNotFound,
            "withdrawable_of({id})",
        );
        assert_eq!(
            h.client.try_vested_of(&id).unwrap_err().unwrap(),
            Error::StreamNotFound,
            "vested_of({id})",
        );
        assert_eq!(
            h.client.try_refundable_of(&id).unwrap_err().unwrap(),
            Error::StreamNotFound,
            "refundable_of({id})",
        );
    }

    // Views are read-only: the last probe left no event, and neither the
    // boundary nor the live population moved.
    assert!(
        published_by_stream(&h).is_empty(),
        "views must publish no events",
    );
    assert_eq!(h.client.stream_count(), count);
    assert!(h.client.stream_exists(&first));
    assert!(h.client.stream_exists(&second));
    h.assert_pool_exact();
}

/// Probing beyond the count is free: it cannot move a stroop, change a stream
/// record, or advance the ledger. A full snapshot must compare equal across a
/// dense band of probes and the `u64::MAX` extreme.
#[test]
fn probing_beyond_the_count_moves_no_funds() {
    let h = Harness::new();
    let first = h.create_simple(1_000 * ONE, 100 * DAY);
    let second = h.create_simple(500 * ONE, 50 * DAY);
    h.advance(10 * DAY);

    let before = h.snapshot();
    let count = h.client.stream_count();

    // Probe a dense band above the boundary plus the extreme, through both the
    // boolean view and an error-returning view.
    let mut id = count;
    while id < count + 32 {
        assert!(!h.client.stream_exists(&id));
        assert!(h.client.try_get_stream(&id).is_err());
        assert!(h.client.try_vested_of(&id).is_err());
        id += 1;
    }
    assert!(!h.client.stream_exists(&u64::MAX));
    assert!(h.client.try_get_stream(&u64::MAX).is_err());

    // Nothing observable changed: not a balance, not a stream field, not the
    // ledger the snapshot pins.
    assert_eq!(
        h.snapshot(),
        before,
        "probing beyond the count must not mutate any observable state",
    );
    assert_eq!(h.client.stream_count(), count);
    assert!(h.client.stream_exists(&first));
    assert!(h.client.stream_exists(&second));
    h.assert_pool_exact();
}

/// The whole scenario, end to end through the public ABI: three streams issued
/// across the boundary, one drained, one cancelled, one left running — with
/// every balance, event, and stream state asserted, and funds conservation
/// checked at the end.
#[test]
fn the_boundary_scenario_conserves_funds_end_to_end() {
    let h = Harness::new();
    let sender_start = h.balance(&h.sender);
    let recipient_start = h.balance(&h.recipient);

    // Three streams issued from the same boundary.
    let drained = h.create_simple(1_000 * ONE, 100 * DAY); // id 0
    let cancelled = h.create_simple(900 * ONE, 90 * DAY); // id 1
    let running = h.create_simple(400 * ONE, 40 * DAY); // id 2
    assert_eq!([drained, cancelled, running], [0, 1, 2]);

    let count = h.client.stream_count();
    assert_eq!(count, 3);
    let deposited = 1_000 * ONE + 900 * ONE + 400 * ONE;
    assert_eq!(h.pool(), deposited, "every deposit is pooled");
    assert_eq!(h.balance(&h.sender), sender_start - deposited);
    assert!(!h.client.stream_exists(&count), "id 3 is beyond the count");
    h.assert_pool_exact();

    // 30 days in: the first stream is 30% vested and is drained completely.
    h.advance(30 * DAY);
    let paid = h.client.withdraw(&drained, &None);
    let events = published_by_stream(&h);
    assert_eq!(paid, 300 * ONE, "30% of 1_000 vested");
    assert_eq!(h.balance(&h.recipient), recipient_start + paid);
    assert_eq!(h.pool(), deposited - paid);
    assert_eq!(h.get(drained).withdrawn, paid);
    let expected = Withdrawn {
        stream_id: drained,
        recipient: h.recipient.clone(),
        amount: paid,
        withdrawn: paid,
        deposited: 1_000 * ONE,
        status: StreamStatus::Active,
    };
    assert_eq!(
        events,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the drain must publish exactly one Withdrawn event matching storage",
    );

    // Cancelling the second refunds the unvested remainder and settles the
    // schedule at the cancellation instant; the vested 300 stays pooled for
    // the recipient.
    h.client.cancel(&cancelled);
    let events = published_by_stream(&h);
    let refunded = 600 * ONE;
    assert_eq!(h.balance(&h.sender), sender_start - deposited + refunded);
    assert_eq!(h.pool(), deposited - paid - refunded);
    let settled = h.get(cancelled);
    assert_eq!(settled.status, StreamStatus::Cancelled);
    assert_eq!(
        settled.deposited,
        300 * ONE,
        "deposited is rewritten to vested"
    );
    assert_eq!(
        settled.end_time,
        T0 + 30 * DAY,
        "schedule collapses onto the cancel instant"
    );
    assert_eq!(h.client.withdrawable_of(&cancelled), 300 * ONE);
    let expected = Cancelled {
        stream_id: cancelled,
        sender: h.sender.clone(),
        recipient: h.recipient.clone(),
        refunded,
        vested: 300 * ONE,
        withdrawn: 0,
        end_time: T0 + 30 * DAY,
    };
    assert_eq!(
        events,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the cancel must publish exactly one Cancelled event matching storage",
    );

    // The third stream was never touched, and its schedule still accords with
    // the elapsed clock.
    let running_stream = h.get(running);
    assert_eq!(running_stream.status, StreamStatus::Active);
    assert_eq!(running_stream.withdrawn, 0);
    assert_eq!(running_stream.deposited, 400 * ONE);
    assert_eq!(
        h.client.vested_of(&running),
        300 * ONE,
        "30 of 40 days elapsed"
    );

    // The boundary is unchanged by every terminal operation: the count still
    // counts what it issued, and the first id past it does not exist.
    assert_eq!(
        h.client.stream_count(),
        count,
        "terminal operations do not move the counter"
    );
    assert!(!h.client.stream_exists(&count));
    assert!(!h.client.stream_exists(&(count + 1)));
    assert!(!h.client.stream_exists(&u64::MAX));
    for issued in 0..count {
        assert!(
            h.client.stream_exists(&issued),
            "issued id {issued} still exists"
        );
    }

    // Funds conservation, stated independently of the pool helper: every token
    // the sender paid out is either still pooled or has reached the recipient.
    let sender_out = sender_start - h.balance(&h.sender);
    let recipient_in = h.balance(&h.recipient) - recipient_start;
    assert_eq!(
        sender_out,
        h.pool() + recipient_in,
        "sender outflow must equal pooled funds plus recipient inflow",
    );
    assert_eq!(
        sender_out,
        1_700 * ONE,
        "1_000 + 900 + 400 deposited, 600 refunded"
    );
    assert_eq!(recipient_in, paid);
    h.assert_pool_exact();
}
