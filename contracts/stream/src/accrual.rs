/// Computes accrued stream amount without relying on Soroban environment state.
///
/// This helper is intentionally pure to make the core vesting math easy to unit test.
///
/// Rules:
/// - Returns `0` before `cliff_time`.
/// - Returns `0` for invalid schedules (`start_time >= end_time`) or negative rates.
/// - Uses `min(current_time, end_time)` so accrual is capped at stream end.
/// - Multiplies elapsed seconds by `rate_per_second`, and on multiplication overflow
///   returns `deposit_amount` (safe upper bound before final clamping).
/// - Final result is clamped to `[0, deposit_amount]`.
pub fn calculate_accrued_amount(
    start_time: u64,
    cliff_time: u64,
    end_time: u64,
    rate_per_second: i128,
    deposit_amount: i128,
    current_time: u64,
) -> i128 {
    if current_time < cliff_time {
        return 0;
    }

    if start_time >= end_time || rate_per_second < 0 {
        return 0;
    }

    let elapsed_now = current_time.min(end_time);
    let elapsed_seconds = match elapsed_now.checked_sub(start_time) {
        Some(elapsed) => elapsed as i128,
        None => return 0,
    };

    let accrued = match elapsed_seconds.checked_mul(rate_per_second) {
        Some(amount) => amount,
        None => deposit_amount,
    };

    accrued.min(deposit_amount).max(0)
}

#[cfg(test)]
mod tests {
    use super::calculate_accrued_amount;

    #[test]
    fn returns_zero_before_cliff() {
        let accrued = calculate_accrued_amount(0, 500, 1000, 1, 1000, 499);
        assert_eq!(accrued, 0);
    }

    #[test]
    fn accrues_from_start_at_cliff() {
        let accrued = calculate_accrued_amount(0, 500, 1000, 1, 1000, 500);
        assert_eq!(accrued, 500);
    }

    #[test]
    fn caps_at_end_time_and_deposit() {
        let accrued = calculate_accrued_amount(0, 0, 1000, 2, 1000, 9_999);
        assert_eq!(accrued, 1000);
    }

    #[test]
    fn returns_zero_for_invalid_schedule() {
        let accrued = calculate_accrued_amount(10, 10, 10, 1, 1000, 10);
        assert_eq!(accrued, 0);
    }

    #[test]
    fn returns_zero_for_negative_rate() {
        let accrued = calculate_accrued_amount(0, 0, 1000, -1, 1000, 100);
        assert_eq!(accrued, 0);
    }

    #[test]
    fn multiplication_overflow_returns_capped_deposit() {
        let accrued = calculate_accrued_amount(0, 0, u64::MAX, i128::MAX, 10_000, u64::MAX);
        assert_eq!(accrued, 10_000);
    }
}
