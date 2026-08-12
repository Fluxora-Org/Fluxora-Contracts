//! Accrual math.
//!
//! Every function here is pure: it takes a [`Stream`] and a wall-clock
//! timestamp and returns a number. No `Env`, no storage, no host calls. That
//! makes the whole vesting model property-testable without a Soroban host, and
//! it keeps the interesting arithmetic in one auditable place.
//!
//! # The stream clock
//!
//! The central idea is that a stream has its own clock which stops while the
//! stream is paused. [`stream_time`] maps wall-clock time onto that clock:
//!
//! ```text
//! stream_time(now) = (paused_at.unwrap_or(now)) - paused_total
//! ```
//!
//! While paused, the numerator is frozen at the instant of the pause, so the
//! clock does not advance. After a resume, `paused_total` has absorbed the
//! paused interval, so the clock resumes exactly where it stopped. Every other
//! quantity — elapsed time, the cliff gate, vesting — is expressed against this
//! clock, which is why pausing stretches the schedule without ever changing the
//! total value delivered.
//!
//! Note that this differs from the naive formulation
//! `effective_now = min(now, end_time + paused_total)`, which is correct only
//! *after* a resume. During an in-progress pause that formula keeps accruing,
//! because the current pause has not yet been added to `paused_total`. Reading
//! `paused_at` is what makes the freeze actually freeze.
//!
//! # Stated invariants
//!
//! These hold for every stream at every instant, and every entry point is
//! responsible for preserving them. They are asserted after *every* operation
//! by the test suite (`Harness::assert_invariants`), exhaustively across
//! operation orderings by `test::monotonicity`, and over random schedules by
//! `test::props`.
//!
//! **I1 — Bounds.** `0 <= withdrawn <= vested(t) <= deposited`.
//!
//! **I2 — Monotonic in time.** For a fixed stream state and `t1 <= t2`,
//! `vested(t1) <= vested(t2)`.
//!
//! **I3 — Monotonic across calls.** For a *fixed* `t`, no entry point may
//! reduce `vested(t)`. Formally, if a call transforms stream state `S -> S'`,
//! then `vested(S', t) >= vested(S, t)`.
//!
//! **I4 — Conservation.** `vested(t) + refundable(t) == deposited`, exactly.
//!
//! **I5 — Pause coherence.** `paused_at.is_some()` if and only if
//! `status == Paused`, and while paused the clock does not advance.
//!
//! ## Why I3 is the dangerous one
//!
//! I2 is the obvious property and is easy to get right. **I3 is the one that
//! actually broke.** It is easy to violate by accident because it is a property
//! of *state transitions*, not of the accrual formula, so reading `vested` in
//! isolation never reveals it.
//!
//! `top_up` originally rounded its duration extension up, so the new duration
//! slightly overshot, the rate fell slightly, and `vested(t)` for the *same* `t`
//! came out lower after the call than before. That breaks I1 — a recipient who
//! had already withdrawn at the old rate now holds more than `vested` — and
//! from there `cancel`, which sets `deposited = vested`, drives the stream's
//! liability negative and refunds the sender tokens the recipient already has.
//!
//! The general rule this yields: **any operation that changes `deposited`,
//! `start_time`, `end_time`, `cliff_time` or `paused_total` must be checked
//! against I3**, because those are exactly the inputs to `vested`. Operations
//! that touch only `withdrawn`, `recipient` or `status` cannot violate it.
//! Today that means `top_up` and `cancel` need the check and the rest do not,
//! but the test suite verifies all of them so a future entry point cannot
//! quietly join the first group.
//!
//! ## Why I3 requires freezing the clock, and why that is not a detail
//!
//! I3 can only be observed by reading `vested` at one timestamp, performing
//! exactly one call, and reading `vested` again **at that same timestamp**.
//!
//! This is the reason the bug survived a suite that already had good coverage
//! of `top_up`. Every hand-written test advanced time around the operations it
//! exercised — deposit, wait, withdraw, wait, top up, wait, assert — because
//! that is how you write a readable test for a contract whose whole subject is
//! the passage of time. But `vested` is *supposed* to grow as the clock
//! advances. So a 93-stroop backwards step vanished into the accrual that
//! happened alongside it, and every assertion still passed. The regression was
//! real, deterministic, and reachable from a two-line test — and invisible to
//! roughly a hundred existing ones, because they all measured the wrong
//! difference.
//!
//! So the test design follows from the invariant rather than from convenience:
//! I2 (monotonic in time) is tested by holding *state* still and advancing the
//! clock, and I3 (monotonic across calls) is tested by holding the *clock*
//! still and advancing the state. Conflating the two hides exactly the class of
//! defect that matters most, because a violation of I3 is a fund-safety bug
//! while a violation of I2 is merely a wrong number.
//!
//! `test::monotonicity` implements the frozen-clock half across every entry
//! point and every ordering of them; `test::props` implements the
//! advancing-clock half over random schedules.

use crate::error::Error;
use crate::types::Stream;

/// The stream's own clock, in the same units and origin as `start_time` and
/// `end_time`. Stops while the stream is paused.
///
/// Saturates at zero rather than underflowing; a stream whose `paused_total`
/// exceeds the current instant simply reads as time zero, which clamps to "no
/// elapsed time" downstream.
pub fn stream_time(stream: &Stream, now: u64) -> u64 {
    let frozen_at = match stream.paused_at {
        Some(paused_at) => paused_at,
        None => now,
    };
    frozen_at.saturating_sub(stream.paused_total)
}

/// Total scheduled duration, in seconds.
///
/// Guaranteed non-zero at creation ([`Error::InvalidTimeRange`]), but it can
/// legitimately become zero after a cancel that lands at `start_time` — see
/// [`vested`] for how that case is handled.
pub fn duration(stream: &Stream) -> u64 {
    stream.end_time.saturating_sub(stream.start_time)
}

/// Seconds of the schedule actually consumed, clamped to `[0, duration]`.
pub fn elapsed(stream: &Stream, now: u64) -> u64 {
    let clock = stream_time(stream, now);
    let capped = if clock > stream.end_time {
        stream.end_time
    } else {
        clock
    };
    capped.saturating_sub(stream.start_time)
}

/// Whether the cliff gate has opened.
///
/// The gate is evaluated against the stream clock, so a stream paused across
/// its cliff does not silently pass the cliff while frozen.
pub fn cliff_reached(stream: &Stream, now: u64) -> bool {
    stream_time(stream, now) >= stream.cliff_time
}

/// Amount vested at `now`: what the recipient has earned in total, ever.
///
/// Rounds **down**. Integer division truncating in the recipient's disfavour is
/// the correct direction: the residue stays in the contract and is returned to
/// the sender when the stream settles, so the pool can never be short.
///
/// Before the cliff this is zero — the cliff *gates* the payout, it does not
/// delay accrual, so at the cliff instant the recipient becomes entitled to
/// everything accrued since `start_time`, not since `cliff_time`.
pub fn vested(stream: &Stream, now: u64) -> Result<i128, Error> {
    if !cliff_reached(stream, now) {
        return Ok(0);
    }

    let total_duration = duration(stream);

    // A zero duration means the schedule has collapsed onto a single instant,
    // which only happens after `cancel` lands at `start_time`. In that case
    // `deposited` has already been rewritten to the amount vested at the
    // cancel, so returning it in full is exactly right — and it avoids a
    // division by zero.
    if total_duration == 0 {
        return Ok(stream.deposited);
    }

    let consumed = elapsed(stream, now);
    if consumed >= total_duration {
        return Ok(stream.deposited);
    }

    let numerator = stream
        .deposited
        .checked_mul(consumed as i128)
        .ok_or(Error::Overflow)?;
    let raw = numerator
        .checked_div(total_duration as i128)
        .ok_or(Error::Overflow)?;

    // Clamp explicitly rather than trusting the arithmetic. `consumed` is
    // already capped at `total_duration`, so this should be unreachable, but
    // the invariant is load-bearing enough to assert rather than assume.
    Ok(if raw > stream.deposited {
        stream.deposited
    } else {
        raw
    })
}

/// Amount the recipient can withdraw right now: vested minus already withdrawn.
///
/// Saturates at zero. `withdrawn` can momentarily exceed `vested` only if a
/// cancel rewrote the schedule, and even then the cancel path guarantees
/// `deposited >= withdrawn`, so this is defence in depth.
pub fn withdrawable(stream: &Stream, now: u64) -> Result<i128, Error> {
    let earned = vested(stream, now)?;
    let available = earned
        .checked_sub(stream.withdrawn)
        .ok_or(Error::Overflow)?;
    Ok(if available < 0 { 0 } else { available })
}

/// Amount still locked for the recipient's future: deposited minus vested.
///
/// This is what the sender gets back if they cancel at `now`.
pub fn refundable(stream: &Stream, now: u64) -> Result<i128, Error> {
    let earned = vested(stream, now)?;
    stream.deposited.checked_sub(earned).ok_or(Error::Overflow)
}

/// Outstanding liability of this stream against the pooled contract balance:
/// everything deposited that has not yet left the contract.
///
/// The pool invariant is that the contract's token balance is always at least
/// the sum of this quantity across all live streams.
pub fn liability(stream: &Stream) -> Result<i128, Error> {
    stream
        .deposited
        .checked_sub(stream.withdrawn)
        .ok_or(Error::Overflow)
}
