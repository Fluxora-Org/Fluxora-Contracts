//! Stage 2 — recipient transfer.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::Error;

#[test]
fn transfer_redirects_future_payouts() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(h.get(id).recipient, h.other);

    h.advance(50 * DAY);
    h.client.withdraw(&id, &None);

    assert_eq!(h.balance(&h.other), 1_000_000 * ONE + 1_000 * ONE);
    assert_eq!(h.balance(&h.recipient), 0);
    h.assert_pool_exact();
}

/// Unwithdrawn accrual moves with the stream. Recipients should draw down
/// before transferring; the docs say so and this test pins the behaviour.
#[test]
fn unwithdrawn_accrual_moves_with_the_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);

    h.client.transfer_recipient(&id, &h.other);

    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    assert_eq!(h.balance(&h.other), 1_000_000 * ONE + 300 * ONE);
    assert_eq!(h.balance(&h.recipient), 0, "old recipient keeps nothing");
    h.assert_pool_exact();
}

#[test]
fn transfer_preserves_the_schedule_and_the_accounting() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.withdraw(&id, &None);

    let before = h.get(id);
    h.client.transfer_recipient(&id, &h.other);
    let after = h.get(id);

    assert_eq!(after.recipient, h.other);
    assert_eq!(after.deposited, before.deposited);
    assert_eq!(after.withdrawn, before.withdrawn);
    assert_eq!(after.start_time, before.start_time);
    assert_eq!(after.end_time, before.end_time);
    assert_eq!(after.status, before.status);
}

#[test]
fn transfer_chains() {
    let h = Harness::new();
    let third = Address::generate(&h.env);
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.client.transfer_recipient(&id, &h.other);
    h.client.transfer_recipient(&id, &third);
    assert_eq!(h.get(id).recipient, third);

    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.balance(&third), 1_000 * ONE);
    h.assert_pool_exact();
}

#[test]
fn transferring_to_the_current_recipient_is_a_no_op() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let before = h.get(id);

    h.client.transfer_recipient(&id, &h.recipient);
    assert_eq!(h.get(id), before);
}

// --- Cliff and terminal boundaries ----------------------------------------

/// Transfer immediately before the cliff moves the still-locked claim. The
/// new recipient cannot withdraw before the cliff, but can claim the accrued
/// amount as soon as the cliff opens.
#[test]
fn transfer_one_second_before_cliff_moves_claim_to_new_recipient() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 50 * DAY;
    let end = start + 100 * DAY;
    let id = h.create(1_000 * ONE, start, end, cliff, true, true, true);

    h.warp_to(cliff - 1);
    h.client.transfer_recipient(&id, &h.other);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);
    assert_eq!(h.balance(&h.other), 1_000_000 * ONE);

    h.warp_to(cliff);
    assert_eq!(h.client.withdraw(&id, &None), 500 * ONE);
    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(h.balance(&h.other), 1_000_000 * ONE + 500 * ONE);
}

/// The cliff ledger is inclusive: transferring at the exact cliff preserves
/// the newly opened claim for the recipient who receives the stream.
#[test]
fn transfer_at_cliff_preserves_the_open_claim() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 50 * DAY;
    let id = h.create(1_000 * ONE, start, start + 100 * DAY, cliff, true, true, true);

    h.warp_to(cliff);
    h.client.transfer_recipient(&id, &h.other);

    assert_eq!(h.client.withdrawable_of(&id), 500 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 500 * ONE);
    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(h.balance(&h.other), 1_000_000 * ONE + 500 * ONE);
}

/// Transfer after the cliff moves both the already-open claim and all future
/// accrual; the old recipient receives no payout.
#[test]
fn transfer_after_cliff_moves_accrued_and_future_claims() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 50 * DAY;
    let end = start + 100 * DAY;
    let id = h.create(1_000 * ONE, start, end, cliff, true, true, true);

    h.warp_to(cliff + 1);
    let accrued = 1_000 * ONE * (50 * DAY + 1) as i128 / (100 * DAY) as i128;
    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(h.client.withdraw(&id, &None), accrued);

    h.warp_to(end);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE - accrued);
    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(h.balance(&h.other), 1_000_000 * ONE + 1_000 * ONE);
}

/// End is inclusive: the recipient transferred to on the end ledger receives
/// the complete vested claim.
#[test]
fn transfer_at_end_moves_the_complete_claim() {
    let h = Harness::new();
    let start = h.now();
    let end = start + 100 * DAY;
    let id = h.create(1_000 * ONE, start, end, end, true, true, true);

    h.warp_to(end);
    h.client.transfer_recipient(&id, &h.other);

    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(h.balance(&h.other), 1_000_000 * ONE + 1_000 * ONE);
}

/// Once the end claim has been withdrawn, the stream is depleted. Transfer
/// retries remain rejected and cannot redirect a second withdrawal.
#[test]
fn transfer_after_depletion_is_rejected_and_retry_is_stable() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);
    h.client.withdraw(&id, &None);

    for _ in 0..2 {
        let err = h
            .client
            .try_transfer_recipient(&id, &h.other)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::StreamTerminated);
    }
    assert_eq!(h.get(id).recipient, h.recipient);
    assert_eq!(h.balance(&h.other), 1_000_000 * ONE);
}

// --- Guards ---------------------------------------------------------------

/// A compliance-bound sender can pin the payee at creation. This is the whole
/// point of the `transferable` flag.
#[test]
fn a_non_transferable_stream_cannot_be_reassigned_ever() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        false,
    );

    for skip in [0u64, DAY, 50 * DAY, 200 * DAY] {
        h.advance(skip);
        let err = h
            .client
            .try_transfer_recipient(&id, &h.other)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::NotTransferable);
    }
    assert_eq!(h.get(id).recipient, h.recipient);
}

#[test]
fn a_stream_cannot_be_transferred_to_its_own_sender() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let err = h
        .client
        .try_transfer_recipient(&id, &h.sender)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::SelfStream);
}

/// A cancelled stream may still hold an unwithdrawn tail, so its claim remains
/// transferable.
#[test]
fn a_cancelled_stream_with_a_tail_can_still_be_transferred() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.cancel(&id);

    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    assert_eq!(h.balance(&h.other), 1_000_000 * ONE + 300 * ONE);
    h.assert_pool_exact();
}

#[test]
fn a_depleted_stream_cannot_be_transferred() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);
    h.advance(10 * DAY);
    h.client.withdraw(&id, &None);

    let err = h
        .client
        .try_transfer_recipient(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);
}
