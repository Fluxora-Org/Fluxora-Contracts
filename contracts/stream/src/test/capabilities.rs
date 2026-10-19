//! Issue #1726 — capability flags: NotCancellable, NotPausable, NotTransferable.
//!
//! These tests assert every acceptance criterion from issue #1726:
//!
//! 1. `get_stream` reports the three capability flags as set at creation.
//! 2. No supported entry point can mutate the flags after creation.
//! 3. Each flag causes its corresponding operation to be rejected.
//! 4. Streams can be created with every relevant flag combination and the
//!    corresponding operations are rejected in each case.
//!
//! # What is covered
//!
//! | Category | Tests |
//! |---|---|
//! | `get_stream` reporting | Flags are returned verbatim from `get_stream` |
//! | Individual flag enforcement | `NotCancellable`, `NotPausable`, `NotTransferable` |
//! | Delegate path enforcement | `delegate_cancel`, `delegate_pause`, `delegate_transfer_recipient` |
//! | Flag combinations | All four relevant multi-flag combinations |
//! | Immutability | Flags are unchanged after every supported mutation operation |
//! | Control cases | Operations succeed when the corresponding flag is `true` |
//!
//! # What is deliberately absent
//!
//! The per-operation regression tests (`a_non_cancellable_stream_cannot_be_cancelled_ever`,
//! etc.) are in `cancel.rs`, `pause.rs`, and `transfer.rs` and are not
//! duplicated here. This module provides the cross-cutting coverage that those
//! files do not: `get_stream` reporting, explicit immutability proof, and
//! multi-flag combination testing.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Assert that `get_stream` reports exactly the three expected flag values.
fn assert_flags(h: &Harness, id: u64, cancellable: bool, pausable: bool, transferable: bool) {
    let s = h.get(id);
    assert_eq!(
        s.cancellable, cancellable,
        "stream {id}: cancellable expected {cancellable}, got {}",
        s.cancellable
    );
    assert_eq!(
        s.pausable, pausable,
        "stream {id}: pausable expected {pausable}, got {}",
        s.pausable
    );
    assert_eq!(
        s.transferable, transferable,
        "stream {id}: transferable expected {transferable}, got {}",
        s.transferable
    );
}

// ---------------------------------------------------------------------------
// get_stream reporting
//
// Verify that get_stream returns the capability flags exactly as supplied to
// create_stream, for every combination of the three boolean arguments.
// ---------------------------------------------------------------------------

#[test]
fn get_stream_reports_all_flags_true() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        true,
    );
    assert_flags(&h, id, true, true, true);
}

#[test]
fn get_stream_reports_not_cancellable() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        true,
        true,
    );
    assert_flags(&h, id, false, true, true);
}

#[test]
fn get_stream_reports_not_pausable() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        false,
        true,
    );
    assert_flags(&h, id, true, false, true);
}

#[test]
fn get_stream_reports_not_transferable() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        false,
    );
    assert_flags(&h, id, true, true, false);
}

#[test]
fn get_stream_reports_all_flags_false() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        false,
        false,
    );
    assert_flags(&h, id, false, false, false);
}

// ---------------------------------------------------------------------------
// Individual flag enforcement — direct entry points
//
// Each test creates a stream with one flag cleared, attempts the corresponding
// operation, and asserts the expected error. A control case (same operation on
// a stream with the flag set) confirms it succeeds when permitted.
// ---------------------------------------------------------------------------

// --- NotCancellable ---------------------------------------------------------

/// `cancel` is rejected with `NotCancellable` when `cancellable == false`.
#[test]
fn not_cancellable_rejects_cancel() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        true,
        true,
    );

    h.advance(30 * DAY);
    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotCancellable);

    // Funds must stay in the pool; stream must remain live.
    assert_eq!(h.pool(), 1_000 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Active);
    h.assert_pool_exact();
}

/// `cancel` succeeds (control) when `cancellable == true`.
#[test]
fn cancellable_flag_true_allows_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    h.assert_pool_exact();
}

// --- NotPausable ------------------------------------------------------------

/// `pause` is rejected with `NotPausable` when `pausable == false`.
#[test]
fn not_pausable_rejects_pause() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        false,
        true,
    );

    h.advance(30 * DAY);
    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotPausable);

    // Stream stays Active; accrual continues.
    assert_eq!(h.get(id).status, StreamStatus::Active);
    h.assert_pool_exact();
}

/// `pause` succeeds (control) when `pausable == true`.
#[test]
fn pausable_flag_true_allows_pause() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.pause(&id);
    assert_eq!(h.get(id).status, StreamStatus::Paused);
    h.assert_pool_exact();
}

// --- NotTransferable --------------------------------------------------------

/// `transfer_recipient` is rejected with `NotTransferable` when `transferable == false`.
#[test]
fn not_transferable_rejects_transfer_recipient() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        false,
    );

    h.advance(30 * DAY);
    let err = h
        .client
        .try_transfer_recipient(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NotTransferable);

    // Recipient must not have changed.
    assert_eq!(h.get(id).recipient, h.recipient);
    h.assert_pool_exact();
}

/// `transfer_recipient` succeeds (control) when `transferable == true`.
#[test]
fn transferable_flag_true_allows_transfer_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(h.get(id).recipient, h.other);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Delegate path enforcement
//
// The delegate variants check the same flags as the direct entry points.
// Verify that the flags block operation even when invoked through a delegate.
// ---------------------------------------------------------------------------

/// `delegate_cancel` is rejected with `NotCancellable` on a non-cancellable stream.
#[test]
fn not_cancellable_rejects_delegate_cancel() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        true,
        true,
    );

    h.client
        .grant_delegate(&id, &h.sender, &h.other, &op::CANCEL, &None);

    h.advance(30 * DAY);
    let err = h
        .client
        .try_delegate_cancel(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NotCancellable);

    assert_eq!(h.get(id).status, StreamStatus::Active);
    h.assert_pool_exact();
}

/// `delegate_pause` is rejected with `NotPausable` on a non-pausable stream.
#[test]
fn not_pausable_rejects_delegate_pause() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        false,
        true,
    );

    h.client
        .grant_delegate(&id, &h.sender, &h.other, &op::PAUSE, &None);

    h.advance(30 * DAY);
    let err = h
        .client
        .try_delegate_pause(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NotPausable);

    assert_eq!(h.get(id).status, StreamStatus::Active);
    h.assert_pool_exact();
}

/// `delegate_transfer_recipient` is rejected with `NotTransferable` on a non-transferable stream.
#[test]
fn not_transferable_rejects_delegate_transfer_recipient() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        false,
    );

    // Grant TRANSFER_RECIPIENT to `other` from the recipient.
    h.client
        .grant_delegate(&id, &h.recipient, &h.other, &op::TRANSFER_RECIPIENT, &None);

    h.advance(30 * DAY);
    let third = Address::generate(&h.env);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &h.other, &third)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NotTransferable);

    assert_eq!(h.get(id).recipient, h.recipient);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Flag combinations
//
// Create streams with every relevant multi-flag combination and verify that
// each disabled operation is rejected independently.
// ---------------------------------------------------------------------------

// --- NotCancellable + NotPausable -------------------------------------------

#[test]
fn not_cancellable_and_not_pausable_both_rejected() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        false,
        true,
    );
    assert_flags(&h, id, false, false, true);

    h.advance(30 * DAY);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotCancellable);

    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotPausable);

    // Transfer must still succeed (transferable == true).
    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(h.get(id).recipient, h.other);
    h.assert_pool_exact();
}

// --- NotCancellable + NotTransferable ---------------------------------------

#[test]
fn not_cancellable_and_not_transferable_both_rejected() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        true,
        false,
    );
    assert_flags(&h, id, false, true, false);

    h.advance(30 * DAY);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotCancellable);

    let err = h
        .client
        .try_transfer_recipient(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NotTransferable);

    // Pause must still succeed (pausable == true).
    h.client.pause(&id);
    assert_eq!(h.get(id).status, StreamStatus::Paused);
    h.assert_pool_exact();
}

// --- NotPausable + NotTransferable ------------------------------------------

#[test]
fn not_pausable_and_not_transferable_both_rejected() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        false,
        false,
    );
    assert_flags(&h, id, true, false, false);

    h.advance(30 * DAY);

    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotPausable);

    let err = h
        .client
        .try_transfer_recipient(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NotTransferable);

    // Cancel must still succeed (cancellable == true).
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    h.assert_pool_exact();
}

// --- All three disabled ------------------------------------------------------

#[test]
fn all_three_flags_false_all_three_operations_rejected() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        false,
        false,
    );
    assert_flags(&h, id, false, false, false);

    h.advance(30 * DAY);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotCancellable);

    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotPausable);

    let err = h
        .client
        .try_transfer_recipient(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NotTransferable);

    // Stream is still Active; nothing was modified.
    assert_eq!(h.get(id).status, StreamStatus::Active);
    assert_eq!(h.pool(), 1_000 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Immutability
//
// Capability flags are established at creation and must not change afterwards.
// This section verifies that every supported mutation operation leaves the
// three flags exactly as they were at creation.
//
// Because there is no setter for these fields, the proof is exhaustive over the
// public API: run every mutating entry point on a stream and confirm the flags
// are unchanged after each one. The flags are creation-time properties, not
// mutable configuration.
// ---------------------------------------------------------------------------

/// After `top_up`, capability flags are unchanged.
#[test]
fn top_up_does_not_alter_capability_flags() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        false,
        false,
    );
    assert_flags(&h, id, false, false, false);

    h.advance(DAY);
    h.client.top_up(&id, &(100 * ONE));

    assert_flags(&h, id, false, false, false);
    h.assert_pool_exact();
}

/// After `withdraw`, capability flags are unchanged.
#[test]
fn withdraw_does_not_alter_capability_flags() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        false,
        false,
    );
    assert_flags(&h, id, false, false, false);

    h.advance(50 * DAY);
    h.client.withdraw(&id, &None);

    assert_flags(&h, id, false, false, false);
    h.assert_pool_exact();
}

/// After `pause` and `resume`, capability flags are unchanged.
#[test]
fn pause_and_resume_do_not_alter_capability_flags() {
    let h = Harness::new();
    let start = h.now();
    // pausable must be true to exercise pause/resume; the other two are false.
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        true,
        false,
    );
    assert_flags(&h, id, false, true, false);

    h.advance(30 * DAY);
    h.client.pause(&id);
    assert_flags(&h, id, false, true, false);

    h.advance(10 * DAY);
    h.client.resume(&id);
    assert_flags(&h, id, false, true, false);
    h.assert_pool_exact();
}

/// After `cancel`, capability flags are unchanged.
///
/// Cancellation rewrites `deposited` and `end_time` but must not touch the
/// three capability fields.
#[test]
fn cancel_does_not_alter_capability_flags() {
    let h = Harness::new();
    let start = h.now();
    // cancellable must be true to exercise cancel; the other two are false.
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        false,
        false,
    );
    assert_flags(&h, id, true, false, false);

    h.advance(40 * DAY);
    h.client.cancel(&id);

    assert_flags(&h, id, true, false, false);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    h.assert_pool_exact();
}

/// After `transfer_recipient`, capability flags are unchanged.
///
/// Transfer changes the recipient address only. The three capability fields
/// must remain at their creation values.
#[test]
fn transfer_recipient_does_not_alter_capability_flags() {
    let h = Harness::new();
    let start = h.now();
    // transferable must be true to exercise transfer; the other two are false.
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        false,
        true,
    );
    assert_flags(&h, id, false, false, true);

    h.advance(30 * DAY);
    h.client.transfer_recipient(&id, &h.other);

    assert_flags(&h, id, false, false, true);
    assert_eq!(h.get(id).recipient, h.other);
    h.assert_pool_exact();
}

/// Capability flags survive a full sequence of every supported mutation.
///
/// Creates a stream with all three flags false, then runs the operations that
/// *can* execute despite that (top_up, withdraw, extend_stream_ttl) and verifies
/// the flags are unchanged after each step. This is the exhaustive immutability
/// proof: no combination of supported mutations can alter creation-time flags.
#[test]
fn capability_flags_are_immutable_across_all_supported_mutations() {
    let h = Harness::new();
    let start = h.now();
    // All three disabled so there is no mutation operation that could
    // accidentally set one of them to true.
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false,
        false,
        false,
    );

    // Step 0: flags at creation.
    assert_flags(&h, id, false, false, false);

    // Step 1: top_up — extends the schedule.
    h.advance(DAY);
    h.client.top_up(&id, &(100 * ONE));
    assert_flags(&h, id, false, false, false);

    // Step 2: withdraw — reduces the outstanding liability.
    h.advance(49 * DAY);
    h.client.withdraw(&id, &None);
    assert_flags(&h, id, false, false, false);

    // Step 3: extend_stream_ttl — permissionless maintenance; changes no
    // stream fields, but confirmed here for completeness.
    h.client.extend_stream_ttl(&id);
    assert_flags(&h, id, false, false, false);

    // Step 4: final state check — status must still be Active.
    assert_eq!(h.get(id).status, StreamStatus::Active);
    h.assert_pool_exact();
}
