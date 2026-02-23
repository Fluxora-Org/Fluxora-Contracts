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

/// Tests for Issue #47: calculate_accrued is capped after end_time
///
/// These tests verify that accrual stops at end_time regardless of how much
/// time has passed. The result must always equal min(rate * duration, deposit_amount).
#[cfg(test)]
mod accrued_after_end_time {
    use crate::accrual::calculate_accrued_amount;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// A standard stream used across tests:
    ///   start=1000, cliff=1000, end=2000, rate=1/s, deposit=1000
    ///   => total streamable = 1 * (2000-1000) = 1000 == deposit
    fn standard_stream() -> (u64, u64, u64, i128, i128) {
        let start_time: u64 = 1_000;
        let cliff_time: u64 = 1_000;
        let end_time: u64 = 2_000;
        let rate_per_second: i128 = 1;
        let deposit_amount: i128 = 1_000;
        (
            start_time,
            cliff_time,
            end_time,
            rate_per_second,
            deposit_amount,
        )
    }

    // -----------------------------------------------------------------------
    // Core Issue #47 tests: accrual capped at end_time
    // -----------------------------------------------------------------------

    /// Exactly at end_time: result must equal full deposit amount.
    #[test]
    fn exactly_at_end_time_equals_deposit() {
        let (start, cliff, end, rate, deposit) = standard_stream();
        let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, end);
        assert_eq!(
            accrued, deposit,
            "at end_time, accrued should equal deposit_amount"
        );
    }

    /// One second past end_time: result must still equal deposit (no extra accrual).
    #[test]
    fn one_second_after_end_time_still_capped() {
        let (start, cliff, end, rate, deposit) = standard_stream();
        let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, end + 1);
        assert_eq!(
            accrued, deposit,
            "one second past end_time should not accrue more than deposit_amount"
        );
    }

    /// Long after end_time (10x the stream duration): result still capped at deposit.
    #[test]
    fn long_after_end_time_still_capped() {
        let (start, cliff, end, rate, deposit) = standard_stream();
        let far_future = end + 10_000;
        let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, far_future);
        assert_eq!(
            accrued, deposit,
            "long after end_time, accrued must be capped at deposit_amount"
        );
    }

    /// u64::MAX as current_time: must not overflow and must cap at deposit.
    #[test]
    fn max_time_does_not_overflow() {
        let (start, cliff, end, rate, deposit) = standard_stream();
        let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, u64::MAX);
        assert_eq!(
            accrued, deposit,
            "u64::MAX current_time should cap safely at deposit_amount"
        );
    }

    // -----------------------------------------------------------------------
    // Edge cases: boundary conditions around end_time
    // -----------------------------------------------------------------------

    /// One second BEFORE end_time: accrued must be less than deposit.
    #[test]
    fn one_second_before_end_time_less_than_deposit() {
        let (start, cliff, end, rate, deposit) = standard_stream();
        let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, end - 1);
        assert!(
            accrued < deposit,
            "one second before end_time, accrued ({accrued}) should be less than deposit ({deposit})"
        );
        assert_eq!(accrued, 999, "should have accrued 999 out of 1000");
    }

    /// Exactly at start_time (== cliff_time): should accrue 0.
    #[test]
    fn at_start_time_accrues_zero() {
        let (start, cliff, end, rate, deposit) = standard_stream();
        let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, start);
        assert_eq!(accrued, 0, "at start_time, nothing should have accrued yet");
    }

    /// Midway through stream: should accrue exactly half the deposit.
    #[test]
    fn midway_accrues_half_deposit() {
        let (start, cliff, end, rate, deposit) = standard_stream();
        let midpoint = (start + end) / 2; // 1500
        let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, midpoint);
        assert_eq!(
            accrued, 500,
            "halfway through, should accrue half the deposit"
        );
    }

    // -----------------------------------------------------------------------
    // High rate streams: deposit is the binding cap
    // -----------------------------------------------------------------------

    /// Rate so high that rate * duration >> deposit: cap must be deposit, not rate * time.
    #[test]
    fn high_rate_caps_at_deposit_at_end_time() {
        // rate=10/s, duration=1000s => total streamable=10_000 but deposit=5_000
        let accrued = calculate_accrued_amount(
            0,     // start
            0,     // cliff
            1_000, // end
            10,    // rate_per_second
            5_000, // deposit (lower than rate * duration)
            1_000, // current_time == end_time
        );
        assert_eq!(
            accrued, 5_000,
            "when rate*duration > deposit, result must cap at deposit_amount"
        );
    }

    /// High rate, long after end: still capped at deposit.
    #[test]
    fn high_rate_long_after_end_still_caps_at_deposit() {
        let accrued = calculate_accrued_amount(
            0, 0, 1_000, 10, 5_000, 999_999, // far future
        );
        assert_eq!(accrued, 5_000);
    }

    // -----------------------------------------------------------------------
    // Cliff after end_time edge: before cliff, always zero
    // -----------------------------------------------------------------------

    /// current_time is past end_time but before cliff_time: must return 0.
    #[test]
    fn past_end_but_before_cliff_returns_zero() {
        // Unusual but valid schedule: cliff > end (degenerate)
        // start=0, cliff=5000, end=1000 => start < end but cliff > end
        // The function should return 0 because current_time < cliff_time
        let accrued = calculate_accrued_amount(
            0,     // start
            5_000, // cliff (way after end)
            1_000, // end
            1,     // rate
            1_000, // deposit
            2_000, // current_time > end but < cliff
        );
        assert_eq!(
            accrued, 0,
            "before cliff, accrual must be zero even if past end_time"
        );
    }

    // -----------------------------------------------------------------------
    // Result consistency: calling twice returns same value
    // -----------------------------------------------------------------------

    /// Calling calculate_accrued_amount is pure/deterministic: same args → same result.
    #[test]
    fn pure_function_same_result_on_repeat_calls() {
        let (start, cliff, end, rate, deposit) = standard_stream();
        let t = end + 500;
        let first = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
        let second = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
        assert_eq!(first, second, "pure function must be deterministic");
        assert_eq!(first, deposit);
    }

    // -----------------------------------------------------------------------
    // Documented cap formula: result == min(rate * (end - start), deposit)
    // -----------------------------------------------------------------------

    /// Verifies the documented cap formula from the issue:
    /// result == min(rate_per_second * (end_time - start_time), deposit_amount)
    #[test]
    fn cap_matches_issue_formula() {
        let start: u64 = 500;
        let cliff: u64 = 500;
        let end: u64 = 1_500;
        let rate: i128 = 3;
        let deposit: i128 = 2_000;

        // rate * duration = 3 * 1000 = 3000, but deposit = 2000
        // so expected = min(3000, 2000) = 2000
        let expected = (rate * (end - start) as i128).min(deposit);

        let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, end + 9_999);
        assert_eq!(
            accrued, expected,
            "result must match the documented cap formula: min(rate*(end-start), deposit)"
        );
    }
}
