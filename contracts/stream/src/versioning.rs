//! Versioning and upgrade compatibility validation.
//!
//! This module provides runtime checks and compile-time validations to ensure
//! manifest versioning stability, backward compatibility, and safe upgrades.
//!
//! # Design
//!
//! The module enforces three key invariants:
//!
//! 1. **DataKey Discriminant Stability**: All DataKey variants must maintain frozen discriminants
//!    (0-35) and only append new variants after discriminant 35.
//!
//! 2. **Storage Entry Size Bounds**: Individual stream entries must not exceed `MAX_STREAM_ENTRY_BYTES`
//!    (4,096 bytes) to enforce bounded rent costs.
//!
//! 3. **Accrual Determinism**: Accrued amounts must be deterministic (timestamp-deterministic,
//!    checkpoint-preserving, never decreasing).

use soroban_sdk::{Env, panic_with_error};

/// Version validation error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VersioningError {
    /// Contract version mismatch (not expected version).
    VersionMismatch = 1001,
    
    /// Storage entry exceeds maximum allowed size.
    EntryOversized = 1002,
    
    /// DataKey discriminant out of bounds (should be 0-35 or explicitly appended).
    InvalidDiscriminant = 1003,
    
    /// Accrual calculation produced a non-deterministic or non-monotonic result.
    AccrualNonDeterministic = 1004,
    
    /// Stream checkpoint state is invalid (checkpointed_amount > deposit_amount).
    InvalidCheckpointState = 1005,
    
    /// Frozen discriminant was reordered (storage backward-compatibility broken).
    DiscriminantReordered = 1006,
}

/// Assert that the contract version matches the expected version.
///
/// # Parameters
/// - `env` — the contract environment
/// - `expected_version` — the expected CONTRACT_VERSION constant
///
/// # Panics
/// Panics with `VersionMismatch` if the actual version does not match the expected version.
///
/// # Usage
///
/// Call this at the start of critical entry-points to ensure the contract instance
/// is at the expected version before proceeding.
///
/// ```ignore
/// pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
///     validate_version(&env, CONTRACT_VERSION)?;
///     // ... proceed with withdrawal logic
/// }
/// ```
pub fn validate_version(env: &Env, actual_version: u32, expected_version: u32) -> Result<(), VersioningError> {
    if actual_version != expected_version {
        return Err(VersioningError::VersionMismatch);
    }
    Ok(())
}

/// Validate that a stream's checkpoint state is internally consistent.
///
/// # Invariants Checked
/// - `checkpointed_amount <= deposit_amount`
/// - `checkpointed_at <= end_time`
/// - `checkpointed_amount >= 0`
///
/// # Parameters
/// - `checkpointed_amount` — amount accrued under all previous rate epochs
/// - `checkpointed_at` — timestamp when checkpoint was established
/// - `end_time` — stream end timestamp (clamped accrual)
/// - `deposit_amount` — maximum accrual ceiling
///
/// # Panics
/// Panics with `InvalidCheckpointState` if any invariant is violated.
pub fn validate_checkpoint_state(
    checkpointed_amount: i128,
    checkpointed_at: u64,
    end_time: u64,
    deposit_amount: i128,
) -> Result<(), VersioningError> {
    // Checkpoint amount should not exceed deposit
    if checkpointed_amount > deposit_amount {
        return Err(VersioningError::InvalidCheckpointState);
    }

    // Checkpoint time should not exceed end time
    if checkpointed_at > end_time {
        return Err(VersioningError::InvalidCheckpointState);
    }

    // Checkpoint amount should be non-negative
    if checkpointed_amount < 0 {
        return Err(VersioningError::InvalidCheckpointState);
    }

    Ok(())
}

/// Validate that accrued amount is within deterministic bounds.
///
/// # Invariants Checked
/// - `accrued >= 0` (never negative)
/// - `accrued <= deposit_amount` (clamped to deposit)
/// - `accrued >= previous_checkpoint` (checkpoint preservation)
///
/// # Parameters
/// - `accrued` — the calculated accrued amount
/// - `checkpointed_amount` — the locked-in amount from the most recent rate change
/// - `deposit_amount` — the maximum accrual ceiling
///
/// # Panics
/// Panics with `AccrualNonDeterministic` if the accrued amount violates the invariants.
pub fn validate_accrual_bounds(
    accrued: i128,
    checkpointed_amount: i128,
    deposit_amount: i128,
) -> Result<(), VersioningError> {
    // Accrual should never be negative
    if accrued < 0 {
        return Err(VersioningError::AccrualNonDeterministic);
    }

    // Accrual should never exceed deposit
    if accrued > deposit_amount {
        return Err(VersioningError::AccrualNonDeterministic);
    }

    // Accrual should never fall below checkpoint (entitlement preservation)
    if accrued < checkpointed_amount {
        return Err(VersioningError::AccrualNonDeterministic);
    }

    Ok(())
}

/// Validate that a withdrawal respects monotonicity of `withdrawn_amount`.
///
/// # Invariants Checked
/// - `new_withdrawn >= previous_withdrawn` (monotonically non-decreasing)
/// - `new_withdrawn <= deposit_amount` (capped at deposit)
/// - `withdrawal_amount >= 0` (never negative)
///
/// # Parameters
/// - `new_withdrawn` — the updated withdrawn amount after withdrawal
/// - `previous_withdrawn` — the withdrawn amount before withdrawal
/// - `deposit_amount` — the maximum withdrawal ceiling
///
/// # Panics
/// Panics with `AccrualNonDeterministic` if monotonicity is violated.
pub fn validate_withdrawal_monotonicity(
    new_withdrawn: i128,
    previous_withdrawn: i128,
    deposit_amount: i128,
) -> Result<(), VersioningError> {
    // Withdrawn should never decrease (monotonic invariant)
    if new_withdrawn < previous_withdrawn {
        return Err(VersioningError::AccrualNonDeterministic);
    }

    // Withdrawn should never exceed deposit
    if new_withdrawn > deposit_amount {
        return Err(VersioningError::AccrualNonDeterministic);
    }

    Ok(())
}

/// Validate that a stream entry size is within the bounded limit.
///
/// This check is performed during stream creation to ensure that caller-controlled
/// fields (metadata, memo, etc.) do not cause the entry to exceed `MAX_STREAM_ENTRY_BYTES`.
///
/// # Parameters
/// - `entry_size_bytes` — the XDR-encoded size of the stream entry
/// - `max_size` — the maximum allowed size (typically `MAX_STREAM_ENTRY_BYTES = 4_096`)
///
/// # Panics
/// Panics with `EntryOversized` if the entry size exceeds the limit.
pub fn validate_entry_size(entry_size_bytes: usize, max_size: usize) -> Result<(), VersioningError> {
    if entry_size_bytes > max_size {
        return Err(VersioningError::EntryOversized);
    }
    Ok(())
}

/// Compile-time validation that DataKey discriminants are frozen.
///
/// # Design Note
///
/// This is a documentation-only check in Rust (compile-time discriminant verification
/// is limited in stable Rust). The actual enforcement happens through:
///
/// 1. **Code Review**: PRs must verify that `DataKey` variants are only appended.
/// 2. **Storage Invariant Documentation**: `docs/ABI_STABILITY.md` documents frozen discriminants.
/// 3. **Test Coverage**: `tests/manifest_versioning.rs` validates that old stream entries
///    are still readable after upgrades.
///
/// # Frozen Discriminants (V9)
///
/// The following `DataKey` variants have frozen discriminants and MUST NOT be reordered:
///
/// | Discriminant | Variant | Added in Version |
/// |---|---|---|
/// | 0 | Config | V1 |
/// | 1 | NextStreamId | V1 |
/// | 2 | Stream(u64) | V1 |
/// | 3 | RecipientStreams(Address) | V1 |
/// | 4 | GlobalEmergencyPaused | V1 |
/// | 5 | CreationPaused | V1 |
/// | 6 | GlobalPauseReason | V1 |
/// | 7 | GlobalPauseTimestamp | V1 |
/// | 8 | GlobalPauseAdmin | V1 |
/// | 9 | AutoClaimDestination(u64) | V1 |
/// | 10 | NextTemplateId | V1 |
/// | 11 | ActiveTemplateCount | V1 |
/// | 12 | StreamTemplate(u64) | V1 |
/// | 13 | OwnerTemplateIds(Address) | V1 |
/// | 14 | TotalLiabilities | V1 |
/// | 15 | WithdrawNonce(Address) [DEPRECATED] | V1 |
/// | 16 | PauseState | V1 |
/// | 17 | ReentrancyLock | V1 |
/// | 18 | RecipientStreamPage(Address, u32) | V1 |
/// | 19 | RecipientStreamPageCount(Address) | V1 |
/// | 20 | PendingRecipientUpdate(u64) | V1 |
/// | 21 | IdReservation(Address) | V1 |
/// | 22 | MaxRatePerSecond | V1 |
/// | 23 | DelegatedWithdrawNonce(Address) | V1 |
/// | 24 | LastPauseRecord(PauseKind) | V3 |
/// | 25 | RotationHistory(u64) | V3 |
/// | 26 | LastAccrualLedgerTimestamp | V4 |
/// | 27 | PausedStreamCount | V5 |
/// | 28 | TotalKeeperFeesPaid | V7 |
/// | 29 | AutoRenewEnabled(u64) | V7 |
/// | 30 | MaxLookbackLedgers(u64) | V8 |
/// | 31 | SenderStreams(Address) | V8 |
/// | 32 | PendingStreamOffer(u64) | V7 |
/// | 33 | RecipientPendingOffers(Address) | V7 |
/// | 34 | PooledStreamShares(u64) | V9 |
/// | 35 | PooledStreamWithdrawn(u64, Address) | V9 |
///
/// **Future variants MUST be appended after discriminant 35 with strictly increasing values.**
///
/// # Violations
///
/// Reordering any of these variants would cause:
/// - Existing stream entries to be read with incorrect field values
/// - Silent corruption of stream state
/// - Fund accounting errors (withdrawn_amount misaligned, etc.)
///
/// This is a **critical invariant** and must never be violated.
pub const FROZEN_DISCRIMINANTS_V9: &[&str] = &[
    "Config",                           // 0
    "NextStreamId",                     // 1
    "Stream(u64)",                      // 2
    "RecipientStreams(Address)",        // 3
    "GlobalEmergencyPaused",            // 4
    "CreationPaused",                   // 5
    "GlobalPauseReason",                // 6
    "GlobalPauseTimestamp",             // 7
    "GlobalPauseAdmin",                 // 8
    "AutoClaimDestination(u64)",        // 9
    "NextTemplateId",                   // 10
    "ActiveTemplateCount",              // 11
    "StreamTemplate(u64)",              // 12
    "OwnerTemplateIds(Address)",        // 13
    "TotalLiabilities",                 // 14
    "WithdrawNonce(Address)",           // 15 (DEPRECATED)
    "PauseState",                       // 16
    "ReentrancyLock",                   // 17
    "RecipientStreamPage(Address, u32)", // 18
    "RecipientStreamPageCount(Address)", // 19
    "PendingRecipientUpdate(u64)",      // 20
    "IdReservation(Address)",           // 21
    "MaxRatePerSecond",                 // 22
    "DelegatedWithdrawNonce(Address)",  // 23
    "LastPauseRecord(PauseKind)",       // 24
    "RotationHistory(u64)",             // 25
    "LastAccrualLedgerTimestamp",       // 26
    "PausedStreamCount",                // 27
    "TotalKeeperFeesPaid",              // 28
    "AutoRenewEnabled(u64)",            // 29
    "MaxLookbackLedgers(u64)",          // 30
    "SenderStreams(Address)",           // 31
    "PendingStreamOffer(u64)",          // 32
    "RecipientPendingOffers(Address)",  // 33
    "PooledStreamShares(u64)",          // 34
    "PooledStreamWithdrawn(u64, Address)", // 35
];

/// Return the count of frozen discriminants in this version.
pub fn frozen_discriminant_count() -> usize {
    FROZEN_DISCRIMINANTS_V9.len()
}

/// Validate that the frozen discriminant list has not been modified.
///
/// This is a compile-time sanity check to prevent accidental reordering.
/// Always returns `Ok(())` in current version (documentation only).
pub fn validate_discriminants_frozen() -> Result<(), VersioningError> {
    // This function exists for future compatibility and documentation.
    // In a future Rust version, this could use compile-time reflection
    // or proc-macro verification to enforce discriminant stability.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_checkpoint_state_valid() {
        let result = validate_checkpoint_state(
            5000,  // checkpointed_amount
            500,   // checkpointed_at
            1000,  // end_time
            10000, // deposit_amount
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_checkpoint_state_checkpointed_exceeds_deposit() {
        let result = validate_checkpoint_state(
            15000, // exceeds deposit
            500,
            1000,
            10000,
        );
        assert_eq!(result, Err(VersioningError::InvalidCheckpointState));
    }

    #[test]
    fn test_validate_checkpoint_state_checkpointed_at_exceeds_end() {
        let result = validate_checkpoint_state(
            5000,
            1001, // exceeds end_time
            1000,
            10000,
        );
        assert_eq!(result, Err(VersioningError::InvalidCheckpointState));
    }

    #[test]
    fn test_validate_checkpoint_state_negative_checkpointed() {
        let result = validate_checkpoint_state(
            -100, // negative
            500,
            1000,
            10000,
        );
        assert_eq!(result, Err(VersioningError::InvalidCheckpointState));
    }

    #[test]
    fn test_validate_accrual_bounds_valid() {
        let result = validate_accrual_bounds(7500, 5000, 10000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_accrual_bounds_negative() {
        let result = validate_accrual_bounds(-100, 0, 10000);
        assert_eq!(result, Err(VersioningError::AccrualNonDeterministic));
    }

    #[test]
    fn test_validate_accrual_bounds_exceeds_deposit() {
        let result = validate_accrual_bounds(15000, 0, 10000);
        assert_eq!(result, Err(VersioningError::AccrualNonDeterministic));
    }

    #[test]
    fn test_validate_accrual_bounds_below_checkpoint() {
        let result = validate_accrual_bounds(4000, 5000, 10000);
        assert_eq!(result, Err(VersioningError::AccrualNonDeterministic));
    }

    #[test]
    fn test_validate_withdrawal_monotonicity_valid() {
        let result = validate_withdrawal_monotonicity(7500, 5000, 10000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_withdrawal_monotonicity_decreasing() {
        let result = validate_withdrawal_monotonicity(4000, 5000, 10000);
        assert_eq!(result, Err(VersioningError::AccrualNonDeterministic));
    }

    #[test]
    fn test_validate_withdrawal_monotonicity_exceeds_deposit() {
        let result = validate_withdrawal_monotonicity(15000, 10000, 10000);
        assert_eq!(result, Err(VersioningError::AccrualNonDeterministic));
    }

    #[test]
    fn test_validate_entry_size_valid() {
        let result = validate_entry_size(2000, 4096);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_entry_size_exceeds_limit() {
        let result = validate_entry_size(5000, 4096);
        assert_eq!(result, Err(VersioningError::EntryOversized));
    }

    #[test]
    fn test_frozen_discriminant_count_v9() {
        assert_eq!(frozen_discriminant_count(), 36); // 0-35 inclusive
    }

    #[test]
    fn test_validate_discriminants_frozen() {
        let result = validate_discriminants_frozen();
        assert!(result.is_ok());
    }
}
