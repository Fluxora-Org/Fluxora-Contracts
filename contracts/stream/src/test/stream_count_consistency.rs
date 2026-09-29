//! Issue #1884/#1699 — assert `stream_count()` stays consistent with the streams
//! that exist.
//!
//! The contract stores the same fact twice: the instance-level counter behind
//! [`crate::FluxoraStream::stream_count`] and the population of
//! `DataKey::Stream(id)` records in persistent storage. Nothing in production
//! ever asserts the two still agree. They can come apart exactly where the
//! issue says they can:
//!
//! * a **creation path that fails partway** — past every validation gate, at
//!   the token transfer — must roll the counter bump back with everything
//!   else;
//! * a **terminal operation** (`cancel`, `delegate_cancel`, a withdrawal that
//!   drives a stream to `Depleted`) rewrites the record in place and must
//!   never make the population shrink underneath the counter.
//!
//! This module states the comparison once, as
//! [`assert_stream_count_consistent`](super::common::assert_stream_count_consistent),
//! and drives it through every path the issue names:
//!
//! | acceptance criterion | test |
//! |---|---|
//! | compares `stream_count` against the number of existing streams | `stream_count_equals_the_number_of_existing_streams` |
//! | holds after failed creations | `stream_count_stays_consistent_after_failed_creations` |
//! | holds after every terminal operation | `…_after_cancel`, `…_after_delegate_cancel`, `…_after_a_depleting_withdraw`, `a_terminal_operation_failing_partway_…` |
//! | a deliberate counter error causes failure | `a_deliberate_counter_error_fails_the_consistency_assertion` (and its mirror, `a_missing_stream_record_fails_the_consistency_assertion`) |
//! | randomized sequences including failed creations | `the_counter_matches_the_population_across_randomized_sequences` |
//!
//! # The invariant, precisely
//!
//! Ids are handed out from the counter, contiguously and never reused
//! (`storage::next_stream_id`), no entry point removes a stream record, and a
//! failing invocation rolls back every write it made. So the population is
//! exactly `0..stream_count()`, and the helper's probe over the *inclusive*
//! range `0..=stream_count()` counts `stream_count()` records exactly when the
//! two representations agree — detecting a counter that is too high, a counter
//! that is too low, and a hole in the population alike.
//!
//! The one legitimate disagreement is an archived entry on a real network
//! (`docs/KNOWN-LIMITATIONS.md`); the test host auto-restores on read, so
//! nothing in this file can trip over it. Deliberately *deleting* a record is
//! a different matter, and is asserted as a detection case.

use std::panic::{catch_unwind, AssertUnwindSafe};

use soroban_sdk::testutils::{Address as _, IssuerFlags};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::Address;

use super::common::*;
use super::create::seed_counter;
use crate::{op, DataKey, Error, StreamStatus};

fn assert_counter_entries_match(h: &Harness) {
    h.env.as_contract(&h.contract_id, || {
        let next: u64 = h
            .env
            .storage()
            .instance()
            .get(&DataKey::NextStreamId)
            .unwrap_or(0);
        let count: u64 = h
            .env
            .storage()
            .instance()
            .get(&DataKey::StreamCount)
            .unwrap_or(0);
        assert_eq!(next, count, "NextStreamId and StreamCount diverged");
    });
}

/// Issue #1884: the two instance counters remain equal across a generated
/// sequence of successful and rejected creations.
#[test]
fn next_stream_id_and_stream_count_stay_equal_across_mixed_creations() {
    let h = Harness::new();
    for round in 0..32u64 {
        if round % 3 == 0 {
            reject_self_stream(&h);
        } else {
            h.create_simple(100 * ONE, DAY + round);
        }
        assert_counter_entries_match(&h);
        assert_eq!(h.client.stream_count(), round - round / 3);
    }
}

// ---------------------------------------------------------------------------
// Failed-creation fixtures
// ---------------------------------------------------------------------------

/// A create rejected at a validation gate (`SelfStream`): fails before
/// `storage::next_stream_id` is ever consulted, so the counter cannot have
/// moved — which is precisely what must be *checked*, not assumed.
fn reject_self_stream(h: &Harness) {
    let start = h.now();
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.sender,
            &h.token,
            &(10 * ONE),
            &start,
            &(start + DAY),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::SelfStream);
}

/// A create that clears every validation gate and fails only when the deposit
/// transfer runs — the "creation path fails partway" case from the issue.
/// Whatever the invocation wrote before the transfer must be rolled back with
/// it, counter and record alike.
fn reject_unaffordable_deposit(h: &Harness) {
    let start = h.now();
    let too_much = h.balance(&h.sender) + 1;
    let result = h.client.try_create_stream(
        &h.sender,
        &h.recipient,
        &h.token,
        &too_much,
        &start,
        &(start + DAY),
        &start,
        &true,
        &true,
        &true,
    );
    assert!(
        result.is_err(),
        "a deposit above the sender's balance must fail",
    );
}

// ---------------------------------------------------------------------------
// AC 1 — the comparison itself
// ---------------------------------------------------------------------------

/// AC: a test compares `stream_count()` against the number of existing
/// streams. Stated on a fresh contract (zero claimed, zero present) and then
/// re-checked after every successful create, plus an independent view-level
/// statement of the same fact that does not go through the helper.
#[test]
fn stream_count_equals_the_number_of_existing_streams() {
    let h = Harness::new();
    assert_eq!(h.client.stream_count(), 0);
    h.assert_stream_count_consistent();

    for expected in 0..6u64 {
        let id = h.create_simple(100 * ONE, DAY + expected);
        assert_eq!(id, expected, "ids must run 0..stream_count() with no gaps");
        assert!(h.client.stream_exists(&id));
        h.assert_stream_count_consistent();
        assert_eq!(h.client.stream_count(), expected + 1);
    }

    // The same fact, spelled out without the helper: every id below the
    // counter has a record, and the id the counter hands out next does not.
    let count = h.client.stream_count();
    for id in 0..count {
        assert!(
            h.client.stream_exists(&id),
            "id {id} must exist below {count}"
        );
    }
    assert!(
        !h.client.stream_exists(&count),
        "id {count} must not exist yet",
    );
}

// ---------------------------------------------------------------------------
// AC 2 — failed creations
// ---------------------------------------------------------------------------

/// AC: the comparison holds after failed creations — both the rejections that
/// happen before the counter is consulted and the partway failure at the
/// deposit transfer. Checked after *every single attempt*, not just at the
/// end, because a counter that drifts and later "catches up" would slip
/// through an end-of-run-only check.
#[test]
fn stream_count_stays_consistent_after_failed_creations() {
    let h = Harness::new();

    // Failures against a fresh contract: still zero claimed, zero present,
    // and no tokens moved.
    reject_self_stream(&h);
    h.assert_stream_count_consistent();
    reject_unaffordable_deposit(&h);
    h.assert_stream_count_consistent();
    assert_eq!(h.client.stream_count(), 0);
    assert_eq!(h.pool(), 0, "a failed create must move no funds");

    // Failures interleaved with successes.
    let mut created = 0u64;
    for round in 0..4u64 {
        assert_eq!(h.create_simple(100 * ONE, DAY + round), created);
        created += 1;
        h.assert_stream_count_consistent();

        reject_self_stream(&h);
        h.assert_stream_count_consistent();
        reject_unaffordable_deposit(&h);
        h.assert_stream_count_consistent();

        h.advance(1);
    }

    // The counter advanced exactly once per successful create, and the
    // population grew in lockstep.
    assert_eq!(h.client.stream_count(), created);
    h.assert_stream_count_consistent();
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// AC 3 — every terminal operation
// ---------------------------------------------------------------------------

/// `cancel`: the record is rewritten to `Cancelled`, never removed — and a
/// rejected repeat of the terminal operation must not move anything either.
#[test]
fn stream_count_stays_consistent_after_cancel() {
    let h = Harness::new();
    let doomed = h.create_simple(1_000 * ONE, 100 * DAY);
    let keep = h.create_simple(1_000 * ONE, 100 * DAY);
    h.assert_stream_count_consistent();

    h.advance(30 * DAY);
    h.client.cancel(&doomed);
    assert_eq!(h.get(doomed).status, StreamStatus::Cancelled);
    assert!(
        h.client.stream_exists(&doomed),
        "cancel must rewrite the record, not remove it",
    );
    h.assert_stream_count_consistent();

    // Rejections on the terminal stream leave everything untouched.
    assert_eq!(
        h.client.try_cancel(&doomed).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client
            .try_top_up(&doomed, &(10 * ONE))
            .unwrap_err()
            .unwrap(),
        Error::StreamTerminated
    );
    assert_eq!(
        h.client.try_pause(&doomed).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    h.assert_stream_count_consistent();

    // Draining the cancelled tail keeps `Cancelled` (it is sticky), keeps the
    // record, and keeps the counter.
    h.client.withdraw(&doomed, &None);
    assert_eq!(h.get(doomed).status, StreamStatus::Cancelled);
    assert_eq!(
        h.client.try_withdraw(&doomed, &None).unwrap_err().unwrap(),
        Error::StreamTerminated
    );
    h.assert_stream_count_consistent();

    // The untouched stream is unaffected, and the next create continues the
    // sequence without a gap or a hole.
    assert!(h.client.stream_exists(&keep));
    assert_eq!(h.create_simple(10 * ONE, DAY), 2);
    assert_eq!(h.client.stream_count(), 3);
    h.assert_stream_count_consistent();
    h.assert_pool_exact();
}

/// `delegate_cancel`: the delegated path to a terminal state must behave
/// exactly like the direct one.
#[test]
fn stream_count_stays_consistent_after_delegate_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client
        .grant_delegate(&id, &h.sender, &h.other, &op::CANCEL, &None);

    h.advance(30 * DAY);
    h.client.delegate_cancel(&id, &h.other);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert!(h.client.stream_exists(&id));
    h.assert_stream_count_consistent();

    // The grant is still live, but the stream is terminal: rejected, unchanged.
    assert_eq!(
        h.client
            .try_delegate_cancel(&id, &h.other)
            .unwrap_err()
            .unwrap(),
        Error::StreamTerminated
    );
    h.assert_stream_count_consistent();
    assert_eq!(h.client.stream_count(), 1);
    h.assert_pool_exact();
}

/// `withdraw` to `Depleted`, through all three routes: direct, delegated, and
/// batch. Depletion rewrites the record and must never shrink the population.
#[test]
fn stream_count_stays_consistent_after_a_depleting_withdraw() {
    let h = Harness::new();
    let direct = h.create_simple(1_000 * ONE, 100 * DAY);
    let delegated = h.create_simple(1_000 * ONE, 100 * DAY);
    let batched_a = h.create_simple(1_000 * ONE, 100 * DAY);
    let batched_b = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client
        .grant_delegate(&delegated, &h.recipient, &h.other, &op::WITHDRAW, &None);
    h.assert_stream_count_consistent();

    // Fully vested: every withdrawal below drains its stream to `Depleted`.
    h.warp_to(T0 + 100 * DAY);

    h.client.withdraw(&direct, &None);
    assert_eq!(h.get(direct).status, StreamStatus::Depleted);
    h.assert_stream_count_consistent();

    h.client.delegate_withdraw(&delegated, &h.other, &None);
    assert_eq!(h.get(delegated).status, StreamStatus::Depleted);
    h.assert_stream_count_consistent();

    h.client
        .batch_withdraw(&h.recipient, &h.ids(&[batched_a, batched_b]));
    assert_eq!(h.get(batched_a).status, StreamStatus::Depleted);
    assert_eq!(h.get(batched_b).status, StreamStatus::Depleted);
    h.assert_stream_count_consistent();

    // Every terminal stream keeps its record and its place in the sequence;
    // further withdrawals are rejected without disturbing either.
    for id in [direct, delegated, batched_a, batched_b] {
        assert!(h.client.stream_exists(&id), "id {id} must not be removed");
        assert_eq!(
            h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
            Error::StreamTerminated
        );
    }
    h.assert_stream_count_consistent();
    assert_eq!(h.client.stream_count(), 4);
    h.assert_pool_exact();
}

/// The adversarial case: a terminal operation that fails *partway*. `cancel`
/// writes `Cancelled` to the record before the refund transfer runs, and the
/// transfer can still fail. Soroban rolls the whole invocation back — but
/// nothing in production asserts that the counter and the population came back
/// in agreement, so assert it here.
#[test]
fn a_terminal_operation_failing_partway_leaves_counter_and_population_agreeing() {
    let h = Harness::new();

    // A clawback-enabled SAC (the mechanism `test::token_errors` uses), so the
    // pooled funds can be drained out from under the refund transfer.
    let issuer = Address::generate(&h.env);
    let asset = h.env.register_stellar_asset_contract_v2(issuer);
    asset.issuer().set_flag(IssuerFlags::ClawbackEnabledFlag);
    let token = asset.address();
    let token_client = TokenClient::new(&h.env, &token);
    let sac_admin = StellarAssetClient::new(&h.env, &token);

    sac_admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.now();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    h.assert_stream_count_consistent();

    h.advance(30 * DAY);
    assert!(h.client.refundable_of(&id) > 0, "sanity: a refund is owed");

    // Drain the pool so the refund transfer fails after `save_stream` has
    // already written `Cancelled`.
    let pool = token_client.balance(&h.contract_id);
    sac_admin.clawback(&h.contract_id, &pool);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);
    assert_eq!(
        h.get(id).status,
        StreamStatus::Active,
        "the failed cancel must roll back",
    );
    h.assert_stream_count_consistent();

    // Fund the refund and cancel for real: the terminal transition itself must
    // leave the two agreeing as well.
    sac_admin.mint(&h.contract_id, &pool);
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    h.assert_stream_count_consistent();
    assert_eq!(h.client.stream_count(), 1);
}

// ---------------------------------------------------------------------------
// AC 4 — the assertion must actually detect a divergence
// ---------------------------------------------------------------------------

/// AC: a deliberate counter error causes failure. Seed the counter to a wrong
/// value — high, low, wildly off, and zero — and require the helper to panic
/// every time. Without this, the tests above would pass just as happily
/// against a helper that asserted nothing.
#[test]
fn a_deliberate_counter_error_fails_the_consistency_assertion() {
    let h = Harness::new();
    for _ in 0..4 {
        h.create_simple(10 * ONE, DAY);
    }
    let real = h.client.stream_count();
    assert_eq!(real, 4);
    h.assert_stream_count_consistent(); // sanity: the true state passes

    for wrong in [real + 1, real - 1, real + 7, 0] {
        seed_counter(&h, wrong);
        assert_ne!(
            h.client.stream_count(),
            real,
            "the seed must actually move the counter for this to mean anything",
        );

        let result = catch_unwind(AssertUnwindSafe(|| h.assert_stream_count_consistent()));
        assert!(
            result.is_err(),
            "counter seeded to {wrong} (real: {real}) must fail the assertion",
        );
    }

    // Restore the true counter and the assertion passes again — the helper is
    // not stuck in "always fail" either.
    seed_counter(&h, real);
    h.assert_stream_count_consistent();
}

/// The mirror image, and the issue's own wording: if a record disappears while
/// the counter still counts it — what "a terminal operation removes a record"
/// would look like — the assertion must catch that too. A counter that is
/// merely *consistent with itself* but wrong about the population is exactly
/// the failure mode this check exists for.
#[test]
fn a_missing_stream_record_fails_the_consistency_assertion() {
    let h = Harness::new();
    for _ in 0..4 {
        h.create_simple(10 * ONE, DAY);
    }
    h.assert_stream_count_consistent();

    // Delete id 1 out of the middle of the population, leaving the counter
    // untouched.
    h.env.as_contract(&h.contract_id, || {
        h.env.storage().persistent().remove(&DataKey::Stream(1));
    });
    assert!(!h.client.stream_exists(&1));
    assert_eq!(
        h.client.stream_count(),
        4,
        "the counter itself did not move"
    );

    let result = catch_unwind(AssertUnwindSafe(|| h.assert_stream_count_consistent()));
    assert!(
        result.is_err(),
        "a hole in the population under an unchanged counter must fail the assertion",
    );
}

// ---------------------------------------------------------------------------
// Validation — randomized sequences including failed creations
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

/// Drive one seeded sequence: successful creates, creates that fail at
/// validation, creates that fail at the deposit transfer, terminal operations
/// (direct, delegated, batch) and ordinary mutations — asserting after every
/// single step that `stream_count()` still equals the number of records that
/// exist, and that it moved only for the creates that actually succeeded.
fn run_randomized_sequence(seed: u64, steps: u32) {
    let h = Harness::new();
    let mut rng = Rng(seed);
    let mut created = 0u64;

    // Seed the population with varied shapes and capabilities, so terminal
    // operations land on cancellable, non-cancellable, pausable and
    // non-pausable streams alike.
    for i in 0..4u64 {
        let start = h.now() + rng.below(10 * DAY);
        let duration = DAY + rng.below(20 * DAY);
        let deposit = (1 + rng.below(500)) as i128 * ONE;
        h.create(
            deposit,
            start,
            start + duration,
            start,
            i % 2 == 0,
            i % 3 != 0,
            true,
        );
        created += 1;
    }
    h.assert_stream_count_consistent();
    assert_eq!(h.client.stream_count(), created);

    for step in 1..=steps {
        let count = h.client.stream_count();
        let id = rng.below(count);

        match rng.below(12) {
            // A create that must succeed.
            0 => {
                let start = h.now() + rng.below(10 * DAY);
                let duration = DAY + rng.below(20 * DAY);
                let cliff = start + rng.below(duration);
                let deposit = (1 + rng.below(500)) as i128 * ONE;
                h.create(deposit, start, start + duration, cliff, true, true, true);
                created += 1;
            }
            // Creates that must fail — at the validation gates …
            1 => reject_self_stream(&h),
            // … and partway through, at the deposit transfer.
            2 => reject_unaffordable_deposit(&h),
            // Terminal operations, in all three shapes.
            3 => {
                let _ = h.client.try_cancel(&id);
            }
            4 => {
                let amount = if rng.below(2) == 0 {
                    None
                } else {
                    Some((1 + rng.below(50)) as i128 * ONE)
                };
                let _ = h.client.try_withdraw(&id, &amount);
            }
            5 => {
                let _ = h
                    .client
                    .try_grant_delegate(&id, &h.sender, &h.other, &op::CANCEL, &None);
            }
            6 => {
                let _ = h.client.try_delegate_cancel(&id, &h.other);
            }
            7 => {
                if count >= 2 {
                    let a = rng.below(count);
                    let b = rng.below(count);
                    if a != b {
                        let _ = h.client.try_batch_withdraw(&h.recipient, &h.ids(&[a, b]));
                    }
                }
            }
            // Ordinary mutations around them.
            8 => {
                let _ = h.client.try_pause(&id);
            }
            9 => {
                let _ = h.client.try_resume(&id);
            }
            10 => {
                let _ = h
                    .client
                    .try_top_up(&id, &((1 + rng.below(5)) as i128 * ONE));
            }
            _ => {
                let _ = h.client.try_transfer_recipient(&id, &h.other);
            }
        }

        // The point of this file: after *every* step the counter and the
        // population agree, and the counter moved only for real successes.
        h.assert_stream_count_consistent();
        assert_eq!(
            h.client.stream_count(),
            created,
            "seed {seed}, step {step}: failed creations must not move the counter",
        );

        // Time moves between operations, sometimes a lot.
        h.advance(1 + rng.below(5 * DAY));
        h.assert_stream_count_consistent();
    }

    assert_eq!(
        h.client.stream_count(),
        created,
        "seed {seed}: final counter must equal the number of successful creates",
    );
}

/// Validation: randomized sequences including failed creations, with the
/// counter/population comparison asserted after every step. Seeds are fixed so
/// a failure replays exactly; the seed and step are in the assertion messages.
#[test]
fn the_counter_matches_the_population_across_randomized_sequences() {
    for i in 0..8u64 {
        let seed = 0xC0FF_EE5E_1699_0001u64.wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        run_randomized_sequence(seed, 40);
    }
}
