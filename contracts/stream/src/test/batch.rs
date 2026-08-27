//! Stage 3 — batch operations.
//!
//! The two batch entrypoints follow deliberately different policies, and the
//! tests below pin both down:
//!
//! * [`batch_withdraw`](crate::FluxoraStream::batch_withdraw) is **all-or-nothing**:
//!   one bad id anywhere — first, middle, or last — reverts the entire call,
//!   including payouts already applied to earlier streams. No accounting is
//!   written, no tokens move, and no event is observable.
//! * [`batch_extend_ttl`](crate::FluxoraStream::batch_extend_ttl) is **per-item**:
//!   unknown ids are skipped, duplicates are idempotent, and the outcome for a
//!   given input is deterministic.

use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, Vec};

use super::common::*;
use crate::{DataKey, Error, MAX_BATCH_SIZE};

/// The stream ids of every `withdrawn` event observable after the last call,
/// in emission order.
///
/// A failed call contributes nothing: the host drops its events with the
/// revert, exactly as a failed transaction emits nothing on chain. So after a
/// failed batch this is empty even though the contract may have *started*
/// paying earlier streams before hitting the bad id — which is the whole point
/// of the assertion.
fn withdrawn_event_ids(h: &Harness) -> std::vec::Vec<u64> {
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

/// Remaining TTL, in ledgers, of a stream entry.
fn ttl_of(h: &Harness, stream_id: u64) -> u32 {
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .get_ttl(&DataKey::Stream(stream_id))
    })
}

/// Advance only the ledger sequence, leaving the clock alone, so rent burns
/// off without moving accrual.
fn age_ledgers(h: &Harness, ledgers: u32) {
    let seq = h.env.ledger().sequence();
    h.env.ledger().set_sequence_number(seq + ledgers);
}

#[test]
fn batch_withdraw_draws_from_every_stream() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..5)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);

    let total = h.client.batch_withdraw(&h.recipient, &h.ids(&ids));

    assert_eq!(total, 5 * 30 * ONE);
    assert_eq!(h.balance(&h.recipient), 150 * ONE);
    for id in &ids {
        assert_eq!(h.get(*id).withdrawn, 30 * ONE);
    }
    h.assert_pool_exact();
}

/// A batch must equal the sum of the individual calls, or the SDK's
/// client-side chunking would change the result.
#[test]
fn a_batch_matches_the_same_withdrawals_done_one_at_a_time() {
    let batched = {
        let h = Harness::new();
        let ids: std::vec::Vec<u64> = (0..4)
            .map(|i| h.create_simple((100 + i) * ONE, (50 + i as u64) * DAY))
            .collect();
        h.advance(37 * DAY);
        let total = h.client.batch_withdraw(&h.recipient, &h.ids(&ids));
        h.assert_pool_exact();
        total
    };

    let individually = {
        let h = Harness::new();
        let ids: std::vec::Vec<u64> = (0..4)
            .map(|i| h.create_simple((100 + i) * ONE, (50 + i as u64) * DAY))
            .collect();
        h.advance(37 * DAY);
        let total: i128 = ids.iter().map(|id| h.client.withdraw(id, &None)).sum();
        h.assert_pool_exact();
        total
    };

    assert_eq!(batched, individually);
}

/// Streams with nothing accrued yet are skipped rather than failing the batch —
/// a recipient with a mix of started and unstarted streams should not have to
/// filter client-side.
#[test]
fn streams_with_nothing_available_are_skipped() {
    let h = Harness::new();
    let ready = h.create_simple(100 * ONE, 100 * DAY);
    let future_start = h.now() + 50 * DAY;
    let not_ready = h.create(
        100 * ONE,
        future_start,
        future_start + 100 * DAY,
        future_start,
        true,
        true,
        true,
    );

    h.advance(10 * DAY);
    let total = h
        .client
        .batch_withdraw(&h.recipient, &h.ids(&[ready, not_ready]));

    assert_eq!(total, 10 * ONE);
    assert_eq!(h.get(not_ready).withdrawn, 0);
    h.assert_pool_exact();
}

#[test]
fn a_batch_of_entirely_empty_streams_returns_zero() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);

    assert_eq!(h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b])), 0);
    h.assert_pool_exact();
}

/// Streams need not share a token; each payout uses its own.
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
    let total = h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b]));

    assert_eq!(total, 150 * ONE, "sum across both tokens");
    assert_eq!(h.balance(&h.recipient), 50 * ONE);
    assert_eq!(
        soroban_sdk::token::Client::new(&h.env, &other_token).balance(&h.recipient),
        100 * ONE,
    );
    h.assert_pool_exact();
}

#[test]
fn a_batch_of_exactly_the_cap_is_accepted() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);

    let total = h.client.batch_withdraw(&h.recipient, &h.ids(&ids));
    assert_eq!(total, MAX_BATCH_SIZE as i128 * 30 * ONE);
    h.assert_pool_exact();
}

/// Oversized batches must be rejected with a clear typed error, not fail
/// opaquely at the network level once resources run out.
#[test]
fn an_oversized_batch_is_rejected_with_a_clear_error() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE + 1)
        .map(|_| h.create_simple(10 * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&ids))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::BatchTooLarge);
    assert_eq!(h.balance(&h.recipient), 0, "nothing drawn");
    h.assert_pool_exact();
}

#[test]
fn an_empty_batch_is_rejected() {
    let h = Harness::new();
    let empty: Vec<u64> = Vec::new(&h.env);

    assert_eq!(
        h.client
            .try_batch_withdraw(&h.recipient, &empty)
            .unwrap_err()
            .unwrap(),
        Error::EmptyBatch,
    );
    assert_eq!(
        h.client.try_batch_extend_ttl(&empty).unwrap_err().unwrap(),
        Error::EmptyBatch,
    );
}

/// A duplicated id would load the stream twice and apply the second withdrawal
/// to a stale copy — silently paying out more than the recipient earned.
#[test]
fn a_duplicated_id_is_rejected_rather_than_double_paying() {
    let h = Harness::new();
    let id = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[id, id]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DuplicateStreamId);

    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(h.get(id).withdrawn, 0);
    h.assert_pool_exact();
}

#[test]
fn an_unknown_id_fails_the_whole_withdrawal_batch() {
    let h = Harness::new();
    let id = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[id, 999]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamNotFound);
    assert_eq!(h.balance(&h.recipient), 0, "rolled back");
    h.assert_pool_exact();
}

#[test]
fn a_batch_marks_drained_streams_depleted() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 10 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b]));

    assert_eq!(h.get(a).status, crate::StreamStatus::Depleted);
    assert_eq!(h.get(b).status, crate::StreamStatus::Active);
    h.assert_pool_exact();
}

#[test]
fn an_oversized_ttl_batch_is_rejected() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE + 1)
        .map(|_| h.create_simple(10 * ONE, 100 * DAY))
        .collect();

    let err = h
        .client
        .try_batch_extend_ttl(&h.ids(&ids))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::BatchTooLarge);
}

// ---------------------------------------------------------------------------
// Atomicity: an invalid item at any position reverts the whole batch
// ---------------------------------------------------------------------------

/// All-or-nothing, position by position: an unknown id at the *start*, in the
/// *middle*, or at the *end* of the batch must produce the same typed error and
/// leave nothing behind. The middle case is the one that used to be dangerous:
/// earlier streams in the batch have already been paid by the time it is
/// reached, and only a full revert can keep them from keeping that money.
#[test]
fn an_unknown_id_anywhere_reverts_the_whole_batch() {
    // Streams are created in order, so ids are 0, 1, 2 in every harness.
    for ids in [[999, 1, 2], [1, 999, 2], [1, 2, 999]] {
        let h = Harness::new();
        let streams: std::vec::Vec<u64> = (0..3)
            .map(|_| h.create_simple(100 * ONE, 100 * DAY))
            .collect();
        h.advance(30 * DAY);

        let recipient_before = h.balance(&h.recipient);
        let sender_before = h.balance(&h.sender);
        let pool_before = h.pool();

        let err = h
            .client
            .try_batch_withdraw(&h.recipient, &h.ids(&ids))
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::StreamNotFound, "ids {ids:?}");

        // No observable events from the failed batch — read before any other
        // invocation replaces the event view. A withdraw that was applied to an
        // earlier stream before the bad id was hit must be invisible too.
        assert!(
            withdrawn_event_ids(&h).is_empty(),
            "failed batch leaked withdrawn events"
        );

        // No token movement: nobody gained, nobody lost, the pool is intact.
        assert_eq!(h.balance(&h.recipient), recipient_before);
        assert_eq!(h.balance(&h.sender), sender_before);
        assert_eq!(h.pool(), pool_before);

        // No storage movement: every stream's accounting is untouched.
        for id in &streams {
            assert_eq!(h.get(*id).withdrawn, 0, "stream {id} was drawn on");
            assert_eq!(h.get(*id).status, crate::StreamStatus::Active);
        }
        h.assert_pool_exact();
    }
}

/// The same all-or-nothing rule holds when the invalid item is a stream that
/// belongs to someone else: the recipient's own streams that would have been
/// paid first are reverted along with it.
#[test]
fn an_unauthorized_stream_anywhere_reverts_the_whole_batch() {
    for ids in [[0, 2, 1], [2, 0, 1], [0, 1, 2]] {
        let h = Harness::new();
        let a = h.create_simple(100 * ONE, 100 * DAY);
        let b = h.create_simple(100 * ONE, 100 * DAY);
        let theirs = h.client.create_stream(
            &h.sender,
            &h.other,
            &h.token,
            &(100 * ONE),
            &h.now(),
            &(h.now() + 100 * DAY),
            &h.now(),
            &true,
            &true,
            &true,
        );
        h.advance(30 * DAY);

        let recipient_before = h.balance(&h.recipient);
        let pool_before = h.pool();

        let err = h
            .client
            .try_batch_withdraw(&h.recipient, &h.ids(&ids))
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::Unauthorized, "ids {ids:?}");

        assert!(withdrawn_event_ids(&h).is_empty());
        assert_eq!(h.balance(&h.recipient), recipient_before);
        assert_eq!(h.pool(), pool_before);
        for id in [a, b] {
            assert_eq!(h.get(id).withdrawn, 0, "stream {id} was drawn on");
        }
        assert_eq!(h.get(theirs).withdrawn, 0);
        h.assert_pool_exact();
    }
}

// ---------------------------------------------------------------------------
// Duplicates: rejected deterministically, wherever they sit
// ---------------------------------------------------------------------------

/// A duplicated id is rejected with the same typed error no matter which
/// position the copy occupies — start, middle, or end — and never double-pays
/// or partially pays. This is the determinism guarantee: the outcome depends
/// only on the multiset of ids, not on their order.
#[test]
fn a_duplicate_is_rejected_at_any_position() {
    for ids in [[0, 0, 1], [0, 1, 0], [1, 0, 0]] {
        let h = Harness::new();
        let a = h.create_simple(100 * ONE, 100 * DAY);
        let b = h.create_simple(100 * ONE, 100 * DAY);
        h.advance(30 * DAY);

        let recipient_before = h.balance(&h.recipient);
        let pool_before = h.pool();

        let err = h
            .client
            .try_batch_withdraw(&h.recipient, &h.ids(&ids))
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::DuplicateStreamId, "ids {ids:?}");

        assert!(withdrawn_event_ids(&h).is_empty());
        assert_eq!(h.balance(&h.recipient), recipient_before);
        assert_eq!(h.pool(), pool_before);
        for id in [a, b] {
            assert_eq!(h.get(id).withdrawn, 0, "stream {id} was drawn on");
        }
        h.assert_pool_exact();
    }
}

// ---------------------------------------------------------------------------
// Retry behaviour
// ---------------------------------------------------------------------------

/// A failed batch is a no-op, so retrying is safe: the identical bad batch
/// fails identically every time, and the valid portion pays out in full as soon
/// as the caller drops the bad id. Nothing was consumed, corrupted, or marked.
#[test]
fn a_failed_batch_leaves_nothing_behind_and_can_be_retried() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    // Same bad batch, twice: deterministic failure, zero side effects each time.
    for _ in 0..2 {
        let err = h
            .client
            .try_batch_withdraw(&h.recipient, &h.ids(&[a, 999, b]))
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::StreamNotFound);
        assert!(withdrawn_event_ids(&h).is_empty());
        assert_eq!(h.balance(&h.recipient), 0);
    }

    // Drop the bad id and the whole batch succeeds, exactly as if the failure
    // had never happened.
    let total = h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b]));
    assert_eq!(total, 60 * ONE);
    assert_eq!(withdrawn_event_ids(&h), [a, b]);
    assert_eq!(h.get(a).withdrawn, 30 * ONE);
    assert_eq!(h.get(b).withdrawn, 30 * ONE);
    h.assert_pool_exact();
}

/// The same story after a duplicate rejection: retry without the duplicate and
/// the full amount lands.
#[test]
fn a_batch_rejected_for_duplicates_can_be_retried_without_them() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[a, a, b]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DuplicateStreamId);
    assert!(withdrawn_event_ids(&h).is_empty());
    assert_eq!(h.balance(&h.recipient), 0);

    let total = h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b]));
    assert_eq!(total, 60 * ONE);
    assert_eq!(h.get(a).withdrawn, 30 * ONE);
    assert_eq!(h.get(b).withdrawn, 30 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Ordering
// ---------------------------------------------------------------------------

/// Success preserves order: `withdrawn` events come out in exactly the order
/// the ids were passed in, not in id order — an indexer reconstructing a batch
/// from events must see the caller's ordering.
#[test]
fn a_successful_batch_emits_withdrawn_events_in_batch_order() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..4)
        .map(|i| h.create_simple((100 + i) * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);

    // Deliberately not sorted, so a pass-by-id-order implementation would fail.
    let shuffled = [ids[2], ids[0], ids[3], ids[1]];

    // Streams have distinct deposits, so each withdrawable amount differs;
    // compute the ground truth before the batch runs.
    let expected_per_stream: std::vec::Vec<i128> =
        ids.iter().map(|id| h.client.withdrawable_of(id)).collect();
    let expected_total: i128 = expected_per_stream.iter().sum();

    let total = h.client.batch_withdraw(&h.recipient, &h.ids(&shuffled));
    assert_eq!(total, expected_total);

    assert_eq!(
        withdrawn_event_ids(&h),
        shuffled,
        "events out of batch order"
    );
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(h.get(*id).withdrawn, expected_per_stream[i]);
    }
    assert_eq!(h.balance(&h.recipient), expected_total);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// TTL sweep: per-item, deterministic
// ---------------------------------------------------------------------------

/// The TTL sweep is the deliberate counterpoint to the withdrawal batch: it is
/// per-item rather than all-or-nothing. Unknown ids are skipped, duplicates are
/// idempotent (extending the same entry twice is harmless), and rerunning the
/// same sweep yields the same count — the determinism guarantee for a keeper
/// working from a slightly stale index.
#[test]
fn a_ttl_batch_is_per_item_and_deterministic() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(50_000);
    let a = h.create_simple(100 * ONE, YEAR);
    let b = h.create_simple(100 * ONE, YEAR);

    // Burn most of the rent off so the extension is a real change, not a no-op.
    age_ledgers(&h, 40_000);
    assert!(ttl_of(&h, a) < 15_000, "TTL should have decayed");

    // One sweep with an unknown id and a duplicate: nothing fails, and the two
    // real streams are restored to the max. The duplicate counts once per
    // occurrence, so the return value is 3, deterministically.
    let extended = h.client.batch_extend_ttl(&h.ids(&[a, a, 999, b]));
    assert_eq!(extended, 3, "duplicate counts per occurrence");
    assert_eq!(ttl_of(&h, a), 50_000);
    assert_eq!(ttl_of(&h, b), 50_000);

    // Rerunning the identical sweep is a no-op with the identical result.
    let again = h.client.batch_extend_ttl(&h.ids(&[a, a, 999, b]));
    assert_eq!(again, extended, "sweep must be deterministic");
    assert_eq!(ttl_of(&h, a), 50_000);
    assert_eq!(ttl_of(&h, b), 50_000);
}
