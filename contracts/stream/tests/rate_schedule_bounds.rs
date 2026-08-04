//! Comprehensive tests for `validate_rate_schedule` and `MAX_RATE_SEGMENTS`.
//!
//! Coverage target: >95% of the `validate_rate_schedule` function surface,
//! including every error variant, edge case, and property.
//!
//! # Security properties validated
//! - Segment-count cap prevents storage-bloat attacks.
//! - Zero-length segments are rejected (cannot inflate segment count for free).
//! - Negative rates are rejected (accrual math is undefined for negative rates).
//! - Checked arithmetic rejects overflow; wrapping/saturating never occurs.
//! - Any single violation short-circuits (returns Err immediately).
//! - Empty schedule is valid (no segments → no work).
//! - Single-segment schedule is valid at boundary sizes.

#![cfg(test)]

use fluxora_stream::accrual::{validate_rate_schedule, RateSegment, MAX_RATE_SEGMENTS};
use fluxora_stream::ContractError;

// =========================================================================
// Helpers
// =========================================================================

/// Short-hand for a valid segment.
fn seg(rate: i128, dur: u64) -> RateSegment {
    RateSegment {
        rate,
        duration_secs: dur,
    }
}

/// Many small valid segments that never overflow.
fn many_small(how_many: usize) -> Vec<RateSegment> {
    (0..how_many).map(|_| seg(1, 1)).collect()
}

// =========================================================================
// 1. Segment-count cap (MAX_RATE_SEGMENTS)
// =========================================================================

#[test]
fn max_segments_exact_count_ok() {
    let s = many_small(MAX_RATE_SEGMENTS as usize);
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn max_segments_plus_one_rejected() {
    let s = many_small(MAX_RATE_SEGMENTS as usize + 1);
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleTooManySegments)
    );
}

#[test]
fn max_segments_plus_many_rejected() {
    let s = many_small(MAX_RATE_SEGMENTS as usize * 2);
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleTooManySegments)
    );
}

#[test]
fn max_segments_one_less_ok() {
    let s = many_small(MAX_RATE_SEGMENTS as usize - 1);
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn empty_schedule_valid() {
    let s: Vec<RateSegment> = vec![];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

// =========================================================================
// 2. Zero-length segments
// =========================================================================

#[test]
fn zero_length_segment_rejected() {
    let s = vec![seg(1, 0)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn zero_length_at_beginning_rejected() {
    let s = vec![seg(1, 0), seg(1, 100)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn zero_length_in_middle_rejected() {
    let s = vec![seg(1, 100), seg(1, 0), seg(1, 100)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn zero_length_at_end_rejected() {
    let s = vec![seg(1, 100), seg(1, 0)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn zero_length_with_max_rate_rejected() {
    let s = vec![seg(i128::MAX, 0)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

// =========================================================================
// 3. Negative rates
// =========================================================================

#[test]
fn negative_rate_rejected() {
    let s = vec![seg(-1, 100)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn negative_rate_large_rejected() {
    let s = vec![seg(-1_000_000, 100)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn negative_rate_i128_min_rejected() {
    let s = vec![seg(i128::MIN, 100)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn negative_rate_after_valid_segment_rejected() {
    let s = vec![seg(1, 100), seg(-1, 100)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn negative_rate_in_middle_rejected() {
    let s = vec![seg(1, 100), seg(-5, 50), seg(1, 100)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn zero_rate_accepted() {
    // Zero is not negative — zero rate is valid (though accrual yields 0).
    let s = vec![seg(0, 100)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

// =========================================================================
// 4. Cumulative-sum overflow (checked_mul / checked_add)
// =========================================================================

#[test]
fn single_segment_at_max_rate_max_duration_overflows() {
    let s = vec![seg(i128::MAX, 2)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn single_segment_max_rate_one_second_ok() {
    let s = vec![seg(i128::MAX, 1)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn cumulative_sum_exact_i128_max_ok() {
    let half = i128::MAX / 2;
    let remainder = i128::MAX - half;
    let s = vec![seg(half, 1), seg(remainder, 1)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn cumulative_sum_exceeds_i128_max_rejected() {
    let half = i128::MAX / 2 + 1;
    let remainder = i128::MAX - i128::MAX / 2;
    let s = vec![seg(half, 1), seg(remainder, 1)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn cumulative_sum_overflows_many_segments() {
    let big = i128::MAX / 128;
    let s: Vec<RateSegment> = (0..256).map(|_| seg(big, 1)).collect();
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

#[test]
fn duration_at_u64_max_with_rate_one_valid() {
    let s = vec![seg(1, u64::MAX)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn duration_at_u64_max_with_large_rate_overflows() {
    let max_dur = u64::MAX as i128;
    let rate = i128::MAX / max_dur + 1;
    let s = vec![seg(rate, u64::MAX)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

// =========================================================================
// 5. Valid schedules (happy path)
// =========================================================================

#[test]
fn single_segment_valid() {
    assert_eq!(validate_rate_schedule(&[seg(1, 100)]), Ok(()));
}

#[test]
fn two_segments_valid() {
    let s = vec![seg(1, 100), seg(2, 200)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn three_segments_valid() {
    let s = vec![seg(1, 100), seg(2, 200), seg(3, 300)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn many_valid_segments_far_from_cap() {
    let s = many_small(100);
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn zero_rate_segments_valid() {
    let s = vec![seg(0, 100), seg(0, 200)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn mix_of_zero_and_positive_rates_valid() {
    let s = vec![seg(0, 100), seg(5, 200), seg(0, 50)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn large_rate_not_overflowing_valid() {
    let s = vec![seg(1_000_000, 1_000_000)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn max_rate_single_second_valid() {
    let s = vec![seg(i128::MAX, 1)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn max_duration_single_rate_one_valid() {
    let s = vec![seg(1, u64::MAX)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

// =========================================================================
// 6. Short-circuit: first violation wins
// =========================================================================

#[test]
fn too_many_segments_reported_before_other_errors() {
    let mut s: Vec<RateSegment> = (0..=MAX_RATE_SEGMENTS).map(|_| seg(1, 1)).collect();
    s[1] = seg(-1, 1);
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleTooManySegments)
    );
}

#[test]
fn zero_length_reported_before_rate_negative() {
    let s = vec![seg(-5, 0)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

// =========================================================================
// 7. i128 boundary segments
// =========================================================================

#[test]
fn product_exactly_i128_max_accepted() {
    let s = vec![seg(i128::MAX, 1)];
    assert_eq!(validate_rate_schedule(&s), Ok(()));
}

#[test]
fn product_one_over_i128_max_rejected() {
    let s = vec![seg(i128::MAX / 2 + 1, 2)];
    assert_eq!(
        validate_rate_schedule(&s),
        Err(ContractError::RateScheduleInvalid)
    );
}

// =========================================================================
// 8. Determinism (pure function)
// =========================================================================

#[test]
fn validate_is_deterministic() {
    let s = vec![seg(10, 100), seg(20, 200), seg(30, 300)];
    let a = validate_rate_schedule(&s);
    let b = validate_rate_schedule(&s);
    assert_eq!(a, b);
}

#[test]
fn validate_error_is_deterministic() {
    let s = vec![seg(-1, 100)];
    let a = validate_rate_schedule(&s);
    let b = validate_rate_schedule(&s);
    assert_eq!(a, b);
}

// =========================================================================
// 9. RateSegment construction invariants
// =========================================================================

#[test]
fn rate_segment_debug_format() {
    let s = seg(42, 100);
    let debug = format!("{:?}", s);
    assert!(debug.contains("42"));
    assert!(debug.contains("100"));
}

#[test]
fn rate_segment_equality() {
    assert_eq!(seg(1, 100), seg(1, 100));
    assert_ne!(seg(1, 100), seg(2, 100));
    assert_ne!(seg(1, 100), seg(1, 200));
}

#[test]
fn rate_segment_copy_works() {
    let a = seg(5, 50);
    let b = a;
    assert_eq!(a, b);
}