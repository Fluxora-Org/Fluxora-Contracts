//! Stage 3 — the pool invariant under randomized operation sequences.
//!
//! Individual tests assert the pool invariant after the operations they perform,
//! but they only cover sequences somebody thought to write down. This file
//! drives long random sequences through the real contract and re-checks every
//! invariant after **every single operation**, which is where unforeseen
//! interactions between pause, cancel, top-up and transfer would surface.
//!
//! Randomness comes from a small deterministic PRNG rather than `rand`, so a
//! failure is reproducible from its seed alone: the failure message prints the
//! seed and the step, and re-running that seed replays the exact sequence.

use super::common::*;
use crate::{accrual, StreamStatus};

/// xorshift64*. Deterministic, seedable, and good enough to shuffle an
/// operation schedule.
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

/// Everything that must hold after every operation, for every stream.
fn check_all_invariants(h: &Harness, seed: u64, step: u32) {
    let ctx = || std::format!("seed {seed}, step {step}");

    let mut liability_total = 0i128;
    let now = h.now();

    for id in 0..h.client.stream_count() {
        let s = h.get(id);

        // Accounting can never go backwards or overdraw.
        assert!(
            s.withdrawn >= 0,
            "{}: stream {id} has negative withdrawn",
            ctx()
        );
        assert!(
            s.deposited >= 0,
            "{}: stream {id} has negative deposit",
            ctx()
        );
        assert!(
            s.withdrawn <= s.deposited,
            "{}: stream {id} withdrew {} of {} deposited",
            ctx(),
            s.withdrawn,
            s.deposited,
        );

        // The recipient can never have been paid more than they earned.
        let vested = h.client.vested_of(&id);
        assert!(
            vested <= s.deposited,
            "{}: stream {id} vested {vested} > deposited {}",
            ctx(),
            s.deposited,
        );
        assert!(
            s.withdrawn <= vested,
            "{}: stream {id} withdrew {} but only {vested} vested",
            ctx(),
            s.withdrawn,
        );

        // Conservation: earned plus refundable is exactly the deposit.
        assert_eq!(
            vested + h.client.refundable_of(&id),
            s.deposited,
            "{}: stream {id} broke conservation",
            ctx(),
        );

        // The views must agree with each other.
        assert_eq!(
            h.client.withdrawable_of(&id),
            vested - s.withdrawn,
            "{}: stream {id} withdrawable disagrees with vested - withdrawn",
            ctx(),
        );

        // Schedule sanity.
        assert!(
            s.end_time >= s.start_time,
            "{}: stream {id} has an inverted schedule",
            ctx(),
        );

        // Status and pause state must not contradict each other.
        match s.status {
            StreamStatus::Paused => assert!(
                s.paused_at.is_some(),
                "{}: stream {id} is Paused with no freeze point",
                ctx(),
            ),
            _ => assert!(
                s.paused_at.is_none(),
                "{}: stream {id} is {:?} but still frozen",
                ctx(),
                s.status,
            ),
        }

        // A frozen stream must not be accruing.
        if let Some(paused_at) = s.paused_at {
            assert!(
                accrual::stream_time(&s, now) <= paused_at,
                "{}: stream {id} advanced its clock while paused",
                ctx(),
            );
        }

        liability_total += accrual::liability(&s).expect("liability overflow");
    }

    // **The pool invariant.** Every unwithdrawn stroop owed to a recipient is
    // actually sitting in the contract.
    assert_eq!(
        h.pool(),
        liability_total,
        "{}: pooled balance {} != outstanding liability {liability_total}",
        ctx(),
        h.pool(),
    );
}

/// No value may be created or destroyed: every token that entered the contract
/// either left to a named party or is still pooled.
fn check_conservation_of_tokens(h: &Harness, seed: u64) {
    // The harness mints 1,000,000 to `sender` and 1,000,000 to `other`.
    let circulating = h.balance(&h.sender) + h.balance(&h.recipient) + h.balance(&h.other);
    assert_eq!(
        circulating + h.pool(),
        2_000_000 * ONE,
        "seed {seed}: tokens were created or destroyed",
    );
}

fn run_sequence(seed: u64, steps: u32) {
    let h = Harness::new();
    let mut rng = Rng(seed);

    // Seed the world with a handful of streams with varied shapes.
    for i in 0..4u64 {
        let start = h.now() + rng.below(10 * DAY);
        let duration = 10 * DAY + rng.below(200 * DAY);
        let cliff = start + rng.below(duration);
        let deposit = (1 + rng.below(1_000)) as i128 * ONE;
        h.create(
            deposit,
            start,
            start + duration,
            cliff,
            i % 2 == 0,
            i % 3 != 0,
            i % 2 == 1,
        );
    }
    check_all_invariants(&h, seed, 0);

    for step in 1..=steps {
        let count = h.client.stream_count();
        let id = rng.below(count);

        // Invariant I3: no call may reduce vested(t) at a fixed t. Snapshot
        // before the operation; the clock does not move until after the check.
        let vested_before = h.vested_snapshot();

        // Every call is a `try_` call: many will legitimately be rejected
        // (paused twice, cancelling a non-cancellable stream, withdrawing
        // nothing). A rejection must leave state untouched, which the
        // invariant check after it confirms.
        match rng.below(10) {
            0 => {
                let start = h.now();
                let duration = 10 * DAY + rng.below(100 * DAY);
                let deposit = (1 + rng.below(500)) as i128 * ONE;
                let _ = h.client.try_create_stream(
                    &h.sender,
                    &h.recipient,
                    &h.token,
                    &deposit,
                    &start,
                    &(start + duration),
                    &(start + rng.below(duration)),
                    &true,
                    &true,
                    &true,
                );
            }
            1..=3 => {
                let amount = if rng.below(2) == 0 {
                    None
                } else {
                    Some((1 + rng.below(100)) as i128 * ONE)
                };
                let _ = h.client.try_withdraw(&id, &amount);
            }
            4 => {
                let _ = h.client.try_pause(&id);
            }
            5 => {
                let _ = h.client.try_resume(&id);
            }
            6 => {
                let _ = h.client.try_cancel(&id);
            }
            7 => {
                let amount = (1 + rng.below(200)) as i128 * ONE;
                let _ = h.client.try_top_up(&id, &amount);
            }
            8 => {
                let to = if rng.below(2) == 0 {
                    h.other.clone()
                } else {
                    h.recipient.clone()
                };
                let _ = h.client.try_transfer_recipient(&id, &to);
            }
            _ => {
                let _ = h.client.try_extend_stream_ttl(&id);
            }
        }

        h.assert_no_vested_regression(&vested_before, &std::format!("seed {seed}, step {step}"));
        check_all_invariants(&h, seed, step);

        // Time moves between operations, sometimes a lot.
        h.advance(1 + rng.below(20 * DAY));
        check_all_invariants(&h, seed, step);
    }

    check_conservation_of_tokens(&h, seed);
}

/// How many seeds and how many steps per seed.
///
/// Overridable so CI can fuzz far deeper than a local `cargo test` should.
/// `FLUXORA_FUZZ_SEEDS` / `FLUXORA_FUZZ_STEPS` are read once per test.
fn fuzz_budget(default_seeds: u64, default_steps: u32) -> (u64, u32) {
    let seeds = std::env::var("FLUXORA_FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_seeds);
    let steps = std::env::var("FLUXORA_FUZZ_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_steps);
    (seeds, steps)
}

#[test]
fn the_pool_invariant_holds_across_random_operation_sequences() {
    let (seeds, steps) = fuzz_budget(24, 40);
    for seed in 1..=seeds {
        run_sequence(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), steps);
    }
}

/// A longer run on a handful of seeds, to reach deeper states — streams that
/// have been paused, resumed, topped up and partially drained several times
/// over before anything interesting is asked of them.
#[test]
fn the_pool_invariant_holds_across_long_sequences() {
    let (seeds, steps) = fuzz_budget(4, 150);
    for i in 0..seeds {
        // Distinct from the seeds used above, and stable across runs.
        run_sequence(0xDEAD_BEEF ^ i.wrapping_mul(0xA24B_AED4_963E_E407), steps);
    }
}
