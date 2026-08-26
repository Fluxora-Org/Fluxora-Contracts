//! Storage access and TTL management.
//!
//! # Why TTL is the hard part
//!
//! Soroban persistent entries have a time-to-live measured in ledgers. When it
//! runs out the entry is archived and becomes unreadable until explicitly
//! restored. A stream running twelve months will outlive its default TTL.
//!
//! If a stream entry archives the tokens are *not* lost — they sit in the
//! contract's pooled balance — but the accounting entry saying who they belong
//! to is inaccessible until someone pays to restore it. For a payroll or grant
//! primitive that is unacceptable, so the contract engineers around it three
//! ways:
//!
//! 1. **Extend on every touch.** Every function that reads or writes a stream
//!    bumps that entry's TTL. An actively-used stream never expires.
//! 2. **Extend generously at creation**, targeting the stream's remaining
//!    lifetime plus a buffer, clamped to the network maximum.
//! 3. **Permissionless top-ups** via `extend_stream_ttl`, so a keeper — or the
//!    recipient, or any passer-by — can keep a claim readable without the
//!    sender's cooperation.

use soroban_sdk::Env;

use crate::error::Error;
use crate::types::{DataKey, Stream};

/// Nominal Stellar ledger close time, in seconds.
///
/// Ledger close time is a network property, not a protocol constant, and it
/// drifts. Using a deliberately conservative value means the ledger count we
/// derive from a wall-clock duration *over*-estimates how many ledgers that
/// duration spans, which errs toward keeping entries alive longer than needed.
/// That is the safe direction to be wrong in.
pub const SECONDS_PER_LEDGER: u64 = 5;

/// Extra headroom, in seconds, added on top of a stream's remaining lifetime
/// when computing its TTL target. 30 days.
///
/// This is what gives the keeper a wide window to act in: a stream only needs
/// sweeping once its TTL falls inside this buffer, not on the day it would
/// otherwise archive.
pub const TTL_BUFFER_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Floor for any stream entry's TTL, in ledgers, regardless of how little
/// lifetime the stream has left. Roughly 30 days at the nominal close time.
///
/// A settled stream still has to stay readable: the recipient may not have
/// withdrawn their tail yet, and the indexer needs to see the final state.
pub const MIN_STREAM_TTL_LEDGERS: u32 = (TTL_BUFFER_SECONDS / SECONDS_PER_LEDGER) as u32;

/// Convert a wall-clock duration into a ledger count, rounding up.
///
/// Saturates at `u32::MAX`; callers clamp to the network maximum anyway.
pub fn seconds_to_ledgers(seconds: u64) -> u32 {
    let ledgers = seconds
        .saturating_add(SECONDS_PER_LEDGER - 1)
        .saturating_div(SECONDS_PER_LEDGER);
    if ledgers > u32::MAX as u64 {
        u32::MAX
    } else {
        ledgers as u32
    }
}

/// How many ledgers this stream's entry should be kept alive for, given the
/// current time.
///
/// Targets the stream's remaining lifetime plus [`TTL_BUFFER_SECONDS`], floored
/// at [`MIN_STREAM_TTL_LEDGERS`] and clamped to the network's `max_entry_ttl`.
///
/// The clamp is not optional: a multi-year stream will exceed the network
/// maximum, so it *will* need periodic extension over its life no matter how
/// generously we extend at creation. That is precisely what the permissionless
/// keeper path exists for.
pub fn ttl_target_ledgers(env: &Env, stream: &Stream) -> u32 {
    let now = env.ledger().timestamp();

    // A paused stream's end date slides forward in wall-clock terms, so include
    // the accumulated pause when working out how much longer it may run.
    let effective_end = stream
        .end_time
        .saturating_add(stream.paused_total)
        .saturating_add(match stream.paused_at {
            Some(paused_at) => now.saturating_sub(paused_at),
            None => 0,
        });

    let remaining = effective_end.saturating_sub(now);
    let target = seconds_to_ledgers(remaining.saturating_add(TTL_BUFFER_SECONDS));
    let floored = target.max(MIN_STREAM_TTL_LEDGERS);

    floored.min(env.storage().max_ttl())
}

/// Bump the instance entry. Tiny, and it carries the id counter, so it is
/// always extended to the network maximum.
pub fn extend_instance(env: &Env) {
    let max = env.storage().max_ttl();
    env.storage().instance().extend_ttl(max, max);
}

/// Bump one stream entry to its computed target.
///
/// The threshold equals the target, so every touch tops the entry back up to a
/// full window rather than waiting for it to decay past some watermark. Rent is
/// cheap relative to a stream archiving under a recipient.
pub fn extend_stream(env: &Env, stream_id: u64, stream: &Stream) {
    let target = ttl_target_ledgers(env, stream);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::Stream(stream_id), target, target);
}

/// Read a stream, bumping its TTL on the way out.
///
/// Every read path in the contract goes through here, which is what implements
/// "extend on every touch".
pub fn load_stream(env: &Env, stream_id: u64) -> Result<Stream, Error> {
    let stream: Stream = env
        .storage()
        .persistent()
        .get(&DataKey::Stream(stream_id))
        .ok_or(Error::StreamNotFound)?;
    extend_stream(env, stream_id, &stream);
    Ok(stream)
}

/// Read a stream without touching its TTL.
///
/// Used by the read-only view functions, which run in simulation and should not
/// pretend to write. Also used by `extend_stream_ttl`, which does its own bump.
pub fn peek_stream(env: &Env, stream_id: u64) -> Result<Stream, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Stream(stream_id))
        .ok_or(Error::StreamNotFound)
}

/// Write a stream back and bump its TTL.
pub fn save_stream(env: &Env, stream_id: u64, stream: &Stream) {
    env.storage()
        .persistent()
        .set(&DataKey::Stream(stream_id), stream);
    extend_stream(env, stream_id, stream);
}

/// Hand out the next stream id and advance the counter.
///
/// Ids are monotonic and never reused, so an id is a stable handle an indexer
/// can key on forever.
///
/// # Design: global, not per-sender, not derived from storage
///
/// One counter (`DataKey::NextStreamId`) is shared by every caller. Ids are
/// **not** namespaced per sender and **not** derived from any property of the
/// stream itself (e.g. a hash of its fields) — a global sequence is the only
/// scheme that gives every id, across every sender, a fixed total order with
/// no coordination needed between callers.
///
/// # Why a rejected `create_stream` can never consume or reuse an id
///
/// `create_stream` calls this function only after every validation gate has
/// passed, so a call rejected on validation (bad schedule, self-stream, dust
/// rate, ...) never reaches here and the counter never moves.
///
/// The remaining case is the deposit transfer at the very end of
/// `create_stream`, which can still fail (insufficient balance or allowance)
/// *after* this function has already bumped the counter and after the new
/// `Stream` entry has already been written. That is safe because a Soroban
/// contract invocation is atomic end to end: if `create_stream` does not
/// return `Ok`, every storage write it made — the counter bump and the
/// `Stream` entry alike — is rolled back along with the failed token
/// transfer, not just the transfer itself. So no id is ever left half
/// allocated, and a retried call after a failure gets the exact id the
/// failed attempt would have received, not the next one. `test::stream_ids`
/// exercises this directly by forcing a deposit transfer to fail and
/// asserting the next successful create reuses that id.
///
/// # Gaps
///
/// There are none. Every id in `0..stream_count()` was assigned to exactly
/// one successful create and, once assigned, is never reassigned — even a
/// stream whose entry later archives under [`stream_exists`] keeps its id
/// permanently retired rather than freeing it for reuse.
pub fn next_stream_id(env: &Env) -> Result<u64, Error> {
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0);
    let next = current.checked_add(1).ok_or(Error::Overflow)?;
    env.storage().instance().set(&DataKey::NextStreamId, &next);
    extend_instance(env);
    Ok(current)
}

/// Whether a stream entry is present and live (i.e. not archived).
///
/// `has` returns false for an archived entry, which is what lets the SDK
/// distinguish "never existed" from "needs restoring" when combined with the
/// id counter.
pub fn stream_exists(env: &Env, stream_id: u64) -> bool {
    env.storage().persistent().has(&DataKey::Stream(stream_id))
}

/// Total number of streams ever created.
pub fn stream_count(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0)
}
