//! Storage access and TTL management.
//!
//! # Why TTL is the hard part
//!
//! Soroban persistent entries have a time-to-live measured in ledgers. When it
//! runs out the entry is archived and becomes unreadable until explicitly
//! restored. A stream running twelve months will outlive its default TTL.
//!
//! If a stream entry archives the tokens are *not* lost — they sit in the
//! contract's pooled balance — but the accounting entry saying who they belong to
//! is inaccessible until someone pays to restore it. For a payroll or grant
//! primitive that is unacceptable, so the contract engineers around it three
//! ways:
//!
//! 1. **Extend on every touch.** Every function that reads or writes a stream
//!    bumps that entry's TTL. An actively-used stream never expires.
//! 2. **Extend generously at creation**, targeting the stream's remaining
//!    lifetime plus a buffer, clamped to the network maximum.
//! 3. **Permissionless top-ups** via `extend_stream_ttl`, so a keeper —or the
//!    recipient, or any passer-by — can keep a claim readable without the
//!    sender's cooperation.
//!
//! # Terminal streams and the TTL extension rule
//!
//! **Extending the TTL of a `Cancelled` or `Depleted` stream is rejected with
//! `Error::StreamTerminated` from both `extend_stream_ttl` and
//! `batch_extend_ttl`.** The reasons are:
//!
//! * **No future state change is possible.** Terminal streams have settled all
//!   accounting. Allowing indefinite TTL extensions would charge callers rent
//!   for a record they cannot modify or interact with in any meaningful way.
//! * **The floor TTL covers the withdrawal tail.** At the instant a stream
//!   enters a terminal state, the contract applies the floor TTL
//!   (`MIN_STREAM_TTL_LEDGERS` ≈ 30 days). A `Cancelled` stream that still
//!   has an unwithdrawn vested tail remains readable for that window, giving
//!   the recipient ample time to withdraw.
//! * **Restoration is the right answer after archival.** If a terminal entry
//!   does archive (e.g. because no one withdrew the tail before the floor
//!   expired), the caller must submit a `RestoreFootprint` operation rather
//!   than extending an already-live entry. Locking callers out of `extend`
//!   makes this distinction explicit.
//!
//! A keeper sweeping streams should filter terminal ids out of its batch before
//! calling `batch_extend_ttl`. The indexer's `status` field is the signal.
//!
//! # Instance vs persistent TTL policy
//!
//! The contract uses two Soroban storage lifetimes, and they are *not* managed
//! the same way:
//!
//! | Entry | Lifetime | TTL target | Who bumps it |
//! |-------|----------|-----------|--------------|
//! | [`DataKey::NextStreamId`] (id counter) | **instance** | always the network `max_ttl()` | every mutating call |
//! | [`DataKey::Stream(id)`] (a stream) | **persistent** | remaining life + [`TTL_BUFFER_SECONDS`], floored at [`MIN_STREAM_TTL_LEDGERS`] | every touch + keeper |
//!
//! **Instance entries are always kept at maximum rent.** They are tiny, and the
//! id counter carries the contract's monotonicity invariant: if `NextStreamId`
//! archived, the next `create_stream` would restart ids from zero and collide
//! with live streams. So [`extend_instance`] pins it to `max_ttl()` on *every*
//! mutating call — creation, every withdrawal, every pause, every keeper sweep.
//!
//! **Persistent entries target the stream's remaining lifetime.** They hold the
//! full accounting record, so they are extended to a window that covers the
//! stream's scheduled end plus a keeper buffer. A stream that is still settling
//! keeps a floor of [`MIN_STREAM_TTL_LEDGERS`] so final state stays readable.
//!
//! **Ordering guarantee.** [`DataKey::NextStreamId`] must *never* expire before
//! the streams it issued. Because the instance entry is always pinned to the
//! maximum while a persistent entry is only ever as long-lived as its target,
//! the instance entry is always at least as fresh as any stream — so a live
//! stream can never outlive its own id-counter, and a keeper sweep always
//! bumps both in the same transaction ([`extend_stream_ttl`] calls
//! [`extend_stream`] *and* [`extend_instance`] together).
//!
//! # Safe handling of each storage type
//!
//! - **Instance `NextStreamId`** is read with a fallback of `0` (a fresh
//!   contract), written monotonically, and always re-extended. A restored
//!   instance correctly resumes from the last persisted id, never reusing one.
//! - **Persistent `Stream(id)`** is read through [`load_stream`] (which bumps
//!   TTL) or [`peek_stream`] (view-only, no write). [`stream_exists`] reports
//!   `false` for an archived entry, which combined with the id counter lets the
//!   contract tell "never existed" apart from "needs restoring".
//!
//! The regression tests in `test/ttl.rs` exercise both lifetimes from seeded
//! ledger states and assert that the instance entry cannot expire before the
//! persistent entries it issued.

use soroban_sdk::{Address, Env};

use crate::error::Error;
use crate::types::{DataKey, DelegateGrant, ReleaseCurve, Stream, StreamRecord, StreamShares};

/// Nominal Stellar ledger close time, in seconds.
///
/// Ledger close time is a network property, not a protocol constant, and it
/// drifts. The safe direction to be wrong in is **up**: an assumed close time
/// at or above the real mean funds at least as many ledgers as the duration
/// needs, while an assumed value below the real mean silently under-funds
/// every entry. Rounding the measured mean up to a whole second buys that
/// direction for free; [`TTL_SAFETY_MARGIN_PERCENT`] covers the other side.
///
/// # Where 5 comes from
///
/// Not assumed any more — **measured**. The mean close time over the RPC
/// node's full retention window (120,960 ledgers ≈ 6.9 days) on Stellar
/// testnet was exactly 5.000 s/ledger when re-checked on 2026-09-28: every
/// one of 1,176 sampled per-ledger gaps closed in exactly 5 s, with no gap
/// above the nominal. See `script/measure-ledger-close.sh` and
/// docs/ledger-close-time.md for the method, the raw statistics and the
/// re-measurement procedure.
///
/// The measurement's headline finding was **not** the value but the margin:
/// the previous version of this constant carried *zero* headroom against
/// drift, so any change in close time landed directly on every TTL — a
/// network that closed even slightly faster would have shortened every
/// funded window in wall-clock terms with nothing in the way. That is what
/// [`TTL_SAFETY_MARGIN_PERCENT`] now adds. This constant stays at the
/// measured mean (5 s, which happens to equal the protocol's target close
/// time); the conversion absorbs variance on top of it.
pub const SECONDS_PER_LEDGER: u64 = 5;

/// Drift margin applied by [`seconds_to_ledgers`] on top of the assumed close
/// time, as a percentage, before converting to ledgers.
///
/// The 2026-09-28 testnet measurement (docs/ledger-close-time.md) observed a
/// dead-flat 5.000 s mean over a 6.9-day window — which means the TTL
/// conversion previously had **zero** headroom: any change in close time
/// would have flowed straight into every funded window.
///
/// # Which direction is dangerous
///
/// A funded TTL of N ledgers spans N × real_close seconds of wall clock. If
/// the network runs *faster* than this conversion assumes, every window
/// shrinks and an entry can archive before its schedule ends — that is the
/// direction to margin. A *slower* network only over-funds: wasteful in
/// rent, never unsafe.
///
/// 20% keeps the conversion fully covering for any sustained real mean close
/// time at or above `5 × 100 / 120 ≈ 4.17 s` — i.e. a network up to ~17%
/// faster than observed. It does not cover a sustained mean below that;
/// that is what [`measure-ledger-close.sh`] re-checks before releases, and
/// a sustained change in either direction is a signal to re-measure.
///
/// [`measure-ledger-close.sh`]: https://github.com/Fluxora-Org/Fluxora-Contracts/blob/main/script/measure-ledger-close.sh
pub const TTL_SAFETY_MARGIN_PERCENT: u64 = 20;

/// Extra headroom, in seconds, added on top of a stream's remaining lifetime
/// when computing its TTL target. 30 days.
///
/// This is what gives the keeper a wide window to act in: a stream only needs
/// sweeping once its TTL falls inside this buffer, not on the day it would
/// otherwise archive.
pub const TTL_BUFFER_SECONDS: u64 = 30 * 24 * 60 * 60;

/// Floor for any stream entry's TTL, in ledgers, regardless of how little
/// lifetime the stream has left. 622,080 ledgers at the pinned close time:
/// 30 days inflated by [`TTL_SAFETY_MARGIN_PERCENT`] and converted. At the
/// observed 5.000 s mean that window spans 36 days; the margin guarantees at
/// least 30 days even if the network ran ~17% faster than observed.
///
/// Coincidence worth knowing: this equals the network's `max_entry_ttl`
/// (30 days at the nominal 5 s close), so the floor is exactly one
/// max-TTL window on the 2026-09-28 settings. They are still independent
/// quantities — the floor is ours, the max is the network's.
///
/// A settled stream still has to stay readable: the recipient may not have
/// withdrawn their tail yet, and the indexer needs to see the final state.
pub const MIN_STREAM_TTL_LEDGERS: u32 =
    (TTL_BUFFER_SECONDS * (100 + TTL_SAFETY_MARGIN_PERCENT) / (SECONDS_PER_LEDGER * 100)) as u32;

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
/// # Why the margin is applied here, not by callers
///
/// Margining at the conversion layer means no call site can forget it, and it
/// is exercised by the same round-trip test that pins the ceiling direction.
/// [`ttl_target_ledgers`] is the only production caller.
///
/// Saturates at `u32::MAX`; callers clamp to the network maximum anyway.
pub fn seconds_to_ledgers(seconds: u64) -> u32 {
    // Inflate the requested duration by the drift margin, then convert at the
    // assumed close time. The margin guarantees full coverage for any real
    // close time at or above assumed × 100 / (100 + margin); see
    // [`TTL_SAFETY_MARGIN_PERCENT`] for why faster, not slower, is the
    // dangerous direction.
    let inflated = seconds
        .saturating_mul(100 + TTL_SAFETY_MARGIN_PERCENT)
        .saturating_div(100);
    let ledgers = inflated
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
/// Targets the stream's remaining lifetime plus `[TTL_BUFFER_SECONDS]`, floored
/// at `[MIN_STREAM_TTL_LEDGERS] and clamped to the network's `max_entry_ttl`.
///
/// A future-dated stream is covered implicitly: `remaining` is measured from
/// now to `end_time`, so the pre-start wait is part of the target. A schedule
/// beyond one TTL window clamps here and is kept alive by the permissionless
/// keeper path — creation deliberately does not reject it (see
/// [`crate::FluxoraStream::create_stream`]).
///
/// The clamp is not optional: a multi-year stream will exceed the network
/// maximum, so it *will* need periodic extension over its life no matter how
/// slowly we extend at creation. That is precisely what the permissionless
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
    // pre-start wait is included in the rent target. The drift margin is
    // applied inside `seconds_to_ledgers`, so the rent target — not just the
    // close-time conversion — carries it.
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
/// Used by the state-changing entry points, which is what implements
/// "extend on every touch". The public views do **not** come through here —
/// they use [`peek_stream`] and never extend TTL (#1686, `docs/ABI.md`,
/// `test::read_ttl_matrix`). Switching a view to this function is a
/// behaviour change callers can observe, and that test fails on it.
pub fn load_stream(env: &Env, stream_id: u64) -> Result<Stream, Error> {
    let stream = read_stream(env, stream_id)?;
    extend_stream(env, stream_id, &stream);
    Ok(stream)
}

/// Read a stream without touching its TTL.
///
/// Used by the read-only view functions, which run in simulation and should not
/// pretend to write. Also used by `extend_stream_ttl`, which does its own bump.
pub fn peek_stream(env: &Env, stream_id: u64) -> Result<Stream, Error> {
    read_stream(env, stream_id)
}

/// Decode a stream, stitching the frozen v1 [`StreamRecord`] back together with
/// its [`ReleaseCurve`] side-car.
///
/// The stored value is the v1 layout ([`StreamRecord`]) — appending a field to
/// it would make every stream written before curves existed undecodable; see
/// the type docs. The curve lives under [`DataKey::StreamCurve`] and is written
/// only for non-linear streams, so a missing entry means
/// [`ReleaseCurve::Linear`] and decodes exactly as a v1 stream always did.
fn read_stream(env: &Env, stream_id: u64) -> Result<Stream, Error> {
    let record: StreamRecord = env
        .storage()
        .persistent()
        .get(&DataKey::Stream(stream_id))
        .ok_or(Error::StreamNotFound)?;
    let curve: ReleaseCurve = env
        .storage()
        .persistent()
        .get(&DataKey::StreamCurve(stream_id))
        .unwrap_or(ReleaseCurve::Linear);
    Ok(record.into_stream(curve))
}

/// Write a stream back and bump its TTL.
///
/// If the stream did not exist before this call, the global stream counter (and the
/// next stream id) is advanced. This makes the counter update atomic with the stream
/// creation: if the caller fails before `save_stream`, the counter never increments.
pub fn save_stream(env: &Env, stream_id: u64, stream: &Stream) {
    let is_new = !env.storage().persistent().has(&DataKey::Stream(stream_id));
    // Store the frozen v1 layout; the curve rides alongside it. See
    // [`StreamRecord`] for why a field is not appended to the stored value.
    env.storage().persistent().set(
        &DataKey::Stream(stream_id),
        &StreamRecord::from_stream(stream),
    );
    if stream.curve == ReleaseCurve::Linear {
        // No entry for linear, so a linear stream pays no rent for a side-car
        // that would only restate the default.
        env.storage()
            .persistent()
            .remove(&DataKey::StreamCurve(stream_id));
    } else {
        env.storage()
            .persistent()
            .set(&DataKey::StreamCurve(stream_id), &stream.curve);
    }
    if is_new {
        let current: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextStreamId)
            .unwrap_or(0);
        let recorded_count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::StreamCount)
            .unwrap_or(current);
        debug_assert_eq!(
            recorded_count, current,
            "NextStreamId and StreamCount must advance together",
        );
        let next = current.checked_add(1).expect("stream id counter overflow");
        env.storage().instance().set(&DataKey::NextStreamId, &next);
        env.storage().instance().set(&DataKey::StreamCount, &next);
        extend_instance(env);
    }
    extend_stream(env, stream_id, stream);
    if stream.curve != ReleaseCurve::Linear {
        // Give the side-car the same lifetime as the record it annotates.
        let target = ttl_target_ledgers(env, stream);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::StreamCurve(stream_id), target, target);
    }
}

/// Return the next stream id without advancing the counter.
///
/// The counter is advanced by `save_stream` when a new stream is persisted first time.
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
    // Missing counter means no stream has been created yet — equivalent to 0.
    // This is a default, not a precondition failure: create_stream is what
    // initialises the counter, and there is no separate `init` entry point.
    let current: u64 = env
        .storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0);
    // The counter is advanced by `save_stream` only after a new stream is
    // persisted, but the exhaustion boundary is checked here so that a create
    // at `u64::MAX` fails with the typed error instead of a panic in the
    // counter increment. Ids are never reused and never wrap.
    if current == u64::MAX {
        return Err(Error::StreamIdExhausted);
    }
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
///
/// This is equivalent to the next stream id because ids are never reused.
pub fn stream_count(env: &Env) -> u64 {
    // Keep the population counter as an independent entry. Falling back to
    // NextStreamId preserves the read view for instances created before this
    // key was introduced; every new write maintains both entries atomically.
    env.storage()
        .instance()
        .get(&DataKey::StreamCount)
        .or_else(|| env.storage().instance().get(&DataKey::NextStreamId))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Pooled-token balance
// ---------------------------------------------------------------------------

/// The token balance Fluxora expects to be holding for `token`.
///
/// A missing entry reads as zero, which is the honest answer for a token no
/// stream has ever been funded with. That default also keeps every test (and
/// any state injected directly into storage) safe: an untracked token looks
/// *surplus*, never short, so a defensive check cannot misfire on it.
pub fn pooled_balance(env: &Env, token: &Address) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::PooledBalance(token.clone()))
        .unwrap_or(0)
}

/// Overwrite the expected pooled balance for `token`.
///
/// Private: the total is only ever moved by [`credit_pool`] / [`debit_pool`],
/// which are checked. Writing it directly would let the reconciliation in
/// [`crate::FluxoraStream`] be defeated from inside the module.
fn set_pooled_balance(env: &Env, token: &Address, amount: i128) {
    env.storage()
        .instance()
        .set(&DataKey::PooledBalance(token.clone()), &amount);
    // The total is instance storage: keep it at maximum rent alongside the id
    // counter, so it cannot archive while a stream that relies on it is live.
    extend_instance(env);
}

/// Record that `amount` of `token` was pulled into the pool.
///
/// Called only after [`crate::pull_deposit`] has verified the token moved
/// exactly `amount`.
pub fn credit_pool(env: &Env, token: &Address, amount: i128) -> Result<(), Error> {
    let next = pooled_balance(env, token)
        .checked_add(amount)
        .ok_or(Error::Overflow)?;
    set_pooled_balance(env, token, next);
    Ok(())
}

/// Record that `amount` of `token` is leaving the pool.
///
/// Debit happens before the outbound transfer: Soroban rolls the whole
/// invocation back if the transfer fails, so the total can never be left
/// debited for tokens that did not move.
pub fn debit_pool(env: &Env, token: &Address, amount: i128) -> Result<(), Error> {
    let next = pooled_balance(env, token)
        .checked_sub(amount)
        .ok_or(Error::Overflow)?;
    set_pooled_balance(env, token, next);
    Ok(())
}

// ---------------------------------------------------------------------------
// Delegation
// ---------------------------------------------------------------------------

/// Persist a delegate grant, borrowing the stream's TTL.
///
/// A replacement does not consume an additional slot. New grants are rejected
/// once the stream already has [`MAX_DELEGATES_PER_STREAM`] stored delegates.
pub fn save_delegate(
    env: &Env,
    stream_id: u64,
    delegate: &Address,
    grant: &DelegateGrant,
) -> Result<(), Error> {
    let key = DataKey::Delegate(stream_id, delegate.clone());
    let persistent = env.storage().persistent();
    let already_granted = persistent.has(&key);
    let count_key = DataKey::DelegateCount(stream_id);
    let count: u32 = persistent.get(&count_key).unwrap_or(0);

    let next_count = if already_granted {
        count
    } else {
        if count >= MAX_DELEGATES_PER_STREAM {
            return Err(Error::TooManyDelegates);
        }
        count.checked_add(1).ok_or(Error::Overflow)?
    };

    persistent.set(&key, grant);
    // Give the grant at least as long to live as the stream itself.
    let stream = peek_stream(env, stream_id).expect("stream must exist when saving delegate");
    let target = ttl_target_ledgers(env, &stream);
    persistent.extend_ttl(&key, target, target);

    if next_count > 0 {
        persistent.set(&count_key, &next_count);
        persistent.extend_ttl(&count_key, target, target);
    }
    Ok(())
}

/// Remove a delegate grant.
pub fn remove_delegate(env: &Env, stream_id: u64, delegate: &Address) {
    let key = DataKey::Delegate(stream_id, delegate.clone());
    let persistent = env.storage().persistent();
    if persistent.has(&key) {
        persistent.remove(&key);

        let count_key = DataKey::DelegateCount(stream_id);
        let count: u32 = persistent.get(&count_key).unwrap_or(0);
        let next_count = count.saturating_sub(1);
        if next_count == 0 {
            persistent.remove(&count_key);
        } else {
            persistent.set(&count_key, &next_count);
            let stream = peek_stream(env, stream_id).expect("stream must exist when revoking delegate");
            let target = ttl_target_ledgers(env, &stream);
            persistent.extend_ttl(&count_key, target, target);
        }
    }
}

/// Retrieve a delegate grant, or `None` if it does not exist.
pub fn load_delegate(env: &Env, stream_id: u64, delegate: &Address) -> Option<DelegateGrant> {
    env.storage()
        .persistent()
        .get(&DataKey::Delegate(stream_id, delegate.clone()))
}

// ---------------------------------------------------------------------------
// Split-stream share allocations
// ---------------------------------------------------------------------------

/// Persist [`StreamShares`] alongside the parent stream and give it the same
/// TTL target.
///
/// Must be called at the same time as [`save_stream`] for a new split stream
/// so the two entries have the same initial TTL. `save_stream` bumps the parent
/// entry; this helper mirrors that bump for the shares entry.
pub fn save_shares(env: &Env, stream_id: u64, stream: &Stream, shares: &StreamShares) {
    let key = DataKey::StreamShares(stream_id);
    env.storage().persistent().set(&key, shares);
    let target = ttl_target_ledgers(env, stream);
    env.storage().persistent().extend_ttl(&key, target, target);
}

/// Read [`StreamShares`] and bump the entry's TTL alongside the parent stream.
///
/// Used by mutating paths (`withdraw_share`) that need to update per-share
/// `withdrawn` counters. Returns `None` if the stream has no shares entry —
/// i.e. it is a plain single-recipient stream.
pub fn load_shares(env: &Env, stream_id: u64, stream: &Stream) -> Option<StreamShares> {
    let key = DataKey::StreamShares(stream_id);
    let shares: Option<StreamShares> = env.storage().persistent().get(&key);
    if shares.is_some() {
        let target = ttl_target_ledgers(env, stream);
        env.storage().persistent().extend_ttl(&key, target, target);
    }
    shares
}

/// Read [`StreamShares`] without touching its TTL.
///
/// Used by the read-only `get_stream_shares` view and by `cancel`, where we
/// need to inspect the shares without disturbing them (cancel does not modify
/// shares at all; each recipient's per-share `withdrawn` is unaffected by the
/// sender reclaiming the unvested remainder). Returns `None` for plain streams.
pub fn peek_shares(env: &Env, stream_id: u64) -> Option<StreamShares> {
    env.storage()
        .persistent()
        .get(&DataKey::StreamShares(stream_id))
}

// ---------------------------------------------------------------------------
// Emergency halt (issue #1818)
// ---------------------------------------------------------------------------
//
// Both entries live in instance storage. They share the contract's own TTL, so
// they are covered by the same "always pinned to `max_ttl()`" policy as
// `NextStreamId`, and they can never archive out from under the guard that
// reads them on every mutating call (see the module docs on lifetimes).
//
// `HaltedAt` doubles as the halt flag: its presence *is* the halt. Using the
// timestamp rather than a separate boolean means the `ContractHalted`
// diagnostic event can report when the stop began, and the resume event can
// report how long settlement was frozen, without a second key to keep in sync.

/// The address allowed to halt and resume the contract, if one was installed.
pub fn halt_operator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::HaltOperator)
}

/// Install the halt operator. Callers must reject a second install.
pub fn set_halt_operator(env: &Env, operator: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::HaltOperator, operator);
    extend_instance(env);
}

/// Whether the contract-level halt is engaged.
pub fn is_halted(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::HaltedAt)
}

/// Unix seconds at which the halt was engaged, if it is engaged.
pub fn halt_started_at(env: &Env) -> Option<u64> {
    env.storage().instance().get(&DataKey::HaltedAt)
}

/// Engage the halt, recording the current ledger timestamp.
pub fn set_halt(env: &Env) {
    env.storage()
        .instance()
        .set(&DataKey::HaltedAt, &env.ledger().timestamp());
    extend_instance(env);
}

/// Lift the halt and forget when it started.
pub fn clear_halt(env: &Env) {
    env.storage().instance().remove(&DataKey::HaltedAt);
    extend_instance(env);
}
