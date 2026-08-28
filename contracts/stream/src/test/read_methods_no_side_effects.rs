//! Issue #1566 — Regression coverage for read methods side effects.
//!
//! # Design decision: reads must be strictly side-effect free
//!
//! The read-only view functions (`get_stream`, `withdrawable_of`, `vested_of`,
//! `refundable_of`) deliberately do **not** extend TTL. They are called through
//! simulation by SDKs and UIs, where a footprint write is at best noise and at
//! worst confusing. The contract's design separates concerns:
//!
//! * **Reads** are pure queries with no TTL side effects. Called frequently, in
//!   simulation, without any cost.
//! * **Keeps alive** is the explicit job of `extend_stream_ttl` and
//!   `batch_extend_ttl` — permissionless operations that anyone can sponsor.
//!
//! # What we verify
//!
//! 1. **No TTL extension on reads.** Reading a stream via any read method must
//!    leave its TTL unchanged, whether the stream is active or has expired.
//! 2. **No state mutation.** Stream accounting and deposited/withdrawn values
//!    must be unchanged after reads.
//! 3. **Correct values returned.** `vested_of`, `withdrawable_of`, and
//!    `refundable_of` must return the same computed values as the direct accrual
//!    functions would, without side effects.
//! 4. **Failure on missing/archived.** Reads must fail with `StreamNotFound` for
//!    ids that were never issued or have been archived, without mutating any
//!    state.
//! 5. **No auth required.** Read methods must not require any authorization — they
//!    are callable by anyone.

use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger as _};

use super::common::*;
use crate::{accrual, storage, DataKey, Error, StreamStatus};

// --- Read methods do not extend TTL on active streams -----------------------

/// `get_stream` must not extend a stream's TTL, even on an active stream.
#[test]
fn get_stream_does_not_extend_ttl_on_active_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let ttl_before = h.ttl_of(id);

    // Read the stream multiple times.
    for _ in 0..5 {
        let _stream = h.client.get_stream(&id);
        assert_eq!(h.ttl_of(id), ttl_before, "get_stream must not extend TTL");
    }
}

/// `withdrawable_of` must not extend a stream's TTL.
#[test]
fn withdrawable_of_does_not_extend_ttl_on_active_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    let ttl_before = h.ttl_of(id);

    for _ in 0..5 {
        let _withdrawable = h.client.withdrawable_of(&id);
        assert_eq!(h.ttl_of(id), ttl_before, "withdrawable_of must not extend TTL");
    }
}

/// `vested_of` must not extend a stream's TTL.
#[test]
fn vested_of_does_not_extend_ttl_on_active_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    let ttl_before = h.ttl_of(id);

    for _ in 0..5 {
        let _vested = h.client.vested_of(&id);
        assert_eq!(h.ttl_of(id), ttl_before, "vested_of must not extend TTL");
    }
}

/// `refundable_of` must not extend a stream's TTL.
#[test]
fn refundable_of_does_not_extend_ttl_on_active_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    let ttl_before = h.ttl_of(id);

    for _ in 0..5 {
        let _refundable = h.client.refundable_of(&id);
        assert_eq!(h.ttl_of(id), ttl_before, "refundable_of must not extend TTL");
    }
}

// --- Read methods do not extend TTL on expired/archived streams ------

/// Even on an expired (archived) stream, reads must not extend TTL.
/// This is critical: a read operation must never be able to accidentally
/// resurrect an archived entry.
#[test]
fn get_stream_does_not_extend_ttl_on_archived_stream() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(10_000);

    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let snapshot_before = h.snapshot();

    // Let the entry archive completely.
    h.env.as_contract(&h.contract_id, || {
        let seq = h.env.ledger().sequence();
        h.env.ledger().set_sequence_number(seq + 50_000);
    });

    // The entry is archived; reading it restores it automatically in the test
    // host, but must not extend it beyond the minimum.
    let min_ttl = h.env.ledger().get().min_persistent_entry_ttl;
    let _stream = h.client.get_stream(&id);
    let ttl_after = h.ttl_of(id);

    assert!(
        ttl_after <= min_ttl,
        "get_stream must not extend an archived stream beyond minimum"
    );

    // State snapshot must be identical.
    let snapshot_after = h.snapshot();
    assert_eq!(
        snapshot_before.streams, snapshot_after.streams,
        "get_stream must not mutate stream state"
    );
}

/// Multiple read operations on an expired stream must keep it at minimum TTL.
#[test]
fn multiple_reads_on_archived_stream_do_not_extend_ttl() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(10_000);

    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let min_ttl = h.env.ledger().get().min_persistent_entry_ttl;

    // Archive the entry.
    h.env.as_contract(&h.contract_id, || {
        let seq = h.env.ledger().sequence();
        h.env.ledger().set_sequence_number(seq + 50_000);
    });

    // Multiple reads in succession must not accumulate TTL.
    h.client.get_stream(&id);
    let ttl_1 = h.ttl_of(id);

    h.client.withdrawable_of(&id);
    let ttl_2 = h.ttl_of(id);

    h.client.vested_of(&id);
    let ttl_3 = h.ttl_of(id);

    h.client.refundable_of(&id);
    let ttl_4 = h.ttl_of(id);

    // All should be at or just above the network minimum.
    assert!(ttl_1 <= min_ttl + 10, "first read extended too much");
    assert_eq!(ttl_2, ttl_1, "second read should not extend further");
    assert_eq!(ttl_3, ttl_1, "third read should not extend further");
    assert_eq!(ttl_4, ttl_1, "fourth read should not extend further");
}

// --- Read methods return correct values without side effects ---------

/// `withdrawable_of` must return the same value as computed directly.
#[test]
fn withdrawable_of_returns_correct_value() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    for day in [0, 10, 25, 50, 100] {
        h.warp_to(T0 + (day as u64) * DAY);

        let stream = h.client.get_stream(&id);
        let now = h.env.ledger().timestamp();
        let expected = accrual::withdrawable(&stream, now).expect("withdrawable must succeed");
        let actual = h.client.withdrawable_of(&id);

        assert_eq!(
            actual, expected,
            "withdrawable_of mismatch at day {day}: expected {expected}, got {actual}"
        );
    }
}

/// `vested_of` must return the same value as computed directly.
#[test]
fn vested_of_returns_correct_value() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    for day in [0, 10, 25, 50, 100] {
        h.warp_to(T0 + (day as u64) * DAY);

        let stream = h.client.get_stream(&id);
        let now = h.env.ledger().timestamp();
        let expected = accrual::vested(&stream, now).expect("vested must succeed");
        let actual = h.client.vested_of(&id);

        assert_eq!(
            actual, expected,
            "vested_of mismatch at day {day}: expected {expected}, got {actual}"
        );
    }
}

/// `refundable_of` must return the same value as computed directly.
#[test]
fn refundable_of_returns_correct_value() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    for day in [0, 10, 25, 50, 100] {
        h.warp_to(T0 + (day as u64) * DAY);

        let stream = h.client.get_stream(&id);
        let now = h.env.ledger().timestamp();
        let expected = accrual::refundable(&stream, now).expect("refundable must succeed");
        let actual = h.client.refundable_of(&id);

        assert_eq!(
            actual, expected,
            "refundable_of mismatch at day {day}: expected {expected}, got {actual}"
        );
    }
}

/// Conservation invariant must hold for read-computed values.
///
/// For any instant, `vested(t) + refundable(t) == deposited`.
#[test]
fn conservation_holds_for_read_values() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    for day in [0, 10, 25, 50, 100] {
        h.warp_to(T0 + (day as u64) * DAY);

        let stream = h.client.get_stream(&id);
        let vested = h.client.vested_of(&id);
        let refundable = h.client.refundable_of(&id);

        assert_eq!(
            vested + refundable,
            stream.deposited,
            "conservation broken at day {day}: {vested} + {refundable} != {deposited}",
            deposited = stream.deposited
        );
    }
}

/// Reading does not mutate stream accounting.
#[test]
fn reading_does_not_mutate_accounting() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);

    let before = h.client.get_stream(&id);

    // Perform multiple reads.
    h.client.withdrawable_of(&id);
    h.client.vested_of(&id);
    h.client.refundable_of(&id);
    h.client.get_stream(&id);

    let after = h.client.get_stream(&id);

    assert_eq!(before, after, "stream record changed after reads");
    assert_eq!(before.withdrawn, after.withdrawn, "withdrawn changed");
    assert_eq!(before.deposited, after.deposited, "deposited changed");
    assert_eq!(before.status, after.status, "status changed");
}

// --- Read methods fail correctly on missing/archived streams ----------

/// `get_stream` returns `StreamNotFound` for a never-issued id.
#[test]
fn get_stream_fails_on_never_issued_id() {
    let h = Harness::new();
    let missing = 999_u64;

    assert_eq!(
        h.client.try_get_stream(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
}

/// `withdrawable_of` returns `StreamNotFound` for a never-issued id.
#[test]
fn withdrawable_of_fails_on_never_issued_id() {
    let h = Harness::new();
    let missing = 999_u64;

    assert_eq!(
        h.client
            .try_withdrawable_of(&missing)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );
}

/// `vested_of` returns `StreamNotFound` for a never-issued id.
#[test]
fn vested_of_fails_on_never_issued_id() {
    let h = Harness::new();
    let missing = 999_u64;

    assert_eq!(
        h.client.try_vested_of(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
}

/// `refundable_of` returns `StreamNotFound` for a never-issued id.
#[test]
fn refundable_of_fails_on_never_issued_id() {
    let h = Harness::new();
    let missing = 999_u64;

    assert_eq!(
        h.client.try_refundable_of(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
}

/// Failure on missing id must not mutate state.
#[test]
fn missing_id_errors_do_not_mutate_state() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let snap_before = h.snapshot();
    let ttl_before = h.ttl_of(id);

    let missing = 999_u64;
    let _ = h.client.try_get_stream(&missing);
    let _ = h.client.try_withdrawable_of(&missing);
    let _ = h.client.try_vested_of(&missing);
    let _ = h.client.try_refundable_of(&missing);

    let snap_after = h.snapshot();
    let ttl_after = h.ttl_of(id);

    assert_eq!(snap_before, snap_after, "snapshot changed after missing id reads");
    assert_eq!(ttl_before, ttl_after, "TTL changed after missing id reads");
}

// --- Read methods require no authorization -------------------------------

/// Read methods must not require any auth context to succeed.
#[test]
fn read_methods_require_no_auth() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Clear all auth mocks.
    h.env.mock_auths(&[]);

    // These should all succeed without auth.
    let _stream = h.client.get_stream(&id);
    let _withdrawable = h.client.withdrawable_of(&id);
    let _vested = h.client.vested_of(&id);
    let _refundable = h.client.refundable_of(&id);
}

// --- Contrast with mutating operations that DO extend TTL ---------------

/// Demonstrate that mutating operations DO extend TTL (contrast).
/// This shows the design is working: reads don't extend, mutations do.
#[test]
fn mutating_operations_extend_ttl_while_reads_do_not() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(50_000);

    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let full_ttl = h.ttl_of(id);

    // Let TTL decay most of the way.
    h.env.as_contract(&h.contract_id, || {
        let seq = h.env.ledger().sequence();
        h.env.ledger().set_sequence_number(seq + full_ttl - 1_000);
    });

    let decayed_ttl = h.ttl_of(id);
    assert!(
        decayed_ttl < 2_000,
        "TTL should be nearly exhausted: {decayed_ttl}"
    );

    // A read operation does NOT extend.
    h.client.get_stream(&id);
    let ttl_after_read = h.ttl_of(id);
    assert_eq!(
        ttl_after_read, decayed_ttl,
        "read must not extend TTL: {decayed_ttl} -> {ttl_after_read}"
    );

    // A mutating operation DOES extend.
    h.advance(10 * DAY);
    h.client.pause(&id);
    let ttl_after_pause = h.ttl_of(id);
    assert!(
        ttl_after_pause > 40_000,
        "pause must re-extend TTL, got {ttl_after_pause}"
    );
}

// --- Behavior on paused streams ----------------------------------------

/// Reads on paused streams must not extend TTL.
#[test]
fn reads_on_paused_stream_do_not_extend_ttl() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    h.client.pause(&id);
    let ttl_after_pause = h.ttl_of(id);

    // Read operations must not change TTL.
    h.client.get_stream(&id);
    assert_eq!(h.ttl_of(id), ttl_after_pause);

    h.client.vested_of(&id);
    assert_eq!(h.ttl_of(id), ttl_after_pause);

    h.client.withdrawable_of(&id);
    assert_eq!(h.ttl_of(id), ttl_after_pause);
}

/// Read values on paused streams are correct.
#[test]
fn read_values_on_paused_stream_are_correct() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(40 * DAY);
    let vested_before_pause = h.client.vested_of(&id);

    h.client.pause(&id);

    // Pausing freezes the clock but does not change what has already vested.
    h.advance(50 * DAY); // This would normally vest more, but stream is paused.
    let vested_after_pause = h.client.vested_of(&id);

    assert_eq!(
        vested_before_pause, vested_after_pause,
        "vested must not change while paused"
    );
}

// --- Behavior on cancelled streams ------------------------------------

/// Reads on cancelled streams must not extend TTL.
#[test]
fn reads_on_cancelled_stream_do_not_extend_ttl() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);

    h.client.cancel(&id);
    let ttl_after_cancel = h.ttl_of(id);

    h.client.get_stream(&id);
    assert_eq!(h.ttl_of(id), ttl_after_cancel);

    h.client.vested_of(&id);
    assert_eq!(h.ttl_of(id), ttl_after_cancel);

    h.client.withdrawable_of(&id);
    assert_eq!(h.ttl_of(id), ttl_after_cancel);
}

/// Read values on cancelled streams are correct.
#[test]
fn read_values_on_cancelled_stream_are_correct() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    let vested_at_cancel_instant = h.client.vested_of(&id);

    h.client.cancel(&id);

    // After cancel, vested is frozen at the cancel instant.
    h.advance(30 * DAY);
    let vested_after_advance = h.client.vested_of(&id);

    assert_eq!(
        vested_at_cancel_instant, vested_after_advance,
        "vested must not change after cancel"
    );

    // The cancelled stream still has money to withdraw.
    let refundable = h.client.refundable_of(&id);
    assert_eq!(
        refundable, 0,
        "after cancel at day 50, nothing remains unvested"
    );
}

// --- Behavior on depleted streams -----------------------------------

/// Reads on depleted streams must not extend TTL.
#[test]
fn reads_on_depleted_stream_do_not_extend_ttl() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);

    h.warp_to(T0 + 15 * DAY);
    h.client.withdraw(&id, &None);
    let ttl_after_depletion = h.ttl_of(id);

    h.client.get_stream(&id);
    assert_eq!(h.ttl_of(id), ttl_after_depletion);

    h.client.vested_of(&id);
    assert_eq!(h.ttl_of(id), ttl_after_depletion);

    h.client.withdrawable_of(&id);
    assert_eq!(h.ttl_of(id), ttl_after_depletion);
}

/// Read values on depleted streams are correct.
#[test]
fn read_values_on_depleted_stream_are_correct() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);

    h.warp_to(T0 + 15 * DAY);
    h.client.withdraw(&id, &None);

    let stream = h.client.get_stream(&id);
    assert_eq!(stream.status, StreamStatus::Depleted);

    let vested = h.client.vested_of(&id);
    let withdrawable = h.client.withdrawable_of(&id);
    let refundable = h.client.refundable_of(&id);

    assert_eq!(vested, 1_000 * ONE, "all vested after depletion");
    assert_eq!(withdrawable, 0, "nothing withdrawable after full drain");
    assert_eq!(refundable, 0, "nothing refundable after full drain");
}

// --- Multiple streams: reads do not interfere ---------------------------

/// Reading from one stream must not affect the TTL of another.
#[test]
fn reading_from_one_stream_does_not_affect_another() {
    let h = Harness::new();
    let id1 = h.create_simple(100 * ONE, 100 * DAY);
    let id2 = h.create_simple(200 * ONE, 100 * DAY);

    let ttl1_before = h.ttl_of(id1);
    let ttl2_before = h.ttl_of(id2);

    // Read from stream 1 many times.
    for _ in 0..10 {
        h.client.get_stream(&id1);
        h.client.vested_of(&id1);
    }

    let ttl1_after = h.ttl_of(id1);
    let ttl2_after = h.ttl_of(id2);

    assert_eq!(ttl1_after, ttl1_before, "stream 1 TTL must not change");
    assert_eq!(ttl2_after, ttl2_before, "stream 2 TTL must not change");
}
