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

use soroban_sdk::{contracttype, Address};

/// The canonical persistent record for a single payment/vesting stream.
///
/// Fields are ordered to match the historical on-chain layout; appending new
/// `Option<_>` fields at the end (with `None` meaning "pre-upgrade default")
/// preserves backward compatibility. Never reorder or remove existing fields.
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

/// Emitted when claim ownership is transferred on a stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimOwnershipTransferred {
    pub stream_id: u64,
    pub old_owner: Option<Address>,
    pub new_owner: Address,
}

/// Emitted when a recipient delegates a share of their stream.
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

/// Pagination result for paginated stream listings.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    /// Stream IDs for this page (sorted ascending).
    pub stream_ids: soroban_sdk::Vec<u64>,
    /// Next cursor for pagination (0 if no more pages).
    pub next_cursor: u64,
}
