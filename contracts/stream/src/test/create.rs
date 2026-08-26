//! Stage 1 — `create_stream`: happy path, custody, and every validation gate.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{Error, StreamStatus};

#[test]
fn create_moves_deposit_into_the_pool() {
    let h = Harness::new();
    let before = h.balance(&h.sender);

    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    assert_eq!(id, 0, "first stream must be id 0");
    assert_eq!(h.balance(&h.sender), before - 1_000 * ONE);
    assert_eq!(h.pool(), 1_000 * ONE);
    h.assert_pool_exact();
}

#[test]
fn create_records_every_field() {
    let h = Harness::new();
    let start = h.now() + DAY;
    let end = start + 100 * DAY;
    let cliff = start + 10 * DAY;

    let id = h.create(500 * ONE, start, end, cliff, true, false, true);
    let s = h.get(id);

    assert_eq!(s.sender, h.sender);
    assert_eq!(s.recipient, h.recipient);
    assert_eq!(s.token, h.token);
    assert_eq!(s.deposited, 500 * ONE);
    assert_eq!(s.withdrawn, 0);
    assert_eq!(s.start_time, start);
    assert_eq!(s.end_time, end);
    assert_eq!(s.cliff_time, cliff);
    assert!(s.cancellable);
    assert!(!s.pausable);
    assert!(s.transferable);
    assert_eq!(s.paused_at, None);
    assert_eq!(s.paused_total, 0);
    assert_eq!(s.status, StreamStatus::Active);
}

#[test]
fn stream_ids_are_monotonic_and_never_reused() {
    let h = Harness::new();
    for expected in 0..5u64 {
        assert_eq!(h.create_simple(10 * ONE, DAY), expected);
    }
    assert_eq!(h.client.stream_count(), 5);
}

#[test]
fn multiple_streams_pool_together_without_interference() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(300 * ONE, 50 * DAY);

    assert_eq!(h.pool(), 400 * ONE);
    h.assert_pool_exact();

    h.advance(25 * DAY);

    // Different rates, independent accrual: a is 25% through, b is 50%.
    assert_eq!(h.client.vested_of(&a), 25 * ONE);
    assert_eq!(h.client.vested_of(&b), 150 * ONE);
}

// --- Validation -----------------------------------------------------------

#[test]
fn rejects_stream_to_self() {
    let h = Harness::new();
    let start = h.now();
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.sender,
            &h.token,
            &(100 * ONE),
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
}

#[test]
fn rejects_non_positive_deposit() {
    let h = Harness::new();
    for deposit in [0i128, -1, -1_000 * ONE] {
        let start = h.now();
        let err = h
            .client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &deposit,
                &start,
                &(start + DAY),
                &start,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::InvalidDeposit, "deposit {deposit}");
    }
}

/// `end_time <= start_time` would make `duration` zero and divide by zero in
/// the accrual math. It must never reach storage.
#[test]
fn rejects_non_positive_duration() {
    let h = Harness::new();
    let start = h.now();
    for end in [start, start - 1, start - DAY] {
        let err = h
            .client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &(100 * ONE),
                &start,
                &end,
                &start,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::InvalidTimeRange, "end {end}");
    }
}

#[test]
fn rejects_cliff_outside_the_schedule() {
    let h = Harness::new();
    let start = h.now();
    let end = start + 100 * DAY;

    for cliff in [start - 1, end + 1, end + DAY] {
        let err = h
            .client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &(100 * ONE),
                &start,
                &end,
                &cliff,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::InvalidCliff, "cliff {cliff}");
    }

    // Both endpoints are legal: cliff == start means no cliff, cliff == end
    // means a single lump sum at maturity.
    h.create(100 * ONE, start, end, start, true, true, true);
    h.create(100 * ONE, start, end, end, true, true, true);
}

/// The dust-rate footgun: a treasury streaming a small grant over a year would
/// otherwise create a stream whose per-second rate truncates to zero.
#[test]
fn rejects_deposit_below_one_stroop_per_second() {
    let h = Harness::new();
    let start = h.now();
    let end = start + YEAR;
    let duration = YEAR as i128;

    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &(duration - 1),
            &start,
            &end,
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DepositRateTooLow);

    // Exactly one stroop per second is the boundary, and it is allowed.
    h.create(duration, start, end, start, true, true, true);
}

/// A year-long USDC stream needs only ~3.16 USDC to clear the rate floor, so
/// the check excludes nothing anyone would realistically create.
#[test]
fn rate_floor_does_not_block_realistic_grants() {
    let h = Harness::new();
    let start = h.now();
    // 4 USDC over a year, in stroops: 40_000_000 > 31_536_000 seconds.
    h.create(4 * ONE, start, start + YEAR, start, true, true, true);
}

#[test]
fn rejects_deposit_that_would_overflow_accrual() {
    let h = Harness::new();
    let start = h.now();
    let end = start + YEAR;

    // deposit * duration must fit in an i128. Proving it here means the
    // multiplication inside `vested` can never overflow later.
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &(i128::MAX / 1_000),
            &start,
            &end,
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Overflow);
}

#[test]
fn backdated_start_vests_immediately() {
    let h = Harness::new();
    let start = h.now() - 50 * DAY;
    let id = h.create(100 * ONE, start, start + 100 * DAY, start, true, true, true);

    // Backdated vesting from a hire date is legitimate; half the schedule has
    // already elapsed, so half is already withdrawable.
    assert_eq!(h.client.vested_of(&id), 50 * ONE);
}

#[test]
fn unknown_stream_id_is_a_typed_error() {
    let h = Harness::new();
    let err = h.client.try_get_stream(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound);
    assert!(!h.client.stream_exists(&999));
}

/// Independent tokens must not be able to satisfy one another's liabilities.
#[test]
fn streams_of_different_tokens_are_accounted_separately() {
    let h = Harness::new();
    let issuer = Address::generate(&h.env);
    let other_asset = h.env.register_stellar_asset_contract_v2(issuer);
    let other_token = other_asset.address();
    soroban_sdk::token::StellarAssetClient::new(&h.env, &other_token)
        .mint(&h.sender, &(1_000 * ONE));

    let start = h.now();
    h.create_simple(100 * ONE, 100 * DAY);
    h.client.create_stream(
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

    assert_eq!(h.pool(), 100 * ONE, "first token pool");
    assert_eq!(
        soroban_sdk::token::Client::new(&h.env, &other_token).balance(&h.contract_id),
        200 * ONE,
        "second token pool",
    );
    h.assert_pool_exact();
}
