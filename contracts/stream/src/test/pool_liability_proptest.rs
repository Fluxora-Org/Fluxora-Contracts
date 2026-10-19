//! Issue #1857 — the contract's token balance always covers the summed live
//! liability.
//!
//! `accrual::liability` is the outstanding claim of one stream against the
//! pooled balance: everything deposited that has not yet left the contract
//! (`deposited - withdrawn`). The contract is sound only while the pool holds
//! at least the sum of those claims across every live stream:
//!
//! ```text
//! token.balance(contract) >= Σ accrual::liability(stream)   for every stream
//! ```
//!
//! If that ever fails, some recipient's claim is unbacked: the accounting says
//! they are owed tokens the contract cannot pay. `Harness::assert_pool_invariant`
//! checks the inequality at hand-picked points; this module turns it into a
//! generated property over randomized operation sequences, re-audited after
//! **every** step so an unforeseen interaction between create, withdraw, batch
//! withdraw, cancel, top-up, pause/resume, recipient transfer and delegated
//! withdrawal cannot slip between assertions.
//!
//! # The audit
//!
//! The property is stated once, as a pure function of what is held and what is
//! claimed ([`audit_pool`]), so the negative controls below can feed it a state
//! the contract could never reach and observe the verdict directly, rather than
//! having to trust that an assertion would have fired:
//!
//! | verdict | meaning |
//! |---|---|
//! | [`PoolAudit::Balanced`] | coverage holds and nothing is stranded |
//! | [`PoolAudit::Unbacked`] | the **#1857** failure — a claim the pool cannot cover |
//! | [`PoolAudit::Surplus`] | the pool holds more than every stream claims |
//!
//! A negative per-stream liability is rejected as unbacked too: a claim is a
//! non-negative quantity, and a negative term would *mask* a genuine shortfall
//! in the sum rather than reveal it. That is why the property depends on
//! `withdraw`'s `requested > available` (`InsufficientWithdrawable`) guard —
//! see the guard-removal note at the bottom of this file.
//!
//! The generated sequences never donate loose tokens to the contract, so the
//! stronger exactness companion (`pool == Σ liability`, "no surplus with no
//! claimant") is asserted on the same traces. Coverage is the issue's property;
//! exactness is what [`the_property_rejects_a_surplus_with_no_claimant`] makes
//! non-vacuous.
//!
//! # Reproducibility
//!
//! Every sequence is a pure function of its seed and its generated schedule.
//! The proptest block generates both, so a failure is reported with the
//! minimal failing input (including `sequence_seed`) printed by proptest, and
//! the panicking audit repeats `seed <N>, step <M>` in its own message. Replay
//! the printed input (or the fixed seed used by the five named tests) and the
//! exact sequence reappears; proptest additionally records the failing RNG seed
//! in the usual `.proptest-regressions` file beside this module. The block uses
//! `ProptestConfig::default()` exactly as `test::props` and
//! `test::accounting_identity` do, so it honours the job's `PROPTEST_CASES`
//! budget rather than pinning its own case count.

use proptest::prelude::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

use super::common::*;
use crate::{accrual, op, DataKey, StreamStatus};

// ---------------------------------------------------------------------------
// The audit — the property, stated as a pure function
// ---------------------------------------------------------------------------

/// Why the pool-coverage property failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unbacked {
    /// One stream's own liability exceeds the entire pooled balance.
    UncoveredClaim {
        index: usize,
        claim: i128,
        held: i128,
    },
    /// A liability came out negative. A claim is never negative, and a negative
    /// term would hide a shortfall in the sum, so the property rejects it.
    NegativeClaim { index: usize, claim: i128 },
    /// The summed live liability is not backed by the pooled balance.
    SumExceedsHeld { held: i128, liability: i128 },
}

/// The result of auditing the pooled balance against the summed liability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoolAudit {
    /// Coverage holds and the pool holds exactly the outstanding claims.
    Balanced,
    /// The **#1857** failure: at least one claim is unbacked.
    Unbacked(Unbacked),
    /// Coverage holds, but the pool holds more than every stream claims —
    /// tokens stranded with no claimant.
    Surplus { held: i128, liability: i128 },
}

/// **The property.** Pure, so the negative controls can exercise it directly.
///
/// Returns [`PoolAudit::Unbacked`] the moment any claim is negative, exceeds
/// the whole pool, or the summed claims exceed the pool.
fn audit_pool(held: i128, liabilities: &[i128]) -> PoolAudit {
    let mut total: i128 = 0;
    for (index, claim) in liabilities.iter().enumerate() {
        if *claim < 0 {
            return PoolAudit::Unbacked(Unbacked::NegativeClaim {
                index,
                claim: *claim,
            });
        }
        if *claim > held {
            return PoolAudit::Unbacked(Unbacked::UncoveredClaim {
                index,
                claim: *claim,
                held,
            });
        }
        total = total
            .checked_add(*claim)
            .expect("summed liability must not overflow i128");
    }
    if held < total {
        return PoolAudit::Unbacked(Unbacked::SumExceedsHeld {
            held,
            liability: total,
        });
    }
    if held > total {
        return PoolAudit::Surplus {
            held,
            liability: total,
        };
    }
    PoolAudit::Balanced
}

/// Outstanding liability of every stream the contract knows about, in id order.
///
/// Every record is included regardless of status: a cancelled stream with an
/// undrawn tail is still a live claim, and a settled one contributes liability
/// zero. All streams share the harness token.
fn live_liabilities(h: &Harness) -> std::vec::Vec<i128> {
    let mut out = std::vec::Vec::new();
    for id in 0..h.client.stream_count() {
        let stream = h.get(id);
        if stream.token != h.token {
            continue;
        }
        out.push(accrual::liability(&stream).expect("liability must not overflow"));
    }
    out
}

/// Audit the current contract state.
fn contract_audit(h: &Harness) -> PoolAudit {
    audit_pool(h.pool(), &live_liabilities(h))
}

/// Assert the #1857 property for the current state. Panics — which proptest
/// records as a case failure and then shrinks — when any claim is unbacked.
fn assert_covers(h: &Harness, ctx: &str) {
    if let PoolAudit::Unbacked(reason) = contract_audit(h) {
        panic!(
            "{ctx}: the pooled balance does not cover the summed live liability \
             ({reason:?}); pooled {}, summed liability {}",
            h.pool(),
            live_liabilities(h).iter().sum::<i128>(),
        );
    }
}

/// Assert the exactness companion: no tokens are pooled with no claimant.
fn assert_no_stranded_liability(h: &Harness, ctx: &str) {
    if let PoolAudit::Surplus { held, liability } = contract_audit(h) {
        panic!(
            "{ctx}: {} pooled with no claimant (pooled {held}, summed liability {liability})",
            held - liability,
        );
    }
}

/// Re-audit after one step: coverage (the issue's property) *and* the
/// exactness companion. The sequences here never donate loose tokens, so a
/// surplus is a bug in the same class. Returns the summed liability so callers
/// can track that the traces are non-trivial.
fn check_after_step(h: &Harness, ctx: &str) -> i128 {
    let liabilities = live_liabilities(h);
    let total: i128 = liabilities.iter().sum();
    match audit_pool(h.pool(), &liabilities) {
        PoolAudit::Balanced => {}
        PoolAudit::Unbacked(reason) => panic!(
            "{ctx}: the pooled balance does not cover the summed live liability \
             ({reason:?}); pooled {}, summed liability {total}",
            h.pool(),
        ),
        PoolAudit::Surplus { held, liability } => panic!(
            "{ctx}: {} pooled with no claimant (pooled {held}, summed liability {liability})",
            held - liability,
        ),
    }
    total
}

// ---------------------------------------------------------------------------
// Generated operation sequences
// ---------------------------------------------------------------------------

/// xorshift64*. Deterministic and seedable, so a failure replays from its seed
/// alone — the convention set by `test::invariants` and
/// `test::stream_count_consistency`.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        // xorshift is a fixed point at zero.
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

// Operation codes. The schedule is a `Vec<(code, advance_raw)>`; the advance
// is folded into a bounded number of seconds between steps.
const OP_CREATE: u8 = 0;
const OP_WITHDRAW: u8 = 1;
const OP_WITHDRAW_ALL: u8 = 2;
const OP_CANCEL: u8 = 3;
const OP_PAUSE: u8 = 4;
const OP_RESUME: u8 = 5;
const OP_TOP_UP: u8 = 6;
const OP_TRANSFER: u8 = 7;
const OP_BATCH_WITHDRAW: u8 = 8;
const OP_DELEGATE_WITHDRAW: u8 = 9;
const OP_DELEGATE_CANCEL: u8 = 10;
const OP_EXTEND_TTL: u8 = 11;
/// Number of distinct operation codes; schedules draw from `0..OP_CODES`.
const OP_CODES: u8 = 12;

/// Longest generated schedule. Bounded so a case stays a few milliseconds and
/// so a shrunk failure is readable.
const MAX_STEPS: usize = 24;

/// What a seeded sequence actually exercised. `the_seeded_sequence_is_a_real_check`
/// uses this to prove the property is not vacuously true over a run of no-ops.
#[derive(Debug, Default, Clone, Copy)]
struct SequenceStats {
    creates: u32,
    withdrawals: u32,
    cancels: u32,
    pauses: u32,
    resumes: u32,
    top_ups: u32,
    transfers: u32,
    batch_withdrawals: u32,
    delegate_withdrawals: u32,
    steps: u32,
    /// Largest summed liability observed at any audit point.
    max_liability: i128,
    /// Largest pooled balance observed at any audit point.
    max_pool: i128,
    /// Whether the summed liability was strictly positive at some step.
    saw_nonzero_liability: bool,
}

/// Deterministically build a schedule from a seed, for the named tests that do
/// not use the proptest block.
fn schedule_from_seed(seed: u64, len: usize) -> std::vec::Vec<(u8, u32)> {
    let mut rng = Rng::new(seed ^ 0x9E37_79B9_7F4A_7C15);
    (0..len)
        .map(|_| (rng.below(OP_CODES as u64) as u8, rng.next() as u32))
        .collect()
}

/// Open `count` streams of varied shape. Deposits always clear the contract's
/// one-stroop-per-second floor (`deposit >= duration`), so every create here is
/// valid and `h.create` may panic rather than being `try_`-guarded.
fn open_population(h: &Harness, rng: &mut Rng, count: u64) {
    for i in 0..count {
        let start = h.now() + rng.below(5 * DAY);
        let duration = DAY + rng.below(30 * DAY);
        let cliff = start + rng.below(duration + 1);
        let deposit = (1 + rng.below(400)) as i128 * ONE;
        h.create(
            deposit,
            start,
            start + duration,
            cliff,
            i % 2 == 0,
            i % 3 != 0,
            true,
        );
    }
}

/// One `create_stream`, counted only if the ABI accepted it.
fn create_one(h: &Harness, rng: &mut Rng, stats: &mut SequenceStats) -> &'static str {
    let start = h.now() + rng.below(5 * DAY);
    let duration = DAY + rng.below(30 * DAY);
    let cliff = start + rng.below(duration + 1);
    let deposit = (1 + rng.below(400)) as i128 * ONE;
    let cancellable = rng.below(2) == 0;
    let pausable = rng.below(3) != 0;
    let transferable = rng.below(2) == 0;

    let accepted = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &deposit,
            &start,
            &(start + duration),
            &cliff,
            &cancellable,
            &pausable,
            &transferable,
            &None,
        )
        .is_ok();
    if accepted {
        stats.creates += 1;
    }
    "create"
}

/// Apply one generated operation. Every call goes through the public ABI with
/// `try_`, because many are legitimately rejected (paused twice, not
/// cancellable, nothing accrued yet); a rejection must leave state untouched,
/// which the audit after the step confirms. Returns a label for failure output.
fn apply_operation(
    h: &Harness,
    rng: &mut Rng,
    code: u8,
    stats: &mut SequenceStats,
) -> &'static str {
    let count = h.client.stream_count();
    if count == 0 || code == OP_CREATE {
        return create_one(h, rng, stats);
    }
    let id = rng.below(count);

    match code {
        OP_WITHDRAW => {
            let amount = (1 + rng.below(50)) as i128 * ONE;
            if h.client.try_withdraw(&id, &Some(amount)).is_ok() {
                stats.withdrawals += 1;
            }
            "withdraw"
        }
        OP_WITHDRAW_ALL => {
            if h.client.try_withdraw(&id, &None).is_ok() {
                stats.withdrawals += 1;
            }
            "withdraw-all"
        }
        OP_CANCEL => {
            if h.client.try_cancel(&id).is_ok() {
                stats.cancels += 1;
            }
            "cancel"
        }
        OP_PAUSE => {
            if h.client.try_pause(&id).is_ok() {
                stats.pauses += 1;
            }
            "pause"
        }
        OP_RESUME => {
            if h.client.try_resume(&id).is_ok() {
                stats.resumes += 1;
            }
            "resume"
        }
        OP_TOP_UP => {
            let amount = (1 + rng.below(20)) as i128 * ONE;
            if h.client.try_top_up(&id, &amount).is_ok() {
                stats.top_ups += 1;
            }
            "top_up"
        }
        OP_TRANSFER => {
            let to = if rng.below(2) == 0 {
                h.other.clone()
            } else {
                h.recipient.clone()
            };
            if h.client.try_transfer_recipient(&id, &to).is_ok() {
                stats.transfers += 1;
            }
            "transfer_recipient"
        }
        OP_BATCH_WITHDRAW => {
            // The batch requires a single shared recipient, so only offer ids
            // that currently belong to `h.recipient`.
            let mut shared = std::vec::Vec::new();
            for candidate in 0..count {
                if h.get(candidate).recipient == h.recipient {
                    shared.push(candidate);
                }
            }
            if shared.len() >= 2 {
                shared.truncate(4);
                if h.client
                    .try_batch_withdraw(&h.recipient, &h.ids(&shared))
                    .is_ok()
                {
                    stats.batch_withdrawals += 1;
                }
            }
            "batch_withdraw"
        }
        OP_DELEGATE_WITHDRAW => {
            let recipient = h.get(id).recipient;
            let _ = h
                .client
                .try_grant_delegate(&id, &recipient, &h.other, &op::WITHDRAW, &None);
            if h.client.try_delegate_withdraw(&id, &h.other, &None).is_ok() {
                stats.delegate_withdrawals += 1;
            }
            "delegate_withdraw"
        }
        OP_DELEGATE_CANCEL => {
            let _ = h
                .client
                .try_grant_delegate(&id, &h.sender, &h.other, &op::CANCEL, &None);
            if h.client.try_delegate_cancel(&id, &h.other).is_ok() {
                stats.cancels += 1;
            }
            "delegate_cancel"
        }
        OP_EXTEND_TTL => {
            let _ = h.client.try_extend_stream_ttl(&id);
            "extend_stream_ttl"
        }
        // The strategy only draws `0..OP_CODES`, so this is unreachable; it
        // keeps the match exhaustive over `u8` without a same-body arm.
        _ => "unknown_op",
    }
}

/// Drive one full generated sequence: open a population, apply every operation,
/// advance the clock, and re-audit after every single step.
fn drive_sequence(seed: u64, population: u64, schedule: &[(u8, u32)]) -> SequenceStats {
    let h = Harness::new();
    let mut rng = Rng::new(seed);
    let mut stats = SequenceStats::default();

    open_population(&h, &mut rng, population);
    stats.creates += population as u32;
    stats.steps = schedule.len() as u32;
    record(&h, &mut stats, seed, 0, "population");

    for (index, (code, advance_raw)) in schedule.iter().enumerate() {
        let step = (index + 1) as u32;
        let label = apply_operation(&h, &mut rng, *code, &mut stats);
        // Time moves between operations, sometimes a lot.
        h.advance(1 + (*advance_raw as u64) % (5 * DAY));
        record(&h, &mut stats, seed, step, label);
    }

    stats
}

/// Audit after a step and fold the outcome into `stats`. The seed and step go
/// into the failure context, so a failing case is reproducible from the message
/// alone as well as from proptest's printed minimal input.
fn record(h: &Harness, stats: &mut SequenceStats, seed: u64, step: u32, label: &str) {
    let total = check_after_step(h, &std::format!("seed {seed}, step {step} ({label})"));
    stats.max_liability = stats.max_liability.max(total);
    stats.max_pool = stats.max_pool.max(h.pool());
    if total > 0 {
        stats.saw_nonzero_liability = true;
    }
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

/// Credit a stream with a liability the pool does not hold, by rewriting its
/// record in place. This is the failure the property exists to catch and is not
/// reachable through the public ABI — which is the point of a negative control.
fn forge_deposit(h: &Harness, id: u64, extra: i128) {
    let mut stream = h.get(id);
    stream.deposited += extra;
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .set(&DataKey::Stream(id), &stream);
    });
}

/// AC: the property holds across **seeded** operation sequences, audited after
/// every step. Fixed seeds make a failure replay exactly.
#[test]
fn the_property_holds_across_seeded_operation_sequences() {
    for i in 0..6u64 {
        let seed = 0x1857_0001u64.wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let schedule = schedule_from_seed(seed, 32);
        let stats = drive_sequence(seed, 1 + (i % 4), &schedule);
        assert_eq!(
            stats.steps as usize,
            schedule.len(),
            "seed {seed}: every scheduled step must be audited",
        );
    }
}

/// AC: the property rejects an unbacked liability. A negative control, so the
/// property cannot pass vacuously: first the honest state audits clean, then
/// forging claims the pool cannot cover must be reported.
#[test]
fn the_property_rejects_an_unbacked_liability() {
    let h = Harness::new();
    // Two streams so the *sum* can exceed the pool while each individual claim
    // still fits inside it — the exact shape the summed-liability property is
    // about.
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    // The honest state audits clean.
    assert_eq!(
        contract_audit(&h),
        PoolAudit::Balanced,
        "the true state must pass before the control is forged",
    );
    assert!(
        catch_unwind(AssertUnwindSafe(|| assert_covers(&h, "control baseline"))).is_ok(),
        "the honest state must pass assert_covers",
    );
    assert!(h.client.stream_exists(&b));

    // Credit stream `a` with 50 tokens the pool does not hold: its claim grows
    // to 150 (still covered on its own, the pool holds 200), but the summed
    // liability becomes 250 > 200.
    forge_deposit(&h, a, 50 * ONE);

    match contract_audit(&h) {
        PoolAudit::Unbacked(Unbacked::SumExceedsHeld { held, liability }) => {
            assert_eq!(held, 200 * ONE, "the pool is unchanged by the forgery");
            assert_eq!(liability, 250 * ONE, "the summed claim grew");
        }
        other => panic!("an unbacked liability must be reported, got {other:?}"),
    }

    // The property's own assertion reports it, so the checker is not a
    // separate code path that merely returns a value nobody acts on.
    let panicked =
        catch_unwind(AssertUnwindSafe(|| assert_covers(&h, "unbacked control"))).is_err();
    assert!(panicked, "assert_covers must reject an unbacked liability");

    let step_panicked = catch_unwind(AssertUnwindSafe(|| {
        check_after_step(&h, "step 0 (unbacked)")
    }))
    .is_err();
    assert!(
        step_panicked,
        "the per-step audit must reject an unbacked liability",
    );

    // The remaining rejection arms, on the same pure audit the property uses.
    assert_eq!(
        audit_pool(50 * ONE, &[60 * ONE]),
        PoolAudit::Unbacked(Unbacked::UncoveredClaim {
            index: 0,
            claim: 60 * ONE,
            held: 50 * ONE,
        }),
        "a single claim larger than the pool is unbacked",
    );
    assert_eq!(
        audit_pool(100 * ONE, &[-1]),
        PoolAudit::Unbacked(Unbacked::NegativeClaim {
            index: 0,
            claim: -1,
        }),
        "a negative claim would mask a shortfall and is rejected",
    );
}

/// AC: the property rejects a surplus with no claimant. The coverage direction
/// tolerates a surplus by design (a donor may strand tokens), so this control
/// targets the exactness companion the generated traces also assert: tokens in
/// the pool that no stream accounts for.
#[test]
fn the_property_rejects_a_surplus_with_no_claimant() {
    let h = Harness::new();
    let _ = h.create_simple(1_000 * ONE, 100 * DAY);
    assert_eq!(contract_audit(&h), PoolAudit::Balanced);

    // Loose tokens: the pool grows but no stream claims them.
    h.token_admin.mint(&h.contract_id, &(25 * ONE));

    match contract_audit(&h) {
        PoolAudit::Surplus { held, liability } => {
            assert_eq!(held - liability, 25 * ONE, "the surplus must be visible");
        }
        other => panic!("a surplus with no claimant must be reported, got {other:?}"),
    }

    let panicked = catch_unwind(AssertUnwindSafe(|| {
        assert_no_stranded_liability(&h, "surplus control")
    }))
    .is_err();
    assert!(
        panicked,
        "assert_no_stranded_liability must reject stranded tokens",
    );

    // Coverage itself still holds here — the two directions are distinct.
    assert!(
        catch_unwind(AssertUnwindSafe(|| assert_covers(&h, "surplus coverage"))).is_ok(),
        "coverage tolerates a surplus; only exactness rejects it",
    );
}

/// AC: the seeded sequence is a real check, not a run of no-ops. Counts the
/// operations that actually succeeded and requires the trace to carry a live,
/// non-zero liability.
#[test]
fn the_seeded_sequence_is_a_real_check() {
    // 0x1857_A11C_E5EED, padded into clippy's preferred 4-digit groups.
    let seed = 0x0001_857A_11CE_5EEDu64;
    let schedule = schedule_from_seed(seed, 40);
    let stats = drive_sequence(seed, 3, &schedule);

    assert_eq!(stats.steps as usize, schedule.len());
    assert!(
        stats.creates > 0,
        "seed {seed}: the sequence must create streams",
    );
    assert!(
        stats.withdrawals > 0,
        "seed {seed}: the sequence must withdraw",
    );
    assert!(stats.cancels > 0, "seed {seed}: the sequence must cancel",);
    assert!(stats.top_ups > 0, "seed {seed}: the sequence must top up",);
    assert!(
        stats.saw_nonzero_liability,
        "seed {seed}: the liability must be non-zero at some step",
    );
    assert!(
        stats.max_liability > 0,
        "seed {seed}: peak liability must be positive",
    );
    assert!(
        stats.max_pool > 0,
        "seed {seed}: the contract must actually hold tokens",
    );
    assert!(
        stats.max_pool >= stats.max_liability,
        "seed {seed}: the pool must cover the peak liability",
    );
}

/// AC: the property holds across a batch withdrawal, which settles several
/// streams' claims in one call.
#[test]
fn the_property_holds_across_a_batch_withdrawal() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..4i128)
        .map(|i| h.create_simple((100 + i) * ONE, 10 * DAY))
        .collect();
    check_after_step(&h, "batch: after creation");

    // Halfway: the batch pays the accrued half of every stream.
    h.advance(5 * DAY);
    let partial = h.client.batch_withdraw(&h.recipient, &h.ids(&ids));
    assert!(partial > 0, "the batch must pay out something");
    check_after_step(&h, "batch: after a partial settlement");

    // Maturity: the batch settles the rest.
    h.advance(5 * DAY);
    let rest = h.client.batch_withdraw(&h.recipient, &h.ids(&ids));
    assert_eq!(
        partial + rest,
        406 * ONE,
        "the batch must move exactly the deposited total",
    );
    check_after_step(&h, "batch: after full settlement");

    assert_eq!(h.pool(), 0, "every claim settled leaves nothing pooled");
    for id in &ids {
        assert_eq!(
            h.get(*id).status,
            StreamStatus::Depleted,
            "a drained stream is Depleted",
        );
    }
}

// ---------------------------------------------------------------------------
// The generated property
// ---------------------------------------------------------------------------

proptest! {
    // `ProptestConfig::default()` reads PROPTEST_CASES (and honours the
    // job's budget) exactly as `test::props` and `test::accounting_identity`
    // do. The schedule and the sequence seed are generated inputs, so proptest
    // prints both in the minimal failing input and the sequence replays from
    // them; no case count is pinned here.
    #![proptest_config(ProptestConfig::default())]

    /// **Issue #1857.** Over a generated sequence of public-ABI operations on
    /// a generated population of streams, the contract's token balance always
    /// covers the summed live liability, and every individual claim is backed —
    /// re-audited after every step.
    #[test]
    fn the_pool_always_covers_the_summed_live_liability(
        sequence_seed in any::<u64>(),
        population in 1usize..5,
        schedule in prop::collection::vec((0u8..OP_CODES, any::<u32>()), 1..(MAX_STEPS + 1)),
    ) {
        let stats = drive_sequence(sequence_seed, population as u64, &schedule);
        prop_assert_eq!(stats.steps as usize, schedule.len());
    }
}
