//! Regression test for `FactoryError` discriminant stability.
//!
//! Guards `docs/error.md`'s factory-section discriminant table. The factory
//! contract persists state by the public discriminant of every user-facing
//! error variant, so a silent reorder or reassignment would corrupt every
//! client-side error decoder.
//!
//! This test is intentionally independent of the Soroban testutils runtime:
//! it is a pure compile-time + cheap runtime assertion so it can run in CI
//! without any token, ledger, or stream-contract deployment.
//!
//! Adding a new variant?
//! 1. Append it at the END of `FactoryError` in `contracts/factory/src/lib.rs`
//!    with the next available integer, never reordering assignments.
//! 2. Add the matching `assert_eq!` to this test on the line that follows
//!    the last existing assertion (keeping ascending order preserved).
//! 3. Update the discriminant table in `docs/error.md` under
//!    `## FactoryError Reference (Factory Contract)`.
//!
//! The companion stream-contract guard lives at
//! `contracts/stream/src/test.rs::test_contract_error_discriminants_are_stable`.

#![cfg(test)]

use fluxora_factory::FactoryError;

/// Compile-time + cheap runtime stability check.
///
/// Asserts the exact `u32` representation of every variant listed in
/// `docs/error.md`. Failure here means a discriminant drift between source
/// and documentation — fix both the source (preferred: append-only) and the
/// docs table before merging.
#[test]
fn test_factory_error_discriminants_are_stable() {
    // ── Initialization & auth ───────────────────────────────────────────
    assert_eq!(FactoryError::AlreadyInitialized as u32, 1);
    assert_eq!(FactoryError::NotInitialized as u32, 2);
    assert_eq!(FactoryError::Unauthorized as u32, 3);

    // ── Per-stream creation guards ──────────────────────────────────────
    assert_eq!(FactoryError::RecipientNotAllowlisted as u32, 4);
    assert_eq!(FactoryError::DepositExceedsCap as u32, 5);
    assert_eq!(FactoryError::DurationTooShort as u32, 6);
    assert_eq!(FactoryError::InvalidTimeRange as u32, 7);
    assert_eq!(FactoryError::InvalidCliff as u32, 8);

    // ── Pause / cross-contract ───────────────────────────────────────────
    assert_eq!(FactoryError::CreationPaused as u32, 9);
    assert_eq!(FactoryError::StreamContractPaused as u32, 10);
    // 11 is the cross-contract failure wrapper (see docs/error.md).
    assert_eq!(FactoryError::StreamContractError as u32, 11);

    // ── Rate bounds ─────────────────────────────────────────────────────
    assert_eq!(FactoryError::RateBelowMin as u32, 12);
    assert_eq!(FactoryError::RateAboveMax as u32, 13);

    // ── Policy setters ───────────────────────────────────────────────────
    assert_eq!(FactoryError::InvalidCap as u32, 14);
    assert_eq!(FactoryError::InvalidMinDuration as u32, 15);

    // ── Memo ─────────────────────────────────────────────────────────────
    assert_eq!(FactoryError::InvalidMemo as u32, 16);
}

/// Each discriminant in the table must be unique. Catches accidental duplicate
/// assignments (e.g. two variants pasted with the same `= N` value).
#[test]
fn test_factory_error_discriminants_are_unique() {
    use FactoryError::*;
    let all = [
        AlreadyInitialized,
        NotInitialized,
        Unauthorized,
        RecipientNotAllowlisted,
        DepositExceedsCap,
        DurationTooShort,
        InvalidTimeRange,
        InvalidCliff,
        CreationPaused,
        StreamContractPaused,
        StreamContractError,
        RateBelowMin,
        RateAboveMax,
        InvalidCap,
        InvalidMinDuration,
        InvalidMemo,
    ];
    let mut seen = std::collections::HashSet::with_capacity(all.len());
    for variant in all.iter() {
        let disc = *variant as u32;
        assert!(
            seen.insert(disc),
            "FactoryError discriminant {disc} appears more than once (variant: {variant:?})",
        );
    }
    assert_eq!(seen.len(), all.len(), "variant count mismatch");
}

/// Compactness check: there must be no gaps in the discriminant range.
/// Gaps indicate either a deleted variant (a breaking ABI change) or a
/// mis-typed numbering. Documented range is `1..=16`.
#[test]
fn test_factory_error_discriminants_are_dense_no_gaps() {
    use FactoryError::*;
    let all = [
        AlreadyInitialized,
        NotInitialized,
        Unauthorized,
        RecipientNotAllowlisted,
        DepositExceedsCap,
        DurationTooShort,
        InvalidTimeRange,
        InvalidCliff,
        CreationPaused,
        StreamContractPaused,
        StreamContractError,
        RateBelowMin,
        RateAboveMax,
        InvalidCap,
        InvalidMinDuration,
        InvalidMemo,
    ];
    let mut discs: Vec<u32> = all.iter().map(|v| *v as u32).collect();
    discs.sort_unstable();
    assert_eq!(
        discs[0], 1,
        "FactoryError first discriminant is {} (expected 1)",
        discs[0]
    );
    let expected_max = all.len() as u32;
    assert_eq!(
        discs[discs.len() - 1],
        expected_max,
        "FactoryError last discriminant is {} (expected {})",
        discs[discs.len() - 1],
        expected_max
    );
    for (i, disc) in discs.iter().enumerate() {
        assert_eq!(
            *disc,
            (i + 1) as u32,
            "FactoryError discriminant gap or reorder at position {} (got {}, expected {})",
            i,
            disc,
            (i + 1) as u32
        );
    }
}
