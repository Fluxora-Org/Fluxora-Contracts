//! Type definitions for the Fluxora stream contract.
//!
//! Contains all `#[contracttype]` structs, enums, and the `ContractError`
//! definition used across the contract. The `DataKey` enum (storage key
//! discriminants) is also defined here — **variant order must never change**.
//!
//! See `docs/storage.md` for the full module map and TTL policy.

#![allow(clippy::too_many_arguments)]

use soroban_sdk::{contracttype, Address, Map};

use crate::{StreamKind, StreamStatus};

// Data types
// ---------------------------------------------------------------------------

/// Maximum number of recipients allowed in a single pooled stream.
pub const MAX_POOL_RECIPIENTS: u32 = 100;

/// Global configuration for the Fluxora protocol.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub token: Address,
    pub admin: Address,
}

/// An active ID reservation held by a caller after `reserve_stream_ids`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdReservation {
    pub start_id: u64,
    pub count: u32,
    pub consumed: u32,
    pub expiry: Option<u64>,
}

/// Struct for per-stream or per-protocol pause records.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamPaused {
    pub stream_id: u64,
    pub reason: soroban_sdk::String,
}

/// Health report for a stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamHealth {
    pub is_underfunded: bool,
    pub is_expired: bool,
    pub accrued_to_date: u128,
    pub remaining_deposit: u128,
    pub seconds_until_depletion: Option<u64>,
}

/// Operational status of a stream, determining which operations are allowed.
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
    /// Optional withdrawal threshold (raw units). Withdrawals below this
    /// amount are skipped unless they are the final drain or the stream is terminal.
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
    pub status: StreamStatus,
    pub cancelled_at: Option<u64>,
    /// Total tokens mathematically accrued up to `checkpointed_at` under all
    /// previous rates. Updated by `decrease_rate_per_second` (and by
    /// `update_rate_per_second` for symmetry) so that the new rate applies only
    /// from `checkpointed_at` forward. Initialised to 0 at stream creation.
    pub checkpointed_amount: i128,
    /// Ledger timestamp of the last rate change (or `start_time` on creation).
    /// `calculate_accrued` uses this as the start of the current rate epoch.
    pub checkpointed_at: u64,
    /// Minimum withdrawal amount in raw token units before a non-terminal payout
    /// is skipped (returns `0`, no transfer).
    ///
    /// Enforced on `withdraw`, `withdraw_to`, `batch_withdraw`, and
    /// `batch_withdraw_to` when `withdrawable < withdraw_dust_threshold`.
    /// **Not** enforced on `delegated_withdraw` or read-only views.
    ///
    /// Bypassed when the stream is in a **terminal state** (`Cancelled` or past
    /// `end_time`) or when the payout is the **final drain**
    /// (`withdrawn_amount + withdrawable == deposit_amount`).
    ///
    /// See [`docs/dust-threshold.md`](../../../docs/dust-threshold.md) for the
    /// entry-point matrix and validation rules (`InvalidDustThreshold` = 35 on
    /// `create_stream_offer` only).
    pub withdraw_dust_threshold: i128,
    /// Optional bounded memo for indexer correlation (e.g. payroll batch ID).
    /// Maximum length: `MAX_MEMO_BYTES` (64 bytes). `None` when not supplied.
    pub memo: Option<soroban_sdk::Bytes>,
    /// The architectural style of the stream (Linear or CliffOnly).
    pub kind: StreamKind,
    /// Ledger sequence number of the last pause or resume toggle.
    /// Used to enforce MIN_PAUSE_INTERVAL_LEDGERS cooldown.
    pub last_pause_toggle_ledger: u32,
    /// Ledger sequence number of the last recipient withdrawal.
    /// Used to enforce MIN_WITHDRAW_INTERVAL_LEDGERS cooldown.
    pub last_withdraw_ledger: u32,
    /// Optional structured metadata emitted for indexer consumption.
    pub metadata: Option<soroban_sdk::Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>,
    /// Optional compliance witness authorized to cancel via signed attestation.
    pub witness: Option<Address>,
    /// Ledger sequence number of the last rate change (or creation).
    /// Used to enforce MIN_RATE_INTERVAL_LEDGERS cooldown.
    pub last_rate_change_ledger: u32,
    /// Delegation depth of this stream in the recipient-share delegation tree.
    /// Root streams use depth 0; delegated children increment by one.
    pub delegation_depth: u32,
    /// Parent stream id when this stream is a delegated child.
    /// `None` marks a root stream.
    pub parent_stream_id: Option<u64>,
    /// If true, the stream is decommissioned and restricted to cancel-or-no-op.
    /// Defaults to false (None) for backward compatibility with existing streams.
    pub decommissioned: Option<bool>,
}

/// Event payload emitted when a stream's decommissioned status is updated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamDecommissioned {
    pub stream_id: u64,
    pub decommissioned: bool,
}

/// Pagination result for recipient stream listing
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    /// Stream IDs for this page (sorted ascending)
    pub stream_ids: soroban_sdk::Vec<u64>,
    /// Next cursor for pagination (0 if no more pages)
    pub next_cursor: u64,
}
#[contracttype]
#[derive(Clone, Debug)]
pub struct CreateStreamParams {
    /// Address that will receive streamed tokens for this stream entry.
    pub recipient: Address,
    /// Total amount escrowed for this stream entry.
    pub deposit_amount: i128,
    /// Streaming speed in tokens per second for this stream entry.
    pub rate_per_second: i128,
    /// Ledger timestamp when accrual starts for this stream entry.
    pub start_time: u64,
    /// Ledger timestamp when withdrawals become enabled for this stream entry.
    pub cliff_time: u64,
    /// Ledger timestamp when accrual stops for this stream entry.
    pub end_time: u64,
    /// Optional withdrawal threshold (raw units) to reduce fee spam.
    pub withdraw_dust_threshold: Option<i128>,
    /// Optional bounded memo for indexer correlation (e.g. payroll batch ID).
    /// Maximum `MAX_MEMO_BYTES` (64) bytes. Pass `None` to omit.
    pub memo: Option<soroban_sdk::Bytes>,
    /// The architectural style of the stream (Linear or CliffOnly).
    pub kind: StreamKind,
    /// Optional structured metadata emitted for indexer consumption.
    pub metadata: Option<soroban_sdk::Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>,
    /// If true, the stream cannot be cancelled or shortened. Defaults to false (None).
    pub irrevocable: Option<bool>,
    /// Optional compliance witness authorized to cancel via signed attestation.
    pub witness: Option<Address>,
}

/// Parameters for creating a payment stream with relative (offset-based) times.
///
/// Computes `start_time`, `cliff_time`, and `end_time` by adding offsets to the
/// current ledger timestamp (`env.ledger().timestamp()`). This eliminates off-chain
/// calculation errors that lead to `StartTimeInPast` failures.
///
/// # Time offsets
/// - `start_delay`: Seconds to add to current timestamp for stream start
/// - `cliff_delay`: Seconds to add to current timestamp for cliff time (must be >= start_delay)
/// - `duration`: Total duration of stream in seconds (end_time = start_time + duration)
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateStreamRelativeParams {
    /// Address that will receive streamed tokens for this stream entry.
    pub recipient: Address,
    /// Total amount escrowed for this stream entry.
    pub deposit_amount: i128,
    /// Streaming speed in tokens per second for this stream entry.
    pub rate_per_second: i128,
    /// Delay (in seconds) before stream accrual starts, relative to current timestamp.
    pub start_delay: u64,
    /// Delay (in seconds) before withdrawals are allowed, relative to current timestamp.
    pub cliff_delay: u64,
    /// Total duration the stream runs (in seconds) from start_time to end_time.
    pub duration: u64,
    /// Optional withdrawal threshold (raw units) to reduce fee spam.
    pub withdraw_dust_threshold: Option<i128>,
    /// Optional bounded memo for indexer correlation (e.g. payroll batch ID).
    /// Maximum `MAX_MEMO_BYTES` (64) bytes. Pass `None` to omit.
    pub memo: Option<soroban_sdk::Bytes>,
    /// The architectural style of the stream (Linear or CliffOnly).
    pub kind: StreamKind,
    pub metadata: Option<soroban_sdk::Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>,
    /// If true, the stream cannot be cancelled or shortened. Defaults to false (None).
    pub irrevocable: Option<bool>,
}

/// Reusable relative schedule (offsets only). Amounts are supplied when creating a stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamScheduleTemplate {
    pub template_id: u64,
    pub owner: Address,
    pub start_delay: u64,
    pub cliff_delay: u64,
    pub duration: u64,
}

/// Namespace for all contract storage keys.
///
/// # Evolution policy
///
/// `DataKey` is a `#[contracttype]` enum. Soroban serialises enum variants by
/// their **discriminant index** (0-based, in declaration order). Changing the
/// order of existing variants, or inserting a new variant anywhere other than
/// the **end** of the enum, will silently shift all subsequent discriminants
/// and make every existing persistent storage entry unreadable.
///
/// Rules for contributors:
/// 1. **Never reorder** existing variants.
/// 2. **Never remove** a variant that has ever been written to a live network.
///    Mark it deprecated in a doc comment instead and stop writing to it.
/// 3. **Always append** new variants at the end of the enum.
/// 4. **Increment `CONTRACT_VERSION`** whenever a new variant is added or an
///    existing variant's associated type changes — both are breaking changes
///    for any off-chain tool that reads storage directly.
/// 5. Document the ledger at which each variant was first deployed so that
///    migration tooling can determine which entries exist on a given instance.
///
/// Current discriminant assignments (must never change) — see enum definition below for order.
#[contracttype]
pub enum DataKey {
    Config,                    // Instance storage for global settings (admin/token).
    NextStreamId,              // Instance storage for the auto-incrementing ID counter.
    Stream(u64),               // Persistent storage for individual stream data (O(1) lookup).
    RecipientStreams(Address), // Persistent storage for recipient stream index (sorted by stream_id).
    /// Global emergency pause flag (bool). This is a contract-wide circuit breaker.
    GlobalEmergencyPaused,
    /// Creation pause flag (bool). Appended to avoid shifting existing key discriminants.
    CreationPaused,
    /// Protocol pause reason (String). Human-readable reason for the pause.
    GlobalPauseReason,
    /// Protocol pause timestamp (u64). Ledger timestamp when pause was activated.
    GlobalPauseTimestamp,
    /// Protocol pause admin (Address). The admin address that activated the pause.
    GlobalPauseAdmin,
    /// Auto-claim destination per stream (Address). Set by recipient to redirect withdrawals.
    AutoClaimDestination(u64),
    /// Monotonic template id counter (`u64`, instance storage).
    NextTemplateId,
    /// Number of templates currently stored (`u64`, instance storage).
    ActiveTemplateCount,
    /// Registered relative schedule template (persistent).
    StreamTemplate(u64),
    /// Template ids owned by an address (persistent `Vec<u64>`; length capped).
    OwnerTemplateIds(Address),
    /// Sum of outstanding deposit liabilities (`i128`, instance storage).
    TotalLiabilities,
    /// Per-recipient nonce counter for delegated-withdraw replay protection.
    /// Appended last to preserve existing discriminant values.
    WithdrawNonce(Address),
    /// Current protocol-wide pause state (Active, CreationPaused, or GlobalEmergencyPaused).
    PauseState,
    /// Reentrancy guard flag (bool) to prevent recursive token transfers.
    ReentrancyLock,
    /// Paged recipient stream index (page number → Vec<u64> of stream IDs).
    RecipientStreamPage(Address, u32),
    /// Number of pages in a recipient's paged stream index.
    RecipientStreamPageCount(Address),
    /// Pending recipient update proposal for a stream (sender-initiated, recipient-accepted).
    PendingRecipientUpdate(u64),
    /// Active ID reservation for a caller (Address → IdReservation).
    IdReservation(Address),
    /// Per-stream max rate cap (i128). Instance storage.
    MaxRatePerSecond,
    /// Per-recipient nonce for delegated-withdraw replay protection.
    DelegatedWithdrawNonce(Address),
    /// Last pause record for stream-level or protocol-level pause.
    LastPauseRecord(PauseKind),
    /// Rotation history for recipient/sender changes on a stream.
    RotationHistory(u64),
    /// Last ledger timestamp observed for accrual clock-regression detection.
    LastAccrualLedgerTimestamp,
    /// Protocol-wide count of streams currently in `StreamStatus::Paused` (`u64`, instance storage).
    PausedStreamCount,
    /// Aggregate sum of all keeper fees paid out via `keeper_cancel` (`i128`, instance storage).
    TotalKeeperFeesPaid,
}

/// Type of pause.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PauseKind {
    Protocol = 0,
    Stream = 1,
}

/// Record of a pause action.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseRecord {
    pub actor: Address,
    pub timestamp: u64,
    pub reason: soroban_sdk::String,
}

/// Event emitted when a keeper cancels a stream.
#[contracttype]
#[derive(Clone, Debug)]
pub struct KeeperCancelled {
    pub stream_id: u64,
    pub keeper: Address,
    pub keeper_fee: i128,
    pub recipient_amount: i128,
    pub sender_refund: i128,
}
