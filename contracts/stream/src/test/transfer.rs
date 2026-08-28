//! Stage 2 — recipient transfer.

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, IntoVal};

use super::common::*;
use crate::{Error, Stream, StreamStatus};

fn assert_claim(
    h: &Harness,
    id: u64,
    deposited: i128,
    withdrawn: i128,
    vested: i128,
    withdrawable: i128,
    refundable: i128,
) {
    let stream = h.get(id);
    assert_eq!(stream.deposited, deposited);
    assert_eq!(stream.withdrawn, withdrawn);
    assert_eq!(h.client.vested_of(&id), vested);
    assert_eq!(h.client.withdrawable_of(&id), withdrawable);
    assert_eq!(h.client.refundable_of(&id), refundable);
    assert_eq!(vested + refundable, deposited);
    assert_eq!(vested - withdrawn, withdrawable);
    h.assert_pool_exact();
}

fn assert_only_recipient_changed(before: &Stream, after: &Stream, new_recipient: &Address) {
    let mut expected = before.clone();
    expected.recipient = new_recipient.clone();
    assert_eq!(after, &expected);
}

#[test]
fn transfer_before_accrual_moves_the_entire_claim() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let new_recipient_before = h.balance(&h.other);

    assert_claim(&h, id, 1_000 * ONE, 0, 0, 0, 1_000 * ONE);
    let before = h.get(id);
    h.client.transfer_recipient(&id, &h.other);
    assert_only_recipient_changed(&before, &h.get(id), &h.other);
    assert_claim(&h, id, 1_000 * ONE, 0, 0, 0, 1_000 * ONE);

    h.advance(100 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);

    assert_eq!(h.balance(&h.other), new_recipient_before + 1_000 * ONE);
    assert_eq!(h.balance(&h.recipient), 0);
    h.assert_pool_exact();
}

#[test]
fn transfer_after_partial_withdrawal_preserves_exact_balances() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let new_recipient_before = h.balance(&h.other);

    h.advance(40 * DAY);
    assert_eq!(h.client.withdraw(&id, &Some(150 * ONE)), 150 * ONE);
    assert_eq!(h.balance(&h.recipient), 150 * ONE);
    assert_claim(
        &h,
        id,
        1_000 * ONE,
        150 * ONE,
        400 * ONE,
        250 * ONE,
        600 * ONE,
    );

    let before = h.get(id);
    h.client.transfer_recipient(&id, &h.other);
    assert_only_recipient_changed(&before, &h.get(id), &h.other);
    assert_claim(
        &h,
        id,
        1_000 * ONE,
        150 * ONE,
        400 * ONE,
        250 * ONE,
        600 * ONE,
    );

    assert_eq!(h.client.withdraw(&id, &None), 250 * ONE);
    assert_eq!(h.balance(&h.recipient), 150 * ONE);
    assert_eq!(h.balance(&h.other), new_recipient_before + 250 * ONE);

    h.advance(60 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 600 * ONE);
    assert_eq!(h.balance(&h.recipient), 150 * ONE);
    assert_eq!(h.balance(&h.other), new_recipient_before + 850 * ONE);
    h.assert_pool_exact();
}

#[test]
fn transfer_while_paused_preserves_the_frozen_claim() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let new_recipient_before = h.balance(&h.other);

    h.advance(30 * DAY);
    h.client.pause(&id);
    let paused_at = h.now();
    h.advance(50 * DAY);

    let before = h.get(id);
    assert_claim(&h, id, 1_000 * ONE, 0, 300 * ONE, 300 * ONE, 700 * ONE);

    h.client.transfer_recipient(&id, &h.other);
    let after = h.get(id);

    assert_only_recipient_changed(&before, &after, &h.other);
    assert_eq!(after.status, StreamStatus::Paused);
    assert_eq!(after.paused_at, Some(paused_at));
    assert_claim(&h, id, 1_000 * ONE, 0, 300 * ONE, 300 * ONE, 700 * ONE);

    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    h.advance(20 * DAY);
    assert_eq!(
        h.client.withdrawable_of(&id),
        0,
        "pause still freezes accrual"
    );

    h.client.resume(&id);
    h.advance(10 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 100 * ONE);
    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(h.balance(&h.other), new_recipient_before + 400 * ONE);
    assert_claim(&h, id, 1_000 * ONE, 400 * ONE, 400 * ONE, 0, 600 * ONE);
}

#[test]
fn transfer_immediately_before_cancellation_preserves_exact_settlement() {
    let h = Harness::new();
    let id = h.create_simple(1_000, 100);
    let sender_before_cancel = h.balance(&h.sender);
    let new_recipient_before = h.balance(&h.other);

    h.advance(99);
    assert_claim(&h, id, 1_000, 0, 990, 990, 10);

    let before = h.get(id);
    h.client.transfer_recipient(&id, &h.other);
    assert_only_recipient_changed(&before, &h.get(id), &h.other);
    assert_claim(&h, id, 1_000, 0, 990, 990, 10);

    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert_eq!(h.balance(&h.sender), sender_before_cancel + 10);
    assert_claim(&h, id, 990, 0, 990, 990, 0);

    assert_eq!(h.client.withdraw(&id, &None), 990);
    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(h.balance(&h.other), new_recipient_before + 990);
    h.assert_pool_exact();
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
    let before = h.get(id);

    let err = h
        .client
        .try_transfer_recipient(&id, &h.sender)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::SelfStream);
    assert_eq!(h.get(id), before, "failed transfer changed stream state");

    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(h.get(id).recipient, h.other, "valid retry did not succeed");
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
#[should_panic(expected = "Unauthorized")]
fn the_old_recipient_cannot_withdraw_after_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.transfer_recipient(&id, &h.other);

    h.client
        .mock_auths(&[MockAuth {
            address: &h.recipient,
            invoke: &MockAuthInvoke {
                contract: &h.contract_id,
                fn_name: "withdraw",
                args: (&id, &Option::<i128>::None).into_val(&h.env),
                sub_invokes: &[],
            },
        }])
        .withdraw(&id, &None);
}

#[test]
fn the_new_recipient_can_withdraw_after_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let new_recipient_before = h.balance(&h.other);
    h.advance(30 * DAY);
    h.client.transfer_recipient(&id, &h.other);

    let paid = h
        .client
        .mock_auths(&[MockAuth {
            address: &h.other,
            invoke: &MockAuthInvoke {
                contract: &h.contract_id,
                fn_name: "withdraw",
                args: (&id, &Option::<i128>::None).into_val(&h.env),
                sub_invokes: &[],
            },
        }])
        .withdraw(&id, &None);

    assert_eq!(paid, 300 * ONE);
    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(h.balance(&h.other), new_recipient_before + 300 * ONE);
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
