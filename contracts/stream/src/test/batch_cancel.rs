//! Stage 3 (continued) — batched cancellation.
//!
//! [`batch_cancel`](crate::FluxoraStream::batch_cancel) is the wind-down
//! counterpart to [`batch_withdraw`](crate::FluxoraStream::batch_withdraw): the
//! same shape (one shared party, one authorization for the whole vector, a
//! `Vec` bounded by [`MAX_BATCH_SIZE`]), the same all-or-nothing failure model,
//! and the same refusal to skip a bad element silently.
//!
//! What differs — and what these tests exist to pin — is how a *refusal* is
//! reported:
//!
//! * `batch_withdraw` has no per-element refusal: a stream with nothing
//!   withdrawable is skipped.
//! * `batch_cancel` cannot skip. A stream that cannot be cancelled is a
//!   statement about the caller's vector, not about the batch's progress, so the
//!   batch is refused and the returned [`BatchCancelOutcome`] names the
//!   **position** of the offending element and the `Error` that stopped it.
//!
//! A Soroban contract error is a bare `u32` on the wire, so that position cannot
//! ride on the error; it is data, and data is what the return value is for. The
//! tests below therefore assert the same all-or-nothing state machine
//! `batch_withdraw` proves, plus the one thing it does not: an index.

use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, IntoVal, TryFromVal, Val, Vec};

use super::common::*;
use crate::{BatchCancelOutcome, DataKey, Error, StreamStatus, MAX_BATCH_SIZE};

/// Discriminant of [`Error::NotCancellable`], as it appears in
/// [`BatchCancelOutcome::refused_reason`].
const NOT_CANCELLABLE: u32 = 8;
/// Discriminant of [`Error::StreamTerminated`], as it appears in
/// [`BatchCancelOutcome::refused_reason`].
const STREAM_TERMINATED: u32 = 14;

/// The stream ids of every `cancelled` event observable after the last call, in
/// emission order.
///
/// A refused or failed batch contributes nothing: the host drops its events with
/// the revert, exactly as a failed transaction emits nothing on chain. So after
/// either, this is empty even though the contract may have *started* cancelling
/// earlier streams before it stopped — which is the whole point of the
/// assertions built on it.
///
/// The event view is per invocation, so this must be read immediately after the
/// call under test, before any other invocation replaces it.
fn cancelled_event_ids(h: &Harness) -> std::vec::Vec<u64> {
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
            (name.0.as_slice() == b"cancelled").then_some(*stream_id)
        })
        .collect()
}

/// Number of `transfer` events the token contract emitted after the last call.
///
/// Used to pin the zero-value policy: settling a stream with nothing unvested
/// issues no transfer at all, not a transfer of zero. Read immediately after the
/// call under test — see [`cancelled_event_ids`].
fn token_transfer_count(h: &Harness) -> usize {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.token)
        .events()
        .iter()
        .filter(|event| {
            let ContractEventBody::V0(v0) = &event.body;
            matches!(
                v0.topics.first(),
                Some(ScVal::Symbol(name)) if name.0.as_slice() == b"transfer"
            )
        })
        .count()
}

/// Remaining TTL, in ledgers, of a stream entry.
fn ttl_of(h: &Harness, stream_id: u64) -> u32 {
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .get_ttl(&DataKey::Stream(stream_id))
    })
}

/// A stream that cannot be cancelled: the sender pinned that at creation.
fn create_uncancellable(h: &Harness) -> u64 {
    let start = h.now();
    h.create(
        100 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        true,
        true,
    )
}

/// A stream whose *sender* is somebody else, for the ownership guard.
///
/// Cancellation is sender-gated, so unlike `batch_withdraw`'s foreign-recipient
/// fixture this one has to be funded by — and belong to — another address.
fn create_foreign(h: &Harness) -> u64 {
    let start = h.now();
    h.client.create_stream(
        &h.other,
        &h.recipient,
        &h.token,
        &(100 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    )
}

fn malformed_ids(h: &Harness, valid_id: u64) -> Vec<u64> {
    let mut raw: Vec<Val> = Vec::new(&h.env);
    raw.push_back(valid_id.into_val(&h.env));
    raw.push_back(true.into_val(&h.env));
    Vec::<u64>::try_from_val(&h.env, &&raw).unwrap()
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

/// The whole point of the entry point: a programme is wound down in one call,
/// one authorization, one instant.
#[test]
fn batch_cancel_settles_every_stream_and_refunds_the_unvested_remainder() {
    let h = Harness::new();
    let stream_ids: std::vec::Vec<u64> = (0..3)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);

    let sender_before = h.balance(&h.sender);
    let vested_before = h.vested_snapshot();

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&stream_ids));
    let events = cancelled_event_ids(&h);

    assert_eq!(
        outcome,
        BatchCancelOutcome {
            refunded: 210 * ONE,
            refused_index: None,
            refused_reason: None,
        }
    );
    assert_eq!(events, stream_ids, "one cancelled event per stream");
    for id in &stream_ids {
        let s = h.get(*id);
        assert_eq!(s.status, StreamStatus::Cancelled);
        assert_eq!(
            s.deposited,
            30 * ONE,
            "deposited collapses to what vested at the cancel instant"
        );
    }
    assert_eq!(
        h.balance(&h.sender),
        sender_before + 210 * ONE,
        "70% of each deposit came back"
    );
    h.assert_no_vested_regression(&vested_before, "batch_cancel");
    h.assert_pool_exact();
}

/// A batch must equal the sum of the individual calls, or the SDK's client-side
/// chunking would change the outcome. Sixteen streams is exactly the cap, and a
/// single authorization settles all of them.
#[test]
fn a_batch_of_cancellations_matches_the_same_cancels_done_one_at_a_time() {
    let build = |h: &Harness| -> std::vec::Vec<u64> {
        (0..MAX_BATCH_SIZE)
            .map(|i| h.create_simple((100 + i as i128) * ONE, (100 + i as u64) * DAY))
            .collect()
    };

    let batched = {
        let h = Harness::new();
        let stream_ids = build(&h);
        h.advance(37 * DAY);
        let outcome = h.client.batch_cancel(&h.sender, &h.ids(&stream_ids));
        h.assert_pool_exact();
        (
            outcome.refunded,
            stream_ids
                .iter()
                .map(|id| h.get(*id))
                .collect::<std::vec::Vec<_>>(),
            h.balance(&h.sender),
        )
    };

    let individually = {
        let h = Harness::new();
        let stream_ids = build(&h);
        h.advance(37 * DAY);
        let mut total = 0;
        for id in &stream_ids {
            let before = h.balance(&h.sender);
            h.client.cancel(id);
            total += h.balance(&h.sender) - before;
        }
        h.assert_pool_exact();
        (
            total,
            stream_ids
                .iter()
                .map(|id| h.get(*id))
                .collect::<std::vec::Vec<_>>(),
            h.balance(&h.sender),
        )
    };

    assert_eq!(batched, individually);
}

// ---------------------------------------------------------------------------
// Refusals are reported by index
// ---------------------------------------------------------------------------

/// The acceptance criterion, in its most direct form: a batch holding a stream
/// that cannot be cancelled settles **nothing** and names the position of the
/// offender.
#[test]
fn a_non_cancellable_member_refuses_the_batch_by_index_and_changes_nothing() {
    let h = Harness::new();
    let first = h.create_simple(100 * ONE, 100 * DAY);
    let locked = create_uncancellable(&h);
    let last = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let sender_before = h.balance(&h.sender);
    let pool_before = h.pool();
    let vector = [first, locked, last];
    let ttls_before: std::vec::Vec<u32> = vector.iter().map(|id| ttl_of(&h, *id)).collect();

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&vector));
    let events = cancelled_event_ids(&h);

    // Reported by index, with the reason that stopped it.
    assert_eq!(
        outcome.refused_index,
        Some(1),
        "position of the locked stream"
    );
    assert_eq!(outcome.refused_reason, Some(NOT_CANCELLABLE));
    assert_eq!(outcome.refunded, 0, "a refused batch refunds nothing");

    // No state change at all: no schedule collapsed, no tokens moved, no events,
    // not even a TTL bump from reading the batch.
    assert!(events.is_empty(), "refused batch leaked events");
    assert_eq!(h.balance(&h.sender), sender_before);
    assert_eq!(h.pool(), pool_before);
    for id in vector {
        let s = h.get(id);
        assert_eq!(s.status, StreamStatus::Active, "stream {id} was cancelled");
        assert_eq!(s.deposited, 100 * ONE, "stream {id} deposit was rewritten");
    }
    for (i, id) in vector.iter().enumerate() {
        assert_eq!(ttl_of(&h, *id), ttls_before[i], "stream {id} was touched");
    }
    h.assert_pool_exact();
}

/// An already-terminal member refuses for the other reason. The index has the
/// same shape; the discriminant is what tells the caller whether the vector can
/// ever be submitted as-is.
#[test]
fn an_already_cancelled_member_refuses_with_stream_terminated() {
    let h = Harness::new();
    let live = h.create_simple(100 * ONE, 100 * DAY);
    let already = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.cancel(&already);

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&[live, already]));
    let events = cancelled_event_ids(&h);

    assert_eq!(outcome.refused_index, Some(1));
    assert_eq!(outcome.refused_reason, Some(STREAM_TERMINATED));
    assert_eq!(outcome.refunded, 0);

    assert!(
        events.is_empty(),
        "the refused batch emitted no event of its own"
    );
    assert_eq!(
        h.get(live).status,
        StreamStatus::Active,
        "the cancellable member before the offender is untouched"
    );
    h.assert_pool_exact();
}

/// The reported index is the *position in the submitted vector* — not the stream
/// id — and it is the first offending position when the vector holds more than
/// one stream that cannot be cancelled.
#[test]
fn the_refusal_names_the_first_offending_position_wherever_it_sits() {
    // Ids are handed out in creation order: 0 cancellable, 1 and 2 pinned. The
    // expected index is the *position* of the first pinned element, which is not
    // the same thing as the lowest pinned id — `[2, 1, 0]` names position 0 for
    // the stream whose id is 1.
    for (vector, expected_index) in [
        (std::vec![0u64, 1, 2], 1),
        (std::vec![0, 2, 1], 1),
        (std::vec![0, 2], 1),
        (std::vec![1, 0, 2], 0),
        (std::vec![2, 1, 0], 0),
        (std::vec![1], 0),
    ] {
        let h = Harness::new();
        let cancellable = h.create_simple(100 * ONE, 100 * DAY);
        create_uncancellable(&h);
        create_uncancellable(&h);
        assert_eq!(cancellable, 0, "ids are handed out in creation order");
        h.advance(30 * DAY);

        let outcome = h.client.batch_cancel(&h.sender, &h.ids(&vector));
        let events = cancelled_event_ids(&h);

        assert_eq!(outcome.refused_index, Some(expected_index), "{vector:?}");
        assert_eq!(outcome.refused_reason, Some(NOT_CANCELLABLE), "{vector:?}");
        assert_eq!(outcome.refunded, 0, "{vector:?}");
        assert!(events.is_empty(), "leaked events for {vector:?}");
        assert_eq!(
            h.get(cancellable).status,
            StreamStatus::Active,
            "cancelled the only cancellable stream for {vector:?}"
        );
        h.assert_pool_exact();
    }
}

/// A refused batch is a no-op, so the fix is cheap: drop the element the index
/// points at and the rest of the vector settles in full.
#[test]
fn a_refused_batch_can_be_retried_without_the_offending_element() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let locked = create_uncancellable(&h);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let refused = h.client.batch_cancel(&h.sender, &h.ids(&[a, locked, b]));
    assert!(cancelled_event_ids(&h).is_empty());
    assert_eq!(refused.refused_index, Some(1));
    assert_eq!(refused.refused_reason, Some(NOT_CANCELLABLE));

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&[a, b]));
    let events = cancelled_event_ids(&h);
    assert_eq!(outcome.refunded, 140 * ONE);
    assert_eq!(outcome.refused_index, None);
    assert_eq!(outcome.refused_reason, None);
    assert_eq!(events, std::vec![a, b]);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Batch-level rejections: typed errors, all-or-nothing
// ---------------------------------------------------------------------------

/// An unknown id fails the whole call, exactly as `batch_withdraw` does — the
/// batch does not cancel what it can and skip what it cannot read.
#[test]
fn an_unknown_id_fails_the_whole_batch() {
    for vector in [std::vec![999u64], std::vec![999, 0], std::vec![0, 999]] {
        let h = Harness::new();
        let a = h.create_simple(100 * ONE, 100 * DAY);
        let b = h.create_simple(100 * ONE, 100 * DAY);
        h.advance(30 * DAY);
        let sender_before = h.balance(&h.sender);

        let err = h
            .client
            .try_batch_cancel(&h.sender, &h.ids(&vector))
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::StreamNotFound, "vector {vector:?}");
        assert!(cancelled_event_ids(&h).is_empty());
        assert_eq!(h.balance(&h.sender), sender_before);
        for id in [a, b] {
            assert_eq!(h.get(id).status, StreamStatus::Active);
        }
        h.assert_pool_exact();
    }
}

/// A stream belonging to somebody else is a typed `Unauthorized` for the whole
/// batch, not a refusal: the caller's own streams that would have been cancelled
/// first are rolled back with it.
#[test]
fn a_stream_belonging_to_another_sender_fails_the_whole_batch() {
    for vector in [std::vec![0u64, 2, 1], std::vec![0, 1, 2], std::vec![2, 0]] {
        let h = Harness::new();
        let mine_a = h.create_simple(100 * ONE, 100 * DAY);
        let mine_b = h.create_simple(100 * ONE, 100 * DAY);
        let theirs = create_foreign(&h);
        h.advance(30 * DAY);
        let sender_before = h.balance(&h.sender);

        let err = h
            .client
            .try_batch_cancel(&h.sender, &h.ids(&vector))
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::Unauthorized, "vector {vector:?}");
        assert!(cancelled_event_ids(&h).is_empty());
        assert_eq!(h.balance(&h.sender), sender_before);
        for id in [mine_a, mine_b, theirs] {
            assert_eq!(h.get(id).status, StreamStatus::Active, "stream {id} moved");
        }
        h.assert_pool_exact();
    }
}

/// A duplicated id would be settled twice from one copy: the second pass would
/// pay out of a schedule the first pass had already collapsed.
#[test]
fn a_duplicated_id_is_rejected_before_anything_is_settled() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    let sender_before = h.balance(&h.sender);

    let err = h
        .client
        .try_batch_cancel(&h.sender, &h.ids(&[a, b, a]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DuplicateStreamId);
    assert!(cancelled_event_ids(&h).is_empty());
    assert_eq!(h.balance(&h.sender), sender_before);
    for id in [a, b] {
        assert_eq!(h.get(id).status, StreamStatus::Active);
    }
    h.assert_pool_exact();
}

/// Structural rejection happens before authorization is demanded, so a caller is
/// never made to sign for a batch that was not going to run.
#[test]
fn structural_rejection_precedes_authorization() {
    let h = Harness::new();
    let oversized = h.ids(&std::vec![0; MAX_BATCH_SIZE as usize + 1]);
    h.env.mock_auths(&[]);

    let err = h
        .client
        .try_batch_cancel(&h.sender, &oversized)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::BatchTooLarge);
    assert!(h.env.auths().is_empty());
}

/// A malformed serialized element is a typed error, not a trap, and a corrected
/// retry still works.
#[test]
fn malformed_serialized_ids_are_typed_errors_without_partial_mutation() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    let ttl_before = ttl_of(&h, a);
    let malformed = malformed_ids(&h, a);
    h.env.mock_auths(&[]);

    let err = h
        .client
        .try_batch_cancel(&h.sender, &malformed)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::MalformedStreamId);
    assert!(h.env.auths().is_empty());
    assert_eq!(h.get(a).status, StreamStatus::Active);
    assert_eq!(ttl_of(&h, a), ttl_before);

    h.env.mock_all_auths();
    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&[a]));
    assert_eq!(outcome.refunded, 70 * ONE);
    assert_eq!(h.get(a).status, StreamStatus::Cancelled);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// The boundary: 0 / 1 / MAX / MAX+1
// ---------------------------------------------------------------------------

#[test]
fn batch_cancel_size_zero_is_empty_batch() {
    let h = Harness::new();
    let empty: Vec<u64> = Vec::new(&h.env);

    let err = h
        .client
        .try_batch_cancel(&h.sender, &empty)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::EmptyBatch);
}

#[test]
fn batch_cancel_size_one_succeeds() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&[a]));
    assert_eq!(outcome.refunded, 90 * ONE);
    assert_eq!(outcome.refused_index, None);
    assert_eq!(h.get(a).status, StreamStatus::Cancelled);
    h.assert_pool_exact();
}

#[test]
fn batch_cancel_size_exactly_max_succeeds() {
    let h = Harness::new();
    let stream_ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();
    h.advance(5 * DAY);

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&stream_ids));
    let events = cancelled_event_ids(&h);
    assert_eq!(outcome.refunded, MAX_BATCH_SIZE as i128 * 95 * ONE);
    assert_eq!(events, stream_ids);
    h.assert_pool_exact();
}

#[test]
fn batch_cancel_size_max_plus_one_is_batch_too_large() {
    let h = Harness::new();
    let stream_ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE + 1)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);
    let sender_before = h.balance(&h.sender);

    let err = h
        .client
        .try_batch_cancel(&h.sender, &h.ids(&stream_ids))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::BatchTooLarge);
    assert!(cancelled_event_ids(&h).is_empty());
    assert_eq!(h.balance(&h.sender), sender_before);
    for id in &stream_ids {
        assert_eq!(h.get(*id).status, StreamStatus::Active);
    }
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// One instant, and equivalence with single-stream cancellation
// ---------------------------------------------------------------------------

/// Refunds are priced once, at one shared instant, even when the members have
/// different deposits, durations, cliffs and pause histories. The ground truth
/// is read from the contract itself at that same instant, immediately before the
/// call, and the batch must reproduce it exactly.
#[test]
fn every_refund_is_priced_at_the_same_instant() {
    let h = Harness::new();
    let plain = h.create_simple(100 * ONE, 100 * DAY);
    let big = h.create_simple(900 * ONE, 365 * DAY);
    let future_start = h.now() + 10 * DAY;
    let late_start = h.create(
        100 * ONE,
        future_start,
        future_start + 100 * DAY,
        future_start,
        true,
        true,
        true,
    );
    let start = h.now();
    let cliffed = h.create(
        200 * ONE,
        start,
        start + 100 * DAY,
        start + 50 * DAY,
        true,
        true,
        true,
    );
    let paused = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(40 * DAY);
    h.client.pause(&paused);
    h.advance(20 * DAY);

    let vector = [plain, big, late_start, cliffed, paused];
    // Read at the instant the batch will settle at: no clock movement between
    // these reads and the call below.
    let expected_vested: std::vec::Vec<i128> =
        vector.iter().map(|id| h.client.vested_of(id)).collect();
    let expected_refund: std::vec::Vec<i128> =
        vector.iter().map(|id| h.client.refundable_of(id)).collect();
    let expected_total: i128 = expected_refund.iter().sum();

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&vector));

    assert_eq!(outcome.refunded, expected_total);
    for (i, id) in vector.iter().enumerate() {
        assert_eq!(
            h.get(*id).deposited,
            expected_vested[i],
            "stream {id}: deposited must equal its own vested figure at the shared instant"
        );
    }
    h.assert_pool_exact();
}

/// A settled stream is indistinguishable from one cancelled on its own: same
/// storage, same balances. The SDK chunks long vectors client-side, so "batched"
/// must not be a different state machine.
#[test]
fn a_settled_batch_is_indistinguishable_from_individual_cancels() {
    let build = |h: &Harness| -> std::vec::Vec<u64> {
        (0..4)
            .map(|i| h.create_simple((100 + i as i128) * ONE, (100 + i as u64) * DAY))
            .collect()
    };

    let batched = {
        let h = Harness::new();
        let stream_ids = build(&h);
        h.advance(63 * DAY);
        h.client.batch_cancel(&h.sender, &h.ids(&stream_ids));
        h.assert_pool_exact();
        (
            stream_ids
                .iter()
                .map(|id| h.get(*id))
                .collect::<std::vec::Vec<_>>(),
            h.balance(&h.sender),
            h.balance(&h.recipient),
        )
    };

    let individually = {
        let h = Harness::new();
        let stream_ids = build(&h);
        h.advance(63 * DAY);
        for id in &stream_ids {
            h.client.cancel(id);
        }
        h.assert_pool_exact();
        (
            stream_ids
                .iter()
                .map(|id| h.get(*id))
                .collect::<std::vec::Vec<_>>(),
            h.balance(&h.sender),
            h.balance(&h.recipient),
        )
    };

    assert_eq!(batched, individually);
}

/// The recipient's claim is untouched by a batch cancel: everything already
/// vested stays withdrawable through the normal path, so cancellation never
/// seizes earned funds — in a batch any more than in a single cancel.
#[test]
fn a_cancelled_member_keeps_its_vested_remainder_withdrawable() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    h.client.batch_cancel(&h.sender, &h.ids(&[a, b]));

    assert_eq!(h.client.withdrawable_of(&a), 30 * ONE);
    assert_eq!(h.client.withdrawable_of(&b), 30 * ONE);
    assert_eq!(
        h.client.withdraw(&a, &None),
        30 * ONE,
        "vested funds survive a batched cancel"
    );
    assert_eq!(h.client.withdraw(&b, &None), 30 * ONE);
    assert_eq!(h.balance(&h.recipient), 60 * ONE);
    h.assert_pool_exact();
}

/// A batch of fully-vested members settles to `Cancelled` without moving a
/// token: no transfer of zero is issued, per the zero-value policy.
#[test]
fn a_batch_of_fully_vested_members_settles_without_a_transfer() {
    let h = Harness::new();
    let fast = h.create_simple(100 * ONE, 10 * DAY);
    let slow = h.create_simple(100 * ONE, 20 * DAY);
    h.advance(30 * DAY);
    let sender_before = h.balance(&h.sender);
    let pool_before = h.pool();

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&[fast, slow]));
    let transfers = token_transfer_count(&h);

    assert_eq!(outcome.refunded, 0, "nothing unvested, nothing to refund");
    assert_eq!(h.get(fast).status, StreamStatus::Cancelled);
    assert_eq!(h.get(slow).status, StreamStatus::Cancelled);
    assert_eq!(transfers, 0, "zero-value transfers are never issued");
    assert_eq!(h.balance(&h.sender), sender_before, "no refund to move");
    assert_eq!(h.pool(), pool_before, "no transfer issued");
    h.assert_pool_exact();
}

/// Streams need not share a token: one sender can hold streams in several, each
/// refund uses its own stream's token, and the returned total is a sum of amounts
/// denominated in different tokens.
#[test]
fn a_batch_can_span_multiple_tokens() {
    let h = Harness::new();
    let issuer = Address::generate(&h.env);
    let other_token = h.env.register_stellar_asset_contract_v2(issuer).address();
    soroban_sdk::token::StellarAssetClient::new(&h.env, &other_token)
        .mint(&h.sender, &(1_000 * ONE));

    let start = h.now();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &other_token,
        &(200 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    h.advance(50 * DAY);
    let mine_before = h.balance(&h.sender);
    let other_client = soroban_sdk::token::Client::new(&h.env, &other_token);
    let other_before = other_client.balance(&h.sender);

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&[a, b]));

    assert_eq!(
        outcome.refunded,
        150 * ONE,
        "50 in the first token, 100 in the second"
    );
    assert_eq!(outcome.refused_index, None);
    assert_eq!(h.balance(&h.sender), mine_before + 50 * ONE);
    assert_eq!(
        other_client.balance(&h.sender),
        other_before + 100 * ONE,
        "the second refund is denominated in its own token"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

/// The sender authorizes the batch — the one-authorization contract
/// `batch_withdraw` has for its recipient, and a single `require_auth` call
/// covers the whole vector however long it is.
#[test]
fn batch_cancel_is_authorized_by_the_sender() {
    let h = Harness::new();
    let stream_ids: std::vec::Vec<u64> = (0..4)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);

    h.client.batch_cancel(&h.sender, &h.ids(&stream_ids));

    let auths = h.env.auths();
    assert!(!auths.is_empty(), "call required no authorization at all");
    assert_eq!(auths[0].0, h.sender);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn batch_cancel_fails_without_authorization() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    h.env.mock_auths(&[]);
    h.client.batch_cancel(&h.sender, &h.ids(&[a, b]));
}

/// Cancellation on a terminal stream keeps refusing: the batch path inherits the
/// single-stream rule rather than inventing a looser one.
#[test]
fn a_terminal_stream_in_a_batch_refuses_rather_than_settling() {
    let h = Harness::new();
    let live = h.create_simple(100 * ONE, 100 * DAY);
    let drained = h.create_simple(100 * ONE, 10 * DAY);
    h.advance(10 * DAY);
    h.client.withdraw(&drained, &None);
    assert_eq!(h.get(drained).status, StreamStatus::Depleted);

    let outcome = h.client.batch_cancel(&h.sender, &h.ids(&[live, drained]));

    assert_eq!(outcome.refused_index, Some(1));
    assert_eq!(outcome.refused_reason, Some(STREAM_TERMINATED));
    assert_eq!(outcome.refunded, 0);
    assert_eq!(
        h.get(live).status,
        StreamStatus::Active,
        "live stream untouched"
    );
    h.assert_pool_exact();
}

/// A refusal is a settlement decision, not a trap: the contract call succeeds,
/// the stream is left exactly as it was, and it stays fully readable so the
/// caller can resolve the id behind the index before resubmitting.
#[test]
fn a_refused_batch_leaves_the_offending_stream_readable() {
    let h = Harness::new();
    let locked = create_uncancellable(&h);
    h.advance(30 * DAY);

    let refused = h.client.batch_cancel(&h.sender, &h.ids(&[locked]));

    assert_eq!(refused.refused_index, Some(0));
    assert_eq!(refused.refused_reason, Some(NOT_CANCELLABLE));
    assert_eq!(refused.refunded, 0);
    assert!(h.client.stream_exists(&locked));
    assert_eq!(h.get(locked).status, StreamStatus::Active);
    assert!(!h.get(locked).cancellable);
    h.assert_pool_exact();
}
