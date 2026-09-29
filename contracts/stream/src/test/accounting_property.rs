//! Issue #1856 — the accounting identity as a generated property over
//! randomized operation sequences, asserted at every terminal state.
//!
//! ```text
//! withdrawable(t) + refundable(t) == deposited - withdrawn        for every t
//! ```
//!
//! `accrual::withdrawable` is `vested - withdrawn` and `accrual::refundable` is
//! `deposited - vested`, so their sum is exactly the stream's outstanding
//! liability `deposited - withdrawn`: every stroop the contract still holds is
//! either already earned by the recipient (withdrawable) or still locked and
//! refundable to the sender. `test::accounting_identity` (#1711) pins that
//! identity for hand-picked instants and a handful of fixed sequences. This
//! module promotes it to a property **generated across randomized operation
//! sequences**: each proptest case is a distinct seed expanded into a mixed
//! sequence of `create` / `top_up` / `withdraw` / `pause` / `resume` / `cancel`
//! / `transfer_recipient` / `extend_stream_ttl` calls interleaved with clock
//! advances, and the identity is re-checked through the public views
//! (`withdrawable_of`, `refundable_of`) *and* the pure `accrual` helpers after
//! every step, for every stream.
//!
//! The sequence deliberately reaches the states a fixed-fixture test cannot:
//! multiple concurrent streams, partial draws, a pause that spans a cliff,
//! top-ups that move the rate, cancellation mid-schedule, maturity, and
//! depletion. Failed entry-point calls are expected and ignored — the
//! property is about what the *state* must satisfy afterwards, not about which
//! random operations the contract happened to accept.
//!
//! # Reproducibility
//!
//! A case is fully identified by the `(seed, steps)` pair proptest reports on
//! failure, and every assertion message names both. To replay a reported seed
//! exactly, without the proptest runner:
//!
//! ```text
//! ACCOUNTING_PROPERTY_SEED=<seed> ACCOUNTING_PROPERTY_STEPS=<steps> \
//!   cargo test -p fluxora-stream accounting_property -- --nocapture
//! ```
//!
//! `reported_seed_replays` honours those two variables; proptest's own
//! `PROPTEST_CASES` continues to size the generated sweep as it does for
//! `test::props`.

use proptest::prelude::*;

use super::common::*;
use crate::accrual;

/// Longest generated sequence. Bounded because every step is a real host
/// invocation; the identity has already been pinned to a small number of
/// states by the time a sequence this long has run.
const MAX_STEPS: u32 = 24;

/// Cap on concurrent streams so a sequence explores multi-stream accounting
/// without unbounded state growth.
const MAX_STREAMS: u64 = 3;

/// xorshift64* — the same deterministic PRNG the sequence suites use, so a
/// seed maps to one sequence on every platform and toolchain.
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

/// The identity, asserted for **every** stream at the current instant.
///
/// Both the contract views and the pure helpers are checked, and the two are
/// required to agree: a divergence between them is itself a bug (the views are
/// the ABI integrators read; the helpers are what the contract writes).
fn assert_identity(h: &Harness, seed: u64, step: u32, context: &str) {
    let now = h.now();
    for id in 0..h.client.stream_count() {
        let s = h.client.get_stream(&id);
        let withdrawable = h.client.withdrawable_of(&id);
        let refundable = h.client.refundable_of(&id);
        let unwithdrawn = s.deposited - s.withdrawn;

        assert_eq!(
            withdrawable + refundable,
            unwithdrawn,
            "seed {seed}, step {step} ({context}), stream {id}: withdrawable_of \
             {withdrawable} + refundable_of {refundable} != deposited {} - withdrawn {} \
             ({unwithdrawn})",
            s.deposited,
            s.withdrawn,
        );

        // The pure helpers behind those views must produce the same numbers.
        assert_eq!(
            accrual::withdrawable(&s, now).expect("withdrawable must not overflow"),
            withdrawable,
            "seed {seed}, step {step} ({context}), stream {id}: accrual::withdrawable \
             disagrees with withdrawable_of",
        );
        assert_eq!(
            accrual::refundable(&s, now).expect("refundable must not overflow"),
            refundable,
            "seed {seed}, step {step} ({context}), stream {id}: accrual::refundable \
             disagrees with refundable_of",
        );
        assert_eq!(
            accrual::liability(&s).expect("liability must not overflow"),
            unwithdrawn,
            "seed {seed}, step {step} ({context}), stream {id}: accrual::liability \
             disagrees with deposited - withdrawn",
        );

        // I4 (conservation) is the identity's other half and the reason
        // `refundable` is exact rather than rounded.
        assert_eq!(
            accrual::vested(&s, now).expect("vested must not overflow") + refundable,
            s.deposited,
            "seed {seed}, step {step} ({context}), stream {id}: vested + refundable != deposited",
        );

        assert!(
            withdrawable >= 0 && refundable >= 0,
            "seed {seed}, step {step} ({context}), stream {id}: a view went negative \
             (withdrawable {withdrawable}, refundable {refundable})",
        );
    }
}

/// Create one more stream at the current ledger time, with a rate of two
/// stroops per second so every schedule satisfies the contract's rate floor.
fn create_one(h: &Harness, rng: &mut Rng) -> u64 {
    let start = h.now();
    let duration = DAY + rng.below(20 * DAY);
    let cliff = start + rng.below(duration + 1);
    let deposit = (duration as i128) * 2;
    h.create(deposit, start, start + duration, cliff, true, true, true)
}

/// Expand `seed` into a sequence of public-ABI operations and assert the
/// accounting identity after every one of them.
fn run_sequence(seed: u64, steps: u32) {
    let h = Harness::new();
    let mut rng = Rng(seed);

    create_one(&h, &mut rng);
    assert_identity(&h, seed, 0, "after create");

    for step in 1..=steps {
        let count = h.client.stream_count();
        let id = rng.below(count);

        match rng.below(13) {
            // Withdraw, sometimes a full draw and sometimes a partial one.
            0..=2 => {
                let amount = if rng.below(2) == 0 {
                    None
                } else {
                    Some((1 + rng.below(200)) as i128 * ONE)
                };
                let _ = h.client.try_withdraw(&id, &amount);
            }
            // The boundary the identity depends on: one stroop more than
            // `withdrawable_of` must be rejected without moving any state. If
            // that cap ever disappears the recipient can draw past `vested`,
            // `withdrawable` saturates at zero, and the identity breaks.
            3 => {
                let available = h.client.withdrawable_of(&id);
                if available > 0 {
                    let before = h.client.get_stream(&id).withdrawn;
                    let _ = h.client.try_withdraw(&id, &Some(available + 1));
                    let after = h.client.get_stream(&id).withdrawn;
                    assert_eq!(
                        after, before,
                        "seed {seed}, step {step}, stream {id}: withdraw accepted an amount \
                         above withdrawable_of ({available}); withdrawn moved {before} -> {after}",
                    );
                }
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
                // At least one stroop per second of schedule, which is the
                // contract's TopUpTooSmall boundary.
                let amount = 2 + rng.below(1_000) as i128;
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
            9 => {
                if count < MAX_STREAMS {
                    create_one(&h, &mut rng);
                } else {
                    let _ = h.client.try_extend_stream_ttl(&id);
                }
            }
            10 => {
                let _ = h.client.try_extend_stream_ttl(&id);
            }
            _ => {}
        }

        // Move the clock between operations so accrual, cliff gates and
        // maturity all shift under the sequence.
        h.advance(1 + rng.below(10 * DAY));

        assert_identity(&h, seed, step, "after operation and advance");
    }

    // The identity must survive the terminal state the sequence settled into.
    let now = h.now();
    for id in 0..h.client.stream_count() {
        let s = h.client.get_stream(&id);
        assert_eq!(
            accrual::withdrawable(&s, now).expect("withdrawable")
                + accrual::refundable(&s, now).expect("refundable"),
            s.deposited - s.withdrawn,
            "seed {seed}: identity broken in the settled state, stream {id}",
        );
    }
}

proptest! {
    // `ProptestConfig::default()` also picks up `PROPTEST_CASES`, so this suite
    // is sized by the same env knob as `test::props` in the CI proptest job —
    // 64 cases on a PR, 512 on the nightly sweep. Do NOT pin `with_cases`.
    #![proptest_config(ProptestConfig::default())]

    /// **The accounting identity across randomized operation sequences.**
    ///
    /// For a generated mix of create / top-up / withdraw / pause / resume /
    /// cancel / transfer / TTL operations with clock advances in between,
    /// `withdrawable + refundable == deposited - withdrawn` holds for every
    /// stream after every step. The `(seed, steps)` pair is the reproducible
    /// identity of a failing case.
    #[test]
    fn withdrawable_plus_refundable_equals_deposited_minus_withdrawn(
        seed in any::<u64>(),
        steps in 1u32..=MAX_STEPS,
    ) {
        run_sequence(seed, steps);
    }
}

/// Replay a seed reported by a failing proptest case without the runner.
///
/// With `ACCOUNTING_PROPERTY_SEED` unset this exercises a few fixed seeds so
/// the driver itself is covered on every run; with it set, the reported seed
/// (and optional `ACCOUNTING_PROPERTY_STEPS`) is reproduced exactly, which is
/// the documented way to turn a CI failure into a local one.
#[test]
fn reported_seed_replays() {
    if let Ok(raw) = std::env::var("ACCOUNTING_PROPERTY_SEED") {
        let seed: u64 = raw
            .trim()
            .parse()
            .expect("ACCOUNTING_PROPERTY_SEED must be a u64");
        let steps = std::env::var("ACCOUNTING_PROPERTY_STEPS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(MAX_STEPS);
        run_sequence(seed, steps);
        std::eprintln!("accounting_property: replayed seed {seed} for {steps} steps");
    } else {
        for seed in [1u64, 0x9E37_79B9_7F4A_7C15, 0xDEAD_BEEF_CAFE_F00D] {
            run_sequence(seed, MAX_STEPS);
        }
    }
}
