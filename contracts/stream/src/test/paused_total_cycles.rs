//! Issue #1842 — repeated pause and resume cycles accumulating in
//! `paused_total`.
//!
//! `pause.rs` already proves that a handful of cycles each move `paused_total`
//! by the right amount. What it does not do is treat the accumulator as an
//! accumulator: nothing there asserts the *emitted events* carry the running
//! total, and nothing drives the cycles from a generator. So a regression that
//! dropped the running total from the `Resumed` payload — the exact field an
//! indexer reconstructs a stretched schedule from — would still pass.
//!
//! This module closes that gap. Every cycle is checked on three axes at once:
//!
//! * **accounting** — `paused_total` equals the exact sum of the cycle lengths
//!   after every resume, never the last one and never an approximation;
//! * **events** — each pause emits exactly one `paused`, each resume exactly one
//!   `resumed`, and the payload's `paused_total` is the same running total,
//!   with `paused_duration` equal to that cycle's length;
//! * **final state** — the stream clock lags the wall clock by exactly
//!   `paused_total`, so vesting is unaffected by how the pauses were chopped up.
//!
//! The last test drives all of that from a seeded generator, with withdrawals,
//! top-ups and recipient transfers interleaved, because the accumulator's
//! failure mode is drift over many cycles rather than a wrong single step.
//! `the_accumulator_is_checked_not_wrapping` covers the other extreme: the
//! `u64` bound.

use super::common::*;
use crate::{DataKey, Error, Stream, StreamStatus};
use soroban_sdk::{testutils::Events, Map, Symbol, TryFromVal, TryIntoVal, Val};

/// The `Map` payload of the most recent event emitted by the stream contract.
fn last_event(h: &Harness) -> Map<Symbol, Val> {
    let raw = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();
    assert_eq!(
        raw.len(),
        1,
        "each state change must emit exactly one event",
    );
    let ev = raw.first().unwrap();
    let soroban_sdk::xdr::ContractEventBody::V0(body) = &ev.body;
    let data: Val = Val::try_from_val(&h.env, &body.data).unwrap();
    data.try_into_val(&h.env).unwrap()
}

/// `topic[0]` of the most recent event, so a test can prove it decoded the
/// event it meant to rather than merely decoding *an* event.
fn last_event_topic0(h: &Harness) -> Symbol {
    let raw = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();
    let ev = raw.last().unwrap();
    let soroban_sdk::xdr::ContractEventBody::V0(body) = &ev.body;
    let topic: Val = Val::try_from_val(&h.env, body.topics.first().unwrap()).unwrap();
    topic.try_into_val(&h.env).unwrap()
}

fn payload_u64(h: &Harness, map: &Map<Symbol, Val>, field: &str) -> u64 {
    map.get(Symbol::new(&h.env, field))
        .unwrap_or_else(|| panic!("event payload has no field {field}"))
        .try_into_val(&h.env)
        .unwrap()
}

const DEPOSIT: i128 = 1_000 * ONE;
const DURATION: u64 = 100 * DAY;

/// The deposit this module's streams carry: cancellable, pausable, over
/// [`DURATION`].
fn rate_stream(h: &Harness) -> u64 {
    h.create_simple(DEPOSIT, DURATION)
}

/// Vesting after `real_seconds` of *stream* clock, computed with the contract's
/// own floor-rounding formula. Deriving it here rather than assuming a rate
/// keeps the expectation honest: `vested` is `deposited * elapsed / duration`,
/// truncated, so it is not `real_seconds * rate` for every instant.
fn expected_vested(real_seconds: u64) -> i128 {
    DEPOSIT * real_seconds as i128 / DURATION as i128
}

// ---------------------------------------------------------------------------
// The accumulator
// ---------------------------------------------------------------------------

/// The core property: after each resume, `paused_total` is the exact sum of
/// every pause so far — not the last one, not a running average, not rounded.
#[test]
fn repeated_pause_resume_cycles_accumulate_paused_total_exactly() {
    let h = Harness::new();
    let id = rate_stream(&h);

    // Deliberately coprime, uneven lengths so a "last pause wins" or
    // off-by-one accumulator cannot coincide with the sum.
    let cycle_lengths = [1u64, 7, 61, DAY, 3 * DAY + 17, 3600, 11];
    let mut expected_total = 0u64;

    for (cycle, &pause_len) in cycle_lengths.iter().enumerate() {
        let real_before = h.now() - T0 - expected_total;

        h.advance(DAY);
        h.client.pause(&id);
        h.advance(pause_len);
        h.client.resume(&id);
        expected_total += pause_len;

        let s = h.get(id);
        assert_eq!(
            s.paused_total, expected_total,
            "cycle {cycle}: paused_total must be exactly {expected_total} after a {pause_len}s pause",
        );
        assert_eq!(
            s.status,
            StreamStatus::Active,
            "cycle {cycle}: resume must return the stream to Active",
        );
        assert_eq!(s.paused_at, None, "cycle {cycle}: freeze point cleared");

        // Accounting axis: the stream clock lagged the wall clock by exactly the
        // accumulated pause, so vesting depends only on real time.
        let real_after = real_before + DAY;
        assert_eq!(
            h.client.vested_of(&id),
            expected_vested(real_after),
            "cycle {cycle}: vesting must reflect {real_after}s of real accrual",
        );
    }

    assert_eq!(expected_total, 1 + 7 + 61 + DAY + 3 * DAY + 17 + 3600 + 11);

    // Final-state axis: the schedule is stretched by exactly the accumulator.
    h.warp_to(T0 + 100 * DAY + expected_total);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.assert_pool_exact();
}

/// Every cycle emits one `paused` and one `resumed`, and the payloads carry the
/// running total — the field an indexer rebuilds a stretched schedule from.
#[test]
fn each_pause_and_resume_emits_one_event_carrying_the_running_total() {
    let h = Harness::new();
    let id = rate_stream(&h);

    let cycle_lengths = [5u64, 20, 3, 90];
    let mut expected_total = 0u64;

    for (cycle, &pause_len) in cycle_lengths.iter().enumerate() {
        h.advance(10 * DAY);
        let pause_at = h.now();

        h.client.pause(&id);
        assert_eq!(
            last_event_topic0(&h),
            Symbol::new(&h.env, "paused"),
            "cycle {cycle}: pause must emit exactly one `paused`",
        );
        let paused = last_event(&h);
        assert_eq!(
            payload_u64(&h, &paused, "paused_total"),
            expected_total,
            "cycle {cycle}: `paused` must report the total *before* this pause is added",
        );
        assert_eq!(
            payload_u64(&h, &paused, "paused_at"),
            pause_at,
            "cycle {cycle}: `paused` must report the freeze instant",
        );

        h.advance(pause_len);
        h.client.resume(&id);
        assert_eq!(
            last_event_topic0(&h),
            Symbol::new(&h.env, "resumed"),
            "cycle {cycle}: resume must emit exactly one `resumed`",
        );
        let resumed = last_event(&h);
        expected_total += pause_len;
        assert_eq!(
            payload_u64(&h, &resumed, "paused_duration"),
            pause_len,
            "cycle {cycle}: `paused_duration` must be this cycle's length",
        );
        assert_eq!(
            payload_u64(&h, &resumed, "paused_total"),
            expected_total,
            "cycle {cycle}: `resumed` must report the post-resume running total",
        );

        // The event and storage agree, always.
        assert_eq!(h.get(id).paused_total, expected_total);
    }

    assert_eq!(
        h.get(id).paused_total,
        cycle_lengths.iter().sum::<u64>(),
        "the accumulator must equal the sum of the event-reported durations",
    );
}

/// A pause and resume inside the same second adds nothing, emits a `resumed`
/// with `paused_duration == 0`, and leaves the schedule untouched.
#[test]
fn a_zero_length_pause_emits_resumed_with_zero_duration_and_does_not_move_the_total() {
    let h = Harness::new();
    let id = rate_stream(&h);

    h.advance(30 * DAY);
    let vested_at_pause = h.client.vested_of(&id);

    h.client.pause(&id);
    h.client.resume(&id);

    let resumed = last_event(&h);
    assert_eq!(payload_u64(&h, &resumed, "paused_duration"), 0);
    assert_eq!(payload_u64(&h, &resumed, "paused_total"), 0);
    assert_eq!(
        h.get(id).paused_total,
        0,
        "a zero-length pause contributes nothing",
    );
    assert_eq!(
        h.client.vested_of(&id),
        vested_at_pause,
        "and it does not move the schedule either",
    );

    // The next real cycle still starts counting from zero.
    h.advance(2 * DAY);
    h.client.pause(&id);
    h.advance(5);
    h.client.resume(&id);
    assert_eq!(h.get(id).paused_total, 5);
    h.assert_pool_exact();
}

/// Which operation to interleave around a cycle. An enum rather than a string
/// label so the compiler, not a typo, decides which arm runs.
#[derive(Clone, Copy)]
enum Between {
    WithdrawWhilePaused,
    Withdraw,
    TopUp,
    Transfer,
    ExtendTtl,
}

/// Withdrawals, top-ups and recipient transfers threaded through the cycles
/// must not disturb the accumulator: none of them touches the stream clock.
#[test]
fn cycles_interleaved_with_other_operations_keep_the_accumulator_exact() {
    let h = Harness::new();
    let id = rate_stream(&h);

    let mut expected_total = 0u64;

    let plan: [(Between, u64); 5] = [
        (Between::WithdrawWhilePaused, 10),
        (Between::TopUp, 4),
        (Between::Transfer, 6),
        (Between::ExtendTtl, 3),
        (Between::Withdraw, 30),
    ];

    for (step, &(op, pause_len)) in plan.iter().enumerate() {
        h.advance(5 * DAY);
        h.client.pause(&id);
        let total_at_pause = h.get(id).paused_total;

        // Claiming funds earned before the pause is not a clock change.
        if matches!(op, Between::WithdrawWhilePaused) && h.client.withdrawable_of(&id) > 0 {
            h.client.withdraw(&id, &None);
        }

        h.advance(pause_len);
        h.client.resume(&id);
        expected_total += pause_len;

        match op {
            Between::Withdraw | Between::WithdrawWhilePaused => {
                if h.client.withdrawable_of(&id) > 0 {
                    h.client.withdraw(&id, &None);
                }
            }
            Between::TopUp => {
                h.client.top_up(&id, &(10 * ONE));
            }
            Between::Transfer => {
                h.client.transfer_recipient(&id, &h.other);
                h.client.transfer_recipient(&id, &h.recipient);
            }
            Between::ExtendTtl => {
                h.client.extend_stream_ttl(&id);
            }
        }

        assert_eq!(
            total_at_pause,
            expected_total - pause_len,
            "step {step}: pausing must report the total from before this cycle",
        );
        assert_eq!(
            h.get(id).paused_total,
            expected_total,
            "step {step}: paused_total must survive the interleaved operation",
        );
        h.assert_pool_exact();
    }

    assert_eq!(
        h.get(id).paused_total,
        plan.iter().map(|&(_, len)| len).sum::<u64>(),
    );
}

/// Many one-second cycles: the accumulator must not drift, and the boundary is
/// the second, not the ledger (a one-second advance may not close a ledger).
#[test]
fn many_one_second_cycles_accumulate_without_drift() {
    let h = Harness::new();
    let id = rate_stream(&h);

    const CYCLES: u64 = 25;
    for cycle in 0..CYCLES {
        h.client.pause(&id);
        h.advance(1);
        h.client.resume(&id);

        assert_eq!(
            h.get(id).paused_total,
            cycle + 1,
            "cycle {cycle}: the accumulator drifted",
        );
    }

    let expected = CYCLES;
    assert_eq!(h.get(id).paused_total, expected);

    // The whole schedule is stretched by exactly `expected` seconds.
    h.warp_to(T0 + 100 * DAY + expected);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
}

/// The accumulator is a `u64` and is grown with `checked_add`, so a stream that
/// has (however improbably) accumulated to the bound fails the next resume with
/// a typed `Overflow` instead of wrapping to a small number and silently
/// re-vesting the schedule. The rejected resume changes nothing.
#[test]
fn the_accumulator_is_checked_not_wrapping() {
    let h = Harness::new();
    let id = rate_stream(&h);

    h.advance(10 * DAY);
    h.client.pause(&id);

    // Seed the accumulator close enough to the bound that this pause overflows.
    h.env.as_contract(&h.contract_id, || {
        let key = DataKey::Stream(id);
        let mut s: Stream = h.env.storage().persistent().get(&key).unwrap();
        s.paused_total = u64::MAX - 5;
        h.env.storage().persistent().set(&key, &s);
    });

    h.advance(10 * DAY);
    assert_eq!(
        h.client.try_resume(&id).unwrap_err().unwrap(),
        Error::Overflow,
        "a resume that would overflow the accumulator must be refused",
    );

    // Refused atomically: still paused, accumulator untouched.
    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Paused);
    assert!(s.paused_at.is_some(), "the stream must still be frozen");
    assert_eq!(
        s.paused_total,
        u64::MAX - 5,
        "a refused resume must not partially apply the pause",
    );
}

// ---------------------------------------------------------------------------
// Property test
// ---------------------------------------------------------------------------

/// xorshift64*. Deterministic and seedable, so a failure replays from its seed
/// alone — the convention set by `test::invariants`.
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

/// Random pause/resume cycles with other operations interleaved, checked after
/// every single transition: `paused_total` is the exact sum, the stream clock
/// lag equals it, and conservation still holds.
///
/// The generator alternates pause and resume strictly, so every call is a legal
/// transition and a failure is a real accounting bug rather than a test
/// artefact.
fn run_random_cycles(seed: u64, cycles: u32) {
    let h = Harness::new();
    let id = rate_stream(&h);
    let mut rng = Rng(seed);

    let mut expected_total = 0u64;
    let mut real_seconds = 0u64;

    for cycle in 0..cycles {
        // Real accrual between cycles, then a pause of a generated length.
        let real = 1 + rng.below(3 * DAY);
        h.advance(real);
        real_seconds += real;

        h.client.pause(&id);
        assert_eq!(
            h.get(id).paused_total,
            expected_total,
            "seed {seed}, cycle {cycle}: pausing must not move the accumulator",
        );

        let pause_len = rng.below(2 * DAY);
        if pause_len > 0 {
            h.advance(pause_len);
        }
        h.client.resume(&id);
        expected_total += pause_len;

        let s = h.get(id);
        assert_eq!(
            s.paused_total, expected_total,
            "seed {seed}, cycle {cycle}: accumulator must be exactly {expected_total}",
        );
        assert_eq!(s.status, StreamStatus::Active);
        assert_eq!(s.paused_at, None);

        // The stream clock lags the wall clock by exactly the accumulator.
        assert_eq!(
            h.now() - T0,
            real_seconds + expected_total,
            "seed {seed}, cycle {cycle}: wall clock must be real accrual plus paused time",
        );
        assert_eq!(
            h.client.vested_of(&id),
            expected_vested(real_seconds),
            "seed {seed}, cycle {cycle}: vesting must ignore paused time entirely",
        );

        // Interleave an operation that must not touch the accumulator.
        match rng.below(4) {
            0 => {
                let _ = h.client.try_withdraw(&id, &None);
            }
            1 => {
                let _ = h.client.try_top_up(&id, &(5 * ONE));
            }
            2 => {
                let _ = h.client.try_transfer_recipient(&id, &h.other);
                let _ = h.client.try_transfer_recipient(&id, &h.recipient);
            }
            _ => {
                let _ = h.client.try_extend_stream_ttl(&id);
            }
        }

        assert_eq!(
            h.get(id).paused_total,
            expected_total,
            "seed {seed}, cycle {cycle}: an interleaved operation disturbed the accumulator",
        );
        h.assert_pool_exact();
    }

    // Final state: the schedule is stretched by exactly the accumulator.
    assert_eq!(
        h.client.vested_of(&id),
        expected_vested(real_seconds),
        "seed {seed}: final vesting must be real accrual only",
    );
}

#[test]
fn randomized_pause_resume_cycles_accumulate_exactly() {
    for seed in 0..64u64 {
        run_random_cycles(
            0xA24B_AED4_963E_E407 ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            12,
        );
    }
}

#[test]
fn long_randomized_cycle_sequences_accumulate_exactly() {
    for seed in 0..6u64 {
        run_random_cycles(0xDEAD_BEEF_D15C_A5E1u64.wrapping_add(seed), 60);
    }
}
