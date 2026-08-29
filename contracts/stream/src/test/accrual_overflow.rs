//! Stage 3 — arithmetic overflow safety at maximum configuration values.
//!
//! Issue #1552. The accrual helpers operate on `u64` ledger/time values and
//! `i128` amounts. At the maximum supported configuration values — `u64`-wide
//! timestamps and durations, and deposits large enough that `deposit * duration`
//! presses against the `i128` ceiling — a single plain `+` / `*` / `-` could
//! wrap silently, and a host panic on an unchecked division by zero or an
//! overflowed cast would surface as an **opaque host trap** instead of a typed
//! [`crate::Error`].
//!
//! The chosen design is **checked arithmetic in the accrual helpers combined
//! with a bounded domain enforced at creation**:
//!
//! * Every arithmetic helper (`stream_time`, `duration`, `elapsed`,
//!   `withdrawable`, `refundable`, `liability`) saturates or `checked_`
//!   fails rather than wrapping.
//! * `vested` guards its one real multiplication (`deposited * consumed`)
//!   with `checked_mul` mapped to [`Error::Overflow`], and `create_stream`
//!   front-loads that same guard against `deposit * duration` so the accrual
//!   multiplication can never overflow for a stream that ever reached storage.
//! * `top_up` re-establishes both guards against its post-extension figures.
//!
//! These tests pin the boundary down: they drive `accrual` directly (the way
//! `test::props` does, microseconds per case and no host) at the widest
//! timestamps and the largest deposits `create_stream` would accept, and they
//! drive the real contract at `u64`-wide timestamps to confirm the views and
//! lifecycle stay typed — no wrap, no division-by-zero, no opaque host trap.
//!
//! The one thing that must *never* happen on any of these inputs is a panic.
//! An `Err(Error::Overflow)` (or an `Ok` with a bounded value) is correct and
//! expected; a trap is the bug this file exists to catch.

use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use super::common::*;
use crate::types::{Stream, StreamStatus};
use crate::Error;
use crate::{accrual, storage};

/// Largest `u64` timestamp representable by the arithmetic itself.
const MAX_TIME: u64 = u64::MAX;

/// Largest `end_time` the *harness host* can actually store a persistent entry
/// for.
///
/// Pure `accrual` handles `u64::MAX` timestamps fine (those tests are above).
/// But a `Stream` written to host storage gets a TTL derived from how far its
/// `end_time` is past the ledger time, and the test harness's `max_ttl` is not
/// clamped the way mainnet's is. An `end_time` at `u64::MAX` makes that TTL
/// saturate past what the host's `u32` live-until ledger can hold, surfacing as
/// an opaque `Error(Storage, InternalError)` "persistent entry TTL overflow".
/// That is a storage-layer limit of the host, not an accrual-math overflow, so
/// the contract-level schedule tests run at the furthest *deployable* horizon
/// and leave `u64::MAX` to the pure math above.
const MAX_DEPLOYABLE_END: u64 = (u32::MAX as u64) * storage::SECONDS_PER_LEDGER;

/// Build a stream directly, bypassing the contract, so the arithmetic boundary
/// can be tested even for deposits no token would ever hold. Mirrors
/// [`test::props::stream_of`].
fn stream_of(
    deposited: i128,
    start: u64,
    end: u64,
    cliff: u64,
    paused_total: u64,
    paused_at: Option<u64>,
) -> Stream {
    let env = Env::default();
    Stream {
        sender: Address::generate(&env),
        recipient: Address::generate(&env),
        token: Address::generate(&env),
        deposited,
        withdrawn: 0,
        start_time: start,
        end_time: end,
        cliff_time: cliff,
        cancellable: true,
        pausable: true,
        transferable: true,
        paused_total,
        paused_at,
        status: if paused_at.is_some() {
            StreamStatus::Paused
        } else {
            StreamStatus::Active
        },
    }
}

// ---------------------------------------------------------------------------
// Pure accrual: maximum timestamps
// ---------------------------------------------------------------------------

/// With `u64`-wide timestamps the time arithmetic must remain bounded: `vested`
/// stays within `[0, deposited]` and every helper saturates instead of
/// wrapping, no matter how far the ledger clock runs past `end_time`.
#[test]
fn accrual_is_bounded_at_maximum_timestamps() {
    // A schedule pushed to the very top of the u64 range. `end > start` and
    // `cliff in [start, end]` hold, exactly as `create_stream` demands; the
    // deposit * duration product is small, so this is a creation-valid stream.
    let start = MAX_TIME - 100 * DAY;
    let end = MAX_TIME;
    let cliff = start + 50 * DAY;
    let s = stream_of(1_000 * ONE, start, end, cliff, 0, None);

    assert_eq!(accrual::duration(&s), 100 * DAY);
    assert_eq!(accrual::elapsed(&s, start), 0);
    assert_eq!(accrual::elapsed(&s, end), 100 * DAY);

    // The clock is a u64; there is nothing past MAX_TIME, and every helper must
    // clamp rather than wrap.
    for now in [start, start + 10 * DAY, cliff, end, MAX_TIME, MAX_TIME - 1] {
        let vested = accrual::vested(&s, now).expect("no overflow for a valid stream");
        assert!(
            vested >= 0 && vested <= s.deposited,
            "now {now}: vested {vested}"
        );

        let drawable = accrual::withdrawable(&s, now).expect("no overflow");
        let refund = accrual::refundable(&s, now).expect("no overflow");
        assert!(drawable >= 0, "now {now}: drawable went negative");
        assert_eq!(
            vested + refund,
            s.deposited,
            "now {now}: conservation broken",
        );

        // Before the cliff the gate holds; the helpers never panic.
        if now < cliff {
            assert!(!accrual::cliff_reached(&s, now));
            assert_eq!(vested, 0);
        } else {
            assert!(accrual::cliff_reached(&s, now));
        }
    }
}

/// A ledger clock value of `u64::MAX` with a `paused_total` near the ceiling
/// must saturate the stream clock at zero (never underflow), and the resumed
/// path must still compute a finite, ordered schedule.
#[test]
fn stream_time_saturates_at_maximum_pause_accumulation() {
    let start = MAX_TIME - 1_000;
    let s = stream_of(
        1_000,
        start,
        MAX_TIME,
        start,
        MAX_TIME - 10, // paused_total within one step of the ceiling
        None,
    );

    // frozen_at.saturating_sub(paused_total) must not panic or wrap; the clock
    // saturating to the ceiling reads as zero elapsed time.
    accrual::stream_time(&s, MAX_TIME);
    assert_eq!(accrual::elapsed(&s, MAX_TIME), 0);

    // `now` below paused_total reads as time zero as well — the documented
    // freeze behavior.
    let s2 = stream_of(1_000, start, MAX_TIME, start, 500, None);
    assert_eq!(
        accrual::stream_time(&s2, 100),
        0,
        "clock must clamp at zero"
    );
}

/// `paused_at = Some(...)` near the ceiling must pin the clock at the freeze
/// instant and the gate stable: `vested` reads the same value no matter how
/// far past the freeze point `now` runs, and never traps.
#[test]
fn a_pause_at_the_maximum_timestamp_freezes_accrual() {
    let start = MAX_TIME - 10 * DAY;
    let pause_instant = MAX_TIME - 5 * DAY;
    let s = stream_of(1_000 * ONE, start, MAX_TIME, start, 0, Some(pause_instant));

    // The frozen clock points at `pause_instant`, not `now`, so elapsed is the
    // interval up to the pause — 5 days — regardless of `now`.
    assert_eq!(accrual::elapsed(&s, MAX_TIME), 5 * DAY);
    assert_eq!(accrual::elapsed(&s, MAX_TIME - 1), 5 * DAY);

    // Floor of 1000 over half the 10-day schedule = 500.
    assert_eq!(
        accrual::vested(&s, MAX_TIME).expect("no overflow while paused"),
        1_000 * ONE * 5_i128 / 10_i128,
    );
    // A pause-frozen probe reads the identical value at the same instant — the
    // clock does not advance while paused even at the ceiling.
    assert_eq!(
        accrual::vested(&s, pause_instant).expect("no overflow"),
        accrual::vested(&s, MAX_TIME).expect("no overflow"),
    );
}

/// `start_time` at the absolute maximum is illegal (`end <= start`), so the
/// largest legal `start` places `end` at `u64::MAX`. The math must not wrap on
/// the tightest possible high-end schedule.
#[test]
fn maximum_start_time_schedule_is_bounded() {
    let start = MAX_TIME; // not creatable, but read behavior must not trap
    let s = stream_of(1_000, start, start.saturating_add(0), start, 0, None);
    // Zero duration collapses onto the instant: returns the full deposit, never
    // divides by zero and never traps.
    assert_eq!(accrual::vested(&s, MAX_TIME).expect("no trap"), 1_000);
}

// ---------------------------------------------------------------------------
// Pure accrual: maximum rate / deposit combinations
// ---------------------------------------------------------------------------

/// The exact `deposit * duration` boundary `create_stream` enforces. A stream
/// whose product *just fits* i128 must accrue correctly at every fraction; one
/// stroop more must overflow the same way creation rejects it.
#[test]
fn vested_is_correct_at_the_deposit_times_duration_boundary() {
    let duration = 1u64 << 62; // large but safely below u64::MAX
    let d = duration as i128;

    // product == i128::MAX (modulo the lossless cast); validated below.
    let deposit = i128::MAX / d;

    let start = 1_700_000_000u64;
    let end = start + duration;
    let s = stream_of(deposit, start, end, start, 0, None);

    // Half-way through, `deposited * elapsed` still fits, so no overflow and
    // the ratio is floor-correct.
    let half = duration / 2;
    let v = accrual::vested(&s, start + half).expect("no overflow");
    assert_eq!(v, deposit * (half as i128) / d, "floor at half");

    // Full maturity returns the whole deposit exactly.
    assert_eq!(
        accrual::vested(&s, end).expect("no overflow"),
        deposit,
        "must deliver the full deposit at maturity",
    );

    // Just past maturity still returns the full deposit.
    assert_eq!(accrual::vested(&s, end + 1).expect("no overflow"), deposit,);

    // `create_stream` would accept the fit and reject overflow — mirror the
    // guard here so the pure boundary and the contract boundary agree.
    assert!(
        deposit.checked_mul(d).is_some(),
        "boundary product must fit"
    );
    // `checked_mul` returning `None` is exactly "the product exceeded i128",
    // which is the same test `create_stream` applies.
    assert!(
        (deposit + 1).checked_mul(d).is_none(),
        "one stroop more must overflow the creation guard",
    );
}

/// A near-overflow input that no `create_stream` would ever accept (product way
/// past i128) still goes through `vested` **without a panic**: it returns
/// `Err(Error::Overflow)` — a typed error, not a host trap.
#[test]
fn overflowing_vested_is_a_typed_error_never_a_trap() {
    let start = 1_700_000_000u64;
    let duration = u64::MAX >> 2;
    // A deposit at the i128 ceiling times any elapsed interval provably exceeds
    // i128, so the guarded multiplication must fail as a typed error.
    let huge_deposit = i128::MAX;
    let s = stream_of(huge_deposit, start, start + duration, start, 0, None);

    match accrual::vested(&s, start + duration / 2) {
        Ok(v) => {
            // If it came back, it must be bounded — never wrapped negative and
            // never taller than the deposit.
            assert!(v >= 0, "vested wrapped: {v}");
            assert!(v <= huge_deposit, "vested escaped: {v}");
        }
        Err(e) => assert_eq!(e, Error::Overflow, "only Overflow is an acceptable error"),
    }
}

/// `withdrawable`, `refundable` and `liability` must never panic on their
/// subtractions. `checked_sub` on two non-negative `i128` amounts never
/// overflows (the difference always fits in `i128`), so the correct
/// behaviour for an over-drawn stream is a representable result — the
/// documented saturation of `withdrawable` to zero, and the negative
/// liability reading straight through — never a wrap and never a panic.
#[test]
fn accounting_helpers_never_underflow_at_maximum_amounts() {
    let start = 1_700_000_000u64;
    let duration = 100 * DAY;
    let deposit = i128::MAX / 4;

    // Fully drawn at the `i128` ceiling: everything reads back zero/deposit.
    let mut fully = stream_of(deposit, start, start + duration, start, 0, None);
    fully.withdrawn = deposit;
    assert_eq!(accrual::withdrawable(&fully, start + duration).unwrap(), 0);
    assert_eq!(accrual::refundable(&fully, start + duration).unwrap(), 0);
    assert_eq!(accrual::liability(&fully).unwrap(), 0);

    // `withdrawn` one stroop past `deposited` saturates `withdrawable` to zero
    // (the defence-in-depth branch) and reports a -1 liability — both finite,
    // neither a wrap nor a panic.
    let mut over = stream_of(deposit, start, start + duration, start, 0, None);
    over.withdrawn = deposit + 1;
    assert_eq!(accrual::withdrawable(&over, start + duration).unwrap(), 0);
    assert_eq!(accrual::liability(&over).unwrap(), -1);

    // A cancelled (zero-duration) stream with a huge deposit reads back in full
    // through every view without a division by zero.
    let mut settled = stream_of(deposit, start, start, start, 0, None);
    settled.status = StreamStatus::Cancelled;
    assert_eq!(accrual::vested(&settled, start).unwrap(), deposit);
    assert_eq!(accrual::withdrawable(&settled, start).unwrap(), deposit);
    assert_eq!(accrual::refundable(&settled, start).unwrap(), 0);
    assert_eq!(accrual::liability(&settled).unwrap(), deposit);
}

// ---------------------------------------------------------------------------
// Property layer: nothing above ever traps
// ---------------------------------------------------------------------------

// The pure helpers must *never* panic regardless of how extreme the inputs
// are. Sampling a wide spread of `u64`-wide timestamps and deposits up to the
// `i128` ceiling, every call is allowed to `Ok` with a bounded result or
// `Err(Error::Overflow)` — but it must return, never trap.
//
// This deliberately explores inputs `test::props` does not: those generators
// keep `deposit * duration` inside i128 so every case is creation-valid. Here
// deposits are allowed far past that, because the honest contract can be fed a
// malformed or freak value and must degrade to a typed error.
proptest! {
    #![proptest_config(ProptestConfig::default())]

    #[test]
    fn accrual_never_traps_across_maximum_values(
        deposited in 1i128..=i128::MAX,
        start in (MAX_TIME - (40 * 365 * 86_400))..=MAX_TIME,
        len in 1u64..=(40 * 365 * 86_400),
        cliff_frac in 0u64..=100,
        now_delta in 0u64..=(40 * 365 * 86_400),
    ) {
        let end = start.saturating_add(len);
        // Saturate every timestamp so no generator value can wrap the u64 range
        // while building the schedule — wrapping here would be a *test* panic
        // masking the very no-trap property under test.
        let cliff = start.saturating_add(len.saturating_mul(cliff_frac) / 100);
        let now = start.saturating_add(now_delta);
        // Guard: `vested` divides by `duration` only when it is non-zero; a
        // zero-length schedule returns the deposit instead. Mirror that branch
        // so the property stays meaningful across the degenerate case.
        let s = stream_of(deposited, start, end, cliff, 0, None);
        if end == start {
            assert_eq!(accrual::vested(&s, now).expect("no trap"), deposited);
        } else {
            match accrual::vested(&s, now) {
                Ok(v) => prop_assert!(v >= 0 && v <= deposited, "wrapped: {v}"),
                Err(e) => prop_assert_eq!(e, Error::Overflow),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Contract level: maximum timestamps through the real entry points
// ---------------------------------------------------------------------------

/// A stream created out at the furthest horizon the host can store entries for
/// must round-trip through the contract: create, accrue, view, withdraw,
/// deplete — all without a host trap. The deposit stays within the harness's
/// real token supply so the transfers succeed; the timestamps are what push the
/// arithmetic (and the derived TTL) to the storage boundary.
///
/// The true `u64::MAX` ceiling is exercised by the pure accrual tests above;
/// a `Stream` stored at that horizon cannot be persisted by the harness host
/// (see [`MAX_DEPLOYABLE_END`]), which is a storage-layer limit, not accrual
/// math.
#[test]
fn contract_handles_maximum_deployable_timestamps_without_a_trap() {
    let h = Harness::new();
    let start = MAX_DEPLOYABLE_END - 10 * DAY;
    let end = MAX_DEPLOYABLE_END;
    // Deposit keeps the product well inside i128: 1000 * 10 days.
    let deposit = 1_000 * ONE;

    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &h.token,
        &deposit,
        &start,
        &end,
        &start,
        &true,
        &true,
        &true,
    );
    h.assert_pool_exact();

    // Partway through the schedule, exactly as at a normal timestamp.
    h.warp_to(start + 5 * DAY);
    let s = h.get(id);
    assert_eq!(s.end_time, end, "end_time preserved at the ceiling");
    assert_eq!(h.client.vested_of(&id), deposit / 2);

    // Drain the tail at maturity.
    h.warp_to(end);
    assert_eq!(h.client.vested_of(&id), deposit);
    assert_eq!(h.client.withdraw(&id, &None), deposit);
    h.assert_pool_exact();
}

/// `create_stream` must reject a deposit whose `deposit * duration` product
/// would overflow accrual at near-maximum duration. The rejection happens
/// during validation, before any token moves, so it needs no extra funding and
/// the failed attempt is a typed error — never a host trap from the transfer.
#[test]
fn contract_rejects_deposit_that_would_overflow_at_maximum_duration() {
    let h = Harness::new();
    let start = h.now();
    let duration = u64::MAX >> 2; // a genuinely u64-scale duration
    let d = duration as i128;

    // Maximal deposit create_stream would ever accept for this duration.
    let fits = i128::MAX / d;

    // A product one stroop over the i128 ceiling rejects with the typed error.
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &(fits + 1),
            &start,
            &(start + duration),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Overflow);

    // The rejected call moved nothing: no stream, no funds.
    assert_eq!(h.client.stream_count(), 0);
    assert_eq!(h.pool(), 0);
}

/// A top-up whose extension arithmetic would overflow (`amount * duration`
/// past i128, or a `delta` that would push `end_time` past `u64::MAX`) must
/// fail with a typed `Error::Overflow` and leave state untouched — never an
/// opaque host trap and never a partial transfer.
#[test]
fn top_up_overflowing_the_duration_is_a_typed_error() {
    let h = Harness::new();
    let start = MAX_DEPLOYABLE_END - 10 * DAY;
    // Very lean rate so even a modest amount buys a large extension.
    let duration = 10 * DAY;
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &h.token,
        &(duration as i128),
        &start,
        &(start + duration),
        &start,
        &true,
        &true,
        &true,
    );

    // The vast amount overflows `amount * duration` in the very first guarded
    // multiplication, and were that not so the resulting delta would overflow
    // `end_time` past u64::MAX. Either way: a typed Overflow, never a trap.
    let err = h
        .client
        .try_top_up(&id, &(i128::MAX / 2))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Overflow);

    // The failed top-up is a no-op.
    let s = h.get(id);
    assert_eq!(s.deposited, duration as i128);
    assert_eq!(s.end_time, start + duration);
    h.assert_pool_exact();
}

/// The whole lifecycle at `u64`-wide timestamps, including pause/resume and
/// cancel, must stay typed — the changes to `paused_total` and `end_time` are
/// exactly the arithmetic the issue is about.
#[test]
fn lifecycle_stays_typed_at_maximum_deployable_timestamps() {
    let h = Harness::new();
    let start = MAX_DEPLOYABLE_END - 10 * DAY;
    let end = MAX_DEPLOYABLE_END;
    let deposit = 1_000 * ONE;
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &h.token,
        &deposit,
        &start,
        &end,
        &(start + 2 * DAY),
        &true,
        &true,
        &true,
    );

    h.warp_to(start + 4 * DAY);
    h.client.pause(&id);
    h.warp_to(start + 6 * DAY); // frozen across the cliff
    assert_eq!(h.client.vested_of(&id), deposit * 4 / 10);

    h.client.resume(&id);
    h.assert_pool_exact();

    // Now matured on the stretched clock; cancelling settles against the
    // current vesting without inverting the schedule.
    h.client.cancel(&id);
    let s = h.get(id);
    assert!(s.end_time >= s.start_time, "cancel inverted the schedule");
    h.assert_pool_exact();
}
