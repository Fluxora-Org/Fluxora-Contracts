//! Internal, non-ABI implementations backing the `#[contractimpl]` block
//! in `lib.rs` (issue #1520). Bodies are move-only extractions of the
//! original private associated fns, now `pub(crate)` free fns taking
//! `env: &Env`; exported signatures live on the lib.rs wrappers. The
//! split axis is lifecycle operation: exactly one public impl block
//! (thin wrappers) remains in lib.rs.

use soroban_sdk::symbol_short;
use soroban_sdk::{token, Address, Env, Map, Vec};
use crate::delegation;
use crate::events;
use crate::ops::validation;
use crate::*;
use crate::{check_and_bump_rate_cooldown, MIN_RATE_INTERVAL_LEDGERS};

pub(crate) fn update_rate_per_second(
    env: &Env,
    stream_id: u64,
    new_rate_per_second: i128,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            if stream.decommissioned.unwrap_or(false) {
                return Err(ContractError::InvalidState);
            }
    
            if stream.kind != StreamKind::Linear {
                return Err(ContractError::UnsupportedStreamKind);
            }
    
            check_and_bump_rate_cooldown(env, &mut stream)?;
    
            // Only the original sender can update the rate.
            crate::ops::validation::require_stream_sender(&stream.sender);
    
            // Only mutable (non-terminal) streams can be updated.
            if stream.status != StreamStatus::Active && stream.status != StreamStatus::Paused {
                return Err(ContractError::InvalidState);
            }
    
            if new_rate_per_second <= 0 {
                return Err(ContractError::InvalidParams);
            }
    
            let old_rate = stream.rate_per_second;
            // Forward-only semantics: disallow decreases (use decrease_rate_per_second for that).
            if new_rate_per_second <= old_rate {
                return Err(ContractError::InvalidParams);
            }
    
            // Reject rate changes on expired streams: no remaining duration can accrue.
            let now = current_accrual_timestamp(env)?;
            if now >= stream.end_time {
                return Err(ContractError::InvalidState);
            }
    
            // Enforce governance-controlled maximum rate per second cap.
            let max_rate = get_max_rate_per_second(env);
            if new_rate_per_second > max_rate {
                // Emit event when cap is enforced
                events::emit_rate_cap_enforced(
                    env,
                    stream_id,
                    RateCapEnforced {
                        stream_id,
                        attempted_rate: new_rate_per_second,
                        max_rate_per_second: max_rate,
                    },
                );
                return Err(ContractError::RateCapExceeded);
            }
    
            // Validate that the existing deposit still covers the new total streamable amount.
            let duration = (stream.end_time - stream.start_time) as i128;
            let total_streamable = new_rate_per_second
                .checked_mul(duration)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            if stream.deposit_amount < total_streamable {
                return Err(ContractError::InsufficientDeposit);
            }
    
            // Checkpoint accrued-to-date so the rate increase applies forward-only.
            let accrued_now = accrual::calculate_accrued_amount_checkpointed(
                accrual::CheckpointState {
                    checkpointed_amount: stream.checkpointed_amount,
                    checkpointed_at: stream.checkpointed_at,
                    cliff_time: stream.cliff_time,
                    end_time: stream.end_time,
                    deposit_amount: stream.deposit_amount,
                    kind: stream.kind,
                },
                old_rate,
                now,
            );
            stream.checkpointed_amount = accrued_now;
            stream.checkpointed_at = now;
            stream.rate_per_second = new_rate_per_second;
            // `last_rate_change_ledger` already bumped by `check_and_bump_rate_cooldown`.
            save_stream(env, &stream);
    
            events::emit_rate_updated(
                env,
                stream_id,
                RateUpdated {
                    stream_id,
                    old_rate_per_second: old_rate,
                    new_rate_per_second,
                    effective_time: now,
                },
            );
    
            Ok(())
        
}

pub(crate) fn decrease_rate_per_second(
    env: &Env,
    stream_id: u64,
    new_rate_per_second: i128,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            if stream.decommissioned.unwrap_or(false) {
                return Err(ContractError::InvalidState);
            }
    
            if stream.kind != StreamKind::Linear {
                return Err(ContractError::UnsupportedStreamKind);
            }
    
            check_and_bump_rate_cooldown(env, &mut stream)?;
    
            // Sender-only: only the original creator may reduce the rate.
            crate::ops::validation::require_stream_sender(&stream.sender);
    
            // Terminal streams cannot be mutated.
            if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
                return Err(ContractError::StreamTerminalState);
            }
    
            // Reject once the stream has expired; remaining duration would be zero.
            let now = current_accrual_timestamp(env)?;
            if now >= stream.end_time {
                return Err(ContractError::InvalidState);
            }
    
            // Validate the new rate: must be strictly positive and strictly less than the current rate.
            if new_rate_per_second <= 0 {
                return Err(ContractError::InvalidParams);
            }
            let old_rate = stream.rate_per_second;
            if new_rate_per_second >= old_rate {
                // Must use update_rate_per_second for increases.
                return Err(ContractError::InvalidParams);
            }
    
            // ── Checkpoint ────────────────────────────────────────────────────────────
            // Lock in accrual under the OLD rate at this exact instant.  Any value the
            // recipient could have withdrawn before this call remains reachable after.
            let accrued_now = accrual::calculate_accrued_amount_checkpointed(
                accrual::CheckpointState {
                    checkpointed_amount: stream.checkpointed_amount,
                    checkpointed_at: stream.checkpointed_at,
                    cliff_time: stream.cliff_time,
                    end_time: stream.end_time,
                    deposit_amount: stream.deposit_amount,
                    kind: stream.kind,
                },
                old_rate,
                now,
            );
    
            // ── New deposit ceiling ────────────────────────────────────────────────────
            // Maximum tokens payable under the new rate:
            //   checkpoint + new_rate × remaining_seconds
            let remaining_seconds = (stream.end_time - now) as i128;
            let future_accrual = new_rate_per_second
                .checked_mul(remaining_seconds)
                .ok_or(ContractError::ArithmeticOverflow)?;
            let new_deposit = accrued_now
                .checked_add(future_accrual)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            // new_deposit must fit within the old deposit. A lower rate should never
            // increase the maximum payable amount for the remaining duration. If state
            // is inconsistent (e.g. a pre-upgrade or manually-mutated deposit ceiling),
            // reject deterministically with ArithmeticOverflow instead of silently
            // treating the condition as a generic invalid state.
            let old_deposit = stream.deposit_amount;
            if new_deposit > old_deposit {
                return Err(ContractError::ArithmeticOverflow);
            }
            let refund_amount = old_deposit
                .checked_sub(new_deposit)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            // ── CEI: persist state before token transfer ───────────────────────────────
            stream.checkpointed_amount = accrued_now;
            stream.checkpointed_at = now;
            stream.rate_per_second = new_rate_per_second;
            stream.deposit_amount = new_deposit;
            // `last_rate_change_ledger` already bumped by `check_and_bump_rate_cooldown`.
            save_stream(env, &stream);
    
            // Refund the now-unreachable portion of the deposit to the sender.
            if refund_amount > 0 {
                // Reduce liabilities by the refunded portion (no longer owed to recipient).
                let liabilities = read_total_liabilities(env)
                    .checked_sub(refund_amount)
                    .unwrap_or(0);
                write_total_liabilities(env, liabilities);
                push_token(env, &stream.sender, refund_amount)?;
            }
    
            events::emit_rate_decreased(
                env,
                stream_id,
                RateDecreased {
                    stream_id,
                    old_rate_per_second: old_rate,
                    new_rate_per_second,
                    effective_time: now,
                    checkpointed_amount: accrued_now,
                    refund_amount,
                },
            );
    
            Ok(())
        
}

pub(crate) fn shorten_stream_end_time(
    env: &Env,
    stream_id: u64,
    new_end_time: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            if stream.kind != StreamKind::Linear {
                return Err(ContractError::UnsupportedStreamKind);
            }
    
            // Only the original sender can modify the schedule.
            crate::ops::validation::require_stream_sender(&stream.sender);
    
            // Only non-terminal streams may be shortened.
            crate::ops::cancel::require_cancellable_status(stream.status)?;
    
            if stream.irrevocable.unwrap_or(false) {
                return Err(ContractError::Unauthorized);
            }
    
            let now = current_accrual_timestamp(env)?;
    
            // New end time must move strictly earlier and remain strictly in the future.
            if new_end_time <= now
                || new_end_time <= stream.start_time
                || new_end_time < stream.cliff_time
                || new_end_time >= stream.end_time
            {
                return Err(ContractError::InvalidParams);
            }
    
            // Compute new maximum streamable amount under the shortened schedule.
            let new_duration = (new_end_time - stream.start_time) as i128;
            let new_max_streamable = stream
                .rate_per_second
                .checked_mul(new_duration)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            // Already-accrued entitlement must never be reduced by a schedule change.
            // Lock in the accrual at the current timestamp and use it as a floor for the
            // new deposit, mirroring the safety invariant in `decrease_rate_per_second`.
            let accrued_now = accrual::calculate_accrued_amount_checkpointed(
                accrual::CheckpointState {
                    checkpointed_amount: stream.checkpointed_amount,
                    checkpointed_at: stream.checkpointed_at,
                    cliff_time: stream.cliff_time,
                    end_time: stream.end_time,
                    deposit_amount: stream.deposit_amount,
                    kind: stream.kind,
                },
                stream.rate_per_second,
                now,
            );
            let new_deposit = new_max_streamable.max(accrued_now);
    
            // Deposit must still be sufficient to cover the shortened schedule (by construction
            // this should hold given the original validation, but we keep an explicit check).
            if new_deposit > stream.deposit_amount {
                return Err(ContractError::InvalidParams);
            }
    
            let old_end_time = stream.end_time;
            let old_deposit = stream.deposit_amount;
            let refund_amount = old_deposit
                .checked_sub(new_deposit)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            stream.end_time = new_end_time;
            stream.deposit_amount = new_deposit;
            save_stream(env, &stream);
    
            if refund_amount > 0 {
                // Reduce liabilities by the refunded portion (no longer owed to recipient).
                let liabilities = read_total_liabilities(env)
                    .checked_sub(refund_amount)
                    .unwrap_or(0);
                write_total_liabilities(env, liabilities);
                push_token(env, &stream.sender, refund_amount)?;
            }
    
            events::emit_stream_end_shortened(
                env,
                stream_id,
                StreamEndShortened {
                    stream_id,
                    old_end_time,
                    new_end_time,
                    refund_amount,
                },
            );
    
            Ok(())
        
}

pub(crate) fn extend_stream_end_time(
    env: &Env,
    stream_id: u64,
    new_end_time: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            if stream.decommissioned.unwrap_or(false) {
                return Err(ContractError::InvalidState);
            }
    
            if stream.kind != StreamKind::Linear {
                return Err(ContractError::UnsupportedStreamKind);
            }
    
            // Only the original sender can modify the schedule.
            crate::ops::validation::require_stream_sender(&stream.sender);
    
            // Only non-terminal streams may be extended.
            crate::ops::cancel::require_cancellable_status(stream.status)?;
    
            let now = current_accrual_timestamp(env)?;
    
            // Must move end_time forward in time.
            if new_end_time <= stream.end_time
                || new_end_time <= stream.start_time
                || new_end_time < stream.cliff_time
                || new_end_time < now
            {
                return Err(ContractError::InvalidParams);
            }
    
            // Ensure existing deposit still covers the extended schedule at the current rate.
            let new_duration = (new_end_time - stream.start_time) as i128;
            let new_total_streamable = stream
                .rate_per_second
                .checked_mul(new_duration)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            if new_total_streamable > stream.deposit_amount {
                return Err(ContractError::InsufficientDeposit);
            }
    
            let old_end_time = stream.end_time;
            stream.end_time = new_end_time;
            save_stream(env, &stream);
    
            events::emit_stream_end_extended(
                env,
                stream_id,
                StreamEndExtended {
                    stream_id,
                    old_end_time,
                    new_end_time,
                },
            );
    
            Ok(())
        
}

pub(crate) fn top_up_stream(
    env: &Env,
    stream_id: u64,
    funder: Address,
    amount: i128,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            // --- Checks ---
            if amount <= 0 {
                return Err(ContractError::InvalidParams);
            }
    
            let stream = load_stream(env, stream_id)?;
    
            if stream.decommissioned.unwrap_or(false) {
                return Err(ContractError::InvalidState);
            }
    
            if stream.kind != StreamKind::Linear {
                return Err(ContractError::UnsupportedStreamKind);
            }
    
            if stream.status != StreamStatus::Active && stream.status != StreamStatus::Paused {
                return Err(ContractError::InvalidState);
            }
    
            // Reject top-ups on expired streams to prevent zombie fund lock-up.
            // Even if submitted in the same block as expiry, no seconds remain to
            // stream the new funds, so the deposit would be permanently unclaimable.
            let now = current_accrual_timestamp(env)?;
            if now >= stream.end_time {
                return Err(ContractError::InvalidState);
            }
    
            // Allow any authorized address to top up (third-party funding support).
            funder.require_auth();
    
            // --- Effects ---
            // Increase deposit_amount with overflow protection.
            let new_deposit = stream
                .deposit_amount
                .checked_add(amount)
                .ok_or(ContractError::ArithmeticOverflow)?; // overflow
    
            let new_end_time = stream.end_time;
    
            // Persist updated state BEFORE the external token pull (CEI).
            let mut stream = stream;
            stream.deposit_amount = new_deposit;
            save_stream(env, &stream);
    
            // --- Interactions ---
            pull_token(env, &funder, amount)?;
    
            // Increase liabilities to match the additional deposit.
            // Checked arithmetic: a silent wrap here would corrupt the global
            // liability counter and allow the contract to believe it owes far less
            // than it actually does (severe fund-accounting bug).
            let liabilities = read_total_liabilities(env)
                .checked_add(amount)
                .ok_or(ContractError::ArithmeticOverflow)?;
            write_total_liabilities(env, liabilities);
    
            events::emit_stream_topped_up(
                env,
                stream_id,
                StreamToppedUp {
                    stream_id,
                    top_up_amount: amount,
                    new_deposit_amount: new_deposit,
                    new_end_time,
                },
            );
            Ok(())
        
}

pub(crate) fn set_stream_decommissioned(
    env: &Env,
    stream_id: u64,
    sender: Address,
    decommissioned: bool,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            sender.require_auth();
            if stream.sender != sender {
                return Err(ContractError::Unauthorized);
            }
    
            if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
                return Err(ContractError::InvalidState);
            }
    
            if !decommissioned && stream.irrevocable.unwrap_or(false) {
                return Err(ContractError::Unauthorized);
            }
    
            stream.decommissioned = Some(decommissioned);
            save_stream(env, &stream);
    
            events::emit_stream_decommissioned(env, stream_id, decommissioned);
    
            Ok(())
        
}

pub(crate) fn update_rate(
    env: &Env,
    stream_id: u64,
    new_rate_per_second: i128,
    caller: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            // Authorization
            caller.require_auth();
    
            // Load stream
            let mut stream = load_stream(env, stream_id)?;
    
            // Reject terminal states
            if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
                return Err(ContractError::StreamTerminalState);
            }
    
            if stream.decommissioned.unwrap_or(false) {
                return Err(ContractError::InvalidState);
            }
    
            if stream.kind != StreamKind::Linear {
                return Err(ContractError::UnsupportedStreamKind);
            }
    
            // Only sender or admin can update rate
            let admin = get_admin(env)?;
            if caller != stream.sender && caller != admin {
                return Err(ContractError::Unauthorized);
            }
    
            // Validate new rate
            if new_rate_per_second <= 0 {
                return Err(ContractError::InvalidParams);
            }
    
            let old_rate = stream.rate_per_second;
    
            // Reject rate changes on expired streams (no remaining accrual possible).
            let now = current_accrual_timestamp(env)?;
            if now >= stream.end_time {
                return Err(ContractError::InvalidState);
            }
    
            // 🔑 IMPORTANT: Do NOT touch withdrawn_amount
            // This preserves correctness after partial withdrawals
            stream.rate_per_second = new_rate_per_second;
    
            // Save updated stream
            save_stream(env, &stream);
    
            // Emit event
            events::emit_rate_updated(
                env,
                stream_id,
                RateUpdated {
                    stream_id,
                    old_rate_per_second: old_rate,
                    new_rate_per_second,
                    effective_time: now,
                },
            );
    
            Ok(())
        
}

