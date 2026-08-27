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
fn transferring_to_the_current_recipient_is_an_error() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    
    let err = h
        .client
        .try_transfer_recipient(&id, &h.recipient)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::RepeatedTransfer);
}

#[test]
fn new_recipient_replay_fails_due_to_repeated_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    
    h.client.transfer_recipient(&id, &h.other);

    // If the transaction is replayed, the current recipient is now h.other.
    // A replay tries to transfer to h.other again.
    let err = h
        .client
        .try_transfer_recipient(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::RepeatedTransfer);
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
