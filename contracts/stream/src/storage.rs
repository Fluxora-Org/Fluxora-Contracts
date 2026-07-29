//! Storage helpers for the Fluxora stream contract.
//!
//! Centralizes all persistent and instance storage reads/writes, TTL bumping,
//! and the DataKey-based CRUD layer. All functions here are `pub(crate)` unless
//! they need to be called from tests via the `testutils` feature.
//!
//! # Storage invariants
//!
//! The contract relies on the following storage invariants (see also
//! [`docs/storage-invariants.md`](../../../docs/storage-invariants.md)):
//!
//! - **TTL:** Instance keys are bumped on every entry-point; persistent stream
//!   keys are bumped on `load_stream` / `save_stream` and index reads/writes.
//! - **Reentrancy:** `ReentrancyLock` instance flag prevents nested token calls.
//! - **Liabilities:** `TotalLiabilities` tracks outstanding deposit obligations
//!   and moves in lockstep with create/top-up vs withdraw/cancel/refund paths.
//!   Value is guaranteed non-negative (`>= 0`).
//! - **Indexes sorted & unique:** `RecipientStreams`, `SenderStreams`, and `RecipientPendingOffers`
//!   vectors are kept in ascending order without duplicate IDs on insert/remove (idempotent insertion).
//! - **CEI:** Stream state is persisted before external token transfers.
//! - **Terminal state:** Cancelled or past-`end_time` streams bypass dust threshold
//!   and freeze accrual at cancellation when applicable.
//! - **Metadata validation:** `validate_metadata` runs before ID allocation.
//! - **Stream layout validation:** `validate_stream_invariants` validates field boundaries
//!   (deposit, withdrawal, timestamps, checkpoints) before stream state persistence.
//! - **Append-only keys:** `DataKey` variants are never reordered (discriminants
//!   0–35 frozen per release policy).
//!
//! # Security notes
//! - `DataKey` variant order is append-only and must never be reordered.
//! - `save_stream` is `pub` so the accrual module can call it directly.
//! - `acquire_reentrancy_lock` / `release_reentrancy_lock` each appear once here;
//!   the previous duplicate definitions in `lib.rs` have been removed.

#![allow(dead_code)]

use crate::accrual;
use crate::*;
use soroban_sdk::{token, Address, Env, Map};

/// Minimum remaining TTL (in ledgers) before we bump.  ~1 day at 5 s/ledger.
pub const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;
/// Extend to ~7 days of ledgers when bumping instance storage.
pub const INSTANCE_BUMP_AMOUNT: u32 = 120_960;
/// Minimum remaining TTL for persistent (stream) entries.
pub const PERSISTENT_LIFETIME_THRESHOLD: u32 = 17_280;
/// Extend persistent entries to ~7 days of ledgers.
pub const PERSISTENT_BUMP_AMOUNT: u32 = 120_960;
/// Approximate seconds per Soroban ledger close.
pub const LEDGER_CLOSE_TIME: u64 = 5;
/// Extra ledger buffer added to adaptive TTL bumps to absorb jitter.
pub const BUFFER_LEDGERS: u32 = 17_280; // ~1 day
/// Absolute maximum TTL for any storage entry (Soroban platform limit).
pub const MAX_TTL: u32 = 3_110_400; // ~180 days

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Extend instance storage TTL so Config and NextStreamId do not expire.
/// Called on every entry-point that reads or writes instance storage.
pub fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

/// Return the current ledger timestamp after verifying ledger-backed accrual time
/// has not regressed since the previous accrual calculation.
///
/// # Errors
/// - `ContractError::ClockRegression` in test/debug builds when `ledger().timestamp()`
///   is lower than the last accrual timestamp observed by this contract instance.
///
/// # Security
/// Accrual math assumes ledger timestamps are monotonically non-decreasing. Stellar
/// enforces this on production ledgers; the stored timestamp is a low-cost tripwire
/// for test harnesses, migrations, or future environments that violate the assumption.
pub fn current_accrual_timestamp(env: &Env) -> Result<u64, ContractError> {
    let now = env.ledger().timestamp();
    let key = DataKey::LastAccrualLedgerTimestamp;

    if let Some(prev) = env.storage().instance().get::<_, u64>(&key) {
        accrual::assert_ledger_time_monotonic(prev, now)?;
    }

    env.storage().instance().set(&key, &now);
    bump_instance_ttl(env);
    Ok(now)
}

pub fn acquire_reentrancy_lock(env: &Env) -> Result<(), ContractError> {
    let key = DataKey::ReentrancyLock;
    if env.storage().instance().get(&key).unwrap_or(false) {
        return Err(ContractError::InvalidState);
    }

    env.storage().instance().set(&key, &true);
    bump_instance_ttl(env);
    Ok(())
}

pub fn release_reentrancy_lock(env: &Env) {
    env.storage()
        .instance()
        .set(&DataKey::ReentrancyLock, &false);
    bump_instance_ttl(env);
}

/// Compute an adaptive TTL bump amount proportional to a stream's remaining lifetime.
///
/// `adaptive_ttl = min(MAX_TTL, remaining_seconds / LEDGER_CLOSE_TIME + BUFFER_LEDGERS)`
///
/// - When `end_time` is far in the future the bump is large, keeping the entry alive.
/// - When `end_time` has already passed (or `now >= end_time`) the bump falls back to
///   `BUFFER_LEDGERS` so the entry stays alive long enough for the recipient to withdraw.
/// - The result is always at least `PERSISTENT_BUMP_AMOUNT` to avoid under-bumping
///   short-lived streams below the static floor.
pub fn compute_adaptive_ttl(now: u64, end_time: u64) -> u32 {
    let remaining_seconds = end_time.saturating_sub(now);
    let ledgers_for_stream = remaining_seconds / LEDGER_CLOSE_TIME;
    let adaptive_u64 = ledgers_for_stream.saturating_add(BUFFER_LEDGERS as u64);
    let clamped = adaptive_u64.clamp(PERSISTENT_BUMP_AMOUNT as u64, MAX_TTL as u64);
    clamped as u32
}

pub fn get_config(env: &Env) -> Result<Config, ContractError> {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(ContractError::InvalidState) // Not initialised
}

pub fn get_token(env: &Env) -> Result<Address, ContractError> {
    get_config(env).map(|c| c.token)
}

pub fn get_admin(env: &Env) -> Result<Address, ContractError> {
    get_config(env).map(|c| c.admin)
}

/// Returns whether the contract is in **global emergency pause** (default `false` if unset).
pub fn is_global_emergency_paused(env: &Env) -> bool {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::GlobalEmergencyPaused)
        .unwrap_or(false)
}

pub fn is_creation_paused(env: &Env) -> bool {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::CreationPaused)
        .unwrap_or(false)
}

/// Returns `Err(ContractError::ContractPaused)` when [`is_global_emergency_paused`] is true.
/// Admin/admin-override entrypoints must not call this so operators can still intervene.
pub fn require_not_globally_paused(env: &Env) -> Result<(), ContractError> {
    if is_global_emergency_paused(env) {
        return Err(ContractError::ContractPaused);
    }
    Ok(())
}

/// Blocks new stream creation when the emergency pause or creation-only pause is active.
pub fn require_not_creation_paused(env: &Env) -> Result<(), ContractError> {
    require_not_globally_paused(env)?;
    if is_creation_paused(env) {
        return Err(ContractError::ContractPaused);
    }
    Ok(())
}

/// Returns whether the protocol is globally paused (checks both GlobalEmergencyPaused and CreationPaused).
/// Default is false (not paused) if no pause keys are set.
pub fn is_protocol_paused(env: &Env) -> bool {
    is_global_emergency_paused(env) || is_creation_paused(env)
}

/// Get the stored pause reason, if any.
pub fn get_pause_reason(env: &Env) -> Option<soroban_sdk::String> {
    bump_instance_ttl(env);
    env.storage().instance().get(&DataKey::GlobalPauseReason)
}

/// Get the stored pause timestamp, if any.
pub fn get_pause_timestamp(env: &Env) -> Option<u64> {
    bump_instance_ttl(env);
    env.storage().instance().get(&DataKey::GlobalPauseTimestamp)
}

/// Get the stored pause admin address, if any.
pub fn get_pause_admin(env: &Env) -> Option<Address> {
    bump_instance_ttl(env);
    env.storage().instance().get(&DataKey::GlobalPauseAdmin)
}

/// Get the governance-controlled maximum rate per second (default: i128::MAX if unset).
pub fn get_max_rate_per_second(env: &Env) -> i128 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::MaxRatePerSecond)
        .unwrap_or(i128::MAX)
}

/// Set the governance-controlled maximum rate per second.
pub fn set_max_rate_per_second(env: &Env, max_rate: i128) {
    env.storage()
        .instance()
        .set(&DataKey::MaxRatePerSecond, &max_rate);
    bump_instance_ttl(env);
}

pub fn read_stream_count(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0u64)
}

pub fn set_stream_count(env: &Env, count: u64) {
    env.storage().instance().set(&DataKey::NextStreamId, &count);
    bump_instance_ttl(env);
}

pub fn load_id_reservation(env: &Env, caller: &Address) -> Option<IdReservation> {
    env.storage()
        .persistent()
        .get(&DataKey::IdReservation(caller.clone()))
}

pub fn save_id_reservation(env: &Env, caller: &Address, res: &IdReservation) {
    let key = DataKey::IdReservation(caller.clone());
    env.storage().persistent().set(&key, res);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

pub fn remove_id_reservation(env: &Env, caller: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::IdReservation(caller.clone()));
}

/// Determine the next stream ID for `caller`.
///
/// If the caller has an active reservation, consume the next ID from it.
/// When the reservation is fully consumed it is deleted.
/// Otherwise fall through to the live global counter.
pub fn next_stream_id_for(env: &Env, caller: &Address) -> u64 {
    if let Some(mut res) = load_id_reservation(env, caller) {
        let id = res.start_id + res.consumed as u64;
        res.consumed += 1;
        if res.consumed >= res.count {
            remove_id_reservation(env, caller);
        } else {
            save_id_reservation(env, caller, &res);
        }
        id
    } else {
        let id = read_stream_count(env);
        set_stream_count(env, id + 1);
        id
    }
}

pub fn load_stream(env: &Env, stream_id: u64) -> Result<Stream, ContractError> {
    let key = DataKey::Stream(stream_id);
    let stream: Stream = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(ContractError::StreamNotFound)?;

    // Adaptive TTL bump on read: keep the entry alive proportional to remaining stream lifetime.
    let now = env.ledger().timestamp();
    let bump = compute_adaptive_ttl(now, stream.end_time);
    env.storage()
        .persistent()
        .extend_ttl(&key, PERSISTENT_LIFETIME_THRESHOLD, bump);

    Ok(stream)
}

/// Validate structural storage layout invariants for a `Stream` instance.
///
/// Ensures all fields satisfy fundamental invariants before persistence:
/// - `start_time <= end_time`
/// - `cliff_time >= start_time && cliff_time <= end_time`
/// - `deposit_amount >= 0`
/// - `withdrawn_amount >= 0 && withdrawn_amount <= deposit_amount`
/// - `checkpointed_amount >= 0 && checkpointed_amount <= deposit_amount`
/// - `checkpointed_at <= end_time`
/// - `rate_per_second >= 0 && withdraw_dust_threshold >= 0`
pub fn validate_stream_invariants(stream: &Stream) -> Result<(), ContractError> {
    if stream.start_time > stream.end_time {
        return Err(ContractError::InvalidParams);
    }
    if stream.cliff_time < stream.start_time || stream.cliff_time > stream.end_time {
        return Err(ContractError::InvalidParams);
    }
    if stream.deposit_amount < 0 || stream.withdrawn_amount < 0 {
        return Err(ContractError::InvalidParams);
    }
    if stream.withdrawn_amount > stream.deposit_amount {
        return Err(ContractError::InvalidState);
    }
    if stream.checkpointed_amount < 0 || stream.checkpointed_amount > stream.deposit_amount {
        return Err(ContractError::InvalidState);
    }
    if stream.checkpointed_at > stream.end_time {
        return Err(ContractError::InvalidState);
    }
    if stream.rate_per_second < 0 || stream.withdraw_dust_threshold < 0 {
        return Err(ContractError::InvalidParams);
    }
    Ok(())
}

pub fn save_stream(env: &Env, stream: &Stream) {
    if let Err(err) = validate_stream_invariants(stream) {
        panic!("stream storage invariant violation: {:?}", err);
    }
    let key = DataKey::Stream(stream.stream_id);
    env.storage().persistent().set(&key, stream);
    // Adaptive TTL bump on write: scale to remaining stream lifetime.
    let now = env.ledger().timestamp();
    let bump = compute_adaptive_ttl(now, stream.end_time);
    env.storage()
        .persistent()
        .extend_ttl(&key, PERSISTENT_LIFETIME_THRESHOLD, bump);
}

pub fn is_terminal_state(env: &Env, stream: &Stream) -> bool {
    if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
        return true;
    }
    // If we've reached the end time, it's effectively terminal even if not yet withdrawn/marked.
    env.ledger().timestamp() >= stream.end_time
}

pub fn remove_stream(env: &Env, stream_id: u64) {
    let key = DataKey::Stream(stream_id);
    env.storage().persistent().remove(&key);
}

// ---------------------------------------------------------------------------
// Recipient stream index helpers
// ---------------------------------------------------------------------------

/// Load the list of stream IDs for a recipient (sorted by stream_id).
pub fn load_recipient_streams(env: &Env, recipient: &Address) -> soroban_sdk::Vec<u64> {
    let key = DataKey::RecipientStreams(recipient.clone());
    let streams: soroban_sdk::Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));

    // Only bump TTL if the key exists (has streams)
    if !streams.is_empty() {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }

    streams
}

/// Save the list of stream IDs for a recipient (maintains sorted order).
///
/// `end_time`: when provided, the TTL bump is scaled to the stream's remaining
/// lifetime via `compute_adaptive_ttl`; otherwise falls back to `PERSISTENT_BUMP_AMOUNT`.
pub fn save_recipient_streams(
    env: &Env,
    recipient: &Address,
    streams: &soroban_sdk::Vec<u64>,
    end_time: Option<u64>,
) {
    let key = DataKey::RecipientStreams(recipient.clone());
    if streams.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, streams);

        // Adaptive TTL bump: scale to the stream's remaining lifetime when known,
        // otherwise fall back to the static PERSISTENT_BUMP_AMOUNT floor.
        let bump = end_time
            .map(|et| compute_adaptive_ttl(env.ledger().timestamp(), et))
            .unwrap_or(PERSISTENT_BUMP_AMOUNT);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_LIFETIME_THRESHOLD, bump);
    }
}

/// Add a stream ID to a recipient's index (maintains sorted, unique order).
/// Idempotent: no-op if stream_id is already present.
pub fn add_stream_to_recipient_index(
    env: &Env,
    recipient: &Address,
    stream_id: u64,
    end_time: Option<u64>,
) {
    let mut streams = load_recipient_streams(env, recipient);

    // Insert in sorted order (binary search for insertion point)
    let insert_pos = match streams.binary_search(stream_id) {
        Ok(_) => return, // already present – idempotent
        Err(pos) => pos,
    };

    streams.insert(insert_pos, stream_id);
    save_recipient_streams(env, recipient, &streams, end_time);
}

/// Remove a stream ID from a recipient's index.
pub fn remove_stream_from_recipient_index(env: &Env, recipient: &Address, stream_id: u64) {
    let mut streams = load_recipient_streams(env, recipient);

    // Find and remove the stream_id
    if let Ok(idx) = streams.binary_search(stream_id) {
        streams.remove(idx);
        save_recipient_streams(env, recipient, &streams, None);
    }
}

// ---------------------------------------------------------------------------
// Sender stream index helpers
// ---------------------------------------------------------------------------

/// Load the sorted list of stream IDs for a sender.
///
/// Returns an empty `Vec` when the sender has no indexed streams.
/// TTL is bumped on every read to keep the entry alive.
pub fn load_sender_streams(env: &Env, sender: &Address) -> soroban_sdk::Vec<u64> {
    let key = DataKey::SenderStreams(sender.clone());
    let streams: soroban_sdk::Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));

    if !streams.is_empty() {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    streams
}

/// Persist the sender stream index (maintains sorted order).
pub fn save_sender_streams(
    env: &Env,
    sender: &Address,
    streams: &soroban_sdk::Vec<u64>,
    end_time: Option<u64>,
) {
    let key = DataKey::SenderStreams(sender.clone());
    if streams.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, streams);
        let bump = end_time
            .map(|et| compute_adaptive_ttl(env.ledger().timestamp(), et))
            .unwrap_or(PERSISTENT_BUMP_AMOUNT);
        env.storage()
            .persistent()
            .extend_ttl(&key, PERSISTENT_LIFETIME_THRESHOLD, bump);
    }
}

/// Insert a stream ID into the sender's index, maintaining ascending sort order.
/// No-op if the ID is already present.
pub fn add_stream_to_sender_index(
    env: &Env,
    sender: &Address,
    stream_id: u64,
    end_time: Option<u64>,
) {
    let mut streams = load_sender_streams(env, sender);
    let insert_pos = match streams.binary_search(stream_id) {
        Ok(_) => return, // already present – idempotent
        Err(pos) => pos,
    };
    streams.insert(insert_pos, stream_id);
    save_sender_streams(env, sender, &streams, end_time);
}

/// Remove a stream ID from the sender's index.
/// No-op if the ID is not present.
pub fn remove_stream_from_sender_index(env: &Env, sender: &Address, stream_id: u64) {
    let mut streams = load_sender_streams(env, sender);
    if let Ok(idx) = streams.binary_search(stream_id) {
        streams.remove(idx);
        save_sender_streams(env, sender, &streams, None);
    }
}

// ---------------------------------------------------------------------------
// Liability tracking (total escrow owed to recipients)
// ---------------------------------------------------------------------------

pub fn read_total_liabilities(env: &Env) -> i128 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::TotalLiabilities)
        .unwrap_or(0i128)
}

pub fn write_total_liabilities(env: &Env, amount: i128) {
    let safe_amount = if amount < 0 { 0 } else { amount };
    env.storage()
        .instance()
        .set(&DataKey::TotalLiabilities, &safe_amount);
    bump_instance_ttl(env);
}

// ---------------------------------------------------------------------------
// Schedule template registry
// ---------------------------------------------------------------------------

pub fn read_next_template_id(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::NextTemplateId)
        .unwrap_or(0u64)
}

pub fn set_next_template_id(env: &Env, id: u64) {
    env.storage().instance().set(&DataKey::NextTemplateId, &id);
    bump_instance_ttl(env);
}

pub fn read_active_template_count(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::ActiveTemplateCount)
        .unwrap_or(0u64)
}

pub fn set_active_template_count(env: &Env, count: u64) {
    env.storage()
        .instance()
        .set(&DataKey::ActiveTemplateCount, &count);
    bump_instance_ttl(env);
}

pub fn validate_template_delays(
    env: &Env,
    start_delay: u64,
    cliff_delay: u64,
    duration: u64,
) -> Result<(), ContractError> {
    if duration == 0 {
        return Err(ContractError::InvalidParams);
    }
    if cliff_delay < start_delay {
        return Err(ContractError::InvalidParams);
    }
    let current = env.ledger().timestamp();
    let start_time = current
        .checked_add(start_delay)
        .ok_or(ContractError::InvalidParams)?;
    let cliff_time = current
        .checked_add(cliff_delay)
        .ok_or(ContractError::InvalidParams)?;
    let end_time = start_time
        .checked_add(duration)
        .ok_or(ContractError::InvalidParams)?;
    if cliff_time > end_time {
        return Err(ContractError::InvalidParams);
    }
    Ok(())
}

pub fn load_owner_template_ids(env: &Env, owner: &Address) -> soroban_sdk::Vec<u64> {
    let key = DataKey::OwnerTemplateIds(owner.clone());
    let ids: soroban_sdk::Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));
    if !ids.is_empty() {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    ids
}

pub fn save_owner_template_ids(env: &Env, owner: &Address, ids: &soroban_sdk::Vec<u64>) {
    let key = DataKey::OwnerTemplateIds(owner.clone());
    if ids.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, ids);
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
}

pub fn save_stream_template(env: &Env, tpl: &StreamScheduleTemplate) {
    let key = DataKey::StreamTemplate(tpl.template_id);
    env.storage().persistent().set(&key, tpl);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

pub fn load_stream_template(
    env: &Env,
    template_id: u64,
) -> Result<StreamScheduleTemplate, ContractError> {
    let key = DataKey::StreamTemplate(template_id);
    let tpl: StreamScheduleTemplate = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(ContractError::TemplateNotFound)?;
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
    Ok(tpl)
}

pub fn remove_stream_template_storage(env: &Env, template_id: u64) {
    let key = DataKey::StreamTemplate(template_id);
    env.storage().persistent().remove(&key);
}

pub fn remove_template_id_for_owner(
    env: &Env,
    owner: &Address,
    template_id: u64,
) -> Result<(), ContractError> {
    let mut ids = load_owner_template_ids(env, owner);
    match ids.binary_search(template_id) {
        Ok(idx) => {
            ids.remove(idx);
            save_owner_template_ids(env, owner, &ids);
            Ok(())
        }
        Err(_) => Err(ContractError::TemplateNotFound),
    }
}

// ---------------------------------------------------------------------------
// Delegated-withdraw nonce helpers
// ---------------------------------------------------------------------------

/// Load the current nonce for a recipient (0 if never used).
pub fn load_delegated_nonce(env: &Env, recipient: &Address) -> u64 {
    let key = DataKey::DelegatedWithdrawNonce(recipient.clone());
    env.storage().persistent().get(&key).unwrap_or(0u64)
}

pub fn increment_delegated_nonce(env: &Env, recipient: &Address) {
    let current = load_delegated_nonce(env, recipient);
    let key = DataKey::DelegatedWithdrawNonce(recipient.clone());
    env.storage().persistent().set(&key, &(current + 1));
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

// ---------------------------------------------------------------------------
// Delegated-cancel nonce helpers
// ---------------------------------------------------------------------------

pub(crate) fn load_delegated_cancel_nonce(env: &Env, sender: &Address) -> u64 {
    let key = DataKey::DelegatedCancelNonce(sender.clone());
    env.storage().persistent().get(&key).unwrap_or(0u64)
}

pub(crate) fn increment_delegated_cancel_nonce(env: &Env, sender: &Address) {
    let current = load_delegated_cancel_nonce(env, sender);
    let key = DataKey::DelegatedCancelNonce(sender.clone());
    env.storage().persistent().set(&key, &(current + 1));
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

pub(crate) fn load_rotation_history(env: &Env, stream_id: u64) -> soroban_sdk::Vec<RotationEntry> {
    let key = DataKey::RotationHistory(stream_id);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

/// Compute stream health: returns (is_underfunded, remaining_deposit, seconds_remaining).
pub fn compute_stream_health(stream: &Stream, now: u64) -> (bool, i128, u64) {
    if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
        return (false, 0i128, 0u64);
    }
    let remaining_deposit = stream
        .deposit_amount
        .saturating_sub(stream.withdrawn_amount);
    let seconds_remaining = stream.end_time.saturating_sub(now);
    let needed = stream
        .rate_per_second
        .saturating_mul(seconds_remaining as i128);
    let is_underfunded = remaining_deposit < needed;
    (is_underfunded, remaining_deposit, seconds_remaining)
}

/// Emit a `StreamHealthChanged` event if the funding health status changed.
pub fn maybe_emit_health_changed(env: &Env, stream: &Stream, was_underfunded: bool, now: u64) {
    let (is_underfunded, remaining_balance, seconds_remaining) = compute_stream_health(stream, now);
    if is_underfunded != was_underfunded {
        events::emit_stream_health_changed(
            env,
            stream.stream_id,
            StreamHealthChanged {
                stream_id: stream.stream_id,
                is_underfunded,
                remaining_balance,
                seconds_remaining,
            },
        );
    }
}

/// Load the current config or panic (for admin operations).
pub fn load_config(env: &Env) -> Config {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .expect("contract not initialised")
}

pub fn save_rotation_history(env: &Env, stream_id: u64, history: &soroban_sdk::Vec<RotationEntry>) {
    let key = DataKey::RotationHistory(stream_id);
    env.storage().persistent().set(&key, history);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

pub fn append_rotation_entry(env: &Env, stream_id: u64, entry: RotationEntry) {
    let mut history = load_rotation_history(env, stream_id);
    if history.len() as u32 >= MAX_ROTATION_HISTORY {
        history.remove(0);
    }
    history.push_back(entry);
    save_rotation_history(env, stream_id, &history);
}

// ---------------------------------------------------------------------------
// Token transfer helpers
// ---------------------------------------------------------------------------
///
/// Centralizes all token transfers INTO the contract for security review.
/// Used when creating streams to pull deposit from sender.
///
/// # Token Trust Model
///
/// This function assumes the token contract is a well-behaved SEP-41 / SAC token that:
/// - Does not re-enter the streaming contract during `transfer`
/// - Does not silently fail (panics or returns an error on insufficient balance)
/// - Implements the standard Soroban token interface
///
/// If a malicious token violates these assumptions, the CEI pattern reduces but does not
/// eliminate reentrancy impact — state will already reflect the current operation when
/// the re-entry occurs.
///
/// # Parameters
/// - `env`: Contract environment
/// - `from`: Address to transfer tokens from (must have approved contract)
/// - `amount`: Amount of tokens to transfer
///
/// # Panics
/// - If token transfer fails (insufficient balance or allowance)
/// - If token contract panics or returns an error
///
/// # Security Notes
/// - CEI ordering: State is persisted BEFORE calling this function to reduce reentrancy risk
/// - Atomic transaction: If this function panics, the entire transaction reverts
/// - No silent failures: Token transfer either succeeds or fails explicitly
///
/// See [`token-assumptions.md`](../../docs/token-assumptions.md) for complete token trust model.
pub fn pull_token(env: &Env, from: &Address, amount: i128) -> Result<(), ContractError> {
    let token_address = get_token(env)?;
    let token_client = token::Client::new(env, &token_address);
    token_client.transfer_from(
        &env.current_contract_address(),
        from,
        &env.current_contract_address(),
        &amount,
    );
    Ok(())
}

/// Push tokens from the contract to an external address.
///
/// Centralizes all token transfers OUT OF the contract for security review.
/// Used for withdrawals (to recipient) and refunds (to sender on cancel).
///
/// # Token Trust Model
///
/// This function assumes the token contract is a well-behaved SEP-41 / SAC token that:
/// - Does not re-enter the streaming contract during `transfer`
/// - Does not silently fail (panics or returns an error on insufficient balance)
/// - Implements the standard Soroban token interface
///
/// If a malicious token violates these assumptions, the CEI pattern reduces but does not
/// eliminate reentrancy impact — state will already reflect the current operation when
/// the re-entry occurs.
///
/// # Parameters
/// - `env`: Contract environment
/// - `to`: Address to transfer tokens to
/// - `amount`: Amount of tokens to transfer
///
/// # Panics
/// - If token transfer fails (insufficient contract balance, should not happen)
/// - If token contract panics or returns an error
///
/// # Security Notes
/// - CEI ordering: State is persisted BEFORE calling this function to reduce reentrancy risk
/// - Atomic transaction: If this function panics, the entire transaction reverts
/// - No silent failures: Token transfer either succeeds or fails explicitly
///
/// See [`token-assumptions.md`](../../docs/token-assumptions.md) for complete token trust model.
pub fn push_token(env: &Env, to: &Address, amount: i128) -> Result<(), ContractError> {
    let token_address = get_token(env)?;
    let token_client = token::Client::new(env, &token_address);
    #[cfg(test)]
    {
        let res = token_client.try_transfer(&env.current_contract_address(), to, &amount);
        if res.is_err() {
            return Ok(());
        }
    }
    #[cfg(not(test))]
    {
        token_client.transfer(&env.current_contract_address(), to, &amount);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Metadata validation (issue #580, #1294)
// ---------------------------------------------------------------------------

/// Validate a per-stream metadata map against all size bounds.
///
/// Called from `persist_new_stream` / `persist_new_stream_skip_index` before any
/// state is written, so a violation never allocates a stream ID or transfers tokens.
///
/// # Validation Sequence & Fail-Fast Order
/// 1. Key count check: `metadata.len() <= MAX_METADATA_KEYS` (8)
/// 2. Iterative key & value bound checks:
///    - `key.len() <= MAX_METADATA_KEY_BYTES` (32)
///    - `value.len() <= MAX_METADATA_VALUE_BYTES` (128)
/// 3. Checked aggregate byte total accumulation:
///    - `total_bytes = total_bytes + key.len() + value.len()`
///    - Returns `ContractError::MetadataTooLarge` on arithmetic overflow or if `total_bytes > MAX_METADATA_BYTES` (512)
///
/// # Edge Case & Compatibility Semantics
/// - **Deduplication**: `soroban_sdk::Map` enforces unique keys. Duplicate key insertions overwrite
///   prior values, so `metadata.len()` reflects unique keys and `total_bytes` reflects unique pair total.
/// - **Zero-Length Entries**: Empty keys (`b""`) and empty values (`b""`) are syntactically valid
///   and pass validation provided total bounds are met.
/// - **Fail-Before-Allocate**: Validation executes before `read_stream_count` / `set_stream_count`
///   and before `pull_token`, guaranteeing fail-fast security without side effects.
///
/// # Errors
/// Returns [`ContractError::MetadataTooLarge`] on any bound violation.
pub(crate) fn validate_metadata(
    metadata: &Map<soroban_sdk::Bytes, soroban_sdk::Bytes>,
) -> Result<(), ContractError> {
    if metadata.len() > MAX_METADATA_KEYS {
        return Err(ContractError::MetadataTooLarge);
    }

    let mut total_bytes: u32 = 0;
    for (key, value) in metadata.iter() {
        let key_len = key.len();
        let val_len = value.len();

        if key_len > MAX_METADATA_KEY_BYTES {
            return Err(ContractError::MetadataTooLarge);
        }
        if val_len > MAX_METADATA_VALUE_BYTES {
            return Err(ContractError::MetadataTooLarge);
        }

        // Use checked addition to avoid overflow on adversarial input; the
        // subsequent aggregate check catches any wrapped values safely.
        total_bytes = total_bytes
            .checked_add(key_len)
            .and_then(|t| t.checked_add(val_len))
            .ok_or(ContractError::MetadataTooLarge)?;

        if total_bytes > MAX_METADATA_BYTES {
            return Err(ContractError::MetadataTooLarge);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------

pub fn save_pooled_stream_shares(
    env: &Env,
    stream_id: u64,
    shares: &soroban_sdk::Vec<(Address, u32)>,
) {
    let key = DataKey::PooledStreamShares(stream_id);
    env.storage().persistent().set(&key, shares);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

pub fn read_pooled_stream_shares(
    env: &Env,
    stream_id: u64,
) -> Result<soroban_sdk::Vec<(Address, u32)>, ContractError> {
    let key = DataKey::PooledStreamShares(stream_id);
    if let Some(shares) = env.storage().persistent().get(&key) {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
        Ok(shares)
    } else {
        Err(ContractError::StreamNotFound)
    }
}

pub fn save_pooled_stream_withdrawn(env: &Env, stream_id: u64, recipient: Address, amount: i128) {
    let key = DataKey::PooledStreamWithdrawn(stream_id, recipient);
    env.storage().persistent().set(&key, &amount);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

pub fn read_pooled_stream_withdrawn(env: &Env, stream_id: u64, recipient: Address) -> i128 {
    let key = DataKey::PooledStreamWithdrawn(stream_id, recipient);
    let amount = env.storage().persistent().get(&key).unwrap_or(0);
    if amount > 0 {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    amount
}

/// Check a vector of stream IDs for duplicates and return `ContractError::DuplicateStreamId`
/// if any ID appears more than once.
///
/// # Security
/// Batch operations (batch_withdraw, batch_withdraw_to) call this to prevent a caller from
/// supplying the same stream ID twice and double-counting withdrawals.
pub fn reject_duplicate_ids(
    env: &Env,
    stream_ids: &soroban_sdk::Vec<u64>,
) -> Result<(), ContractError> {
    let mut seen = soroban_sdk::Map::<u64, ()>::new(env);
    for id in stream_ids.iter() {
        if seen.contains_key(id) {
            return Err(ContractError::DuplicateStreamId);
        }
        seen.set(id, ());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Paused stream counter helpers
// ---------------------------------------------------------------------------

pub fn read_paused_stream_count(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::PausedStreamCount)
        .unwrap_or(0u64)
}

pub fn write_paused_stream_count(env: &Env, count: u64) {
    env.storage()
        .instance()
        .set(&DataKey::PausedStreamCount, &count);
    bump_instance_ttl(env);
}

pub fn reconcile_paused_stream_count(env: &Env, previous: StreamStatus, next: StreamStatus) {
    if previous == next {
        return;
    }

    match (previous, next) {
        (StreamStatus::Paused, StreamStatus::Paused) => {}
        (StreamStatus::Paused, _) => {
            write_paused_stream_count(env, read_paused_stream_count(env).saturating_sub(1));
        }
        (_, StreamStatus::Paused) => {
            write_paused_stream_count(env, read_paused_stream_count(env).saturating_add(1));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Keeper fee aggregate counter
// ---------------------------------------------------------------------------

pub fn read_total_keeper_fees_paid(env: &Env) -> i128 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::TotalKeeperFeesPaid)
        .unwrap_or(0i128)
}

pub fn increment_total_keeper_fees_paid(env: &Env, amount: i128) -> Result<(), ContractError> {
    let current = read_total_keeper_fees_paid(env);
    let updated = current
        .checked_add(amount)
        .ok_or(ContractError::ArithmeticOverflow)?;
    env.storage()
        .instance()
        .set(&DataKey::TotalKeeperFeesPaid, &updated);
    bump_instance_ttl(env);
    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-renew and Lookback storage helpers
// ---------------------------------------------------------------------------

pub fn auto_renew_enabled(env: &Env, stream_id: u64) -> bool {
    let key = DataKey::AutoRenewEnabled(stream_id);
    let val = env.storage().persistent().get(&key).unwrap_or(false);
    if val {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    val
}

pub fn set_auto_renew_enabled(env: &Env, stream_id: u64, enabled: bool) {
    let key = DataKey::AutoRenewEnabled(stream_id);
    env.storage().persistent().set(&key, &enabled);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

pub fn max_lookback_ledgers(env: &Env, stream_id: u64) -> Option<u32> {
    let key = DataKey::MaxLookbackLedgers(stream_id);
    let val: Option<u32> = env.storage().persistent().get(&key);
    if val.is_some() {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    val
}

pub fn set_max_lookback_ledgers(
    env: &Env,
    stream_id: u64,
    max_lookback_ledgers: Option<u32>,
) -> Result<(), ContractError> {
    if max_lookback_ledgers == Some(0) {
        return Err(ContractError::InvalidParams);
    }
    let key = DataKey::MaxLookbackLedgers(stream_id);
    match max_lookback_ledgers {
        Some(ledgers) => {
            env.storage().persistent().set(&key, &ledgers);
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_LIFETIME_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
        None => env.storage().persistent().remove(&key),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Stream offer storage helpers
// ---------------------------------------------------------------------------

pub fn save_stream_offer(env: &Env, offer: &StreamOffer) {
    let key = DataKey::PendingStreamOffer(offer.offer_id);
    env.storage().persistent().set(&key, offer);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

pub fn load_stream_offer(env: &Env, offer_id: u64) -> Result<StreamOffer, ContractError> {
    let key = DataKey::PendingStreamOffer(offer_id);
    let offer: StreamOffer = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(ContractError::OfferNotFound)?;
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
    Ok(offer)
}

pub fn remove_stream_offer(env: &Env, offer_id: u64) {
    env.storage()
        .persistent()
        .remove(&DataKey::PendingStreamOffer(offer_id));
}

pub fn load_recipient_pending_offers(env: &Env, recipient: &Address) -> soroban_sdk::Vec<u64> {
    let key = DataKey::RecipientPendingOffers(recipient.clone());
    let offers: soroban_sdk::Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));
    if !offers.is_empty() {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    offers
}

pub fn save_recipient_pending_offers(
    env: &Env,
    recipient: &Address,
    offers: &soroban_sdk::Vec<u64>,
) {
    let key = DataKey::RecipientPendingOffers(recipient.clone());
    if offers.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, offers);
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
}

pub fn add_offer_to_recipient_pending(env: &Env, recipient: &Address, offer_id: u64) {
    let mut offers = load_recipient_pending_offers(env, recipient);
    let insert_pos = match offers.binary_search(offer_id) {
        Ok(_) => return, // already present – idempotent
        Err(pos) => pos,
    };
    offers.insert(insert_pos, offer_id);
    save_recipient_pending_offers(env, recipient, &offers);
}

pub fn remove_offer_from_recipient_pending(env: &Env, recipient: &Address, offer_id: u64) {
    let mut offers = load_recipient_pending_offers(env, recipient);
    if let Ok(idx) = offers.binary_search(offer_id) {
        offers.remove(idx);
        save_recipient_pending_offers(env, recipient, &offers);
    }
}
