//! Property-based tests for accrual monotonicity across rate and timestamp boundaries.
//! Issue #1534
//!
//! Exercises the core accrual math (`vested`, `withdrawable`, `refundable`,
//! `stream_time`, `elapsed`, `cliff_reached`) with randomized inputs to verify:
//!
//! 1. **Monotonicity over time** — `vested(t1) <= vested(t2)` for `t1 <= t2`.
//! 2. **Boundedness** — `0 <= vested(t) <= deposited` for all `t`.
//! 3. **Zero before cliff** — `vested(t) == 0` for `t < cliff_time`.
//! 4. **Conservation** — `vested(t) + refundable(t) == deposited`.
//! 5. **No overflow** — never panics on bounded inputs.
//! 6. **Pause monotonicity** — `vested` frozen while paused, resumes after.
//! 7. **Timestamp boundaries** — cliff and end are inclusive/exclusive correctly.
//!
//! # Rounding policy
//!
//! `vested` rounds **down** (floor). Integer division truncates in the
//! recipient's disfavour; the residue stays in the contract and is returned to
//! the sender on cancel, so the pool can never be short.
//!
//! ```text
//! deposited = 1000, duration = 3s, elapsed = 1s
//! vested = floor(1000 * 1 / 3) = 333  (not 333.33)
//!
//! deposited = 1000, duration = 1000s, elapsed = 2000s (past end)
//! vested = 1000  (saturated at deposited)
//! ```
//!
//! ```bash
//! cargo test -p fluxora-stream accrual_monotonicity -- --nocapture
//! PROPTEST_CASES=10000 cargo test -p fluxora-stream accrual_monotonicity
//! ```

extern crate std;

use fluxora_stream::{
    cliff_reached, duration, elapsed, refundable, stream_time, vested, withdrawable, Stream,
    StreamStatus,
};
use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dummy_stream(env: &Env, deposited: i128, start: u64, end: u64, cliff: u64) -> Stream {
    Stream {
        sender: Address::generate(env),
        recipient: Address::generate(env),
        token: Address::generate(env),
        deposited,
        withdrawn: 0,
        start_time: start,
        end_time: end,
        cliff_time: cliff,
        cancellable: true,
        pausable: true,
        transferable: true,
        paused_at: None,
        paused_total: 0,
        status: StreamStatus::Active,
    }
}

fn stream_params() -> impl Strategy<Value = (i128, u64, u64, u64)> {
    (1i128..1_000_000i128, 1u64..500u64).prop_flat_map(|(deposited, duration_val)| {
        let duration_val = duration_val.max(1);
        let end = duration_val;
        (Just(deposited), Just(0u64), Just(end), 0u64..=duration_val).prop_map(
            |(dep, start, e, cliff_off)| {
                let cliff = start + cliff_off.min(e - start);
                (dep, start, e, cliff)
            },
        )
    })
}

// ---------------------------------------------------------------------------
// Property 1: Monotonicity over time
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_vested_monotonic_over_time(
        (deposited, start, end, cliff) in stream_params(),
        times in proptest::collection::vec(0u64..1000u64, 2..=8),
    ) {
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let mut sorted = times;
        sorted.sort();
        let mut prev = vested(&s, sorted[0]).unwrap();
        for &t in sorted.iter().skip(1) {
            let cur = vested(&s, t).unwrap();
            prop_assert!(cur >= prev, "vested not monotonic: vested({t})={cur} < {prev} (start={start} end={end} cliff={cliff} deposited={deposited})");
            prev = cur;
        }
    }

    #[test]
    fn prop_withdrawable_monotonic_over_time(
        (deposited, start, end, cliff) in stream_params(),
        times in proptest::collection::vec(0u64..1000u64, 2..=8),
    ) {
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let mut sorted = times;
        sorted.sort();
        let mut prev = withdrawable(&s, sorted[0]).unwrap();
        for &t in sorted.iter().skip(1) {
            let cur = withdrawable(&s, t).unwrap();
            prop_assert!(cur >= prev, "withdrawable not monotonic at t={t}: {cur} < {prev}");
            prev = cur;
        }
    }
}

// ---------------------------------------------------------------------------
// Property 2: Boundedness
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_vested_bounded(
        (deposited, start, end, cliff) in stream_params(),
        t in 0u64..2000u64,
    ) {
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let v = vested(&s, t).unwrap();
        prop_assert!(v >= 0, "vested negative: {v}");
        prop_assert!(v <= deposited, "vested {v} > deposited {deposited}");
    }

    #[test]
    fn prop_bounded_at_extreme_timestamp(
        (deposited, start, end, cliff) in stream_params(),
    ) {
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let v = vested(&s, u64::MAX).unwrap();
        prop_assert!(v >= 0 && v <= deposited, "boundedness violated at u64::MAX: {v} not in [0, {deposited}]");
    }
}

// ---------------------------------------------------------------------------
// Property 3: Zero before cliff
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_zero_before_cliff(
        (deposited, start, end, cliff) in stream_params(),
        offset in 0u64..500u64,
    ) {
        prop_assume!(cliff > start, "need cliff after start");
        prop_assume!(offset < cliff - start, "offset must be before cliff");
        let t = cliff - 1 - offset;
        // Only test t that is before cliff in stream_time terms (no pause)
        prop_assume!(t < cliff);
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        // cliff_reached should be false, and vested should be 0
        prop_assert!(!cliff_reached(&s, t), "cliff should not be reached at t={t} cliff={cliff}");
        let v = vested(&s, t).unwrap();
        prop_assert_eq!(v, 0);
    }
}

// ---------------------------------------------------------------------------
// Property 4: Conservation
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_conservation(
        (deposited, start, end, cliff) in stream_params(),
        t in 0u64..2000u64,
    ) {
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let v = vested(&s, t).unwrap();
        let r = refundable(&s, t).unwrap();
        prop_assert_eq!(v + r, deposited);
    }
}

// ---------------------------------------------------------------------------
// Property 5: No overflow / panic on bounded inputs
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_no_panic(
        deposited in 0i128..=1_000_000_000i128,
        start in 0u64..10_000u64,
        end in 1u64..10_000u64,
        cliff_off in 0u64..10_000u64,
        now in 0u64..=u64::MAX,
    ) {
        // Ensure start < end for most cases, but also test degenerate
        let (start, end) = if start < end { (start, end) } else { (0, 1) };
        let cliff = start + cliff_off.min(end - start);
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let _ = vested(&s, now);
        let _ = withdrawable(&s, now);
        let _ = refundable(&s, now);
        let _ = stream_time(&s, now);
        let _ = elapsed(&s, now);
        let _ = duration(&s);
        let _ = cliff_reached(&s, now);
    }
}

// ---------------------------------------------------------------------------
// Property 6: Pause freezes accrual
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_pause_freezes_vested(
        (deposited, start, end, cliff) in stream_params(),
        pause_at in 0u64..500u64,
        after in 1u64..500u64,
    ) {
        prop_assume!(pause_at >= start && pause_at < end, "pause must be within schedule");
        let env = Env::default();
        let mut s = dummy_stream(&env, deposited, start, end, cliff);
        // Pause at pause_at
        s.paused_at = Some(pause_at);
        s.status = StreamStatus::Paused;
        // Stream clock frozen at pause_at
        let v_at_pause = vested(&s, pause_at).unwrap();
        let v_after = vested(&s, pause_at + after).unwrap();
        prop_assert_eq!(v_at_pause, v_after);

        // After accounting for paused_total, clock resumes correctly:
        // Simulate resume: paused_total absorbs the pause interval
        let pause_duration = after;
        s.paused_at = None;
        s.paused_total = pause_duration;
        s.status = StreamStatus::Active;
        let v_resumed = vested(&s, pause_at + after).unwrap();
        // vested at resumed time should equal vested at the frozen time (no progress while paused)
        prop_assert_eq!(v_resumed, v_at_pause);
    }
}

// ---------------------------------------------------------------------------
// Property 7: Cliff and end boundaries with rate-like checks
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_cliff_boundary(
        (deposited, start, end, cliff) in stream_params(),
    ) {
        prop_assume!(cliff > start && cliff < end, "need cliff strictly inside schedule");
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let v_before = vested(&s, cliff - 1).unwrap();
        let v_at = vested(&s, cliff).unwrap();
        prop_assert_eq!(v_before, 0);
        prop_assert!(v_at >= 0, "vested at cliff must be >=0, got {v_at}");
        // At cliff, vested should be deposited * (cliff - start) / duration floored
        let expected = deposited * (cliff - start) as i128 / (end - start) as i128;
        prop_assert_eq!(v_at, expected);
    }

    #[test]
    fn prop_end_boundary_saturation(
        (deposited, start, end, cliff) in stream_params(),
    ) {
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let v_at_end = vested(&s, end).unwrap();
        let v_after = vested(&s, end + 1000).unwrap();
        prop_assert_eq!(v_at_end, deposited);
        prop_assert_eq!(v_after, deposited);
    }
}

// ---------------------------------------------------------------------------
// Property 8: Rounding is always down
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_rounding_down(
        (deposited, start, end, cliff) in stream_params(),
        t in 0u64..2000u64,
    ) {
        let env = Env::default();
        let s = dummy_stream(&env, deposited, start, end, cliff);
        let v = vested(&s, t).unwrap();
        // If cliff not reached, vested is 0 regardless of arithmetic
        if !cliff_reached(&s, t) {
            prop_assert_eq!(v, 0);
            return Ok(());
        }
        let dur = duration(&s);
        if dur == 0 {
            prop_assert_eq!(v, deposited);
            return Ok(());
        }
        let el = elapsed(&s, t);
        // Exact rational would be deposited * el / dur; integer division truncates down
        // So v * dur <= deposited * el < (v+1) * dur  (unless saturated)
        if el < dur {
            let lhs = v.checked_mul(dur as i128).unwrap();
            let rhs = deposited.checked_mul(el as i128).unwrap();
            prop_assert!(lhs <= rhs, "rounding up: v*dur {lhs} > deposited*el {rhs}");
            // Next integer would exceed
            let next = (v + 1).checked_mul(dur as i128).unwrap();
            prop_assert!(next > rhs, "not floored: (v+1)*dur {next} <= deposited*el {rhs}");
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic regression tests
// ---------------------------------------------------------------------------

#[test]
fn regression_linear_monotonicity_second_by_second() {
    let env = Env::default();
    let s = dummy_stream(&env, 1000, 0, 1000, 0);
    let mut prev = -1i128;
    for t in 0..=1100u64 {
        let v = vested(&s, t).unwrap();
        assert!(v >= prev, "vested not monotonic at t={t}: {v} < {prev}");
        prev = v;
    }
}

#[test]
fn regression_cliff_at_midpoint() {
    let env = Env::default();
    let s = dummy_stream(&env, 2000, 0, 1000, 500);
    let v_before = vested(&s, 499).unwrap();
    let v_at = vested(&s, 500).unwrap();
    let v_after = vested(&s, 750).unwrap();
    let v_end = vested(&s, 1000).unwrap();
    assert_eq!(v_before, 0);
    assert_eq!(v_at, 1000); // 2000*500/1000
    assert_eq!(v_after, 1500); // 2000*750/1000
    assert_eq!(v_end, 2000);
}

#[test]
fn regression_conservation_at_cliff() {
    let env = Env::default();
    let s = dummy_stream(&env, 1000, 0, 1000, 500);
    for t in [0u64, 499, 500, 750, 1000, 1500] {
        let v = vested(&s, t).unwrap();
        let r = refundable(&s, t).unwrap();
        assert_eq!(v + r, 1000, "conservation failed at t={t}");
    }
}

#[test]
fn regression_overflow_no_panic() {
    let env = Env::default();
    let s = dummy_stream(&env, 1_000_000, 0, 100, 0);
    let v = vested(&s, 2).unwrap();
    assert!(v >= 0 && v <= 1_000_000);
}

#[test]
fn regression_extreme_timestamp_no_panic() {
    let env = Env::default();
    let s = dummy_stream(&env, 1000, 0, 1000, 0);
    let v = vested(&s, u64::MAX).unwrap();
    assert!((0..=1000).contains(&v));
}

#[test]
fn regression_pause_freeze() {
    let env = Env::default();
    let mut s = dummy_stream(&env, 1000, 0, 1000, 0);
    s.paused_at = Some(400);
    s.status = StreamStatus::Paused;
    let v1 = vested(&s, 400).unwrap();
    let v2 = vested(&s, 800).unwrap();
    assert_eq!(v1, v2, "vested should be frozen while paused");
}

#[test]
fn regression_rounding_down_example() {
    // 1000 * 1 / 3 = 333.33 -> 333
    let env = Env::default();
    let s = dummy_stream(&env, 1000, 0, 3, 0);
    assert_eq!(vested(&s, 1).unwrap(), 333);
    assert_eq!(vested(&s, 2).unwrap(), 666);
    assert_eq!(vested(&s, 3).unwrap(), 1000);
}

#[test]
fn regression_zero_deposited() {
    let env = Env::default();
    let s = dummy_stream(&env, 0, 0, 1000, 0);
    for t in [0u64, 500, 1000, 1500] {
        assert_eq!(vested(&s, t).unwrap(), 0);
    }
}
