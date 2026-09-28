use super::common::*;
use crate::{Error, StreamStatus};

fn test_withdraw_then_cancel_at(offset: u64) {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(offset);

    let recipient_bal_before = h.balance(&h.recipient);
    let sender_bal_before = h.balance(&h.sender);

    // Order 1: withdraw, then cancel
    let expected_vested = h.get(id).deposited * (offset.min(duration) as i128) / (duration as i128);

    if expected_vested == 0 {
        assert_eq!(
            h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
            Error::NothingToWithdraw
        );
    } else {
        h.client.withdraw(&id, &None);
    }

    let stream_status = h.get(id).status;
    if stream_status == StreamStatus::Depleted {
        assert_eq!(
            h.client.try_cancel(&id).unwrap_err().unwrap(),
            Error::StreamTerminated
        );
    } else {
        h.client.cancel(&id);
    }

    let stream = h.get(id);

    assert_eq!(
        h.balance(&h.recipient) - recipient_bal_before,
        expected_vested
    );
    assert_eq!(
        h.balance(&h.sender) - sender_bal_before,
        deposit - expected_vested
    );

    assert!(stream.status == StreamStatus::Cancelled || stream.status == StreamStatus::Depleted);
    assert_eq!(stream.deposited, expected_vested);
    assert_eq!(stream.withdrawn, expected_vested);

    assert_eq!(
        (h.balance(&h.recipient) - recipient_bal_before)
            + (h.balance(&h.sender) - sender_bal_before),
        deposit
    );
}

fn test_cancel_then_withdraw_at(offset: u64) {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(offset);

    let recipient_bal_before = h.balance(&h.recipient);
    let sender_bal_before = h.balance(&h.sender);

    let expected_vested = h.get(id).deposited * (offset.min(duration) as i128) / (duration as i128);

    // Order 2: cancel, then withdraw
    h.client.cancel(&id);

    if expected_vested == 0 {
        // The cancel above already made the stream terminal, so a zero-vest
        // withdraw hits the terminal precondition, not the live-stream one:
        // `withdraw` reports `StreamTerminated`, never `NothingToWithdraw`,
        // which is reserved for a still-live stream that has not accrued. Same
        // precedence as `cancel::cancel_at_the_instant_of_creation_refunds_everything`.
        assert_eq!(
            h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
            Error::StreamTerminated
        );
    } else {
        h.client.withdraw(&id, &None);
    }

    let stream = h.get(id);

    assert_eq!(
        h.balance(&h.recipient) - recipient_bal_before,
        expected_vested
    );
    assert_eq!(
        h.balance(&h.sender) - sender_bal_before,
        deposit - expected_vested
    );

    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.deposited, expected_vested);
    assert_eq!(stream.withdrawn, expected_vested);

    assert_eq!(
        (h.balance(&h.recipient) - recipient_bal_before)
            + (h.balance(&h.sender) - sender_bal_before),
        deposit
    );
}

#[test]
fn withdraw_then_cancel_schedule_points() {
    test_withdraw_then_cancel_at(0); // Start
    test_withdraw_then_cancel_at(25 * DAY); // Quarter
    test_withdraw_then_cancel_at(50 * DAY); // Halfway
    test_withdraw_then_cancel_at(99 * DAY); // Near end
    test_withdraw_then_cancel_at(100 * DAY); // Exact end
    test_withdraw_then_cancel_at(150 * DAY); // Post end
}

#[test]
fn cancel_then_withdraw_schedule_points() {
    test_cancel_then_withdraw_at(0); // Start
    test_cancel_then_withdraw_at(25 * DAY); // Quarter
    test_cancel_then_withdraw_at(50 * DAY); // Halfway
    test_cancel_then_withdraw_at(99 * DAY); // Near end
    test_cancel_then_withdraw_at(100 * DAY); // Exact end
    test_cancel_then_withdraw_at(150 * DAY); // Post end
}
