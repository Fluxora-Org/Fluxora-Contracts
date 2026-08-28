//! Storage access and TTL management.
//
//# Why TTL is the hard part
//
// Soroban persistent entries have a time-to-live measured in ledgers. When it
// runs out the entry is archived and becomes unreadable until explicitly
// restored. A stream running twelve months will outlive its default TTL.
//
// If a stream entry archives the tokens are *not* lost — they sit in the
// contract's pooled balance — but the accounting entry saying who they belong
// to is inaccessible until someone pays to restore it. For a payroll or grant
// primitive that is unacceptable, so the contract engineers around it three
// ways:
//
// 1. **Extend on every touch.** Every function that reads or writes a stream
//    bumps that entry's TTL. An actively-used stream never expires.
// 2. **Extend generously at creation**, targeting the stream's remaining
//    lifetime plus a buffer, clamped to the network maximum.
// 3. **Permissionless top-ups**via `extend_stream_ttl`, so a keeper — or the
//    recipient, or any passer-by — can keep a claim readable without the
//    sender's cooperation.

use soroban_sdk::{Address, Bytes, BytesExt, Env};

use crate::error::Error;
use crate::types::{DataKey, DelegateGrant, Stream};

/// Nominal Stellar ledger close time, in seconds.
//
// Ledger close time is a network property, not a protocol constant, and it
// drifts. Using a deliberately conservative value means the ledger count we
// derive from a wall-clock duration *over-*estimates how many ledgers that
// duration spans, which errs toward keeping entries alive longer than needed.
// That is the safe direction to be wrong in.
pub const SECONDS_PER_LEDGER: u64 = 5;

/// Extra headroom, in seconds, added on top of a stream's remaining lifetime
// when computing its TTL target. 30 days.
//
// This is what gives the keeper a wide window to act in: a stream only needs
// sweeping once its TTL falls inside this buffer, not on the day it would
// otherwise archive.
pub const TTL_BUFFER_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Floor for any stream entry's TTL, in ledgers, regardless of how little
// lifetime the stream has left. Roughly 30 days at the nominal close time.
//
// A settled stream still has to stay readable: the recipient may not have
// withdrawn their tail yet, and the indexer needs to see the final state.
pub const MIN_STREAM_TTL_LEDGERS: u32 = (TTL_BUFFER_SECONDS / SECONDS_PER_LEDGER) as u32;

/// Convert a wall-clock duration into a ledger count, rounding up.
///
/// # Why ceiling, not floor
///
/// This only ever feeds the "how long should this entry live" side of the TTL
/// math (see [`ttl_target_ledgers`]), never the "how much has the stream
/// promised" side. Flooring here would trim a fraction of a ledger off of
/// every TTL target — which can only ever *shorten* the window before an
/// entry becomes eligible to archive, never lengthen it. Ceiling guarantees
/// the opposite: the ledger count returned, converted back to seconds, is
/// always at least the requested duration. That guarantee is exercised
/// directly by `seconds_to_ledgers_round_trip_never_undershoots`.
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
/// A future-dated stream is covered implicitly: `remaining` is measured from
/// now to `end_time`, so the pre-start wait is part of the target. A schedule
/// beyond one TTL window clamps here and is kept alive by the permissionless
/// keeper path — creation deliberately does not reject it (see
/// [`crate::FluxoraStream::create_stream`]).
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
    // `remaining` spans now → end_time, so for a future-dated stream the
    // pre-start wait is included in the rent target.
    let target = seconds_to_ledgers(remaining.saturating_add(TTL_BUFFER_SECONDS));
    let floored = target.max(MIN_STREAM_TTL_LEDGERS);

    floored.min(env.storage().max_ttl())
}

/// Bump the instance entry. Tiny, and it carries the id counter, so it is
// always extended to the network maximum.
pub fn extend_instance(env: &Env) {
    let max = env.storage().max_ttl();
    env.storage().instance().extend_ttl(max, max);
}

/// Bump one stream entry to its computed target.
//
/// The threshold equals the target, so every touch tops the entry back up to a
// full window rather than waiting for it to decay past some watermark. Rent is
// cheap relative to a stream archiving under a recipient.
pub fn extend_stream(env: &Env, stream_id: u64, stream: &Stream) {
    if load_pending_refund(env, stream_id).is_some() {
        let max = env.storage().max_ttl();
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Stream(stream_id), max, max);
    } else {
        let target = ttl_target_ledgers(env, stream);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Stream(stream_id), target, target);
    }
}

/// Read a stream, bumping its TTL on the way out.
//
/// Every read path in the contract goes through here, which is what implements
// "extend on every touch".
pub fn load_stream(env: &Env, stream_id: u64) -> Result<Stream, Error> {
    let stream: Stream = env
        .storage()
        .persistent()
        .get(&DataKey::Stream(stream_id))
        .ok_or(Error::StreamNotFound)?;
    extend_stream(env, stream_id, &stream);
    Ok(stream)
}

/// Read a stream without touching its TTl.
//
/// Used by the read-only view functions, which run in simulation and should not
// pretend to write. Also used by `extend_stream_ttl`, which does its own bump.
pub fn peek_stream(env: &Env, stream_id: u64) -> Result<Stream, Error> {
    env
        .storage()
        .persistent()
        .get(&DataKey::Stream(stream_id))
        .ok_or(Error::StreamNotFound)
}

/// Write a stream back and bump its TTl.
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
/// # Exhaustion
///
/// The counter is never allowed to wrap: at `u64::MAX` there is no next
/// representable id, and handing one out would either duplicate an id or wrap
/// the counter and corrupt the monotonicity guarantee. The counter is checked
/// *before* any state is written, so an exhausted call leaves the counter, the
/// pool and the stream set untouched — no duplicate id and no partial stream.
pub fn next_stream_id(env: &Env) -> Result<u64, Error> {
    // Missing counter means no stream has been created yet — equivalent to 0.
    // This is a default, not a precondition failure: create_stream is what
    // initialises the counter, and there is no separate `init` entry point.
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0);

    // Checked *before* the counter is advanced: if we are at the boundary, fail
    // with the typed exhaustion error and write nothing.
    if current == u64::MAX {
        return Err(Error::StreamIdExhausted);
    }

    let next = current.checked_add(1).ok_or(Error::Overflow)?;
    env.storage().instance().set(&DataKey::NextStreamId, &next);
    extend_instance(env);
    Ok(current)
}

/// Whether a stream entry is present and live (i.e. not archived).
//
// ``a`h returns false for an archived entry, which is what lets the SDK distinguish
// "never existed" from "needs restoring" when combined with the
// id counter.
pub fn stream_exists(env: &Env, stream_id: u64) -> bool {
    env.storage().persistent().has(&DataKey::Stream(stream_id))
}

/// Total number of streams ever created.
pub fn stream_count(env: &Env) -> u64 {
    // Same default as `next_stream_id`: an untouched instance has created
    // zero streams. Not a recoverable precondition — callers treat 0 as the
    // honest answer.
    env.storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0)
}

// --- Pending refund storage (cancellation safety) ---

/// Storage key prefix for pending cancellation refunds.
///
/// A pending refund entry records how much of the original deposit still needs to be
// returned to the sender after a cancellation's token transfer failed. Keeping it in
// its own storage slot (rather than on the stream) lets us retry the refund without
// overwriting the stream's terminal state.
fn pending_refund_key(env: &Env, stream_id: u64) -> Bytes {
    let mut key = Bytes::new(env);
    key.extend_from_slice(b"refund_");
    key.extend_from_slice(&stream_id.to_be_bytes());
    key
}

/// Extend the TTL of a pending refund entry to the network maximum.
//
// The pending refund may need to survive until a retry succeeds, which could be far in
// the future. The ceiling is safe because the entry is tiny, and it is cleared once the
// refund is made.
fn extend_pending_refund(env: &Env, stream_id: u64) {
    let max = env.storage().max_ttl();
    let key = pending_refund_key(env, stream_id);
    env.storage().persistent().extend_ttl(&key, max, max);
}

/// Persist the amount that still needs to be refunded to the sender after a failed
/// cancellation refund transfer. This makes the cancellation retryable.
pub fn save_pending_refund(env: &Env, stream_id: u64, refund_amount: i128) {
    let key = pending_refund_key(env, stream_id);
    env.storage().persistent().set(&key, &refund_amount);
    extend_pending_refund(env, stream_id);
}

/// Read the pending refund amount for a stream, bumping its TTL so it stays available
// for a retry.
pub fn load_pending_refund(env: &Env, stream_id: u64) -> Option<i128> {
    let key = pending_refund_key(env, stream_id);
    let amount: Option<i128> = env.storage().persistent().get(&key);
    if amount.is_some() {
        extend_pending_refund(env, stream_id);
    }
    amount
}

/// Remove the pending refund entry after the refund has been successfully retried.
pub fn clear_pending_refund(env: &Env, stream_id: u64) {
    let key = pending_refund_key(env, stream_id);
    env.storage().persistent().remove(&key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_pending_refund_roundtrip() {
        let env = Env::default();
        let stream_id = 42u64;
        let amount: i128 = 1000;

        assert_eq!(load_pending_refund(&env, stream_id), None);

        save_pending_refund(&env, stream_id, amount);
        assert_eq!(load_pending_refund(&env, stream_id), Some(amount));

        clear_pending_refund(&env, stream_id);
        assert_eq!(load_pending_refund(&env, stream_id), None);
    }
}

// ---------------------------------------------------------------------------
// Delegation
// ---------------------------------------------------------------------------

/// Persist a delegate grant, borrowing the stream's TTL.
pub fn save_delegate(env: &Env, stream_id: u64, delegate: &Address, grant: &DelegateGrant) {
    let key = DataKey::Delegate(stream_id, delegate.clone());
    env.storage().persistent().set(&key, grant);
    // Give the grant at least as long to live as the stream itself.
    let stream = peek_stream(env, stream_id).expect("stream must exist when saving delegate");
    let target = ttl_target_ledgers(env, &stream);
    env.storage().persistent().extend_ttl(&key, target, target);
}

/// Remove a delegate grant.
pub fn remove_delegate(env: &Env, stream_id: u64, delegate: &Address) {
    let key = DataKey::Delegate(stream_id, delegate.clone());
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }
}

/// Retrieve a delegate grant, or `None` if it does not exist.
pub fn load_delegate(env: &Env, stream_id: u64, delegate: &Address) -> Option<DelegateGrant> {
    env.storage()
        .persistent()
        .get(&DataKey::Delegate(stream_id, delegate.clone()))
}