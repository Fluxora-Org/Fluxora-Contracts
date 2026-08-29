//! Stage 3 — invariant **I3**: no operation may move `vested(t)` backwards.
//!
//! `top_up` rounding backwards was not a bug in `top_up`; it was an instance of
//! a bug *class*. Any entry point that changes `deposited`, `start_time`,
//! `end_time`, `cliff_time` or `paused_total` — the five inputs to `vested` —
//! can reduce the amount already vested at a fixed instant, and every such
//! reduction lets `withdrawn` exceed `vested` and then lets `cancel` refund the
//! sender tokens the recipient already holds.
//!
//! So rather than regression-testing the one function that broke, this file
//! checks the property across **every operation** and **every ordering** of
//! them.
//!
//! # Why the clock is frozen
//!
//! I3 is about state transitions, not the passage of time. `vested` is *meant*
//! to grow as the clock advances (that is I2, covered in `test::props`). To
//! isolate I3 the clock must not move across the call being measured, or a
//! genuine regression would be masked by concurrent accrual. Every measurement
//! below reads `vested` at one timestamp, performs exactly one call, and reads
//! `vested` again at the same timestamp.
//!
//! This is precisely why the original bug survived the hand-written tests: they
//! all advanced time around operations, so a small backwards step vanished into
//! the accrual that happened alongside it.

use super::common::*;
use crate::MAX_BATCH_SIZE;

/// Every operation a stream can undergo, as an applicable unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Withdraw,
    TopUp,
    Pause,
    Resume,
    Cancel,
    TransferRecipient,
    ExtendTtl,
    BatchWithdraw,
    BatchExtendTtl,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Withdraw => "withdraw",
            Op::TopUp => "top_up",
            Op::Pause => "pause",
            Op::Resume => "resume",
            Op::Cancel => "cancel",
            Op::TransferRecipient => "transfer_recipient",
            Op::ExtendTtl => "extend_stream_ttl",
            Op::BatchWithdraw => "batch_withdraw",
            Op::BatchExtendTtl => "batch_extend_ttl",
        }
    }

    /// Attempt the operation. Rejections are expected and fine — a call that
    /// fails must still leave every invariant intact, which is itself worth
    /// checking, so failures are swallowed rather than asserted against.
    fn apply(self, h: &Harness, id: u64) {
        let ids = h.ids(&[id]);
        match self {
            Op::Withdraw => {
                let _ = h.client.try_withdraw(&id, &None);
            }
            Op::TopUp => {
                let _ = h.client.try_top_up(&id, &(10 * ONE));
            }
            Op::Pause => {
                let _ = h.client.try_pause(&id);
            }
            Op::Resume => {
                let _ = h.client.try_resume(&id);
            }
            Op::Cancel => {
                let _ = h.client.try_cancel(&id);
            }
            Op::TransferRecipient => {
                let _ = h.client.try_transfer_recipient(&id, &h.other);
            }
            Op::ExtendTtl => {
                let _ = h.client.try_extend_stream_ttl(&id);
            }
            Op::BatchWithdraw => {
                let _ = h.client.try_batch_withdraw(&h.recipient, &ids);
            }
            Op::BatchExtendTtl => {
                let _ = h.client.try_batch_extend_ttl(&ids);
            }
        }
    }
}

const ALL_OPS: [Op; 9] = [
    Op::Withdraw,
    Op::TopUp,
    Op::Pause,
    Op::Resume,
    Op::Cancel,
    Op::TransferRecipient,
    Op::ExtendTtl,
    Op::BatchWithdraw,
    Op::BatchExtendTtl,
];

/// Run one operation with the clock held still, and assert I3 plus the
/// post-condition bundle.
fn apply_and_check(h: &Harness, id: u64, op: Op, context: &str) {
    let before = h.vested_snapshot();
    let clock = h.now();

    op.apply(h, id);

    assert_eq!(
        h.now(),
        clock,
        "the clock must not move across the measurement"
    );
    h.assert_no_vested_regression(&before, &std::format!("{context} -> {}", op.name()));
    h.assert_invariants();
    h.assert_pool_invariant();
}

/// A stream partway through its schedule, already partly drawn down, so the
/// operations under test have something to get wrong.
fn primed_stream(h: &Harness) -> u64 {
    // Deliberately inexact: 1000 stroops over 300s is 3.33/sec, which is where
    // the top_up rounding bug lived.
    let start = h.now();
    let id = h.create(1_000, start, start + 300, start, true, true, true);
    h.advance(150);
    let _ = h.client.try_withdraw(&id, &None);
    id
}

// ---------------------------------------------------------------------------
// Single operations
// ---------------------------------------------------------------------------

/// Every operation, applied on its own to a primed stream, at a frozen clock.
#[test]
fn no_single_operation_moves_vested_backwards() {
    for op in ALL_OPS {
        let h = Harness::new();
        let id = primed_stream(&h);
        apply_and_check(&h, id, op, "primed");
    }
}

/// The same, but on a stream that is paused when the operation lands — the
/// state where the accrual clock is frozen and easiest to mishandle.
#[test]
fn no_operation_moves_vested_backwards_while_paused() {
    for op in ALL_OPS {
        let h = Harness::new();
        let id = primed_stream(&h);
        h.client.pause(&id);
        h.advance(40);
        apply_and_check(&h, id, op, "paused");
    }
}

/// The same, on a stream that has already been cancelled — terminal state, and
/// the one where `deposited` has been rewritten under the accrual math.
#[test]
fn no_operation_moves_vested_backwards_after_cancel() {
    for op in ALL_OPS {
        let h = Harness::new();
        let id = primed_stream(&h);
        h.client.cancel(&id);
        apply_and_check(&h, id, op, "cancelled");
    }
}

/// The same, past `end_time`, where `elapsed` is clamped and a rounding change
/// could otherwise unclamp it.
#[test]
fn no_operation_moves_vested_backwards_after_maturity() {
    for op in ALL_OPS {
        let h = Harness::new();
        let id = primed_stream(&h);
        h.advance(1_000);
        apply_and_check(&h, id, op, "matured");
    }
}

/// The same, before the cliff opens, where `vested` is gated to zero and any
/// schedule rewrite could move the gate.
#[test]
fn no_operation_moves_vested_backwards_before_the_cliff() {
    for op in ALL_OPS {
        let h = Harness::new();
        let start = h.now();
        let id = h.create(1_000, start, start + 300, start + 200, true, true, true);
        h.advance(100);
        apply_and_check(&h, id, op, "pre-cliff");
    }
}

// ---------------------------------------------------------------------------
// Orderings
// ---------------------------------------------------------------------------

/// Heap's algorithm — every permutation of the slice, in place.
fn permutations<T: Copy>(items: &mut [T], out: &mut std::vec::Vec<std::vec::Vec<T>>) {
    fn generate<T: Copy>(k: usize, items: &mut [T], out: &mut std::vec::Vec<std::vec::Vec<T>>) {
        if k == 1 {
            out.push(items.to_vec());
            return;
        }
        for i in 0..k {
            generate(k - 1, items, out);
            if k.is_multiple_of(2) {
                items.swap(i, k - 1);
            } else {
                items.swap(0, k - 1);
            }
        }
    }
    let n = items.len();
    generate(n, items, out);
}

/// **Every ordering of the mutating operations.**
///
/// 720 permutations of the six operations that can change stream state, each
/// applied to a fresh primed stream, with I3 and the post-condition bundle
/// checked after every individual call — 4,320 measurements.
///
/// Orderings matter because the operations are not independent: a `top_up`
/// after a `pause` reads a frozen clock, a `cancel` after a `top_up` settles
/// against a rewritten schedule, and a `withdraw` between them fixes
/// `withdrawn` at a value the later operations must not invalidate.
#[test]
fn no_ordering_of_operations_moves_vested_backwards() {
    let mut ops = [
        Op::Withdraw,
        Op::TopUp,
        Op::Pause,
        Op::Resume,
        Op::Cancel,
        Op::TransferRecipient,
    ];
    let mut orderings = std::vec::Vec::new();
    permutations(&mut ops, &mut orderings);
    assert_eq!(
        orderings.len(),
        720,
        "expected every permutation of six ops"
    );

    for ordering in &orderings {
        let h = Harness::new();
        let id = primed_stream(&h);

        let mut trace = std::string::String::new();
        for op in ordering {
            trace.push_str(op.name());
            trace.push(' ');
            apply_and_check(&h, id, *op, &trace);
        }
    }
}

/// Orderings again, but with the clock advancing *between* operations rather
/// than during them. Each individual measurement is still frozen; the stream
/// simply arrives at each operation in a different temporal state.
#[test]
fn no_ordering_moves_vested_backwards_with_time_passing_between_calls() {
    let mut ops = [Op::TopUp, Op::Pause, Op::Resume, Op::Withdraw, Op::Cancel];
    let mut orderings = std::vec::Vec::new();
    permutations(&mut ops, &mut orderings);
    assert_eq!(orderings.len(), 120);

    for (n, ordering) in orderings.iter().enumerate() {
        let h = Harness::new();
        let id = primed_stream(&h);

        let mut trace = std::string::String::new();
        for (step, op) in ordering.iter().enumerate() {
            // Vary the gap per ordering so the operations land on many
            // different points of the schedule across the 120 runs.
            h.advance(1 + ((n * 7 + step * 13) % 90) as u64);
            trace.push_str(op.name());
            trace.push(' ');
            apply_and_check(&h, id, *op, &trace);
        }
    }
}

// ---------------------------------------------------------------------------
// Targeted: the operations that can actually violate I3
// ---------------------------------------------------------------------------

/// `top_up` and `cancel` are the only entry points that rewrite inputs to
/// `vested`. Hammer `top_up` with awkward amounts against an inexact rate,
/// which is exactly the shape that produced the original 93-stroop regression.
#[test]
fn repeated_top_ups_at_awkward_amounts_never_reduce_vested() {
    let h = Harness::new();
    let start = h.now();
    // 7919 over 997s — coprime, so every division truncates.
    let id = h.create(7_919, start, start + 997, start, true, true, true);
    h.advance(503);
    h.client.withdraw(&id, &None);

    for amount in [11i128, 13, 17, 101, 997, 1_009, 7_919] {
        let before = h.vested_snapshot();
        let _ = h.client.try_top_up(&id, &amount);
        h.assert_no_vested_regression(&before, &std::format!("top_up({amount})"));
        h.assert_invariants();
    }
    h.assert_pool_exact();
}

/// `cancel` rewrites both `deposited` and `end_time`. At the cancel instant it
/// must leave `vested` exactly where it was — not merely "not lower".
#[test]
fn cancel_leaves_vested_exactly_unchanged_at_the_cancel_instant() {
    for elapsed in [0u64, 1, 7, 150, 299, 300, 1_000] {
        let h = Harness::new();
        let start = h.now();
        let id = h.create(1_000, start, start + 300, start, true, true, true);
        h.advance(elapsed);

        let before = h.client.vested_of(&id);
        h.client.cancel(&id);
        let after = h.client.vested_of(&id);

        assert_eq!(
            after, before,
            "cancel at +{elapsed}s changed vested from {before} to {after}",
        );
        h.assert_pool_exact();
    }
}

/// A batch touching many streams must preserve I3 for *every* stream in it,
/// not just the one being reasoned about.
#[test]
fn batch_operations_preserve_the_invariant_across_all_streams() {
    let h = Harness::new();
    let mut ids = std::vec::Vec::new();
    for i in 0..MAX_BATCH_SIZE as u64 {
        let start = h.now();
        // Varied, deliberately inexact schedules.
        ids.push(h.create(
            1_000 + i as i128 * 37,
            start,
            start + 300 + i * 11,
            start,
            true,
            true,
            true,
        ));
    }
    h.advance(200);

    let all = h.ids(&ids);
    for round in 0..3 {
        let before = h.vested_snapshot();
        let _ = h.client.try_batch_withdraw(&h.recipient, &all);
        h.assert_no_vested_regression(&before, &std::format!("batch_withdraw round {round}"));
        h.assert_invariants();

        let before = h.vested_snapshot();
        let _ = h.client.try_batch_extend_ttl(&all);
        h.assert_no_vested_regression(&before, &std::format!("batch_extend_ttl round {round}"));
        h.assert_invariants();

        h.advance(50);
    }
    h.assert_pool_exact();
}

/// A rejected call must be a no-op. If a failed operation could still perturb
/// the schedule, I3 would be violated by transactions that appear to have done
/// nothing at all.
#[test]
fn rejected_operations_leave_vested_untouched() {
    let h = Harness::new();
    let start = h.now();
    // Not cancellable, not pausable, not transferable: every guard trips.
    let id = h.create(1_000, start, start + 300, start, false, false, false);
    h.advance(150);

    let before = h.vested_snapshot();
    let deposited = h.get(id).deposited;

    assert!(h.client.try_cancel(&id).is_err());
    assert!(h.client.try_pause(&id).is_err());
    assert!(h.client.try_resume(&id).is_err());
    assert!(h.client.try_transfer_recipient(&id, &h.other).is_err());
    assert!(h.client.try_top_up(&id, &0).is_err());
    assert!(h.client.try_top_up(&id, &-5).is_err());
    assert!(h.client.try_withdraw(&id, &Some(i128::MAX)).is_err());

    h.assert_no_vested_regression(&before, "rejected operations");
    assert_eq!(
        h.get(id).deposited,
        deposited,
        "a rejected call changed the deposit"
    );
    h.assert_invariants();
    h.assert_pool_exact();
}
