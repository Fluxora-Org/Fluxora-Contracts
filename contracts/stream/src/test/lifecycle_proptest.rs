//! Property tests for liability conservation under generated lifecycle sequences.
//!
//! This module generates bounded sequences of lifecycle operations (create, top-up,
//! withdraw, pause, resume, transfer, cancel) and verifies that liability conservation
//! holds after every operation.
//!
//! The key invariant is: pool balance == sum of all stream liabilities
//! where liability = deposited - withdrawn for each stream.
//!
//! Failing seeds are preserved as regression fixtures for reproducible debugging.

use super::common::*;
use crate::accrual;

/// Maximum number of operations in a generated sequence.
const MAX_STEPS: u32 = 50;

/// xorshift64* deterministic PRNG for reproducible sequences.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Check that liability conservation holds: pool balance equals sum of all stream liabilities.
fn check_liability_conservation(h: &Harness, seed: u64, step: u32) {
    let mut liability_total = 0i128;
    let count = h.client.stream_count();

    for id in 0..count {
        let stream = h.get(id);
        if stream.token != h.token {
            continue;
        }
        liability_total += accrual::liability(&stream).expect("liability must not overflow");
    }

    let pool = h.pool();
    assert_eq!(
        pool, liability_total,
        "seed {seed}, step {step}: liability conservation violated: pool {} != total liability {}",
        pool, liability_total
    );
}

/// Run a generated sequence of lifecycle operations from a seed.
fn run_lifecycle_sequence(seed: u64, steps: u32) {
    let h = Harness::new();
    let mut rng = Rng(seed);

    // Seed the world with one initial stream
    {
        let start = h.now() + rng.below(10 * DAY);
        let duration = DAY + rng.below(5 * DAY);
        let end = start + duration;
        // Use start as cliff for simplicity (always valid)
        let cliff = start;
        // Deposit must be >= duration to satisfy rate floor (1 stroop/second)
        // Use minimal deposit to conserve balance
        let deposit = duration as i128 * ONE;
        h.create(deposit, start, end, cliff, true, true, true);
    }

    check_liability_conservation(&h, seed, 0);

    for step in 1..=steps {
        let count = h.client.stream_count();
        if count == 0 {
            break;
        }

        let id = rng.below(count);

        // Snapshot vested before operation to check I3 (no vested regression)
        let vested_before = h.vested_snapshot();

        // Apply a random lifecycle operation
        match rng.below(10) {
            0..=3 => {
                // Withdraw
                let amount = if rng.below(2) == 0 {
                    None
                } else {
                    Some((1 + rng.below(100)) as i128 * ONE)
                };
                let _ = h.client.try_withdraw(&id, &amount);
            }
            4 => {
                // Pause
                let _ = h.client.try_pause(&id);
            }
            5 => {
                // Resume
                let _ = h.client.try_resume(&id);
            }
            6 => {
                // Cancel
                let _ = h.client.try_cancel(&id);
            }
            7 => {
                // Top-up
                let amount = (1 + rng.below(5)) as i128 * ONE;
                let _ = h.client.try_top_up(&id, &amount);
            }
            8 => {
                // Transfer recipient
                let to = if rng.below(2) == 0 {
                    h.other.clone()
                } else {
                    h.recipient.clone()
                };
                let _ = h.client.try_transfer_recipient(&id, &to);
            }
            _ => {
                // Extend TTL (maintenance operation)
                let _ = h.client.try_extend_stream_ttl(&id);
            }
        }

        // Check invariants after operation
        let label = std::format!("seed {}, step {}", seed, step);
        h.assert_no_vested_regression(&vested_before, &label);
        check_liability_conservation(&h, seed, step);

        // Advance time between operations
        h.advance(1 + rng.below(20 * DAY));
        check_liability_conservation(&h, seed, step);
    }
}

/// Get the number of test cases from environment or default.
fn test_case_count(default: u64) -> u64 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Property test: liability conservation holds across generated lifecycle sequences.
#[test]
fn liability_conservation_holds_across_lifecycle_sequences() {
    let cases = test_case_count(256);
    let steps = MAX_STEPS;

    for i in 0..cases {
        // Use a deterministic seed that varies with the case number
        let seed = 0x9E37_79B9_7F4A_7C15u64.wrapping_add(i);
        run_lifecycle_sequence(seed, steps);
    }
}

/// Longer sequences to reach deeper interaction states.
#[test]
fn liability_conservation_holds_across_long_lifecycle_sequences() {
    // Use a smaller default for long sequences to avoid excessive runtime
    let cases = test_case_count(8);
    let steps = 150;

    for i in 0..cases {
        let seed = 0xDEAD_BEEF_u64
            .wrapping_mul(i)
            .wrapping_add(0xA24B_AED4_963E_E407);
        run_lifecycle_sequence(seed, steps);
    }
}

/// Regression test for specific failing seeds.
/// Add seeds here as they are discovered to prevent regressions.
#[test]
fn regression_specific_seeds() {
    // No known failing seeds yet - this is a placeholder for future regressions
    // Format: run_lifecycle_sequence(seed, steps);
    // Example: run_lifecycle_sequence(0x123456789ABCDEF0, 42);
}
