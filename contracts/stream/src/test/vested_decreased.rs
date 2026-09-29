//! Issue #1879 — `Error::VestedDecreased` must be unreachable through any
//! entry point.
//!
//! # The claim under test
//!
//! `Error::VestedDecreased` (33) is a defensive guard on the four mutating
//! entry points that can rewrite an input to [`crate::accrual::vested`]:
//! `top_up`, `pause`, `resume` and `transfer_recipient`. Each records `vested`
//! at the call's ledger timestamp, mutates the schedule, and rejects the call
//! if the recomputed `vested` is lower.
//!
//! `test::error_reachability` classifies the variant as reserved and
//! `docs/ABI.md` documents it as defensive — but a classification is a claim,
//! not a proof, and no test in the suite *searched* for an operation sequence
//! that drives the guard out of a public call. This module is that search.
//!
//! # How the search works
//!
//! Random operation sequences — the four guard-bearing calls interleaved with
//! withdrawals, TTL maintenance and clock advances — run against the three
//! stream shapes the issue names: **paused**, **cliffed**, and
//! **near-maximum**. After every call the search asserts two things:
//!
//! 1. **The guard did not fire.** The observed contract error is never
//!    [`Error::VestedDecreased`]. This is deliberately stronger than checking
//!    state: a firing guard reverts the whole invocation, so the rollback
//!    would hide the event from any post-hoc state comparison.
//! 2. **Invariant I3 held.** `vested` at the frozen call timestamp is
//!    non-decreasing across the transition — the very property the guard
//!    exists to protect.
//!
//! If a case ever trips the guard, [`search`] returns the full operation trace,
//! which is exactly the regression fixture the issue asks for.
//!
//! # Every entry point, not just the carriers
//!
//! The four `delegate_*` counterparts rewrite the same schedule fields and
//! carry **no** guard. [`every_entry_point_that_rewrites_the_schedule_is_covered`]
//! drives the public and delegate entry points on one stream and asserts that
//! none of them, guard or not, yields 33 — so "unreachable through any entry
//! point" is checked literally rather than inferred from the four guard sites.
//!
//! `test::monotonicity` proves the same transition property deterministically
//! over every ordering of a small operation set; this module complements it
//! with randomized search over longer sequences and the named stream shapes.

use proptest::prelude::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, InvokeError};

use super::common::*;
use crate::{op, Error};

// ---------------------------------------------------------------------------
// Operation model
// ---------------------------------------------------------------------------

/// Stream shape a search starts from. The issue's "Validation" line names
/// exactly these three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// Created, then paused before the search begins.
    Paused,
    /// A real cliff at 60% of the schedule, with the search starting one
    /// second before the gate opens.
    Cliffed,
    /// Deposit near `i128::MAX / (8 * duration)` so the accrual arithmetic
    /// runs close to its ceiling without tripping `Overflow` first.
    NearMaximum,
}

/// A single generated step: which call to make, how long to advance first, and
/// (for `top_up`) how many "seconds' worth" of the rate to add.
#[derive(Clone, Copy, Debug)]
struct Step {
    op: u8,
    /// Seconds to advance before the call. Zero keeps the clock frozen.
    gap: u32,
    /// Multiplier on `deposit / duration` for a top-up amount.
    scale: u32,
}

impl Step {
    fn new(op: u8, gap: u32, scale: u32) -> Step {
        Step { op, gap, scale }
    }
}

/// Step opcodes. 0..=3 carry the guard; 4 and 5 interleave state that makes the
/// search reach different `withdrawn` / TTL states.
const TOP_UP: u8 = 0;
const PAUSE: u8 = 1;
const RESUME: u8 = 2;
const TRANSFER: u8 = 3;
const WITHDRAW: u8 = 4;
const EXTEND_TTL: u8 = 5;
const OP_UNIVERSE: u8 = 6;

fn op_name(op: u8) -> &'static str {
    match op {
        TOP_UP => "top_up",
        PAUSE => "pause",
        RESUME => "resume",
        TRANSFER => "transfer_recipient",
        WITHDRAW => "withdraw",
        EXTEND_TTL => "extend_stream_ttl",
        _ => "unknown",
    }
}

/// Every entry point that can rewrite one of the five inputs to `vested`.
///
/// The first four carry the guard; the `delegate_*` four rewrite the same
/// fields without it. Testing all eight is what makes "any entry point" literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryPoint {
    TopUp,
    Pause,
    Resume,
    Transfer,
    DelegateTopUp,
    DelegatePause,
    DelegateResume,
    DelegateTransfer,
}

impl EntryPoint {
    fn name(self) -> &'static str {
        match self {
            EntryPoint::TopUp => "top_up",
            EntryPoint::Pause => "pause",
            EntryPoint::Resume => "resume",
            EntryPoint::Transfer => "transfer_recipient",
            EntryPoint::DelegateTopUp => "delegate_top_up",
            EntryPoint::DelegatePause => "delegate_pause",
            EntryPoint::DelegateResume => "delegate_resume",
            EntryPoint::DelegateTransfer => "delegate_transfer_recipient",
        }
    }

    /// Call the entry point, returning only the contract-level outcome.
    fn call(self, h: &Harness, id: u64, agent: &Address) -> std::result::Result<(), Error> {
        match self {
            EntryPoint::TopUp => contract_outcome(h.client.try_top_up(&id, &(10 * ONE))),
            EntryPoint::Pause => contract_outcome(h.client.try_pause(&id)),
            EntryPoint::Resume => contract_outcome(h.client.try_resume(&id)),
            EntryPoint::Transfer => {
                contract_outcome(h.client.try_transfer_recipient(&id, &h.other))
            }
            EntryPoint::DelegateTopUp => {
                contract_outcome(h.client.try_delegate_top_up(&id, agent, &(10 * ONE)))
            }
            EntryPoint::DelegatePause => contract_outcome(h.client.try_delegate_pause(&id, agent)),
            EntryPoint::DelegateResume => {
                contract_outcome(h.client.try_delegate_resume(&id, agent))
            }
            EntryPoint::DelegateTransfer => contract_outcome(
                h.client
                    .try_delegate_transfer_recipient(&id, agent, &h.recipient),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The search
// ---------------------------------------------------------------------------

/// Flatten a generated `try_*` into the contract-level outcome, discarding any
/// success value.
///
/// Soroban's generated client returns
/// `Result<Result<T, E>, Result<Error, InvokeError>>`, where the inner error
/// `E` is `Error` for some entry points and `ConversionError` for others, and
/// the outer error distinguishes a typed contract error (`Ok(e)`) from a host
/// error (`Err(_)`). Only the contract error is of interest; host errors and
/// conversion errors cannot be discriminant 33, so they collapse to `Ok(())`.
fn contract_outcome<T, E>(
    res: std::result::Result<std::result::Result<T, E>, std::result::Result<Error, InvokeError>>,
) -> std::result::Result<(), Error> {
    match res {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) => Ok(()),
        Err(Ok(e)) => Err(e),
        Err(Err(_)) => Ok(()),
    }
}

/// Build the stream the search runs against and return `(id, deposit, duration)`.
fn build(h: &Harness, shape: Shape, start: u64) -> (u64, i128, u64) {
    match shape {
        Shape::Paused => {
            let duration = 997;
            let deposit = 1_000 * ONE;
            let id = h.create(deposit, start, start + duration, start, true, true, true);
            h.client.pause(&id);
            (id, deposit, duration)
        }
        Shape::Cliffed => {
            let duration = 997;
            let deposit = 1_000 * ONE;
            let cliff = start + duration * 3 / 5;
            let id = h.create(deposit, start, start + duration, cliff, true, true, true);
            // Start one second before the gate so the first advance can cross it.
            h.advance(duration * 3 / 5 - 1);
            (id, deposit, duration)
        }
        Shape::NearMaximum => {
            let duration = 1_000;
            // `deposited * duration` and `new_deposited * new_duration` stay
            // comfortably inside i128, so the case stresses the monotonicity
            // guard instead of returning `Overflow` before reaching it.
            let deposit = i128::MAX / (8 * duration as i128);
            h.token_admin.mint(&h.sender, &(deposit * 3));
            let id = h.create(deposit, start, start + duration, start, true, true, true);
            (id, deposit, duration)
        }
    }
}

/// Run one generated sequence. `Ok(())` means no case fired the guard and I3
/// held throughout; `Err(trace)` is the reproducer for the first violation.
fn search(shape: Shape, steps: &[Step]) -> std::result::Result<(), std::string::String> {
    let h = Harness::new();
    let start = h.now();
    let (id, deposit, duration) = build(&h, shape, start);
    let unit = deposit / duration as i128;

    let mut trace = std::format!("shape={shape:?} deposit={deposit} duration={duration}");

    for (index, step) in steps.iter().enumerate() {
        if step.gap > 0 {
            h.advance(step.gap as u64);
        }
        trace.push_str(&std::format!(
            "\n  [{index}] +{}s {}",
            step.gap,
            op_name(step.op)
        ));

        let before = h.vested_snapshot();

        let outcome = match step.op {
            TOP_UP => {
                // At least one stroop, scaled to the stream's rate so the
                // amount is meaningful for both a 10^10 and a 10^34 deposit.
                let amount = unit * step.scale as i128 + 1;
                trace.push_str(&std::format!("(amount={amount})"));
                contract_outcome(h.client.try_top_up(&id, &amount))
            }
            PAUSE => contract_outcome(h.client.try_pause(&id)),
            RESUME => contract_outcome(h.client.try_resume(&id)),
            TRANSFER => contract_outcome(h.client.try_transfer_recipient(&id, &h.other)),
            WITHDRAW => contract_outcome(h.client.try_withdraw(&id, &None)),
            _ => contract_outcome(h.client.try_extend_stream_ttl(&id)),
        };

        if outcome == Err(Error::VestedDecreased) {
            return Err(std::format!(
                "VestedDecreased (#33) is reachable — the guard fired.\n{trace}"
            ));
        }

        // I3, at the frozen call timestamp.
        let after = h.vested_snapshot();
        for (stream_id, (prev, next)) in before.iter().zip(after.iter()).enumerate() {
            if next < prev {
                return Err(std::format!(
                    "I3 violated at a fixed instant — stream {stream_id} \
                     vested moved backwards, {prev} -> {next}.\n{trace}"
                ));
            }
        }
        h.assert_invariants();
    }
    Ok(())
}

/// Rejection-free strategy for a bounded operation sequence.
fn steps_strategy() -> impl Strategy<Value = std::vec::Vec<Step>> {
    prop::collection::vec((0u8..OP_UNIVERSE, 0u32..700, 0u32..=6), 1..=16).prop_map(|raw| {
        raw.into_iter()
            .map(|(op, gap, scale)| Step::new(op, gap, scale))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    // Default config reads PROPTEST_CASES from the environment (256 locally,
    // larger in CI); pinning it here would silently cap the nightly sweep.
    #![proptest_config(ProptestConfig::default())]

    /// **The search.** Randomize the stream shape and the operation sequence;
    /// no case may produce `Error::VestedDecreased`, and no transition may move
    /// `vested` backwards at a frozen instant.
    ///
    /// A failure prints the operation trace, which is the regression fixture
    /// the issue asks for if the guard ever becomes reachable.
    #[test]
    fn randomized_operation_sequences_never_produce_vested_decreased(
        shape_code in 0u8..3u8,
        steps in steps_strategy(),
    ) {
        let shape = match shape_code {
            0 => Shape::Paused,
            1 => Shape::Cliffed,
            _ => Shape::NearMaximum,
        };
        if let Err(trace) = search(shape, &steps) {
            prop_assert!(false, "{}", trace);
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic coverage
// ---------------------------------------------------------------------------

/// A fixed sequence long enough to pause, resume with a non-zero
/// `paused_total`, top up across the resulting clock shift, withdraw, and
/// transfer — the orderings the randomized search samples. Run once per named
/// shape so the three shapes are covered even when `PROPTEST_CASES` is small.
#[test]
fn named_shapes_are_searched_deterministically() {
    let fixed = [
        Step::new(TOP_UP, 0, 1),
        Step::new(PAUSE, 41, 0),
        Step::new(RESUME, 120, 0),
        Step::new(TOP_UP, 17, 3),
        Step::new(WITHDRAW, 200, 0),
        // Pause again after a resume, so `paused_total > 0` while the schedule
        // is frozen — the state most likely to move `vested` backwards.
        Step::new(PAUSE, 0, 0),
        Step::new(TOP_UP, 90, 2),
        Step::new(RESUME, 30, 0),
        Step::new(TRANSFER, 260, 0),
        Step::new(TOP_UP, 150, 5),
        Step::new(EXTEND_TTL, 0, 0),
    ];

    for shape in [Shape::Paused, Shape::Cliffed, Shape::NearMaximum] {
        if let Err(trace) = search(shape, &fixed) {
            panic!("{trace}");
        }
    }
}

/// **Every entry point that can rewrite the schedule, public and delegate.**
///
/// The guard lives on `top_up`, `pause`, `resume` and `transfer_recipient`,
/// but their `delegate_*` counterparts rewrite the same fields with no guard at
/// all. "Unreachable through any entry point" therefore has to be checked
/// against every one of them, not just the guard sites.
#[test]
fn every_entry_point_that_rewrites_the_schedule_is_covered() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(1_000 * ONE, start, start + 997, start, true, true, true);

    let agent = Address::generate(&h.env);
    // Sender-side grants cover pause/resume/top-up; the recipient issues the
    // transfer grant.
    h.client.grant_delegate(
        &id,
        &h.sender,
        &agent,
        &(op::PAUSE | op::RESUME | op::TOP_UP),
        &None,
    );
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);
    h.advance(311);

    // Ordered so every call lands on its live precondition: top-up, pause,
    // resume, transfer — then the same again through the delegate paths.
    let entries = [
        EntryPoint::TopUp,
        EntryPoint::Pause,
        EntryPoint::Resume,
        EntryPoint::Transfer,
        EntryPoint::DelegateTopUp,
        EntryPoint::DelegatePause,
        EntryPoint::DelegateResume,
        EntryPoint::DelegateTransfer,
    ];

    for entry in entries {
        let before = h.vested_snapshot();
        let outcome = entry.call(&h, id, &agent);
        assert_ne!(
            outcome,
            Err(Error::VestedDecreased),
            "{} produced Error::VestedDecreased (#33)",
            entry.name(),
        );
        h.assert_no_vested_regression(&before, entry.name());
        h.assert_invariants();
    }
    h.assert_pool_exact();
}
