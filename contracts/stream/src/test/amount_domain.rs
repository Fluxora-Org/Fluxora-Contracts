use super::common::*;
use crate::Error;

// Design decisions:
// - Zero amounts are rejected with `Error::InvalidAmount` for top-up and withdraw,
//   and `Error::InvalidDeposit` for create (they are errors, not no-ops).
// - Withdrawing exactly the available balance succeeds and leaves the stream
//   with zero withdrawable balance; further withdrawals are `InsufficientWithdrawable`.

[#test]
fn create_stream_rejects_zero_negative_and_handles_extremes() {
    let h = Harness::new();
    let start = h.now();

    // Zero deposit -> InvalidDeposit
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &0i128,
            &start,
            &(start + 10 * DAY),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidDeposit);

    // Negative deposit -> InvalidDeposit
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            & h.token,
            &-1i128,
            &start,
            &(start + 10 * DAY),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidDeposit);

    // i128::MIN -> treated as negative -> InvalidDeposit
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            & h.recipient,
            & h.token,
            &i128::MIN,
            &start,
            &(start + 10 * DAY),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidDeposit);

    // i128::MAX: creation validation may pass but transfer should fail due to
    // insufficient sender balance in the test harness. Accept either token
    // transfer failure or missing token semantics.
    let res = h.client.try_create_stream(
        &h.sender,
        &h.recipient,
        &h.token,
        &i128::MAX,
        &start,
        &(start + 1),
        &start,
        &true,
        &true,
        &true,
    );
    if let Ok(Ok(_)) = res {
        panic!("huge deposit unexpectedly succeeded");
    }
}

[#test]
fn top_up_rejects_zero_negative_and_extreme_amounts() {
    let h = Harness::new();
    let _start = h.now();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Zero amount -> InvalidAmount
    let err = h.client.try_top_up(&id, &0i128).unwrap_err().unwrap();
    assert_eq!(err, Error::InvalidAmount);

    // Negative amount -> InvalidAmount
    let err = h.client.try_top_up(&id, &-1i128).unwrap_err().unwrap();
    assert_eq!(err, Error::InvalidAmount);

    // i128::MIN -> InvalidAmount
    let err = h.client.try_top_up(&id, &i128::MIN).unwrap_err().unwrap();
    assert_eq!(err, Error::InvalidAmount);

    // i128::MAX -> the checked arithmetic overflows before any transfer is
    // attempted, so the call must surface the typed Overflow error rather than
    // panicking (or, on a smaller stream where the math fits, the harness token
    // rejects the transfer due to insufficient balance — a token error).
    let res = h.client.try_top_up(&id, &i128::MAX);
    if let Ok(Ok(_)) = res {
        panic!("huge top_up unexpectedly succeeded");
    }
}

[#test]
fn withdraw_validates_amount_bounds_and_limits() {
    let h = Harness::new();
    let _start = h.now();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // advance to accrue some vested amount
    h.advance(10 * DAY);
    let available = h.client.withdrawable_of(&id);
    assert!(available > 0, "sanity: expected some withdrawable balance");

    // Zero -> InvalidAmount
    let err = h
        .client
        .try_withdraw(&id, &Some(0i128))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidAmount);

    // Negative -> InvalidAmount
    let err = h
        .client
        .try_withdraw(&id, &Some(-1i128))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidAmount);

    // Too large -> InsufficientWithdrawable
    let err = h
        .client
        .try_withdraw(&id, &Some(i128::MAX))
        .unwrap_error()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable);
}

#[test]
fn minimal_positive_amounts_are_accepted() {
    let h = Harness::new();
    let _start = h.now();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Top up with 1 token (the smallest positive representable amount)
    let result = h.client.try_top_up(&id, &1i128);
    assert!(result.is_ok(), "top_up of 1 should succeed");

    // Advance a little to make some withdrawable balance.
    h.advance(1 * DAY);
    let available = h.client.withdrawable_of(&id);
    assert!(available > 0, "expected positive withdrawable");

    // Withdraw 1 token.
    let result = h.client.try_withdraw(&id, &Some(1i128));
    assert!(result.is_ok(), "withdraw of 1 should succeed");
}

#[test]
fn withdraw_exact_balance_updates_stream_state() {
    let h = Harness::new();
    let _start = h.now();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Advance to vest some amount.
    h.advance(50 * DAY);
    let available = h.client.withdrawable_of(&id);
    assert!(available > 0, "expected some withdrawable balance");

    // Withdraw exactly the available balance.
    let result = h.client.try_withdraw(&id, &Some(available));
    assert!(result.is_ok(), "exact balance withdrawal should succeed");

    // The stream must have no withdrawable balance left.
    let remaining = h.client.withdrawable_of(&id);
    assert_eq!(remaining, 0, "exact withdrawal should deplete the stream");

    // Any subsequent withdrawal attempt is over-balance.
    let err = h
        .client
        .try_withdraw(&id, &Some(1i128))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable);
}

#[test]
fn repeated_withdrawals_are_bounded_by_available_balance() {
    let h = Harness::new();
    let _start = h.now();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(10 * DAY);
    let available = h.client.withdrawable_of(&id);
    assert!(available > 0);

    let half = available / 2;
    let first = h.client.try_withdraw(&id, &Some(half));
    assert!(first.is_ok());
    let second = h.client.try_withdraw(&id, &Some(half));
    assert!(second.is_ok());

    let remaining = h.client.withdrawable_of(&id);
    let expected_remaining = available - half - half;
    assert_eq!(remaining, expected_remaining);
}
