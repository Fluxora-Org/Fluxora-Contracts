//! Stage 3 — batch operations.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Vec};

use super::common::*;
use crate::{Error, MAX_BATCH_SIZE};

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
