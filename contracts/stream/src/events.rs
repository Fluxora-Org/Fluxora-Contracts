//! Event definitions and emission.
//!
//! Stream discovery is an off-chain concern — the contract keeps no per-user
//! index (see the `lib.rs` module docs). That makes these events the *only* way
//! an indexer learns that a stream exists or that its state moved, so they are
//! load-bearing infrastructure rather than optional telemetry.
//!
//! # Contract
//!
//! * Every state change emits exactly one event.
//! * Events are declared with `#[contractevent]`, so their schemas land in the
//!   contract's interface spec. Tooling and the TypeScript SDK generate typed
//!   decoders from that spec instead of hand-rolling topic parsers.
//! * The static topic is the struct name in snake_case. `stream_id` is always a
//!   topic, as are the addresses an indexer routes on, so a consumer can filter
//!   server-side by event kind, by stream, or by party.
//! * Each payload carries enough state to reconstruct the stream without
//!   replaying from genesis.
//!
//! Field order and topic placement are ABI. Adding a field at the end of a struct is a compatible
//! change (additive versioning); reordering, removing, or re-topicking an existing field is an
//! incompatible breaking change. Indexers should tolerate unknown trailing fields.
//! Event ordering within a single operation is deterministic. Currently, a single state change
//! emits exactly one event, guaranteeing the event order aligns with the operation order.
//!
//! Note that item-level doc comments on a `#[contractevent]` struct are copied
//! into the contract spec and therefore into the deployed wasm. Statements of
//! the ABI belong there; the reasoning behind them belongs here, where it costs
//! the contract nothing.
//!
//! # `Cancelled.vested`: total vested, not the withdrawable remainder
//!
//! Issue #1584. `cancelled` publishes `refunded`, `vested` and `withdrawn`,
//! which makes it a public accounting statement, and `vested` had two defensible
//! readings: everything the recipient has earned over the life of the stream, or
//! only the part of it they can still pull. The two differ exactly when the
//! recipient withdrew before the cancel.
//!
//! The ruling is **total vested**, cumulative and inclusive of `withdrawn`,
//! because that is the reading that keeps the event self-checking:
//!
//! * Conservation holds unconditionally: `refunded + vested` equals the
//!   pre-cancel `deposited` for every stream. Under the withdrawable reading
//!   that identity fails for any partially withdrawn stream, and a reconciler
//!   could no longer tell a broken contract from an ordinary one.
//! * No information is lost. The event carries `withdrawn`, so the remainder is
//!   `vested - withdrawn`. The reverse does not hold — from the remainder
//!   alone, total vested is unrecoverable.
//! * It matches storage one-for-one: cancellation rewrites `deposited` to the
//!   vested amount, so an indexer mirroring `deposited` assigns the field
//!   directly. [`cancelled`] reads it straight off the settled stream for that
//!   reason, which is what makes divergence between event and storage
//!   unrepresentable rather than merely untested.
//!
//! The cancel itself moves nothing to the recipient: they still pull
//! `vested - withdrawn` through the normal withdraw path, which is why that
//! amount stays pooled in the contract. Every cancellation state is asserted
//! against storage and token balances in `test::cancel_events`.
use soroban_sdk::{contractevent, Address, Env};

use crate::types::{Stream, StreamStatus};

/// A new stream was created. Carries the complete initial state — this is the
/// event an indexer builds its sender/recipient mapping from.
#[contractevent]
pub struct StreamCreated {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub sender: Address,
    #[topic]
    pub recipient: Address,
    pub token: Address,
    pub deposited: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub cliff_time: u64,
    pub cancellable: bool,
    pub pausable: bool,
    pub transferable: bool,
}

/// The recipient drew down accrued funds. Emitted once per stream, including
/// once per drawn-from stream inside a `batch_withdraw`.
#[contractevent]
pub struct Withdrawn {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub recipient: Address,
    /// Amount moved in this call.
    pub amount: i128,
    /// Cumulative withdrawn after this call.
    pub withdrawn: i128,
    pub deposited: i128,
    pub status: StreamStatus,
}

/// The sender cancelled, collapsing the schedule onto the cancellation instant.
///
/// Amounts are readings at that instant and reconcile with storage and the
/// token ledger exactly:
///
/// ```text
/// refunded + vested == deposited before the cancel   (conservation)
/// vested - withdrawn == still claimable == tokens left pooled for this stream
/// ```
///
/// `vested` is the cumulative total, not the withdrawable remainder — see the
/// module docs for why.
#[contractevent]
pub struct Cancelled {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub sender: Address,
    #[topic]
    pub recipient: Address,
    /// Returned to the sender by this call: pre-cancel `deposited` minus
    /// `vested`. Zero when the stream had already fully vested.
    pub refunded: i128,
    /// Total vested at cancellation, including what was already withdrawn.
    /// Equal to the post-cancel `deposited`.
    pub vested: i128,
    /// Cumulative withdrawn before the cancel. Never moved by the cancel.
    pub withdrawn: i128,
    /// Rewritten end of the collapsed schedule.
    pub end_time: u64,
}

/// Accrual frozen.
#[contractevent]
pub struct Paused {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub sender: Address,
    pub paused_at: u64,
    pub paused_total: u64,
}

/// Accrual resumed. `paused_total` is the post-resume cumulative figure, so an
/// indexer can recompute the schedule without tracking individual intervals.
#[contractevent]
pub struct Resumed {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub sender: Address,
    pub paused_duration: u64,
    pub paused_total: u64,
}

/// Funds added. Carries the new `end_time` because a top-up extends the
/// duration rather than raising the rate.
#[contractevent]
pub struct ToppedUp {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub sender: Address,
    pub amount: i128,
    pub deposited: i128,
    pub end_time: u64,
}

/// The recipient reassigned the stream.
#[contractevent]
pub struct RecipientTransferred {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub old_recipient: Address,
    #[topic]
    pub new_recipient: Address,
}

/// A delegate grant was issued.
#[contractevent]
pub struct DelegateGranted {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub grantor: Address,
    #[topic]
    pub delegate: Address,
    pub ops: u32,
    pub expires_at: Option<u64>,
}

/// A delegate grant was revoked.
#[contractevent]
pub struct DelegateRevoked {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub grantor: Address,
    #[topic]
    pub delegate: Address,
}

/// A stream entry's TTL was topped up. Lets a keeper confirm its sweep landed.
#[contractevent]
pub struct TtlExtended {
    #[topic]
    pub stream_id: u64,
    pub extended_to_ledgers: u32,
}

// ---------------------------------------------------------------------------
// Emission helpers
// ---------------------------------------------------------------------------

pub fn stream_created(env: &Env, stream_id: u64, stream: &Stream) {
    StreamCreated {
        stream_id,
        sender: stream.sender.clone(),
        recipient: stream.recipient.clone(),
        token: stream.token.clone(),
        deposited: stream.deposited,
        start_time: stream.start_time,
        end_time: stream.end_time,
        cliff_time: stream.cliff_time,
        cancellable: stream.cancellable,
        pausable: stream.pausable,
        transferable: stream.transferable,
    }
    .publish(env);
}

pub fn withdrawn(env: &Env, stream_id: u64, stream: &Stream, amount: i128) {
    Withdrawn {
        stream_id,
        recipient: stream.recipient.clone(),
        amount,
        withdrawn: stream.withdrawn,
        deposited: stream.deposited,
        status: stream.status,
    }
    .publish(env);
}

/// Emit [`Cancelled`] for a stream that has already been collapsed and saved.
///
/// Issue #1584: `vested` is deliberately *not* a parameter. Cancellation
/// rewrites `deposited` to the amount vested at that instant, so reading the
/// figure back off the settled stream is what guarantees the event and storage
/// can never disagree - the two cannot be wired up out of order or drift apart
/// in a later edit. `refunded` is still passed in because it is a token
/// movement, not stream state; `cancel` debug-asserts the conservation identity
/// that ties the two together.
///
/// # Preconditions
///
/// `stream` must be the post-cancel state: `status == Cancelled`, `deposited`
/// already reduced to the vested amount, and `end_time` already collapsed.
pub fn cancelled(env: &Env, stream_id: u64, stream: &Stream, refunded: i128) {
    Cancelled {
        stream_id,
        sender: stream.sender.clone(),
        recipient: stream.recipient.clone(),
        refunded,
        // Total vested at the cancellation instant == post-cancel `deposited`.
        vested: stream.deposited,
        withdrawn: stream.withdrawn,
        end_time: stream.end_time,
    }
    .publish(env);
}

pub fn paused(env: &Env, stream_id: u64, stream: &Stream, paused_at: u64) {
    Paused {
        stream_id,
        sender: stream.sender.clone(),
        paused_at,
        paused_total: stream.paused_total,
    }
    .publish(env);
}

pub fn resumed(env: &Env, stream_id: u64, stream: &Stream, paused_duration: u64) {
    Resumed {
        stream_id,
        sender: stream.sender.clone(),
        paused_duration,
        paused_total: stream.paused_total,
    }
    .publish(env);
}

pub fn topped_up(env: &Env, stream_id: u64, stream: &Stream, amount: i128) {
    ToppedUp {
        stream_id,
        sender: stream.sender.clone(),
        amount,
        deposited: stream.deposited,
        end_time: stream.end_time,
    }
    .publish(env);
}

pub fn recipient_transferred(
    env: &Env,
    stream_id: u64,
    old_recipient: &Address,
    new_recipient: &Address,
) {
    RecipientTransferred {
        stream_id,
        old_recipient: old_recipient.clone(),
        new_recipient: new_recipient.clone(),
    }
    .publish(env);
}

pub fn ttl_extended(env: &Env, stream_id: u64, extended_to_ledgers: u32) {
    TtlExtended {
        stream_id,
        extended_to_ledgers,
    }
    .publish(env);
}

pub fn delegate_granted(
    env: &Env,
    stream_id: u64,
    grantor: &Address,
    delegate: &Address,
    ops: u32,
    expires_at: Option<u64>,
) {
    DelegateGranted {
        stream_id,
        grantor: grantor.clone(),
        delegate: delegate.clone(),
        ops,
        expires_at,
    }
    .publish(env);
}

pub fn delegate_revoked(env: &Env, stream_id: u64, grantor: &Address, delegate: &Address) {
    DelegateRevoked {
        stream_id,
        grantor: grantor.clone(),
        delegate: delegate.clone(),
    }
    .publish(env);
}
