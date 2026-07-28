//! Type definitions for the Fluxora stream contract.
//!
//! This module defines the `Stream` persistent-storage struct and a small set
//! of supplementary event/pagination structs that must live in a dedicated
//! submodule (for Soroban `#[contracttype]` codegen determinism and so they
//! can be imported from sibling modules such as `accrual` or `storage`
//! without creating cycles).
//!
//! All other contract types — including `ContractError`, `DataKey` (with its
//! frozen 0–35 discriminants), `Config`, the event payloads, `PauseKind`,
//! `StreamKind`, `StreamStatus`, `CreateStreamParams`, and similar enums —
//! live at the crate root in `lib.rs`. The `DataKey` variant order is the
//! single source of truth for storage discriminant stability; do **not**
//! duplicate those definitions here.
//!
//! # Adding a new struct to this module
//!
//! Only place a struct here when (a) it needs `#[contracttype]` and (b) it is
//! referenced by more than one other file in `src/`, or (c) it must be
//! imported by a test crate that depends on a concrete type path. All other
//! types belong at the crate root.

use soroban_sdk::{contracttype, Address, Map};

/// The canonical persistent record for a single payment/vesting stream.
///
/// The status controls the stream's lifecycle and affects both accrual calculation
/// and operation availability. Status transitions follow strict rules to maintain
/// system integrity and prevent unauthorized state changes.
///
/// ## State Transition Rules
///
/// ```text
/// Active ↔ Paused    (via pause_stream/resume_stream)
/// Active → Cancelled (via cancel_stream, terminal)
/// Paused → Cancelled (via cancel_stream, terminal)
/// Active → Completed (via withdraw when withdrawn_amount == deposit_amount, terminal)
/// ```
///
/// Terminal states (`Completed`, `Cancelled`) cannot transition to other states.
///
/// ## Time-Terminal Behavior
///
/// Stream status values are imported from the crate root (`lib.rs`) where they
/// are defined alongside the `#[soroban_sdk::contract]` implementation.

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamEvent {
    Paused(u64),
    Resumed(u64),
    StreamCancelled(u64),
    StreamCompleted(u64),
    StreamClosed(u64),
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamCreated {
    pub stream_id: u64,
    pub sender: Address,
    pub recipient: Address,
    pub deposit_amount: i128,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub cliff_time: u64,
    pub end_time: u64,
    /// Optional withdrawal threshold (raw units) utilized by threshold monitors.
    /// Withdrawals below this amount are skipped unless they are the final drain 
    /// or the stream is terminal. Used to prevent dust sweep spam.
    pub withdraw_dust_threshold: i128,
    /// Optional bounded memo for indexer correlation (e.g. payroll batch ID).
    /// `None` when no memo was supplied at creation time.
    pub memo: Option<soroban_sdk::Bytes>,
    /// Optional structured metadata emitted for indexer consumption.
    /// Mirrors the validated `metadata` field stored on the stream.
    pub metadata: Option<Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>,
}

/// Emitted when a stream is cloned via `clone_stream`.
///
/// Carries both the source stream ID (for audit trail) and the full parameters
/// of the newly created stream so indexers can correlate the two without a
/// separate `get_stream_state` call.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamCloned {
    /// The newly created stream's ID.
    pub new_stream_id: u64,
    /// The source stream that was cloned.
    pub source_stream_id: u64,
    /// Sender of the new stream (same as the caller / original sender).
    pub sender: Address,
    /// Recipient of the new stream (may differ from the source stream's recipient).
    pub recipient: Address,
    /// Deposit amount locked into the new stream.
    pub deposit_amount: i128,
    /// Rate per second inherited from the source stream.
    pub rate_per_second: i128,
    /// Absolute start time of the new stream.
    pub start_time: u64,
    /// Cliff time of the new stream (preserves the source cliff offset).
    pub cliff_time: u64,
    /// End time of the new stream.
    pub end_time: u64,
    /// Withdrawal threshold inherited from the source stream, 
    /// ensuring threshold monitors continue to respect the same boundary.
    pub withdraw_dust_threshold: i128,
}

/// Result of a single stream creation attempt in a partial batch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateStreamResult {
    /// True if the stream was created successfully.
    pub success: bool,
    /// The unique identifier of the created stream (None if success is false).
    pub stream_id: Option<u64>,
    /// The error code if the creation failed (None if success is true).
    pub error: Option<u32>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Withdrawal {
    pub stream_id: u64,
    pub recipient: Address,
    pub amount: i128,
}

/// Emitted when a recipient withdraws to a specified destination via `withdraw_to`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct WithdrawalTo {
    pub stream_id: u64,
    pub recipient: Address,
    pub destination: Address,
    pub amount: i128,
}

/// Emitted when a recipient rotates their receiving address for a stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientUpdated {
    pub stream_id: u64,
    pub old_recipient: Address,
    pub new_recipient: Address,
}

/// Emitted when a recipient delegates a portion of their stream to a new recipient.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientShareDelegated {
    pub parent_stream_id: u64,
    pub child_stream_id: u64,
    pub delegator: Address,
    pub delegatee: Address,
    pub share_bps: u32,
    pub new_parent_rate: i128,
    pub child_rate: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRecipientUpdate {
    pub stream_id: u64,
    pub proposed_recipient: Address,
}

/// Per-stream result for `batch_withdraw`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct BatchWithdrawResult {
    pub stream_id: u64,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawToParam {
    pub stream_id: u64,
    pub destination: Address,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RateUpdated {
    pub stream_id: u64,
    pub old_rate_per_second: i128,
    pub new_rate_per_second: i128,
    /// Ledger timestamp when the rate update became effective.
    pub effective_time: u64,
}

/// Event emitted when a rate update is rejected due to exceeding the governance cap.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RateCapEnforced {
    pub stream_id: u64,
    pub attempted_rate: i128,
    pub max_rate_per_second: i128,
}

/// Emitted when the sender safely decreases the streaming rate via `decrease_rate_per_second`.
///
/// The `checkpointed_amount` field records how many tokens were mathematically
/// accrued under the **old** rate at the moment of the rate change. The new rate
/// is applied only to the remaining stream duration from `effective_time` onward.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RateDecreased {
    pub stream_id: u64,
    pub old_rate_per_second: i128,
    pub new_rate_per_second: i128,
    /// Ledger timestamp when the decrease became effective (== `checkpointed_at`).
    pub effective_time: u64,
    /// Accrued amount locked in at `effective_time` under the old rate.
    pub checkpointed_amount: i128,
    /// Tokens refunded to the sender: `old_deposit - new_max_payable`.
    pub refund_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamEndShortened {
    /// Stream whose schedule was shortened.
    pub stream_id: u64,
    /// Previous `end_time` before this mutation.
    pub old_end_time: u64,
    /// New `end_time` after this mutation.
    pub new_end_time: u64,
    /// Tokens refunded to sender: `old_deposit_amount - new_deposit_amount`.
    pub refund_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamEndExtended {
    pub stream_id: u64,
    pub old_end_time: u64,
    pub new_end_time: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamToppedUp {
    pub stream_id: u64,
    pub top_up_amount: i128,
    pub new_deposit_amount: i128,
    /// `end_time` after the top-up (unchanged by top-up itself; included so
    /// indexers can correlate with any subsequent `extend_stream_end_time` call).
    pub new_end_time: u64,
}

/// Emitted when the stream sender is rotated via `transfer_sender`.
///
/// The `old_sender` loses all sender-role privileges (pause, cancel, rate updates, etc.)
/// and the `new_sender` gains them immediately. Recipient entitlement is unchanged.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SenderTransferred {
    pub stream_id: u64,
    pub old_sender: Address,
    pub new_sender: Address,
}

/// Emitted when a stream's claim ownership is transferred.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimOwnershipTransferred {
    pub stream_id: u64,
    pub old_owner: Option<Address>,
    pub new_owner: Address,
}

/// Emitted when a stream's funding health status transitions between
/// adequately funded and underfunded states.
///
/// A stream is **underfunded** when `remaining_balance < rate_per_second × seconds_remaining`.
/// Terminal streams (`Completed`, `Cancelled`) always have `seconds_remaining = 0`
/// and are never considered underfunded.
///
/// This event is only emitted when the `is_underfunded` flag actually changes,
/// not on every mutation.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamHealthChanged {
    pub stream_id: u64,
    pub is_underfunded: bool,
    pub remaining_balance: i128,
    pub seconds_remaining: u64,
}

/// Emitted when the contract admin toggles the global emergency pause flag.
#[contracttype]
#[derive(Clone, Debug)]
pub struct GlobalEmergencyPauseChanged {
    pub paused: bool,
}

/// Emitted when the admin sweeps excess tokens from the contract.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ExcessSwept {
    pub to: Address,
    pub amount: i128,
}

/// Emitted when a recipient sets an auto-claim destination.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AutoClaimSet {
    pub stream_id: u64,
    pub destination: Address,
}

/// Emitted when a recipient revokes their auto-claim destination.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AutoClaimRevoked {
    pub stream_id: u64,
}

/// Emitted when an auto-claim is triggered.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AutoClaimTriggered {
    pub stream_id: u64,
    pub destination: Address,
    pub amount: i128,
}

/// Payload for a valid auto-claim destination.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoClaimValidPayload {
    pub destination: Address,
    pub claimable: i128,
}

/// Payload for an invalid auto-claim destination.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoClaimInvalidPayload {
    pub destination: Address,
}

/// Status of auto-claim configuration for a stream.
///
/// Returned by `get_auto_claim_status` to allow callers to validate
/// the auto-claim destination before executing `trigger_auto_claim`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutoClaimStatus {
    /// No auto-claim destination has been set for this stream.
    NotSet,
    /// Auto-claim destination is set and valid.
    ValidDestination(AutoClaimValidPayload),
    /// Auto-claim destination is set but invalid (zero address or contract itself).
    InvalidDestination(AutoClaimInvalidPayload),
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct GlobalResumed {
    pub resumed_at: u64,
}

/// Emitted when the contract admin toggles the creation-pause flag via `set_contract_paused`.
///
/// When `paused == true`, `create_stream` and `create_streams` revert with
/// `ContractError::ContractPaused`. All other operations are unaffected.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ContractPauseChanged {
    pub paused: bool,
}

/// Emitted when the protocol is globally paused via `pause_protocol`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProtocolPaused {
    pub reason: soroban_sdk::String,
    pub paused_at: u64,
}

/// Emitted when the protocol is globally resumed via `resume_protocol`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProtocolResumed {
    pub resumed_at: u64,
}

/// Information about the current protocol pause state.
/// Returned by `get_pause_info()` query entrypoint.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PauseInfo {
    pub is_paused: bool,
    pub reason: Option<soroban_sdk::String>,
    pub paused_at: Option<u64>,
    pub paused_by: Option<Address>,
}

/// Role type for rotation history entries.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RotationRole {
    Recipient = 0,
    Sender = 1,
}

/// Audit log entry for recipient or sender rotation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationEntry {
    pub old_addr: Address,
    pub new_addr: Address,
    pub ledger: u32,
    pub role: RotationRole,
    pub authoriser: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    pub stream_id: u64,
    pub sender: Address,
    pub recipient: Address,
    pub claim_owner: Option<Address>,
    pub deposit_amount: i128,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub cliff_time: u64,
    pub end_time: u64,
    pub withdrawn_amount: i128,
    pub status: crate::StreamStatus,
    pub cancelled_at: Option<u64>,
    /// Total tokens mathematically accrued up to `checkpointed_at` under all
    /// previous rates. Updated by `decrease_rate_per_second` (and by
    /// `update_rate_per_second` for symmetry) so that the new rate applies only
    /// from `checkpointed_at` forward. Initialised to 0 at stream creation.
    pub checkpointed_amount: i128,
    /// Ledger timestamp of the last rate change (or `start_time` on creation).
    /// `calculate_accrued` uses this as the start of the current rate epoch.
    pub checkpointed_at: u64,
    /// Minimum withdrawal amount in raw token units before a non-terminal
    /// payout is skipped (returns `0`, no transfer).
    ///
    /// ## Threshold Monitor Behavior
    /// This threshold dictates when indexers and external bot monitors should trigger a `batch_withdraw_to`
    /// sweep. It affects the relevant state model by preserving CPU/gas and preventing dust withdrawal spam.
    ///
    /// ## Edge Cases & State Model
    /// - **Storage/Gas**: Prevents storage state growth and gas exhaustion by rejecting micro-withdrawals.
    /// - **Upgrade/Compat**: `withdraw_dust_threshold` defaults to `0` (or `None` in `CreateStreamParams`)
    ///   for legacy v1 streams to guarantee backward compatibility during contract upgrades.
    /// - **Terminal Streams**: This threshold is intentionally bypassed if the stream reaches a terminal state
    ///   (`Completed`, `Cancelled`) or if it's the final drain.
    pub withdraw_dust_threshold: i128,
    /// Optional bounded memo for indexer correlation.
    pub memo: Option<soroban_sdk::Bytes>,
    /// The architectural style of the stream (Linear, CliffOnly, or CliffSlope).
    pub kind: crate::StreamKind,
    /// Ledger sequence number of the last pause or resume toggle.
    pub last_pause_toggle_ledger: u32,
    /// Ledger sequence number of the last recipient withdrawal.
    pub last_withdraw_ledger: u32,
    /// Optional structured metadata emitted for indexer consumption.
    pub metadata: Option<soroban_sdk::Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>,
    /// Optional compliance witness authorized to cancel via signed attestation.
    pub witness: Option<Address>,
    /// Whether this stream is a pooled multi-recipient stream.
    pub is_pooled: Option<bool>,
    /// Ledger sequence number of the last rate change (or creation).
    pub last_rate_change_ledger: u32,
    /// Delegation depth in the recipient-share delegation tree (root = 0).
    pub delegation_depth: u32,
    /// Parent stream id when this stream is a delegated child.
    pub parent_stream_id: Option<u64>,
    /// If true, the stream is decommissioned and restricted to cancel-or-no-op.
    /// Defaults to false (None) for backward compatibility with existing streams.
    pub decommissioned: Option<bool>,
    /// If true, the sender cannot cancel or shorten the stream. Defaults to
    /// false (None) for streams created before this field was appended.
    pub irrevocable: Option<bool>,
}

/// Event payload emitted when a stream's decommissioned status is updated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamDecommissioned {
    pub stream_id: u64,
    pub decommissioned: bool,
}


#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    /// Stream IDs for this page (sorted ascending).
    pub stream_ids: soroban_sdk::Vec<u64>,
    /// Next cursor for pagination (0 if no more pages).
    pub next_cursor: u64,
}
