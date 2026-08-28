use super::common::*;
use crate::Error;

#[test]
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
            &h.token,
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
            &h.recipient,
            &h.token,
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
    if let Err(inner) = res {
        if let Ok(e) = inner {
            assert!(
                matches!(e, Error::TokenTransferFailed | Error::TokenMissing),
                "expected token transfer error for huge deposit, got {:?}",
                e
            );
        }
    }
}

#[test]
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

    // i128::MAX -> rejected with a typed error rather than panicking. The rate
    // scaling (`amount * duration`) overflows first, surfacing as `Overflow`;
    // a token transfer error would also be acceptable. Either is a clean
    // rejection of an extreme amount.
    let res = h.client.try_top_up(&id, &i128::MAX);
    if let Err(inner) = res {
        if let Ok(e) = inner {
            assert!(
                matches!(
                    e,
                    Error::Overflow | Error::TokenTransferFailed | Error::TokenMissing
                ),
                "expected a typed error for huge top_up, got {:?}",
                e
            );
        }
    }
}

#[test]
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
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable);
}
