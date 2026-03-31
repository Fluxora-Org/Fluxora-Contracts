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

    if rate_per_second < 0 {
        return 0;
    }

    let elapsed_now = current_time.min(end_time);
    let elapsed_seconds = if elapsed_now < start_time {
        0
    } else {
        (elapsed_now - start_time) as i128
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

#[cfg(test)]
mod invariants {
    use super::calculate_accrued_amount;

    fn sample_streams() -> &'static [(u64, u64, u64, i128, i128)] {
        &[
            (0, 0, 1_000, 1, 1_000),
            (1_000, 1_000, 2_000, 1, 1_000),
            (0, 500, 1_000, 2, 1_500),
            (0, 0, 1_000, 10, 5_000),
            (0, 0, 10_000, 0, 0),
            (0, 0, 1_000, 3, 500),
        ]
    }

    #[test]
    fn accrued_non_negative_and_bounded_by_deposit() {
        for &(start, cliff, end, rate, deposit) in sample_streams() {
            let times = [
                0,
                start.saturating_sub(1),
                start,
                cliff,
                start.saturating_add(cliff) / 2,
                end.saturating_sub(1),
                end,
                end.saturating_add(1),
            ];

            for &t in &times {
                let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, t);

                assert!(
                    accrued >= 0,
                    "accrued negative for stream {:?} at t={}",
                    (start, cliff, end, rate, deposit),
                    t
                );

                assert!(
                    accrued <= deposit,
                    "accrued greater than deposit for stream {:?} at t={}",
                    (start, cliff, end, rate, deposit),
                    t
                );
            }
        }
    }

    #[test]
    fn accrued_is_monotonic_in_time_after_cliff() {
        for &(start, cliff, end, rate, deposit) in sample_streams() {
            if cliff >= end {
                continue;
            }

            let t0 = if cliff > start { cliff } else { start };
            let span = end.saturating_sub(t0);

            let mut times_buf = [t0, t0, t0, t0, end];
            let mut len: usize = 1;

            if span > 1 {
                times_buf[len] = t0.saturating_add(span / 3);
                len += 1;

                times_buf[len] = t0.saturating_add(span / 2);
                len += 1;

                times_buf[len] = end.saturating_sub(1);
                len += 1;
            }

            times_buf[len] = end;
            len += 1;

            let mut prev = calculate_accrued_amount(start, cliff, end, rate, deposit, times_buf[0]);

            for &t in times_buf.iter().take(len).skip(1) {
                let now = calculate_accrued_amount(start, cliff, end, rate, deposit, t);

                assert!(
                    now >= prev,
                    "accrued not monotonic for stream {:?}: at t={} got {}, previous {}",
                    (start, cliff, end, rate, deposit),
                    t,
                    now,
                    prev
                );
                prev = now;
            }
        }
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

/// Property-based monotonicity and invariant tests for `calculate_accrued_amount`.
///
/// These tests systematically verify the mathematical properties that must hold
/// for all valid (and some degenerate) stream configurations:
///
/// 1. **Monotonicity**: accrued(t1) <= accrued(t2) for all t1 <= t2 after cliff.
/// 2. **Boundedness**: 0 <= accrued(t) <= deposit_amount for all t.
/// 3. **Zero before cliff**: accrued(t) == 0 for all t < cliff_time.
/// 4. **Saturation**: accrued(t) == deposit_amount for all t >= end_time (when rate*duration >= deposit).
/// 5. **Determinism**: same inputs always produce the same output.
/// 6. **Elapsed underflow guard**: returns 0 when elapsed_now < start_time (cliff < start edge).
#[cfg(test)]
mod property_monotonicity {
    use super::calculate_accrued_amount;

    // -----------------------------------------------------------------------
    // Test fixtures: (start, cliff, end, rate, deposit)
    // -----------------------------------------------------------------------

    /// Streams covering a wide range of shapes: no-cliff, mid-cliff, end-cliff,
    /// high-rate (deposit-capped), zero-rate, and near-overflow.
    const STREAMS: &[(u64, u64, u64, i128, i128)] = &[
        // (start, cliff, end, rate, deposit)
        (0, 0, 1_000, 1, 1_000),         // standard linear, no cliff
        (0, 500, 1_000, 1, 1_000),       // cliff at midpoint
        (0, 1_000, 1_000, 1, 1_000),     // cliff == end (degenerate: nothing ever accrues)
        (1_000, 1_000, 2_000, 2, 2_000), // non-zero start, rate=2
        (0, 0, 1_000, 10, 5_000),        // high rate, deposit is binding cap
        (0, 0, 10_000, 0, 0),            // zero rate, zero deposit
        (0, 0, 1_000, 3, 500),           // rate*duration > deposit (deposit caps)
        (0, 0, u64::MAX, 1, i128::MAX),  // near-overflow duration
        (100, 200, 1_000, 5, 4_500),     // cliff after start
        (0, 0, 1_000, 1, 2_000),         // deposit > rate*duration (excess deposit)
    ];

    /// Dense time grid for a stream: samples before, at, and after every boundary.
    fn time_grid(start: u64, cliff: u64, end: u64) -> [u64; 12] {
        let duration = end.saturating_sub(start);
        let mid = start.saturating_add(duration / 2);
        let q1 = start.saturating_add(duration / 4);
        let q3 = start.saturating_add((duration / 4).saturating_mul(3));
        let mut times = [
            0,
            start.saturating_sub(1),
            start,
            cliff.saturating_sub(1),
            cliff,
            q1,
            mid,
            q3,
            end.saturating_sub(1),
            end,
            end.saturating_add(1),
            end.saturating_add(1_000),
        ];
        times.sort();
        times
    }

    fn sort_array(arr: &mut [u64]) {
        for i in 0..arr.len() {
            for j in 0..arr.len() - i - 1 {
                if arr[j] > arr[j + 1] {
                    // let temp = arr[j];
                    // arr[j] = arr[j + 1];
                    // arr[j + 1] = temp;
                    arr.swap(j, j + 1);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Property 1: Monotonicity — accrued never decreases as time advances
    // -----------------------------------------------------------------------

    /// For every stream and every pair of consecutive time points t1 <= t2,
    /// accrued(t1) <= accrued(t2).
    #[test]
    fn prop_monotonic_over_dense_grid() {
        for &(start, cliff, end, rate, deposit) in STREAMS {
            let mut times = time_grid(start, cliff, end);
            sort_array(&mut times);
            let mut prev = calculate_accrued_amount(start, cliff, end, rate, deposit, times[0]);

            for &t in times.iter().skip(1) {
                let now = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
                assert!(
                    now >= prev,
                    "monotonicity violated for stream ({start},{cliff},{end},{rate},{deposit}): \
                     accrued({t})={now} < previous={prev}"
                );
                prev = now;
            }
        }
    }

    /// Monotonicity holds across a fine-grained sweep of every second in a short stream.
    #[test]
    fn prop_monotonic_second_by_second() {
        // start=0, cliff=100, end=500, rate=2, deposit=1000
        let (start, cliff, end, rate, deposit) = (0u64, 100u64, 500u64, 2i128, 1_000i128);
        let mut prev = calculate_accrued_amount(start, cliff, end, rate, deposit, 0);
        for t in 1..=600u64 {
            let now = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
            assert!(
                now >= prev,
                "second-by-second monotonicity violated at t={t}: got {now}, prev={prev}"
            );
            prev = now;
        }
    }

    // -----------------------------------------------------------------------
    // Property 2: Boundedness — result always in [0, deposit_amount]
    // -----------------------------------------------------------------------

    /// For every stream and every time point, 0 <= accrued <= deposit_amount.
    #[test]
    fn prop_bounded_by_deposit_over_dense_grid() {
        for &(start, cliff, end, rate, deposit) in STREAMS {
            for &t in time_grid(start, cliff, end).iter() {
                let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
                assert!(
                    accrued >= 0,
                    "negative accrual for stream ({start},{cliff},{end},{rate},{deposit}) at t={t}: {accrued}"
                );
                assert!(
                    accrued <= deposit,
                    "accrual exceeds deposit for stream ({start},{cliff},{end},{rate},{deposit}) at t={t}: \
                     accrued={accrued} > deposit={deposit}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Property 3: Zero before cliff
    // -----------------------------------------------------------------------

    /// accrued(t) == 0 for all t strictly before cliff_time.
    #[test]
    fn prop_zero_before_cliff() {
        for &(start, cliff, end, rate, deposit) in STREAMS {
            if cliff == 0 {
                continue; // no pre-cliff window to test
            }
            for t in [0u64, 1, cliff.saturating_sub(1)] {
                if t >= cliff {
                    continue;
                }
                let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
                assert_eq!(
                    accrued, 0,
                    "expected 0 before cliff for stream ({start},{cliff},{end},{rate},{deposit}) at t={t}, got {accrued}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Property 4: Saturation at end_time
    // -----------------------------------------------------------------------

    /// When rate * (end - start) >= deposit, accrued(t) == deposit for all t >= end_time.
    #[test]
    fn prop_saturates_at_end_time_when_rate_covers_deposit() {
        // Streams where rate * duration >= deposit (deposit is the binding cap)
        let saturating_streams: &[(u64, u64, u64, i128, i128)] = &[
            (0, 0, 1_000, 1, 1_000),
            (0, 0, 1_000, 10, 5_000),
            (0, 0, 1_000, 3, 500),
            (1_000, 1_000, 2_000, 2, 2_000),
        ];
        for &(start, cliff, end, rate, deposit) in saturating_streams {
            for &t in &[end, end + 1, end + 1_000, end + 1_000_000] {
                let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
                assert_eq!(
                    accrued, deposit,
                    "expected saturation at deposit={deposit} for stream ({start},{cliff},{end},{rate},{deposit}) \
                     at t={t}, got {accrued}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Property 5: Determinism
    // -----------------------------------------------------------------------

    /// Same inputs always produce the same output (pure function).
    #[test]
    fn prop_deterministic() {
        for &(start, cliff, end, rate, deposit) in STREAMS {
            for &t in time_grid(start, cliff, end).iter() {
                let a = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
                let b = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
                assert_eq!(
                    a, b,
                    "non-deterministic result for stream ({start},{cliff},{end},{rate},{deposit}) at t={t}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Property 6: Elapsed underflow guard (covers line 31 — None branch)
    // -----------------------------------------------------------------------

    /// When cliff_time < start_time (degenerate: cliff before start), and
    /// current_time is in [cliff_time, start_time), elapsed_now = current_time < start_time,
    /// so checked_sub returns None and the function returns 0.
    ///
    /// This covers the previously uncovered `None => return 0` branch in the
    /// `elapsed_seconds` calculation (accrual.rs line 31).
    #[test]
    fn prop_elapsed_underflow_returns_zero() {
        // cliff=0 < start=500, end=1000: current_time=200 passes cliff check
        // but elapsed_now=200 < start=500 → checked_sub underflows → 0
        let accrued = calculate_accrued_amount(
            500,   // start_time
            0,     // cliff_time (before start — degenerate)
            1_000, // end_time
            1,     // rate
            1_000, // deposit
            200,   // current_time: past cliff but before start
        );
        assert_eq!(
            accrued, 0,
            "elapsed underflow (current_time < start_time after cliff) must return 0"
        );
    }

    /// Boundary: current_time == start_time with cliff < start → accrued == 0 (elapsed == 0).
    #[test]
    fn prop_elapsed_zero_at_start_with_early_cliff() {
        let accrued = calculate_accrued_amount(500, 0, 1_000, 1, 1_000, 500);
        assert_eq!(
            accrued, 0,
            "at start_time with early cliff, elapsed=0 so accrued=0"
        );
    }

    /// One second past start with early cliff → accrues normally.
    #[test]
    fn prop_accrues_normally_after_start_with_early_cliff() {
        let accrued = calculate_accrued_amount(500, 0, 1_000, 1, 1_000, 501);
        assert_eq!(accrued, 1, "one second past start should accrue 1 token");
    }

    // -----------------------------------------------------------------------
    // Property 7: Linearity between cliff and end (when deposit is not binding)
    // -----------------------------------------------------------------------

    /// When deposit > rate * duration, accrued(t) == rate * (t - start) for t in [start, end].
    #[test]
    fn prop_linear_when_deposit_not_binding() {
        // deposit=2000 > rate*duration=1000: pure linear region
        let (start, cliff, end, rate, deposit) = (0u64, 0u64, 1_000u64, 1i128, 2_000i128);
        for t in [0u64, 1, 100, 250, 500, 750, 999, 1_000] {
            let expected = (rate * (t.min(end) - start) as i128).min(deposit).max(0);
            let accrued = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
            assert_eq!(
                accrued, expected,
                "linear accrual mismatch at t={t}: expected={expected}, got={accrued}"
            );
        }
    }
}

// ===========================================================================
// i128 boundary streams: near-max rate/deposit scenarios — pure math tests
//
// These tests exercise `calculate_accrued_amount` directly at i128-scale
// values, independent of the Soroban environment.
//
// Scope: every observable property of the accrual function at near-max values.
// Exclusions: token transfer mechanics and contract storage (covered in test.rs
// and integration_suite.rs). Gas budget is not applicable to pure functions.
// ===========================================================================
#[cfg(test)]
mod i128_boundary {
    use super::calculate_accrued_amount;

    // -----------------------------------------------------------------------
    // 1. Near-max deposit, rate=deposit, duration=1s
    // -----------------------------------------------------------------------

    /// At t=0 (start), accrued is 0 for near-max deposit.
    #[test]
    fn near_max_deposit_zero_at_start() {
        let deposit = i128::MAX / 2;
        let rate = i128::MAX / 2;
        let accrued = calculate_accrued_amount(0, 0, 1, rate, deposit, 0);
        assert_eq!(accrued, 0);
    }

    /// At t=1 (end), accrued equals deposit for near-max stream.
    #[test]
    fn near_max_deposit_equals_deposit_at_end() {
        let deposit = i128::MAX / 2;
        let rate = i128::MAX / 2;
        let accrued = calculate_accrued_amount(0, 0, 1, rate, deposit, 1);
        assert_eq!(accrued, deposit);
    }

    /// Long after end_time, accrued is still capped at deposit.
    #[test]
    fn near_max_deposit_capped_long_after_end() {
        let deposit = i128::MAX / 2;
        let rate = i128::MAX / 2;
        let accrued = calculate_accrued_amount(0, 0, 1, rate, deposit, u64::MAX);
        assert_eq!(accrued, deposit);
    }

    // -----------------------------------------------------------------------
    // 2. Overflow path: elapsed * rate overflows i128 → returns deposit_amount
    // -----------------------------------------------------------------------

    /// elapsed=2, rate=i128::MAX/2+1 → product overflows → returns deposit.
    /// This directly exercises the `None => deposit_amount` branch in checked_mul.
    #[test]
    fn overflow_in_multiplication_returns_deposit() {
        // rate = i128::MAX/2 + 1, elapsed = 2 → product = i128::MAX + 2 → overflow
        let rate = i128::MAX / 2 + 1;
        let deposit = i128::MAX / 4; // deposit < rate*2, so overflow path is hit
                                     // start=0, cliff=0, end=10, current=2 → elapsed=2
        let accrued = calculate_accrued_amount(0, 0, 10, rate, deposit, 2);
        // overflow → returns deposit_amount, then clamped to deposit
        assert_eq!(accrued, deposit, "overflow must return deposit_amount");
    }

    /// i128::MAX rate, elapsed=2 → overflow → returns deposit.
    #[test]
    fn max_rate_overflow_returns_deposit() {
        let deposit = 42_i128;
        let accrued = calculate_accrued_amount(0, 0, 100, i128::MAX, deposit, 2);
        assert_eq!(accrued, deposit);
    }

    // -----------------------------------------------------------------------
    // 3. Near-max deposit with cliff boundary
    // -----------------------------------------------------------------------

    /// Before cliff, accrued is 0 even for near-max deposit.
    #[test]
    fn near_max_deposit_zero_before_cliff() {
        let deposit = i128::MAX / 1_000_000;
        let rate = deposit / 1_000;
        // cliff=500, current=499 → must return 0
        let accrued = calculate_accrued_amount(0, 500, 1_000, rate, deposit, 499);
        assert_eq!(accrued, 0);
    }

    /// Exactly at cliff, accrual uses elapsed from start_time.
    #[test]
    fn near_max_deposit_at_cliff_uses_start_time() {
        let deposit = i128::MAX / 1_000_000;
        let rate = deposit / 1_000;
        // start=0, cliff=500, end=1000, current=500 → elapsed=500
        let accrued = calculate_accrued_amount(0, 500, 1_000, rate, deposit, 500);
        let expected = 500_i128 * rate;
        assert_eq!(accrued, expected);
    }

    // -----------------------------------------------------------------------
    // 4. Monotonicity at near-max scale
    // -----------------------------------------------------------------------

    /// Accrual is monotonically non-decreasing across a dense time grid.
    #[test]
    fn near_max_deposit_monotonic_over_time_grid() {
        let deposit = i128::MAX / 1_000_000;
        let rate = deposit / 1_000;
        let times = [0u64, 1, 100, 499, 500, 501, 750, 999, 1_000, 1_001, 999_999];
        let mut prev = calculate_accrued_amount(0, 500, 1_000, rate, deposit, times[0]);
        for &t in times.iter().skip(1) {
            let now = calculate_accrued_amount(0, 500, 1_000, rate, deposit, t);
            assert!(
                now >= prev,
                "monotonicity violated at t={t}: got {now}, prev={prev}"
            );
            prev = now;
        }
    }

    // -----------------------------------------------------------------------
    // 5. Boundedness at near-max scale
    // -----------------------------------------------------------------------

    /// For all time points, 0 <= accrued <= deposit at near-max scale.
    #[test]
    fn near_max_deposit_bounded_at_all_times() {
        let deposit = i128::MAX / 1_000_000;
        let rate = deposit / 1_000;
        let times = [0u64, 1, 499, 500, 750, 1_000, 1_001, u64::MAX / 2, u64::MAX];
        for &t in &times {
            let accrued = calculate_accrued_amount(0, 500, 1_000, rate, deposit, t);
            assert!(accrued >= 0, "negative accrual at t={t}: {accrued}");
            assert!(
                accrued <= deposit,
                "accrual exceeds deposit at t={t}: {accrued}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 6. Deposit > rate * duration (excess deposit)
    // -----------------------------------------------------------------------

    /// When deposit > rate * duration, accrued at end == rate * duration (not deposit).
    #[test]
    fn near_max_deposit_excess_deposit_caps_at_total_streamable() {
        let rate: i128 = i128::MAX / 1_000_000;
        let duration: u64 = 1_000;
        let total_streamable = rate * duration as i128;
        let deposit = total_streamable + 999_999; // excess deposit

        let accrued = calculate_accrued_amount(0, 0, duration, rate, deposit, duration);
        assert_eq!(
            accrued, total_streamable,
            "must cap at rate*duration, not deposit"
        );
    }

    // -----------------------------------------------------------------------
    // 7. Determinism at near-max scale
    // -----------------------------------------------------------------------

    /// Same near-max inputs always produce the same output.
    #[test]
    fn near_max_deposit_deterministic() {
        let deposit = i128::MAX / 2;
        let rate = i128::MAX / 2;
        let a = calculate_accrued_amount(0, 0, 1, rate, deposit, 1);
        let b = calculate_accrued_amount(0, 0, 1, rate, deposit, 1);
        assert_eq!(a, b);
    }
}
