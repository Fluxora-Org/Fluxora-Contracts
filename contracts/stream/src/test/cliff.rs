//! Stage 2 — cliff semantics.
//!
//! The cliff **gates** the payout; it does not delay accrual. This surprises
//! people, so it gets its own file.

use super::common::*;
use crate::Error;

#[test]
fn nothing_is_withdrawable_one_second_before_the_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 90 * DAY;
    let id = h.create(
        1_200 * ONE,
        start,
        start + 360 * DAY,
        cliff,
        true,
        true,
        true,
    );

    h.warp_to(cliff - 1);
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);
}

/// At the cliff instant the recipient becomes entitled to everything accrued
/// since `start_time` — a quarter of a year-long stream, not zero.
#[test]
fn cliff_releases_all_accrual_since_start_not_since_the_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 90 * DAY;
    let id = h.create(
        1_200 * ONE,
        start,
        start + 360 * DAY,
        cliff,
        true,
        true,
        true,
    );

    h.warp_to(cliff);

    // 90 of 360 days elapsed => a quarter of the deposit, all at once.
    assert_eq!(h.client.vested_of(&id), 300 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    h.assert_pool_exact();
}

/// The transition must be a step at exactly `cliff_time`, with no off-by-one.
#[test]
fn the_cliff_step_lands_on_the_exact_second() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 100;
    let id = h.create(1_000, start, start + 1_000, cliff, true, true, true);

    h.warp_to(cliff - 1);
    assert_eq!(h.client.vested_of(&id), 0, "one second before");

    h.warp_to(cliff);
    assert_eq!(h.client.vested_of(&id), 100, "at the cliff");

    h.warp_to(cliff + 1);
    assert_eq!(h.client.vested_of(&id), 101, "one second after");
}

/// After the cliff opens, accrual continues linearly as if the cliff had never
/// existed.
#[test]
fn accrual_is_linear_after_the_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 90 * DAY;
    let id = h.create(
        1_200 * ONE,
        start,
        start + 360 * DAY,
        cliff,
        true,
        true,
        true,
    );

    for days in [90u64, 180, 270, 360] {
        h.warp_to(start + days * DAY);
        let expected = 1_200 * ONE * days as i128 / 360;
        assert_eq!(h.client.vested_of(&id), expected, "at day {days}");
    }
}

/// `cliff == end` is a legal degenerate case: a single lump sum at maturity,
/// which is how a straightforward vesting bonus is expressed.
#[test]
fn cliff_at_end_time_is_a_lump_sum() {
    let h = Harness::new();
    let start = h.now();
    let end = start + 365 * DAY;
    let id = h.create(1_000 * ONE, start, end, end, true, true, true);

    h.warp_to(end - 1);
    assert_eq!(h.client.vested_of(&id), 0);

    h.warp_to(end);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

#[test]
fn cliff_at_start_time_means_no_cliff() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        true,
    );

    h.advance(1);
    assert!(h.client.vested_of(&id) > 0, "accrual begins immediately");
}

/// Cancelling before the cliff refunds everything: pre-cliff the recipient's
/// entitlement is zero by definition, even though time has passed.
#[test]
fn cancelling_before_the_cliff_refunds_the_whole_deposit() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 90 * DAY;
    let id = h.create(
        1_200 * ONE,
        start,
        start + 360 * DAY,
        cliff,
        true,
        true,
        true,
    );
    let before = h.balance(&h.sender);

    h.warp_to(cliff - 1);
    h.client.cancel(&id);

    assert_eq!(h.balance(&h.sender), before + 1_200 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();

    // And the entitlement stays zero once the cliff time passes in wall-clock
    // terms — the cancel already settled it.
    h.warp_to(cliff + 10 * DAY);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    h.assert_pool_exact();
}
