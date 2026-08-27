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
//! Field order and topic placement are ABI. Adding a field is a compatible
//! change; reordering or re-topicking one is not.

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

/// The sender cancelled. `refunded` went back to the sender; `vested` is what
/// the recipient keeps and may still withdraw.
#[contractevent]
pub struct Cancelled {
    #[topic]
    pub stream_id: u64,
    #[topic]
    pub sender: Address,
    #[topic]
    pub recipient: Address,
    pub refunded: i128,
    pub vested: i128,
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

pub fn cancelled(env: &Env, stream_id: u64, stream: &Stream, refunded: i128, vested: i128) {
    Cancelled {
        stream_id,
        sender: stream.sender.clone(),
        recipient: stream.recipient.clone(),
        refunded,
        vested,
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
