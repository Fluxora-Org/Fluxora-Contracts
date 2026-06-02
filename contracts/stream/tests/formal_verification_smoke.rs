#![cfg(test)]
extern crate std;

use crate::accrual::calculate_accrued_amount;

#[test]
fn smoke_accrual_examples() {
    // Basic sanity checks used as a CI smoke test for accrual arithmetic
    let r = calculate_accrued_amount(0, 0, 1000, 1, 1000, 500);
    assert_eq!(r, 500);

    let r2 = calculate_accrued_amount(0, 100, 200, 1, 100, 150);
    assert_eq!(r2, 50);
}
