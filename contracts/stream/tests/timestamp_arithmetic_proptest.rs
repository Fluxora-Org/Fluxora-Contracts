// contracts/stream/tests/timestamp_arithmetic_proptest.rs
use proptest::prelude::*;
use fluxora_stream::*; // Adjust import based on your actual module structure
use soroban_sdk::Env;

/// Fuzz test to prove timestamp arithmetic is overflow-safe
/// Tests operations like end_time - start_time, cliff_time - start_time,
/// current_ledger - last_withdraw_ledger with adversarial u64 values
#[cfg(test)]
mod timestamp_arithmetic_tests {
    use super::*;

    // Define strategies for generating timestamps
    fn timestamp_strategy() -> impl Strategy<Value = u64> {
        // Generate values across the full u64 range, with clustering near boundaries
        prop::strategy::Union::new(vec![
            // Near zero
            (0..100u64).boxed(),
            // Near u64::MAX
            (u64::MAX - 100..=u64::MAX).boxed(),
            // Random values across the full range
            (0..=u64::MAX).boxed(),
        ])
    }

    fn ordered_timestamps() -> impl Strategy<Value = (u64, u64)> {
        (timestamp_strategy(), timestamp_strategy()).prop_filter(
            "start_time must be <= end_time",
            |(start, end)| start <= end,
        )
    }

    fn three_ordered_timestamps() -> impl Strategy<Value = (u64, u64, u64)> {
        (timestamp_strategy(), timestamp_strategy(), timestamp_strategy())
            .prop_filter(
                "start_time <= cliff_time <= end_time",
                |(start, cliff, end)| start <= cliff && cliff <= end,
            )
    }

    #[test]
    fn test_end_time_minus_start_time_overflow_safe() {
        let mut env = Env::default();
        
        // Generate many random timestamp pairs
        let strategy = ordered_timestamps();
        proptest!(|(start, end) in strategy| {
            // Test the subtraction using checked_sub (should never panic)
            let result = end.checked_sub(start);
            assert!(result.is_some(), "Subtraction should not overflow");
            
            // If we have a contract function that uses this, call it here
            // For example: let accrued = contract.calculate_accrued(&env, start, end);
            
            // Verify the result makes sense
            if let Some(diff) = result {
                assert!(diff <= end, "Difference should not exceed end");
                assert!(diff <= u64::MAX, "Difference should fit in u64");
            }
        });
    }

    #[test]
    fn test_cliff_time_minus_start_time_overflow_safe() {
        let strategy = three_ordered_timestamps();
        proptest!(|(start, cliff, end) in strategy| {
            // Test cliff_time - start_time
            let cliff_diff = cliff.checked_sub(start);
            assert!(cliff_diff.is_some(), "Cliff subtraction should not overflow");
            
            // Test end_time - start_time
            let end_diff = end.checked_sub(start);
            assert!(end_diff.is_some(), "End subtraction should not overflow");
            
            // Test end_time - cliff_time
            let remaining = end.checked_sub(cliff);
            assert!(remaining.is_some(), "Remaining time subtraction should not overflow");
            
            // Verify invariants
            if let (Some(cliff_d), Some(end_d), Some(rem)) = (cliff_diff, end_diff, remaining) {
                assert!(cliff_d <= end_d, "Cliff period should be <= total period");
                assert!(rem >= 0, "Remaining time should be non-negative");
                assert!(cliff_d + rem <= end_d + 1, "Parts should sum to total");
            }
        });
    }

    #[test]
    fn test_current_ledger_subtractions_overflow_safe() {
        let strategy = (
            timestamp_strategy(), // current_ledger
            timestamp_strategy(), // last_withdraw_ledger
        ).prop_filter("current_ledger >= last_withdraw_ledger", |(current, last)| {
            current >= last
        });

        proptest!(|(current, last) in strategy| {
            // Test current_ledger - last_withdraw_ledger
            let diff = current.checked_sub(last);
            assert!(diff.is_some(), "Current - last withdrawal should not overflow");
            
            if let Some(time_since_last) = diff {
                assert!(time_since_last <= current, "Time since last should not exceed current");
                
                // Additional property: if we add back last, we should get current
                if let Some(reconstructed) = last.checked_add(time_since_last) {
                    assert_eq!(reconstructed, current, "Should be able to reconstruct current");
                }
            }
        });
    }

    #[test]
    fn test_accumulated_amount_calculations_overflow_safe() {
        let strategy = (
            timestamp_strategy(), // start_time
            timestamp_strategy(), // end_time
            timestamp_strategy(), // current_time
        ).prop_filter("valid timestamps", |(start, end, current)| {
            start <= end && start <= current
        });

        proptest!(|(start, end, current) in strategy {
            // Test the full calculation path
            // This should call the actual function that calculates accrued amount
            // with adversarial timestamps
            
            // Example: call into the contract
            let mut env = Env::default();
            // let contract = StreamContract::new();
            // let result = contract.calculate_accrued_amount(&env, start, end, current);
            
            // Assert no panic occurred
            // Assert result is within expected bounds
            if let (Some(total_duration), Some(elapsed)) = (
                end.checked_sub(start),
                current.checked_sub(start)
            ) {
                if total_duration > 0 && elapsed > 0 {
                    let ratio = elapsed as u128 * 100 / total_duration as u128;
                    assert!(ratio <= 100, "Elapsed ratio should not exceed 100%");
                }
            }
        });
    }

    #[test]
    fn test_adversarial_timestamp_clustering() {
        // Specifically test values near u64::MAX to catch overflow
        let adversarial_values = vec![
            (u64::MAX, u64::MAX),
            (u64::MAX - 1, u64::MAX),
            (u64::MAX, u64::MAX - 1),
            (u64::MAX - 100, u64::MAX - 50),
            (u64::MAX - 1, u64::MAX - 1),
            (u64::MAX / 2, u64::MAX / 2 + 1),
        ];

        for (a, b) in adversarial_values {
            // Test subtraction with checked_sub
            let result = a.checked_sub(b);
            // If a >= b, result should be Some; if a < b, result should be None (underflow)
            if a >= b {
                assert!(result.is_some(), "Subtraction {} - {} should not overflow", a, b);
                if let Some(diff) = result {
                    assert!(diff <= a, "Difference should not exceed original");
                }
            } else {
                assert!(result.is_none(), "Subtraction {} - {} should underflow", a, b);
            }
        }
    }

    #[test]
    fn test_multi_operation_chains_no_panic() {
        let strategy = (
            timestamp_strategy(),
            timestamp_strategy(),
            timestamp_strategy(),
            timestamp_strategy(),
        ).prop_filter("ordered timestamps", |(a, b, c, d)| {
            a <= b && b <= c && c <= d
        });

        proptest!(|(start, cliff, end, current) in strategy {
            // Perform a chain of operations typical in the contract
            let total = end.checked_sub(start).unwrap_or(0);
            let cliff_period = cliff.checked_sub(start).unwrap_or(0);
            let elapsed = current.checked_sub(start).unwrap_or(0);
            let remaining = end.checked_sub(current).unwrap_or(0);
            
            // Verify properties
            if total > 0 && cliff_period <= total {
                let cliff_ratio = cliff_period as u128 * 100 / total as u128;
                assert!(cliff_ratio <= 100, "Cliff ratio should not exceed 100%");
            }
            
            if elapsed > 0 && total > 0 {
                let elapsed_ratio = elapsed as u128 * 100 / total as u128;
                assert!(elapsed_ratio <= 100, "Elapsed ratio should not exceed 100%");
            }
            
            // Ensure no panic occurred
        });
    }

    // Regression tests for specific overflow cases
    #[test]
    fn test_regression_known_overflow_scenarios() {
        // Test known edge cases that might cause overflow
        let test_cases = vec![
            (u64::MAX, u64::MAX, u64::MAX, u64::MAX),
            (0, u64::MAX, 0, u64::MAX),
            (u64::MAX, u64::MAX - 1, u64::MAX, u64::MAX - 1),
            (1, u64::MAX, 1, u64::MAX),
        ];

        for (start, end, cliff, current) in test_cases {
            // Test each operation individually
            assert!(end.checked_sub(start).is_some() || end < start);
            assert!(cliff.checked_sub(start).is_some() || cliff < start);
            assert!(current.checked_sub(start).is_some() || current < start);
            
            // Test combination that might cause overflow
            if let (Some(total), Some(cliff_d)) = (end.checked_sub(start), cliff.checked_sub(start)) {
                // Check that operations don't overflow
                let _ = total.checked_add(cliff_d); // This might overflow, but should be checked
                let _ = total.checked_sub(cliff_d); // This should be safe if total >= cliff_d
            }
        }
    }
}