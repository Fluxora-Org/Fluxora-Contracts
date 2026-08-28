//! Stage 4 — the stream id invariant: unique, strictly monotonic, never
//! reused, independent of fixture order.
//!
//! `test::create` pins the happy-path shape (`stream_ids_are_monotonic_and_never_reused`)
//! but only across a short, uninterrupted run of successful creates. This file
//! states the invariant as a property checked after *every* create attempt —
//! successful or not — and exercises the two ways a create can fail:
//!
//! * **Validation rejection** — `create_stream` returns before ever calling
//!   `storage::next_stream_id`, so the counter is untouched by construction.
//! * **A failed deposit transfer** — the counter *has* been bumped and the
//!   `Stream` entry *has* been written by the time the token transfer runs.
//!   This only stays safe because a Soroban invocation is atomic: an `Err`
//!   return unwinds every storage write the call made, not merely the token
//!   transfer. See `storage::next_stream_id` for the full argument.
//!
//! Transfers and top-ups are interleaved throughout to confirm that mutating
//! an *existing* stream never perturbs id allocation for the next create.
//!
//! # Design decision
//!
//! Ids are global — one instance-level counter shared by every sender, not
//! namespaced per sender and not derived from any stream field. See
//! `storage::next_stream_id` for the rationale and how gaps (there are none)
//! are handled.

use super::common::*;
use crate::Error;

/// Accumulates every id a harness has handed out and checks the invariant
/// incrementally: no repeats, strictly increasing, and always in lockstep
/// with `stream_count()`.
struct IdLedger {
    seen: std::vec::Vec<u64>,
}

impl IdLedger {
    fn new() -> Self {
        Self {
            seen: std::vec::Vec::new(),
        }
    }

    /// Record a freshly created id, asserting it is neither a duplicate nor
    /// out of order relative to every id seen so far.
    fn record(&mut self, id: u64) {
        assert!(
            !self.seen.contains(&id),
            "id {id} was handed out twice; already have {:?}",
            self.seen
        );
        if let Some(&last) = self.seen.last() {
            assert!(
                id > last,
                "id {id} is not strictly greater than the previously issued id {last}"
            );
        }
        self.seen.push(id);
    }

    /// Cross-check against the contract's own view: `stream_count()` must
    /// equal the number of successful creates, and the ids must be exactly
    /// `0..stream_count()` with no gaps.
    fn assert_matches_contract(&self, h: &Harness) {
        let count = h.client.stream_count();
        assert_eq!(
            self.seen.len() as u64,
            count,
            "stream_count() disagrees with the number of successful creates"
        );
        for (i, &id) in self.seen.iter().enumerate() {
            assert_eq!(id, i as u64, "ids must run 0..stream_count() with no gaps");
        }
        for id in 0..count {
            assert!(h.client.stream_exists(&id), "id {id} should exist");
        }
    }
}

/// A `create_stream` call guaranteed to fail on validation, before
/// `storage::next_stream_id` is ever reached: sender and recipient are the
/// same address.
fn attempt_self_stream_rejection(h: &Harness) {
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

/// A `create_stream` call that clears every validation gate — so the id
/// counter is bumped and the `Stream` entry is written — and only then fails,
/// because the sender cannot cover the deposit. This is the adversarial case:
/// proving the counter bump does not survive the rolled-back transfer.
fn attempt_unaffordable_deposit_rejection(h: &Harness) {
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
        "a deposit exceeding the sender's balance must fail"
    );
}

// ---------------------------------------------------------------------------
// Many streams, no interference
// ---------------------------------------------------------------------------

#[test]
fn many_streams_get_unique_strictly_increasing_ids() {
    let h = Harness::new();
    let mut ledger = IdLedger::new();

    for i in 0..60u64 {
        let id = h.create_simple(10 * ONE, DAY + i);
        ledger.record(id);
        ledger.assert_matches_contract(&h);
    }
}

// ---------------------------------------------------------------------------
// Failed creates must not consume or reuse an id
// ---------------------------------------------------------------------------

/// Every validation gate in `create_stream`, attempted between successful
/// creates, must leave the counter untouched — checked after every single
/// attempt, not just at the end.
#[test]
fn validation_failures_never_consume_an_id() {
    let h = Harness::new();
    let mut ledger = IdLedger::new();
    let start = h.now();

    ledger.record(h.create_simple(10 * ONE, DAY));
    ledger.assert_matches_contract(&h);

    let before = h.client.stream_count();

    attempt_self_stream_rejection(&h);
    assert_eq!(h.client.stream_count(), before);

    assert_eq!(
        h.client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &0,
                &start,
                &(start + DAY),
                &start,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::InvalidDeposit
    );
    assert_eq!(h.client.stream_count(), before);

    assert_eq!(
        h.client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &(10 * ONE),
                &start,
                &start,
                &start,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::InvalidTimeRange
    );
    assert_eq!(h.client.stream_count(), before);

    assert_eq!(
        h.client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &(10 * ONE),
                &start,
                &(start + DAY),
                &(start + 2 * DAY),
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::InvalidCliff
    );
    assert_eq!(h.client.stream_count(), before);

    assert_eq!(
        h.client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &(DAY as i128 - 1),
                &start,
                &(start + DAY),
                &start,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::DepositRateTooLow
    );
    assert_eq!(h.client.stream_count(), before);

    assert_eq!(
        h.client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &h.token,
                &(i128::MAX / 1_000),
                &start,
                &(start + YEAR),
                &start,
                &true,
                &true,
                &true,
            )
            .unwrap_err()
            .unwrap(),
        Error::Overflow
    );
    assert_eq!(
        h.client.stream_count(),
        before,
        "every rejected create above must leave the counter untouched"
    );

    ledger.record(h.create_simple(10 * ONE, DAY));
    ledger.assert_matches_contract(&h);
}

/// The case the issue calls out by name: a create that fails only because the
/// deposit transfer itself fails, *after* the id counter would already have
/// been bumped and the `Stream` entry already written. A retry must land on
/// the exact id the failed attempt never actually claimed.
#[test]
fn a_failed_deposit_transfer_does_not_consume_or_reuse_an_id() {
    let h = Harness::new();
    let mut ledger = IdLedger::new();

    ledger.record(h.create_simple(10 * ONE, DAY));
    ledger.assert_matches_contract(&h);

    let before = h.client.stream_count();
    attempt_unaffordable_deposit_rejection(&h);
    assert_eq!(
        h.client.stream_count(),
        before,
        "a failed deposit transfer must not advance the id counter"
    );

    let id = h.create_simple(10 * ONE, DAY);
    assert_eq!(
        id, before,
        "retried create must reuse the id the failed attempt never actually claimed"
    );
    ledger.record(id);
    ledger.assert_matches_contract(&h);
}

/// Retry the same failing deposit several times in a row before the sender
/// finally has an affordable request queued: the counter must not creep
/// forward across repeated failures.
#[test]
fn repeated_failed_retries_never_advance_the_counter() {
    let h = Harness::new();
    let mut ledger = IdLedger::new();
    ledger.record(h.create_simple(10 * ONE, DAY));

    let before = h.client.stream_count();
    for _ in 0..5 {
        attempt_unaffordable_deposit_rejection(&h);
        assert_eq!(h.client.stream_count(), before);
    }

    let id = h.create_simple(10 * ONE, DAY);
    assert_eq!(id, before);
    ledger.record(id);
    ledger.assert_matches_contract(&h);
}

/// Interleave successful creates with a mix of rejected retries, checking the
/// full invariant after every single create attempt — not just at the end of
/// the run.
#[test]
fn ids_stay_unique_and_ordered_across_interleaved_failures_and_successes() {
    let h = Harness::new();
    let mut ledger = IdLedger::new();

    for round in 0..30u64 {
        if round % 3 == 0 {
            attempt_self_stream_rejection(&h);
        }
        if round % 4 == 0 {
            attempt_unaffordable_deposit_rejection(&h);
        }

        let id = h.create_simple(10 * ONE, DAY + round);
        ledger.record(id);
        ledger.assert_matches_contract(&h);

        h.advance(1);
    }
}

// ---------------------------------------------------------------------------
// Transfers and top-ups must not perturb id allocation
// ---------------------------------------------------------------------------

/// Recipient transfers and top-ups mutate an existing stream's state but must
/// never influence which id the next `create_stream` receives.
#[test]
fn transfers_and_top_ups_do_not_perturb_id_allocation() {
    let h = Harness::new();
    let mut ledger = IdLedger::new();

    for _ in 0..5u64 {
        ledger.record(h.create_simple(1_000 * ONE, 100 * DAY));
    }
    ledger.assert_matches_contract(&h);

    h.client.transfer_recipient(&0, &h.other);
    let _ = h.client.try_transfer_recipient(&0, &h.sender); // rejected: SelfStream
    h.client.top_up(&1, &(50 * ONE));
    let _ = h.client.try_top_up(&2, &0); // rejected: InvalidAmount
    h.client.cancel(&3);
    h.client.transfer_recipient(&3, &h.other); // a cancelled stream's tail stays transferable
    h.advance(10 * DAY);

    ledger.record(h.create_simple(10 * ONE, DAY));
    ledger.record(h.create_simple(10 * ONE, DAY));
    ledger.assert_matches_contract(&h);
}

/// The full scenario the issue asks for in one place: many streams, some
/// creates retried after a real failure, transfers and top-ups threaded
/// through the run, with the id invariant checked after every create.
#[test]
fn stress_many_creates_with_retries_and_transfers_preserve_the_id_invariant() {
    let h = Harness::new();
    let mut ledger = IdLedger::new();

    for round in 0..80u64 {
        match round % 4 {
            0 => attempt_self_stream_rejection(&h),
            1 => attempt_unaffordable_deposit_rejection(&h),
            _ => {}
        }

        let id = h.create_simple(10 * ONE, DAY + round);
        ledger.record(id);
        ledger.assert_matches_contract(&h);

        if id > 0 {
            let target = id - 1;
            match round % 3 {
                0 => h.client.transfer_recipient(&target, &h.other),
                1 => {
                    let _ = h.client.try_top_up(&target, &(5 * ONE));
                }
                _ => {}
            }
        }
        h.advance(1);
    }

    assert_eq!(h.client.stream_count(), ledger.seen.len() as u64);
    ledger.assert_matches_contract(&h);
}
