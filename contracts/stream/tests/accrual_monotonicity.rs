//! Property-based tests for accrual monotonicity across rate and timestamp boundaries.
//!
//! Exercises the core accrual math (`calculate_accrued_amount` and
//! `calculate_accrued_amount_checkpointed`) with randomized inputs to verify:
//!
//! 1. **Monotonicity over time** — `accrued(t1) <= accrued(t2)` for all `t1 <= t2`.
//! 2. **Boundedness** — `0 <= accrued(t) <= deposit_amount` for all `t`.
//! 3. **Zero before cliff** — `accrued(t) == 0` for all `t < cliff_time`.
//! 4. **Checkpoint conservation** — `accrued(checkpointed_at) == checkpointed_amount`
//!    after a rate decrease.
//! 5. **No overflow** — the function never panics on any bounded input.
//! 6. **All stream kinds** — `Linear`, `CliffOnly`, and `CliffSlope`.
//! 7. **Rate change monotonicity** — withdrawable never decreases after a checkpoint.
//!
//! # Rounding policy
//!
//! All arithmetic is exact integer multiplication. `rate_per_second` is an integer
//! expressing tokens per full second. Any precision loss occurs *before* this
//! function when an external fractional rate is floored to integer tokens/sec.
//! Within the core math, the operation is exact — no rounding occurs.
//!
//! ```text
//! Example: rate = 3 tokens/s, elapsed = 5s
//! accrued = 3 * 5 = 15 (exact, no rounding)
//!
//! Example: rate = 1 token/s, deposit = 1000, elapsed = 2000s
//! accrued = min(1 * 2000, 1000) = 1000 (saturated at deposit)
//! ```
//!
//! For multi-epoch accrual (after rate changes), the checkpoint mechanism
//! preserves already-earned amounts:
//!
//! ```text
//! Epoch 1: rate=10, t=[0..50) → accrued = 10 * 50 = 500
//! Rate decrease at t=50: checkpointed_amount=500, new_rate=5
//! Epoch 2: rate=5, t=[50..100] → added = 5 * 50 = 250
//! Total: min(500 + 250, deposit) = 750
//! ```
//!
//! Run with:
//!
//! ```bash
//! cargo test -p fluxora-stream accrual_monotonicity -- --nocapture
//! ```
//!
//! For deeper local coverage:
//!
//! ```bash
//! PROPTEST_CASES=10000 cargo test -p fluxora-stream accrual_monotonicity
//! ```

extern crate std;

use fluxora_stream::accrual::CheckpointState;
use fluxora_stream::accrual::{calculate_accrued_amount, calculate_accrued_amount_checkpointed};
use fluxora_stream::StreamKind;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

/// Generates `(start, cliff, end, rate, deposit)` for `Linear` streams.
///
/// Invariants:
/// - `start <= cliff <= end`
/// - `deposit >= rate * (end - start)` (deposit covers full stream)
/// - All values bounded to avoid i128 overflow during strategy generation.
fn linear_stream_params() -> impl Strategy<Value = (u64, u64, u64, i128, i128)> {
    (1u64..500u64, 0u64..500u64, 1i128..200i128).prop_flat_map(|(duration, cliff_offset, rate)| {
        let duration = duration.max(1);
        let cliff = cliff_offset.min(duration);
        let end = duration;
        let min_deposit = rate.saturating_mul(duration as i128);
        let max_deposit = min_deposit.saturating_add(min_deposit.max(1));
        (
            Just(rate),
            Just(cliff),
            Just(end),
            min_deposit..=max_deposit,
        )
            .prop_map(move |(r, c, e, d)| (0u64, c, e, r, d))
    })
}

/// Generates `(cliff, end, deposit)` for `CliffOnly` streams.
fn cliff_only_params() -> impl Strategy<Value = (u64, u64, u64, i128, i128)> {
    (10u64..500u64, 1i128..10_000i128).prop_map(|(duration, deposit)| {
        let cliff = duration;
        (0u64, cliff, duration, 0i128, deposit)
    })
}

/// Generates `(start, cliff, end, rate, deposit)` for `CliffSlope` streams.
///
/// CliffSlope accrues linearly from `cliff_time` (not `start_time`), so the
/// deposit must cover `rate * (end - cliff)`.
fn cliff_slope_params() -> impl Strategy<Value = (u64, u64, u64, i128, i128)> {
    (1u64..500u64, 0u64..500u64, 1i128..200i128).prop_flat_map(|(duration, cliff_offset, rate)| {
        let duration = duration.max(1);
        let cliff = cliff_offset.min(duration);
        let end = duration;
        let stream_duration = end.saturating_sub(cliff).max(1);
        let min_deposit = rate.saturating_mul(stream_duration as i128);
        let max_deposit = min_deposit.saturating_add(min_deposit.max(1));
        (
            Just(rate),
            Just(cliff),
            Just(end),
            min_deposit..=max_deposit,
        )
            .prop_map(move |(r, c, e, d)| (0u64, c, e, r, d))
    })
}

/// All stream kinds combined.
fn all_stream_params() -> impl Strategy<Value = (u64, u64, u64, i128, i128, StreamKind)> {
    prop_oneof![
        linear_stream_params().prop_map(|(s, c, e, r, d)| (s, c, e, r, d, StreamKind::Linear)),
        cliff_only_params().prop_map(|(s, c, e, r, d)| (s, c, e, r, d, StreamKind::CliffOnly)),
        cliff_slope_params().prop_map(|(s, c, e, r, d)| (s, c, e, r, d, StreamKind::CliffSlope)),
    ]
}

/// Sorted vector of 2–8 timestamps bounded by `[0, max_time]`.
fn time_sequence(max_time: u64) -> impl Strategy<Value = std::vec::Vec<u64>> {
    proptest::collection::vec(0u64..=max_time, 2..=8).prop_map(|mut v| {
        v.sort();
        v.dedup();
        v
    })
}

/// Stream params that allow a rate decrease (rate >= 2).
fn rate_decrease_params() -> impl Strategy<Value = (u64, u64, u64, i128, i128)> {
    (2i128..200i128, 2u64..500u64).prop_flat_map(|(rate, duration)| {
        let duration = duration.max(1);
        let cliff = 0u64;
        let min_deposit = rate.saturating_mul(duration as i128);
        let max_deposit = min_deposit.saturating_add(rate * 10);
        (
            Just(rate),
            Just(cliff),
            Just(duration),
            min_deposit..=max_deposit,
        )
            .prop_map(|(r, c, e, d)| (0u64, c, e, r, d))
    })
}

/// Randomized rate-decrease steps: `(advance_by, new_rate_fraction)`.
fn rate_decrease_steps() -> impl Strategy<Value = std::vec::Vec<(u64, i128)>> {
    proptest::collection::vec((0u64..100u64, 1i128..100i128), 1..=6)
}

// ---------------------------------------------------------------------------
// Property 1: Monotonicity over time for all stream kinds
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// For any valid stream configuration, accrued(t) is monotonically
    /// non-decreasing across any sorted time sequence.
    #[test]
    fn prop_accrual_monotonic_over_time(
        (start, cliff, end, rate, deposit, kind) in all_stream_params(),
        times in time_sequence(1_000),
    ) {
        let state = CheckpointState {
            checkpointed_amount: 0,
            checkpointed_at: start,
            cliff_time: cliff,
            end_time: end,
            deposit_amount: deposit,
            kind,
        };

        let mut prev = calculate_accrued_amount_checkpointed(state, rate, times[0]);
        for &t in times.iter().skip(1) {
            let now = calculate_accrued_amount_checkpointed(state, rate, t);
            prop_assert!(
                now >= prev,
                "monotonicity violated for kind={kind:?} ({start},{cliff},{end},{rate},{deposit}): \
                 accrued({t})={now} < accrued(prev_t)={prev}"
            );
            prev = now;
        }
    }

    /// Monotonicity holds second-by-second across the cliff boundary for Linear streams.
    #[test]
    fn prop_monotonicity_across_cliff_boundary(
        (start, cliff, end, rate, deposit) in linear_stream_params(),
    ) {
        // Sweep from cliff-5 to cliff+5 (clamped to valid range)
        let t0 = cliff.saturating_sub(5);
        let t1 = (cliff + 5).min(end);

        if t0 >= t1 {
            return Ok(());
        }

        let mut prev = calculate_accrued_amount(start, cliff, end, rate, deposit, t0);
        for t in (t0 + 1)..=t1 {
            let now = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
            prop_assert!(
                now >= prev,
                "cliff boundary monotonicity violated at t={t}: {now} < {prev} \
                 for stream ({start},{cliff},{end},{rate},{deposit})"
            );
            prev = now;
        }
    }

    /// Monotonicity holds second-by-second across the end_time boundary.
    #[test]
    fn prop_monotonicity_across_end_boundary(
        (start, cliff, end, rate, deposit) in linear_stream_params(),
    ) {
        let t0 = end.saturating_sub(5);
        let t1 = end + 5;

        let mut prev = calculate_accrued_amount(start, cliff, end, rate, deposit, t0);
        for t in (t0 + 1)..=t1 {
            let now = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
            prop_assert!(
                now >= prev,
                "end boundary monotonicity violated at t={t}: {now} < {prev} \
                 for stream ({start},{cliff},{end},{rate},{deposit})"
            );
            prev = now;
        }
    }
}

// ---------------------------------------------------------------------------
// Property 2: Boundedness — result always in [0, deposit_amount]
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// For any valid stream and any timestamp, accrued(t) is in [0, deposit_amount].
    #[test]
    fn prop_accrual_bounded_by_deposit(
        (start, cliff, end, rate, deposit, kind) in all_stream_params(),
        t in 0u64..1_500u64,
    ) {
        let state = CheckpointState {
            checkpointed_amount: 0,
            checkpointed_at: start,
            cliff_time: cliff,
            end_time: end,
            deposit_amount: deposit,
            kind,
        };

        let accrued = calculate_accrued_amount_checkpointed(state, rate, t);
        prop_assert!(
            accrued >= 0,
            "negative accrual for kind={kind:?}: accrued={accrued} at t={t} \
             for ({start},{cliff},{end},{rate},{deposit})"
        );
        prop_assert!(
            accrued <= deposit,
            "accrual exceeds deposit for kind={kind:?}: accrued={accrued} > deposit={deposit} \
             at t={t} for ({start},{cliff},{end},{rate},{deposit})"
        );
    }

    /// Boundedness holds at extreme timestamps (u64::MAX).
    #[test]
    fn prop_bounded_at_extreme_timestamp(
        (start, cliff, end, rate, deposit, kind) in all_stream_params(),
    ) {
        let state = CheckpointState {
            checkpointed_amount: 0,
            checkpointed_at: start,
            cliff_time: cliff,
            end_time: end,
            deposit_amount: deposit,
            kind,
        };

        let accrued = calculate_accrued_amount_checkpointed(state, rate, u64::MAX);
        prop_assert!(
            accrued >= 0 && accrued <= deposit,
            "boundedness violated at u64::MAX for kind={kind:?}: accrued={accrued}, deposit={deposit}"
        );
    }
}

// ---------------------------------------------------------------------------
// Property 3: Zero before cliff
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// For any stream with a non-zero cliff, accrued(t) == 0 for all t < cliff_time.
    #[test]
    fn prop_zero_before_cliff(
        (start, cliff, end, rate, deposit, kind) in all_stream_params(),
        offset in 0u64..500u64,
    ) {
        prop_assume!(cliff > 0, "need non-zero cliff to test pre-cliff window");
        prop_assume!(offset < cliff, "offset must be before cliff");

        let state = CheckpointState {
            checkpointed_amount: 0,
            checkpointed_at: start,
            cliff_time: cliff,
            end_time: end,
            deposit_amount: deposit,
            kind,
        };

        let t = cliff - 1 - offset;
        let accrued = calculate_accrued_amount_checkpointed(state, rate, t);
        prop_assert_eq!(accrued, 0);
    }
}

// ---------------------------------------------------------------------------
// Property 4: No overflow / panic on any bounded input
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The function never panics for any combination of bounded inputs.
    #[test]
    fn prop_no_panic(
        deposit in 0i128..=1_000_000_000i128,
        rate in 0i128..=10_000_000i128,
        cliff in 0u64..=10_000u64,
        end in 1u64..=10_000u64,
        now in 0u64..=u64::MAX,
        checkpointed_amount in 0i128..=1_000_000_000i128,
        checkpointed_at in 0u64..=10_000u64,
    ) {
        let state = CheckpointState {
            checkpointed_amount: checkpointed_amount.min(deposit),
            checkpointed_at,
            cliff_time: cliff,
            end_time: end,
            deposit_amount: deposit,
            kind: StreamKind::Linear,
        };

        // Must not panic under any circumstances
        let _ = calculate_accrued_amount_checkpointed(state, rate, now);

        // Also test the convenience wrapper
        let _ = calculate_accrued_amount(0, cliff, end, rate, deposit, now);
    }
}

// ---------------------------------------------------------------------------
// Property 5: Checkpoint conservation — accrued(checkpointed_at) == checkpointed_amount
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// After a rate decrease, calling calculate_accrued_amount_checkpointed
    /// at the checkpointed_at timestamp returns exactly checkpointed_amount.
    #[test]
    fn prop_checkpoint_conservation(
        (start, cliff, end, old_rate, deposit) in rate_decrease_params(),
        advance in 1u64..=200u64,
    ) {
        prop_assume!(end > 1, "need some stream duration");
        let decrease_at = advance.min(end - 1).max(start.max(cliff));

        // Compute accrued at the decrease point under the old rate
        let accrued_at_decrease = calculate_accrued_amount(
            start, cliff, end, old_rate, deposit, decrease_at,
        );

        let new_rate = (old_rate / 2).max(1);

        // Build checkpoint state as the contract would after decrease_rate_per_second
        let state = CheckpointState {
            checkpointed_amount: accrued_at_decrease,
            checkpointed_at: decrease_at,
            cliff_time: cliff,
            end_time: end,
            deposit_amount: deposit,
            kind: StreamKind::Linear,
        };

        // At the checkpoint timestamp, accrued must equal checkpointed_amount
        let accrued_at_checkpoint =
            calculate_accrued_amount_checkpointed(state, new_rate, decrease_at);
        prop_assert_eq!(accrued_at_checkpoint, accrued_at_decrease);

        // At any future time, accrued must be >= checkpointed_amount
        for t in [decrease_at, decrease_at + 1, end, end + 100] {
            let accrued = calculate_accrued_amount_checkpointed(state, new_rate, t);
            prop_assert!(
                accrued >= accrued_at_decrease,
                "accrued({t})={accrued} < checkpointed_amount={accrued_at_decrease}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 6: Rate-change monotonicity — withdrawable never decreases
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// After a rate decrease checkpoint, the withdrawable amount
    /// (accrued - withdrawn) is monotonically non-decreasing.
    #[test]
    fn prop_rate_decrease_preserves_withdrawable(
        (start, cliff, end, initial_rate, deposit) in rate_decrease_params(),
        steps in rate_decrease_steps(),
    ) {
        prop_assume!(end > 1);

        let mut current_rate = initial_rate;
        let mut checkpoint_amount = 0i128;
        let mut checkpoint_at = start;
        let mut current_time = start;
        let mut previous_withdrawable = 0i128;

        for (advance, new_rate_factor) in steps {
            current_time = current_time.saturating_add(advance).min(end);

            // Build state with current checkpoint
            let state = CheckpointState {
                checkpointed_amount: checkpoint_amount,
                checkpointed_at: checkpoint_at,
                cliff_time: cliff,
                end_time: end,
                deposit_amount: deposit,
                kind: StreamKind::Linear,
            };

            // Compute accrued at current time
            let accrued_now =
                calculate_accrued_amount_checkpointed(state, current_rate, current_time);
            let withdrawable_now = accrued_now; // No withdrawals in this test

            // Withdrawable must be >= previous
            prop_assert!(
                withdrawable_now >= previous_withdrawable,
                "withdrawable decreased: {withdrawable_now} < {previous_withdrawable} \
                 at t={current_time}, rate={current_rate}"
            );

            // Simulate rate decrease
            let new_rate = new_rate_factor.min(current_rate - 1).max(1);
            if new_rate < current_rate && current_time < end {
                // Checkpoint: lock in current accrued
                checkpoint_amount = accrued_now;
                checkpoint_at = current_time;
                current_rate = new_rate;

                // Verify same-timestamp withdrawable is preserved after checkpoint
                let state_after = CheckpointState {
                    checkpointed_amount: checkpoint_amount,
                    checkpointed_at: checkpoint_at,
                    cliff_time: cliff,
                    end_time: end,
                    deposit_amount: deposit,
                    kind: StreamKind::Linear,
                };
                let accrued_after_checkpoint =
                    calculate_accrued_amount_checkpointed(state_after, current_rate, current_time);
                prop_assert!(
                    accrued_after_checkpoint >= accrued_now,
                    "same-timestamp accrued decreased after checkpoint: {accrued_after_checkpoint} < {accrued_now}"
                );
            }

            previous_withdrawable = previous_withdrawable.max(withdrawable_now);
        }
    }
}

// ---------------------------------------------------------------------------
// Property 7: CliffOnly streams — zero before cliff, deposit after
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// CliffOnly: accrued is 0 before cliff, deposit_amount at/after cliff.
    #[test]
    fn prop_cliff_only_boundary(
        cliff in 1u64..500u64,
        deposit in 1i128..10_000i128,
        t in 0u64..1_000u64,
    ) {
        let state = CheckpointState {
            checkpointed_amount: 0,
            checkpointed_at: 0,
            cliff_time: cliff,
            end_time: cliff + 100,
            deposit_amount: deposit,
            kind: StreamKind::CliffOnly,
        };

        let accrued = calculate_accrued_amount_checkpointed(state, 0, t);
        if t < cliff {
            prop_assert_eq!(accrued, 0);
        } else {
            prop_assert_eq!(accrued, deposit);
        }
    }
}

// ---------------------------------------------------------------------------
// Property 8: CliffSlope — monotonic from cliff_time
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// CliffSlope: zero before cliff, linear from cliff_time onward, capped at deposit.
    #[test]
    fn prop_cliff_slope_monotonic(
        cliff in 0u64..250u64,
        end in 1u64..500u64,
        rate in 1i128..200i128,
        deposit in 1i128..10_000i128,
        times in time_sequence(800),
    ) {
        prop_assume!(end > cliff, "end must be after cliff");

        let state = CheckpointState {
            checkpointed_amount: 0,
            checkpointed_at: 0,
            cliff_time: cliff,
            end_time: end,
            deposit_amount: deposit,
            kind: StreamKind::CliffSlope,
        };

        let mut prev = calculate_accrued_amount_checkpointed(state, rate, times[0]);
        for &t in times.iter().skip(1) {
            let now = calculate_accrued_amount_checkpointed(state, rate, t);

            // Monotonicity
            prop_assert!(
                now >= prev,
                "CliffSlope monotonicity violated at t={t}: {now} < {prev}"
            );

            // Boundedness
            prop_assert!(
                now >= 0 && now <= deposit,
                "CliffSlope boundedness violated at t={t}: {now} not in [0, {deposit}]"
            );

            // Zero before cliff
            if t < cliff {
                prop_assert_eq!(now, 0);
            }

            prev = now;
        }
    }
}

// ---------------------------------------------------------------------------
// Property 9: Negative rate returns zero (Linear kind)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Negative rate must always return 0 for Linear streams.
    #[test]
    fn prop_negative_rate_returns_zero(
        rate in -1_000_000i128..=-1i128,
        t in 0u64..1_000u64,
    ) {
        let accrued = calculate_accrued_amount(0, 0, 1000, rate, 1000, t);
        prop_assert_eq!(accrued, 0);
    }
}

// ---------------------------------------------------------------------------
// Deterministic regression tests — one per boundary type
// ---------------------------------------------------------------------------

/// Standard linear stream: monotonicity at every second.
#[test]
fn regression_linear_monotonicity_second_by_second() {
    let (start, cliff, end, rate, deposit) = (0u64, 0u64, 1000u64, 1i128, 1000i128);
    let mut prev = -1i128;
    for t in 0..=1100u64 {
        let now = calculate_accrued_amount(start, cliff, end, rate, deposit, t);
        assert!(now >= prev, "violation at t={t}: {now} < {prev}");
        prev = now;
    }
}

/// Cliff at midpoint: zero before cliff, linear after.
#[test]
fn regression_cliff_at_midpoint() {
    let accrued_before = calculate_accrued_amount(0, 500, 1000, 2, 2000, 499);
    let accrued_at = calculate_accrued_amount(0, 500, 1000, 2, 2000, 500);
    let accrued_after = calculate_accrued_amount(0, 500, 1000, 2, 2000, 750);
    let accrued_end = calculate_accrued_amount(0, 500, 1000, 2, 2000, 1000);

    assert_eq!(accrued_before, 0, "must be 0 before cliff");
    assert_eq!(accrued_at, 1000, "at cliff: rate * (cliff - start)");
    assert_eq!(accrued_after, 1500, "midway after cliff");
    assert_eq!(accrued_end, 2000, "at end: deposit cap");
    assert!(accrued_before <= accrued_at);
    assert!(accrued_at <= accrued_after);
    assert!(accrued_after <= accrued_end);
}

/// High rate: deposit is binding cap.
#[test]
fn regression_high_rate_deposit_cap() {
    let accrued = calculate_accrued_amount(0, 0, 100, 1000, 500, 100);
    assert_eq!(
        accrued, 500,
        "must cap at deposit when rate * duration > deposit"
    );
}

/// Checkpoint conservation: same-timestamp accrued equals checkpointed_amount.
#[test]
fn regression_checkpoint_conservation() {
    let deposit = 1000i128;
    let rate = 10i128;
    let decrease_at = 50u64;

    let accrued_before = calculate_accrued_amount(0, 0, 100, rate, deposit, decrease_at);
    assert_eq!(accrued_before, 500);

    let new_rate = 5i128;
    let state = CheckpointState {
        checkpointed_amount: accrued_before,
        checkpointed_at: decrease_at,
        cliff_time: 0,
        end_time: 100,
        deposit_amount: deposit,
        kind: StreamKind::Linear,
    };

    let accrued_at_checkpoint = calculate_accrued_amount_checkpointed(state, new_rate, decrease_at);
    assert_eq!(
        accrued_at_checkpoint, 500,
        "must equal checkpointed_amount at checkpoint timestamp"
    );

    // Future time must be >= checkpointed_amount
    let accrued_future = calculate_accrued_amount_checkpointed(state, new_rate, 100);
    assert!(accrued_future >= 500);
}

/// Overflow protection: large rate * elapsed returns deposit (no panic).
#[test]
fn regression_overflow_no_panic() {
    let deposit = 1_000_000i128;
    let rate = i128::MAX / 2 + 1;
    let state = CheckpointState {
        checkpointed_amount: 0,
        checkpointed_at: 0,
        cliff_time: 0,
        end_time: 100,
        deposit_amount: deposit,
        kind: StreamKind::Linear,
    };
    // elapsed=2, rate*elapsed overflows → must return deposit, no panic
    let accrued = calculate_accrued_amount_checkpointed(state, rate, 2);
    assert_eq!(accrued, deposit);
}

/// u64::MAX timestamp: no panic, result bounded.
#[test]
fn regression_extreme_timestamp_no_panic() {
    let accrued = calculate_accrued_amount(0, 0, 1000, 1, 1000, u64::MAX);
    assert!((0..=1000).contains(&accrued));
}

/// CliffOnly: zero before cliff, deposit after.
#[test]
fn regression_cliff_only_boundary() {
    let state = CheckpointState {
        checkpointed_amount: 0,
        checkpointed_at: 0,
        cliff_time: 50,
        end_time: 150,
        deposit_amount: 1000,
        kind: StreamKind::CliffOnly,
    };
    assert_eq!(calculate_accrued_amount_checkpointed(state, 0, 49), 0);
    assert_eq!(calculate_accrued_amount_checkpointed(state, 0, 50), 1000);
    assert_eq!(calculate_accrued_amount_checkpointed(state, 0, 1000), 1000);
}

/// CliffSlope: linear from cliff, capped at deposit.
#[test]
fn regression_cliff_slope_boundary() {
    let state = CheckpointState {
        checkpointed_amount: 0,
        checkpointed_at: 0,
        cliff_time: 100,
        end_time: 200,
        deposit_amount: 500,
        kind: StreamKind::CliffSlope,
    };
    assert_eq!(calculate_accrued_amount_checkpointed(state, 5, 99), 0);
    assert_eq!(calculate_accrued_amount_checkpointed(state, 5, 100), 0);
    assert_eq!(calculate_accrued_amount_checkpointed(state, 5, 150), 250);
    assert_eq!(calculate_accrued_amount_checkpointed(state, 5, 200), 500);
    assert_eq!(calculate_accrued_amount_checkpointed(state, 5, 300), 500);
}

/// Negative rate returns zero.
#[test]
fn regression_negative_rate_returns_zero() {
    assert_eq!(calculate_accrued_amount(0, 0, 1000, -1, 1000, 500), 0);
    assert_eq!(calculate_accrued_amount(0, 0, 1000, -100, 1000, 500), 0);
}

/// Zero deposit always returns zero.
#[test]
fn regression_zero_deposit_returns_zero() {
    assert_eq!(calculate_accrued_amount(0, 0, 1000, 5, 0, 500), 0);
    assert_eq!(calculate_accrued_amount(0, 500, 1000, 5, 0, 500), 0);
}

/// Zero rate always returns zero.
#[test]
fn regression_zero_rate_returns_zero() {
    assert_eq!(calculate_accrued_amount(0, 0, 1000, 0, 1000, 500), 0);
    assert_eq!(calculate_accrued_amount(0, 500, 1000, 0, 1000, 1000), 0);
}
