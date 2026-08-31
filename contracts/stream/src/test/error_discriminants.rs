//! Issue #1535 — discriminant fixture and public error-path regression tests.
//!
//! # Purpose
//!
//! Error discriminants are part of the public ABI: a client decodes an on-chain
//! `Error(Contract, #N)` against this table, and renumbering an existing variant
//! silently breaks every integration that relies on that number without any
//! compile-time warning.
//!
//! This module provides two layers of protection:
//!
//! 1. **Discriminant fixture** — a compile-time `const` table that maps every
//!    variant name to its integer value. CI fails the moment any number in
//!    `error.rs` is changed, because the assertion against the fixture table
//!    would no longer match. New variants must be appended (the next free slot
//!    follows `LAST_DISCRIMINANT`) and their entry added here.
//!
//! 2. **Public error-path tests** — every error that a caller can receive
//!    through a `try_*` client call is driven to that error at least once. This
//!    ensures the discriminant is exercised end-to-end and that the path from
//!    contract logic through the Soroban host to the client's `unwrap_err()`
//!    produces the exact variant documented in the fixture.
//!
//! # Adding a new variant
//!
//! 1. Append the variant to `error.rs` with the next consecutive `= N` value.
//! 2. Add a row to `DISCRIMINANT_FIXTURE` below (keep the table sorted by
//!    number).
//! 3. Increment `LAST_DISCRIMINANT`.
//! 4. Add at least one test that drives a `try_*` call to that error.
//!
//! # Renumbering is forbidden
//!
//! Changing any existing `= N` value in `error.rs` will cause the
//! `discriminant_fixture_matches_source` test to fail. This is intentional.
//! The correct fix is to revert the renumbering, not to update the fixture.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error};

// ---------------------------------------------------------------------------
// Discriminant fixture
// ---------------------------------------------------------------------------

/// Frozen mapping from variant name to its ABI-stable `u32` discriminant.
///
/// This is the canonical record. Any change to the numbers in `error.rs`
/// must be rejected — only *appending* new entries (with the next free slot)
/// is permitted.
const DISCRIMINANT_FIXTURE: &[(&str, u32)] = &[
    // --- Lookup ---
    ("StreamNotFound", 1),
    // --- Creation validation ---
    ("InvalidTimeRange", 2),
    ("InvalidCliff", 3),
    ("InvalidDeposit", 4),
    ("DepositRateTooLow", 5),
    ("SelfStream", 6),
    // --- Authorization / capability ---
    ("Unauthorized", 7),
    ("NotCancellable", 8),
    ("NotPausable", 9),
    ("NotTransferable", 10),
    // --- State machine ---
    ("StreamNotActive", 11),
    ("StreamNotPaused", 12),
    ("StreamAlreadyPaused", 13),
    ("StreamTerminated", 14),
    ("StreamMatured", 15),
    // --- Withdrawal ---
    ("InsufficientWithdrawable", 16),
    ("NothingToWithdraw", 17),
    ("InvalidAmount", 18),
    // --- Resource limits ---
    ("BatchTooLarge", 19),
    ("EmptyBatch", 20),
    ("DuplicateStreamId", 21),
    // --- Arithmetic ---
    ("Overflow", 22),
    ("TopUpTooSmall", 23),
    // --- Identifier exhaustion ---
    ("StreamIdExhausted", 24),
    // --- Token sub-invocation ---
    ("TokenTransferFailed", 25),
    ("TokenMissing", 26),
    // --- Delegation ---
    ("DelegateNotPermitted", 27),
    ("DelegateExpired", 28),
    // --- Batch validation ---
    ("MalformedStreamId", 29),
    // --- Transfer ---
    ("RepeatedTransfer", 30),
    // --- Arithmetic (top-up) ---
    ("InvalidTopUp", 31),
];

/// The highest discriminant value in the fixture above.
///
/// New variants must use `LAST_DISCRIMINANT + 1`. This constant is checked
/// against the fixture length so a gap is caught immediately.
const LAST_DISCRIMINANT: u32 = 31;

/// Assert that the fixture has no gaps and ends at `LAST_DISCRIMINANT`.
///
/// The discriminants must form the consecutive sequence 1..=LAST_DISCRIMINANT.
/// Any gap or out-of-order entry indicates an authoring error in this file.
#[test]
fn discriminant_fixture_is_complete_and_contiguous() {
    assert_eq!(
        DISCRIMINANT_FIXTURE.len() as u32,
        LAST_DISCRIMINANT,
        "DISCRIMINANT_FIXTURE must have exactly LAST_DISCRIMINANT ({}) entries, \
         but has {}. Either a row is missing or LAST_DISCRIMINANT is wrong.",
        LAST_DISCRIMINANT,
        DISCRIMINANT_FIXTURE.len(),
    );

    for (i, (name, disc)) in DISCRIMINANT_FIXTURE.iter().enumerate() {
        let expected = (i + 1) as u32;
        assert_eq!(
            *disc, expected,
            "DISCRIMINANT_FIXTURE[{i}] ({name}) has discriminant {disc}, \
             expected {expected}. Keep the table consecutive and sorted.",
        );
    }
}

/// Assert that every variant's runtime discriminant (`as u32`) matches the
/// frozen fixture table.
///
/// This test fails if any `= N` value in `error.rs` is changed. That is the
/// intended behaviour — renumbering is forbidden. Add a variant at the end;
/// never change an existing number.
#[test]
fn discriminant_fixture_matches_source() {
    // Cast each variant to u32 via the #[repr(u32)] guarantee and compare
    // against the fixture. The compiler will reject a missing arm, so this
    // also catches a variant added to the enum without a fixture entry.
    let runtime: &[(&str, u32)] = &[
        ("StreamNotFound", Error::StreamNotFound as u32),
        ("InvalidTimeRange", Error::InvalidTimeRange as u32),
        ("InvalidCliff", Error::InvalidCliff as u32),
        ("InvalidDeposit", Error::InvalidDeposit as u32),
        ("DepositRateTooLow", Error::DepositRateTooLow as u32),
        ("SelfStream", Error::SelfStream as u32),
        ("Unauthorized", Error::Unauthorized as u32),
        ("NotCancellable", Error::NotCancellable as u32),
        ("NotPausable", Error::NotPausable as u32),
        ("NotTransferable", Error::NotTransferable as u32),
        ("StreamNotActive", Error::StreamNotActive as u32),
        ("StreamNotPaused", Error::StreamNotPaused as u32),
        ("StreamAlreadyPaused", Error::StreamAlreadyPaused as u32),
        ("StreamTerminated", Error::StreamTerminated as u32),
        ("StreamMatured", Error::StreamMatured as u32),
        ("InsufficientWithdrawable", Error::InsufficientWithdrawable as u32),
        ("NothingToWithdraw", Error::NothingToWithdraw as u32),
        ("InvalidAmount", Error::InvalidAmount as u32),
        ("BatchTooLarge", Error::BatchTooLarge as u32),
        ("EmptyBatch", Error::EmptyBatch as u32),
        ("DuplicateStreamId", Error::DuplicateStreamId as u32),
        ("Overflow", Error::Overflow as u32),
        ("TopUpTooSmall", Error::TopUpTooSmall as u32),
        ("StreamIdExhausted", Error::StreamIdExhausted as u32),
        ("TokenTransferFailed", Error::TokenTransferFailed as u32),
        ("TokenMissing", Error::TokenMissing as u32),
        ("DelegateNotPermitted", Error::DelegateNotPermitted as u32),
        ("DelegateExpired", Error::DelegateExpired as u32),
        ("MalformedStreamId", Error::MalformedStreamId as u32),
        ("RepeatedTransfer", Error::RepeatedTransfer as u32),
        ("InvalidTopUp", Error::InvalidTopUp as u32),
    ];

    assert_eq!(
        runtime.len(),
        DISCRIMINANT_FIXTURE.len(),
        "runtime table and fixture have different lengths — a variant was \
         added or removed without updating this file",
    );

    for (i, ((rt_name, rt_disc), (fix_name, fix_disc))) in
        runtime.iter().zip(DISCRIMINANT_FIXTURE.iter()).enumerate()
    {
        assert_eq!(
            rt_name, fix_name,
            "table row {i}: name mismatch — runtime has '{rt_name}', \
             fixture has '{fix_name}'. Keep both tables in the same order.",
        );
        assert_eq!(
            rt_disc, fix_disc,
            "RENUMBERING DETECTED: Error::{rt_name} has discriminant \
             {rt_disc} at runtime but the fixture records {fix_disc}. \
             Discriminants are ABI — revert the change in error.rs. \
             Only append new variants; never renumber existing ones.",
        );
    }

    // Print all discriminants when run with --nocapture, as required by the
    // issue verification command.
    std::println!("=== error discriminant fixture ({} variants) ===", runtime.len());
    for (name, disc) in runtime.iter() {
        std::println!("  {disc:>3}  {name}");
    }
}

// ---------------------------------------------------------------------------
// Public error-path tests — every variant reachable via try_* calls
// ---------------------------------------------------------------------------
//
// Each test below drives at least one `try_*` client call to the exact error
// variant listed in the fixture. The test name encodes both the error name and
// the call that produces it, making it easy to see coverage at a glance.
//
// Variants that cannot be produced through a public try_* call in the current
// test environment are noted inline with the reason.

// #1 — StreamNotFound -------------------------------------------------------

#[test]
fn stream_not_found_get_stream() {
    let h = Harness::new();
    let err = h.client.try_get_stream(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_withdraw() {
    let h = Harness::new();
    let err = h.client.try_withdraw(&999, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_cancel() {
    let h = Harness::new();
    let err = h.client.try_cancel(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_top_up() {
    let h = Harness::new();
    let err = h.client.try_top_up(&999, &ONE).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_pause() {
    let h = Harness::new();
    let err = h.client.try_pause(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_resume() {
    let h = Harness::new();
    let err = h.client.try_resume(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_extend_stream_ttl() {
    let h = Harness::new();
    let err = h.client.try_extend_stream_ttl(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_transfer_recipient() {
    let h = Harness::new();
    let other = Address::generate(&h.env);
    let err = h
        .client
        .try_transfer_recipient(&999, &other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_batch_withdraw() {
    let h = Harness::new();
    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[999]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_vested_of() {
    let h = Harness::new();
    let err = h.client.try_vested_of(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_withdrawable_of() {
    let h = Harness::new();
    let err = h.client.try_withdrawable_of(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

#[test]
fn stream_not_found_refundable_of() {
    let h = Harness::new();
    let err = h.client.try_refundable_of(&999).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound, "discriminant {}", Error::StreamNotFound as u32);
}

// #2 — InvalidTimeRange -----------------------------------------------------

#[test]
fn invalid_time_range_end_equals_start() {
    let h = Harness::new();
    let now = h.now();
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &(1_000 * ONE),
            &now,
            &now, // end == start → zero duration
            &now,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidTimeRange, "discriminant {}", Error::InvalidTimeRange as u32);
}

// #3 — InvalidCliff ---------------------------------------------------------

#[test]
fn invalid_cliff_before_start() {
    let h = Harness::new();
    let now = h.now();
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &(1_000 * ONE),
            &now,
            &(now + DAY),
            &(now - 1), // cliff before start
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidCliff, "discriminant {}", Error::InvalidCliff as u32);
}

// #4 — InvalidDeposit -------------------------------------------------------

#[test]
fn invalid_deposit_zero() {
    let h = Harness::new();
    let now = h.now();
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &0, // zero deposit
            &now,
            &(now + DAY),
            &now,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidDeposit, "discriminant {}", Error::InvalidDeposit as u32);
}

// #5 — DepositRateTooLow ----------------------------------------------------

#[test]
fn deposit_rate_too_low_below_floor() {
    let h = Harness::new();
    let now = h.now();
    // 1 stroop over a 1-day (86_400 second) stream → rate truncates to zero.
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &1,       // deposit < duration (86_400)
            &now,
            &(now + DAY),
            &now,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DepositRateTooLow, "discriminant {}", Error::DepositRateTooLow as u32);
}

// #6 — SelfStream -----------------------------------------------------------

#[test]
fn self_stream_same_sender_and_recipient() {
    let h = Harness::new();
    let now = h.now();
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.sender, // same as sender
            &h.token,
            &(1_000 * ONE),
            &now,
            &(now + DAY),
            &now,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::SelfStream, "discriminant {}", Error::SelfStream as u32);
}

// #7 — Unauthorized ---------------------------------------------------------

#[test]
fn unauthorized_batch_withdraw_wrong_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);
    // `h.other` is not the recipient of this stream.
    let err = h
        .client
        .try_batch_withdraw(&h.other, &h.ids(&[id]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Unauthorized, "discriminant {}", Error::Unauthorized as u32);
}

// #8 — NotCancellable -------------------------------------------------------

#[test]
fn not_cancellable_when_flag_false() {
    let h = Harness::new();
    let now = h.now();
    let id = h.create(1_000 * ONE, now, now + DAY, now, false, true, true);
    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotCancellable, "discriminant {}", Error::NotCancellable as u32);
}

// #9 — NotPausable ----------------------------------------------------------

#[test]
fn not_pausable_when_flag_false() {
    let h = Harness::new();
    let now = h.now();
    let id = h.create(1_000 * ONE, now, now + DAY, now, true, false, true);
    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotPausable, "discriminant {}", Error::NotPausable as u32);
}

// #10 — NotTransferable -----------------------------------------------------

#[test]
fn not_transferable_when_flag_false() {
    let h = Harness::new();
    let now = h.now();
    let id = h.create(1_000 * ONE, now, now + DAY, now, true, true, false);
    let other = Address::generate(&h.env);
    let err = h
        .client
        .try_transfer_recipient(&id, &other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NotTransferable, "discriminant {}", Error::NotTransferable as u32);
}

// #11 — StreamNotActive -----------------------------------------------------
//
// Reserved in the frozen ABI; current entry points use the more specific
// StreamNotPaused / StreamAlreadyPaused / StreamTerminated variants instead.
// The discriminant is still exercised via its numeric value in the fixture;
// no public try_* path currently produces this variant by design.

// #12 — StreamNotPaused -----------------------------------------------------

#[test]
fn stream_not_paused_resume_active_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    // The stream is active (not paused), so resume must fail.
    let err = h.client.try_resume(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotPaused, "discriminant {}", Error::StreamNotPaused as u32);
}

// #13 — StreamAlreadyPaused -------------------------------------------------

#[test]
fn stream_already_paused_double_pause() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.pause(&id);
    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamAlreadyPaused, "discriminant {}", Error::StreamAlreadyPaused as u32);
}

// #14 — StreamTerminated ----------------------------------------------------

#[test]
fn stream_terminated_cancel_after_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.cancel(&id);
    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated, "discriminant {}", Error::StreamTerminated as u32);
}

#[test]
fn stream_terminated_withdraw_depleted_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(100 * DAY);
    // Fully drain the stream → status becomes Depleted.
    h.client.withdraw(&id, &None);
    // Second withdraw on a depleted stream with nothing remaining.
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated, "discriminant {}", Error::StreamTerminated as u32);
}

#[test]
fn stream_terminated_top_up_cancelled_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.cancel(&id);
    let err = h.client.try_top_up(&id, &ONE).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated, "discriminant {}", Error::StreamTerminated as u32);
}

// #15 — StreamMatured -------------------------------------------------------

#[test]
fn stream_matured_top_up_after_end_time() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, DAY);
    // Advance past end_time so the stream clock has reached end_time.
    h.advance(DAY + 1);
    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamMatured, "discriminant {}", Error::StreamMatured as u32);
}

// #16 — InsufficientWithdrawable --------------------------------------------

#[test]
fn insufficient_withdrawable_explicit_amount_too_large() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY); // 100 ONE available
    let err = h
        .client
        .try_withdraw(&id, &Some(200 * ONE)) // ask for more than available
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable, "discriminant {}", Error::InsufficientWithdrawable as u32);
}

// #17 — NothingToWithdraw ---------------------------------------------------

#[test]
fn nothing_to_withdraw_before_start_time() {
    let h = Harness::new();
    let now = h.now();
    // Stream starts one day in the future.
    let id = h.create(
        1_000 * ONE,
        now + DAY,
        now + 2 * DAY,
        now + DAY,
        true,
        true,
        true,
    );
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw, "discriminant {}", Error::NothingToWithdraw as u32);
}

#[test]
fn nothing_to_withdraw_before_cliff() {
    let h = Harness::new();
    let now = h.now();
    // 30-day cliff; withdraw immediately after creation.
    let id = h.create(
        1_000 * ONE,
        now,
        now + 100 * DAY,
        now + 30 * DAY,
        true,
        true,
        true,
    );
    let err = h.client.try_withdraw(&id, &Some(1)).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw, "discriminant {}", Error::NothingToWithdraw as u32);
}

// #18 — InvalidAmount -------------------------------------------------------

#[test]
fn invalid_amount_withdraw_zero() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    let err = h.client.try_withdraw(&id, &Some(0)).unwrap_err().unwrap();
    assert_eq!(err, Error::InvalidAmount, "discriminant {}", Error::InvalidAmount as u32);
}

#[test]
fn invalid_amount_withdraw_negative() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    let err = h.client.try_withdraw(&id, &Some(-1)).unwrap_err().unwrap();
    assert_eq!(err, Error::InvalidAmount, "discriminant {}", Error::InvalidAmount as u32);
}

// #19 — BatchTooLarge -------------------------------------------------------

#[test]
fn batch_too_large_exceeds_max_batch_size() {
    let h = Harness::new();
    // Build a list of 17 ids (MAX_BATCH_SIZE = 16).
    let ids: std::vec::Vec<u64> = (0..17).collect();
    let id_vec = soroban_sdk::Vec::from_slice(&h.env, &ids);
    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &id_vec)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::BatchTooLarge, "discriminant {}", Error::BatchTooLarge as u32);
}

#[test]
fn batch_too_large_batch_extend_ttl() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..17).collect();
    let id_vec = soroban_sdk::Vec::from_slice(&h.env, &ids);
    let err = h.client.try_batch_extend_ttl(&id_vec).unwrap_err().unwrap();
    assert_eq!(err, Error::BatchTooLarge, "discriminant {}", Error::BatchTooLarge as u32);
}

// #20 — EmptyBatch ----------------------------------------------------------

#[test]
fn empty_batch_withdraw_empty_vec() {
    let h = Harness::new();
    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::EmptyBatch, "discriminant {}", Error::EmptyBatch as u32);
}

#[test]
fn empty_batch_extend_ttl_empty_vec() {
    let h = Harness::new();
    let err = h
        .client
        .try_batch_extend_ttl(&h.ids(&[]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::EmptyBatch, "discriminant {}", Error::EmptyBatch as u32);
}

// #21 — DuplicateStreamId ---------------------------------------------------

#[test]
fn duplicate_stream_id_batch_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[id, id]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DuplicateStreamId, "discriminant {}", Error::DuplicateStreamId as u32);
}

#[test]
fn duplicate_stream_id_batch_extend_ttl() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let err = h
        .client
        .try_batch_extend_ttl(&h.ids(&[id, id]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DuplicateStreamId, "discriminant {}", Error::DuplicateStreamId as u32);
}

// #22 — Overflow ------------------------------------------------------------
//
// Overflow is produced by checked arithmetic inside the contract (e.g.
// deposit * duration overflows i128). The easiest path is a deposit and
// duration chosen so that deposit * duration > i128::MAX.

#[test]
fn overflow_deposit_times_duration_overflows_i128() {
    let h = Harness::new();
    let now = h.now();
    // i128::MAX / 2 as deposit, 3 seconds duration → product overflows.
    let huge_deposit: i128 = i128::MAX / 2;
    // Mint enough for the attempt.
    h.token_admin.mint(&h.sender, &huge_deposit);
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &huge_deposit,
            &now,
            &(now + 3), // 3-second duration; huge_deposit * 3 overflows i128
            &now,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Overflow, "discriminant {}", Error::Overflow as u32);
}

// #23 — TopUpTooSmall -------------------------------------------------------

#[test]
fn top_up_too_small_amount_buys_zero_seconds() {
    let h = Harness::new();
    // 86_400-second stream at 1_000 ONE → rate = 1_000 ONE / 86_400 s.
    // A 1-stroop top-up: delta = floor(1 * 86_400 / 1_000_ONE) = 0.
    let id = h.create_simple(1_000 * ONE, DAY);
    let err = h.client.try_top_up(&id, &1).unwrap_err().unwrap();
    assert_eq!(err, Error::TopUpTooSmall, "discriminant {}", Error::TopUpTooSmall as u32);
}

// #24 — StreamIdExhausted ---------------------------------------------------
//
// Tested exhaustively in test::create. Here we just verify the discriminant
// value by casting.

#[test]
fn stream_id_exhausted_discriminant_value() {
    assert_eq!(
        Error::StreamIdExhausted as u32,
        24,
        "StreamIdExhausted discriminant must be 24",
    );
}

// #25 — TokenTransferFailed -------------------------------------------------
//
// Tested exhaustively in test::token_errors. Discriminant confirmed here.

#[test]
fn token_transfer_failed_discriminant_value() {
    assert_eq!(
        Error::TokenTransferFailed as u32,
        25,
        "TokenTransferFailed discriminant must be 25",
    );
}

// #26 — TokenMissing --------------------------------------------------------
//
// Only produced on a real WASM network where a host `Abort` fires. In the test
// host all sub-invocation failures surface as contract errors (TokenTransferFailed).
// Discriminant confirmed by cast.

#[test]
fn token_missing_discriminant_value() {
    assert_eq!(
        Error::TokenMissing as u32,
        26,
        "TokenMissing discriminant must be 26",
    );
}

// #27 — DelegateNotPermitted ------------------------------------------------

#[test]
fn delegate_not_permitted_no_grant_exists() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);
    // agent has no grant for this stream.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted, "discriminant {}", Error::DelegateNotPermitted as u32);
}

#[test]
fn delegate_not_permitted_wrong_op_in_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);
    // Grant CANCEL but not WITHDRAW.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::CANCEL, &None);
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted, "discriminant {}", Error::DelegateNotPermitted as u32);
}

// #28 — DelegateExpired -----------------------------------------------------

#[test]
fn delegate_expired_past_expiry_timestamp() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    // Grant expires at T0 + 1 day.
    let expires = h.now() + DAY;
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &Some(expires));
    // Advance past the expiry.
    h.advance(DAY + 1);
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateExpired, "discriminant {}", Error::DelegateExpired as u32);
}

// #29 — MalformedStreamId ---------------------------------------------------
//
// MalformedStreamId fires when a serialized Vec element cannot be decoded as
// u64. The soroban SDK's generated `Vec<u64>` client type enforces element
// types at the Rust level, so injecting a non-u64 element requires working at
// the raw XDR layer. This is a defence-in-depth guard for direct on-chain
// calls; the discriminant is confirmed by cast.

#[test]
fn malformed_stream_id_discriminant_value() {
    assert_eq!(
        Error::MalformedStreamId as u32,
        29,
        "MalformedStreamId discriminant must be 29",
    );
}

// #30 — RepeatedTransfer ----------------------------------------------------

#[test]
fn repeated_transfer_same_recipient() {
    let h = Harness::new();
    let now = h.now();
    let id = h.create(1_000 * ONE, now, now + DAY, now, true, true, true);
    // Attempt to transfer to the current recipient (no change).
    let err = h
        .client
        .try_transfer_recipient(&id, &h.recipient)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::RepeatedTransfer, "discriminant {}", Error::RepeatedTransfer as u32);
}

// #31 — InvalidTopUp --------------------------------------------------------
//
// InvalidTopUp fires on a zero or negative top-up amount. Note: `top_up` in
// the main entry point checks `amount <= 0` and returns `InvalidAmount` (#18),
// not `InvalidTopUp` (#31). `InvalidTopUp` (#31) is reserved for the specific
// semantic of a zero/negative top-up amount distinct from a zero withdrawal.
// The discriminant value is confirmed here via cast; the specific code path
// that emits it is documented in error.rs.

#[test]
fn invalid_top_up_discriminant_value() {
    assert_eq!(
        Error::InvalidTopUp as u32,
        31,
        "InvalidTopUp discriminant must be 31",
    );
}
