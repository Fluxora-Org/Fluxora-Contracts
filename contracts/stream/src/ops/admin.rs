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
use crate::{KEEPER_GRACE_PERIOD_SECONDS, KEEPER_FEE_BPS, MAX_ROTATION_HISTORY};

pub(crate) fn set_admin(
    env: &Env,
    new_admin: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let mut config = get_config(env)?;
            let old_admin = config.admin.clone();
    
            // Only current admin can update admin
            old_admin.require_auth();
    
            // Update admin in config
            config.admin = new_admin.clone();
            env.storage().instance().set(&DataKey::Config, &config);
    
            // Bump TTL after instance write
            bump_instance_ttl(env);
    
            // Emit event with old and new admin addresses
            env.events()
                .publish((symbol_short!("AdminUpd"),), (old_admin, new_admin));
    
            Ok(())
        
}

pub(crate) fn set_max_rate_per_second(
    env: &Env,
    max_rate: i128,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            // Only admin can set governance parameters
            get_admin(env)?.require_auth();
    
            if max_rate <= 0 {
                return Err(ContractError::InvalidParams);
            }
    
            crate::storage::set_max_rate_per_second(env, max_rate);
    
            Ok(())
        
}

pub(crate) fn cancel_stream_as_admin(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            get_admin(env)?.require_auth();
    
            let mut stream = load_stream(env, stream_id)?;
    
            crate::ops::cancel::cancel_stream_internal(env, &mut stream)
        
}

pub(crate) fn witnessed_cancel_stream(
    env: &Env,
    stream_id: u64,
    witness_public_key: soroban_sdk::BytesN<32>,
    deadline: u64,
    witness_signature: soroban_sdk::BytesN<64>,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
    
            delegation::validate_witness_cancel_deadline(env, deadline)?;
    
            let mut stream = load_stream(env, stream_id)?;
    
            let witness_addr = stream
                .witness
                .as_ref()
                .ok_or(ContractError::InvalidParams)?;
    
            if crate::ops::validation::ed25519_pubkey_from_address(env, witness_addr) != witness_public_key.to_array() {
                return Err(ContractError::InvalidSignature);
            }
    
            let msg = delegation::build_witnessed_cancel_message(env, stream_id, deadline);
            env.crypto()
                .ed25519_verify(&witness_public_key, &msg, &witness_signature);
    
            crate::ops::cancel::cancel_stream_internal(env, &mut stream)
        
}

pub(crate) fn keeper_cancel(
    env: &Env,
    stream_id: u64,
    keeper: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            keeper.require_auth();
    
            let mut stream = load_stream(env, stream_id)?;
    
            // Reject streams already in a terminal state.
            crate::ops::cancel::require_cancellable_status(stream.status)?;
    
            if stream.irrevocable.unwrap_or(false) {
                return Err(ContractError::Unauthorized);
            }
    
            let now = env.ledger().timestamp();
    
            // Grace period must have elapsed past end_time.
            let eligible_at = stream
                .end_time
                .checked_add(KEEPER_GRACE_PERIOD_SECONDS)
                .ok_or(ContractError::ArithmeticOverflow)?;
            if now < eligible_at {
                return Err(ContractError::KeeperGracePeriodNotElapsed);
            }
    
            // Compute accrued amount at the moment of keeper cancellation.
            // Since now >= end_time, this is capped at deposit_amount.
            let accrued = accrual::calculate_accrued_amount_checkpointed(
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
    
            // Recipient's outstanding claimable balance (accrued minus prior withdrawals).
            let recipient_amount = accrued.saturating_sub(stream.withdrawn_amount).max(0);
    
            // Unstreamed portion of the deposit; this is where the keeper fee is taken from.
            let sender_refund_gross = stream
                .deposit_amount
                .checked_sub(accrued)
                .ok_or(ContractError::InvalidState)?
                .max(0);
    
            // Keeper fee: KEEPER_FEE_BPS basis points of the gross sender refund.
            let keeper_fee = sender_refund_gross
                .checked_mul(KEEPER_FEE_BPS as i128)
                .ok_or(ContractError::ArithmeticOverflow)?
                / 10_000;
    
            let sender_refund = sender_refund_gross
                .checked_sub(keeper_fee)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            // Capture pre-mutation health for transition detection.
            let (was_underfunded, _, _) = compute_stream_health(&stream, now);
    
            // CEI: write terminal state before any external token transfer.
            let previous_status = stream.status;
            stream.status = StreamStatus::Cancelled;
            stream.cancelled_at = Some(now);
            save_stream(env, &stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            // Reduce liabilities by the total outstanding balance (recipient + sender portions).
            let total_outstanding = recipient_amount
                .checked_add(sender_refund_gross)
                .ok_or(ContractError::ArithmeticOverflow)?;
            if total_outstanding > 0 {
                let liabilities = read_total_liabilities(env)
                    .checked_sub(total_outstanding)
                    .unwrap_or(0);
                write_total_liabilities(env, liabilities);
            }
    
            // Transfer accrued portion directly to the recipient.
            if recipient_amount > 0 {
                push_token(env, &stream.recipient, recipient_amount)?;
            }
    
            // Transfer sender refund (net of keeper fee).
            if sender_refund > 0 {
                push_token(env, &stream.sender, sender_refund)?;
            }
    
            // Transfer keeper incentive.
            // Counter is incremented AFTER the transfer succeeds (CEI ordering).
            if keeper_fee > 0 {
                push_token(env, &keeper, keeper_fee)?;
                increment_total_keeper_fees_paid(env, keeper_fee)?;
            }
    
            events::emit_keeper_cancelled(
                env,
                stream.stream_id,
                KeeperCancelled {
                    stream_id: stream.stream_id,
                    keeper,
                    keeper_fee,
                    recipient_amount,
                    sender_refund,
                },
            );
    
            maybe_emit_health_changed(env, &stream, was_underfunded, now);
    
            Ok(())
        
}

pub(crate) fn get_keeper_fee_split(
    env: &Env,
    stream_id: u64,
) -> Result<(i128, i128), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
    
            // Only Active / Paused streams are keeper-cancellable.
            crate::ops::cancel::require_cancellable_status(stream.status)?;
    
            let now = env.ledger().timestamp();
    
            // Grace period not yet elapsed → not eligible; return zeros rather than an error
            // so callers can query without needing to catch an error for the common polling case.
            let eligible_at = stream
                .end_time
                .checked_add(KEEPER_GRACE_PERIOD_SECONDS)
                .ok_or(ContractError::ArithmeticOverflow)?;
            if now < eligible_at {
                return Ok((0, 0));
            }
    
            let accrued = accrual::calculate_accrued_amount_checkpointed(
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
    
            let sender_refund_gross = stream
                .deposit_amount
                .checked_sub(accrued)
                .ok_or(ContractError::InvalidState)?
                .max(0);
    
            Ok(compute_keeper_fee_split(
                sender_refund_gross,
                KEEPER_FEE_BPS,
            ))
        
}

pub(crate) fn pause_stream_as_admin(
    env: &Env,
    stream_id: u64,
    reason: PauseReason,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let admin = get_admin(env)?;
            admin.require_auth();
    
            let mut stream = load_stream(env, stream_id)?;
    
            if stream.status == StreamStatus::Paused {
                return Err(ContractError::StreamAlreadyPaused);
            }
            if is_terminal_state(env, &stream) {
                return Err(ContractError::StreamTerminalState);
            }
            if stream.status != StreamStatus::Active {
                return Err(ContractError::InvalidState);
            }
    
            // Check pause/resume cooldown to prevent rapid-toggle DoS
            let current_ledger = env.ledger().sequence();
            let ledgers_since_last_toggle =
                current_ledger.saturating_sub(stream.last_pause_toggle_ledger);
            if ledgers_since_last_toggle < MIN_PAUSE_INTERVAL_LEDGERS {
                return Err(ContractError::PauseCooldownActive);
            }
    
            let previous_status = stream.status;
            stream.status = StreamStatus::Paused;
            stream.last_pause_toggle_ledger = current_ledger;
            stream.paused_at_timestamp = env.ledger().timestamp();
            save_stream(env, &stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            let reason_str = match reason {
                PauseReason::Operational => soroban_sdk::String::from_str(env, "Operational"),
                PauseReason::Administrative => soroban_sdk::String::from_str(env, "Administrative"),
                PauseReason::Emergency => soroban_sdk::String::from_str(env, "Emergency"),
                PauseReason::Compliance => soroban_sdk::String::from_str(env, "Compliance"),
            };
            let record = PauseRecord {
                actor: load_config(env).admin,
                timestamp: env.ledger().timestamp(),
                reason: reason_str.clone(),
            };
            env.storage()
                .instance()
                .set(&DataKey::LastPauseRecord(PauseKind::Stream), &record);
    
            events::emit_stream_paused(
                env,
                stream_id,
                StreamPaused {
                    stream_id,
                    reason: reason_str,
                },
            );
            Ok(())
        
}

pub(crate) fn resume_stream_as_admin(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            get_admin(env)?.require_auth();
            let mut stream = load_stream(env, stream_id)?;
    
            if stream.status == StreamStatus::Active {
                return Err(ContractError::StreamNotPaused);
            }
            if is_terminal_state(env, &stream) {
                return Err(ContractError::StreamTerminalState);
            }
            if stream.status != StreamStatus::Paused {
                return Err(ContractError::StreamNotPaused);
            }
    
            // Check pause/resume cooldown to prevent rapid-toggle DoS
            let current_ledger = env.ledger().sequence();
            let ledgers_since_last_toggle =
                current_ledger.saturating_sub(stream.last_pause_toggle_ledger);
            if ledgers_since_last_toggle < MIN_PAUSE_INTERVAL_LEDGERS {
                return Err(ContractError::PauseCooldownActive);
            }
    
            let previous_status = stream.status;
            let paused_duration = env.ledger().timestamp().saturating_sub(stream.paused_at_timestamp);
            stream.cumulative_paused_duration = stream
                .cumulative_paused_duration
                .checked_add(paused_duration)
                .ok_or(ContractError::ArithmeticOverflow)?;
            stream.paused_at_timestamp = 0;
            stream.status = StreamStatus::Active;
            stream.last_pause_toggle_ledger = current_ledger;
            save_stream(env, &stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            events::emit_stream_resumed(env, stream_id);
            Ok(())
        
}

pub(crate) fn bulk_resume_streams_as_admin(
    env: &Env,
    stream_ids: soroban_sdk::Vec<u64>,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            get_admin(env)?.require_auth();
    
            let n = stream_ids.len();
            if n == 0 {
                return Ok(());
            }
    
            // --- Batch validation: reject duplicate stream IDs (O(n)) ---
            reject_duplicate_ids(env, &stream_ids)?;
    
            let current_ledger = env.ledger().sequence();
            let mut streams = soroban_sdk::Vec::<Stream>::new(env);
    
            // ── Phase 1: Validate all IDs (no mutations) ─────────────────────────
            for i in 0..n {
                let id = stream_ids.get(i).unwrap();
    
                // Duplicate detection removed - now handled by reject_duplicate_ids
    
                let stream = load_stream(env, id)?;
    
                if stream.status == StreamStatus::Active {
                    return Err(ContractError::StreamNotPaused);
                }
                if is_terminal_state(env, &stream) {
                    return Err(ContractError::StreamTerminalState);
                }
                if stream.status != StreamStatus::Paused {
                    return Err(ContractError::StreamNotPaused);
                }
    
                let ledgers_since_last_toggle =
                    current_ledger.saturating_sub(stream.last_pause_toggle_ledger);
                if ledgers_since_last_toggle < MIN_PAUSE_INTERVAL_LEDGERS {
                    return Err(ContractError::PauseCooldownActive);
                }
    
                streams.push_back(stream);
            }
    
            // ── Phase 2: Apply resumes ───────────────────────────────────────────
            let now = env.ledger().timestamp();
            for i in 0..n {
                let mut stream = streams.get(i).unwrap();
                let stream_id = stream.stream_id;
                let previous_status = stream.status;
                let paused_duration = now.saturating_sub(stream.paused_at_timestamp);
                stream.cumulative_paused_duration = stream
                    .cumulative_paused_duration
                    .checked_add(paused_duration)
                    .ok_or(ContractError::ArithmeticOverflow)?;
                stream.paused_at_timestamp = 0;
                stream.status = StreamStatus::Active;
                stream.last_pause_toggle_ledger = current_ledger;
                save_stream(env, &stream);
                reconcile_paused_stream_count(env, previous_status, stream.status);
    
                events::emit_stream_resumed(env, stream_id);
            }
    
            Ok(())
        
}

pub(crate) fn set_global_emergency_paused(
    env: &Env,
    paused: bool,
) {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let admin = get_admin(env).unwrap();
            admin.require_auth();
    
            env.storage()
                .instance()
                .set(&DataKey::GlobalEmergencyPaused, &paused);
            bump_instance_ttl(env);
    
            events::emit_global_emergency_pause_changed(env, GlobalEmergencyPauseChanged { paused });
        
}

pub(crate) fn global_resume(
    env: &Env,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let admin = get_admin(env)?;
            admin.require_auth();
    
            if !is_global_emergency_paused(env) {
                return Err(ContractError::InvalidState);
            }
    
            env.storage()
                .instance()
                .set(&DataKey::GlobalEmergencyPaused, &false);
            bump_instance_ttl(env);
    
            events::emit_global_resumed(
                env,
                GlobalResumed {
                    resumed_at: env.ledger().timestamp(),
                },
            );
    
            Ok(())
        
}

pub(crate) fn set_contract_paused(
    env: &Env,
    paused: bool,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            get_admin(env)?.require_auth();
    
            env.storage()
                .instance()
                .set(&DataKey::CreationPaused, &paused);
            bump_instance_ttl(env);
    
            events::emit_contract_pause_changed(env, ContractPauseChanged { paused });
    
            Ok(())
        
}

pub(crate) fn pause_protocol(
    env: &Env,
    admin: Address,
    reason: Option<soroban_sdk::String>,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            admin.require_auth();
    
            // Verify caller is the stored admin
            let stored_admin = get_admin(env)?;
            if admin != stored_admin {
                return Err(ContractError::Unauthorized);
            }
    
            // Idempotent: if already paused, return silently
            if is_protocol_paused(env) {
                // Idempotent: re-pausing is a no-op
                return Ok(());
            }
    
            // Set the global emergency pause flag
            env.storage()
                .instance()
                .set(&DataKey::GlobalEmergencyPaused, &true);
    
            // Store audit trail information
            let reason_str = reason.unwrap_or_else(|| soroban_sdk::String::from_str(env, ""));
            // Enforce MAX_PAUSE_REASON_BYTES to prevent unbounded ledger-entry growth.
            if reason_str.len() > MAX_PAUSE_REASON_BYTES as u32 {
                return Err(ContractError::PauseReasonTooLong);
            }
            env.storage()
                .instance()
                .set(&DataKey::GlobalPauseReason, &reason_str);
    
            let now = env.ledger().timestamp();
            env.storage()
                .instance()
                .set(&DataKey::GlobalPauseTimestamp, &now);
            env.storage()
                .instance()
                .set(&DataKey::GlobalPauseAdmin, &admin);
    
            bump_instance_ttl(env);
    
            // Emit ProtocolPaused event AFTER storage is written
            events::emit_protocol_paused(
                env,
                admin.clone(),
                ProtocolPaused {
                    reason: reason_str,
                    paused_at: now,
                },
            );
    
            Ok(())
        
}

pub(crate) fn resume_protocol(
    env: &Env,
    admin: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            admin.require_auth();
    
            // Verify caller is the stored admin
            let stored_admin = get_admin(env)?;
            if admin != stored_admin {
                return Err(ContractError::Unauthorized);
            }
    
            // Idempotent: if not paused, return silently
            if !is_protocol_paused(env) {
                // Idempotent: resuming when not paused is a no-op
                return Ok(());
            }
    
            // Clear all pause-related storage
            env.storage()
                .instance()
                .set(&DataKey::GlobalEmergencyPaused, &false);
            env.storage().instance().remove(&DataKey::GlobalPauseReason);
            env.storage()
                .instance()
                .remove(&DataKey::GlobalPauseTimestamp);
            env.storage().instance().remove(&DataKey::GlobalPauseAdmin);
    
            bump_instance_ttl(env);
    
            // Emit ProtocolResumed event
            let now = env.ledger().timestamp();
            events::emit_protocol_resumed(env, admin, ProtocolResumed { resumed_at: now });
    
            Ok(())
        
}

pub(crate) fn sweep_excess(
    env: &Env,
    recipient: Address,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            // Only admin can sweep excess tokens
            let admin = get_admin(env)?;
            admin.require_auth();
    
            // NOTE: recipient.require_auth() was intentionally removed.
            // The sweep destination is chosen by the authenticated admin, so the recipient
            // does NOT need to co-sign. This enables sweeping to cold/offline treasury
            // wallets that cannot sign Soroban transactions.
    
            // Get contract's token balance
            let token_address = get_token(env)?;
            let token_client = token::Client::new(env, &token_address);
            let contract_balance = token_client.balance(&env.current_contract_address());
    
            // Get total outstanding liabilities (sum of all active stream deposits)
            let total_liabilities = read_total_liabilities(env);
    
            // Calculate excess: balance - liabilities
            // If liabilities exceed balance, there's no excess (should not happen in normal operation)
            let excess = contract_balance.saturating_sub(total_liabilities);
    
            // If no excess, return early (no transfer needed)
            if excess <= 0 {
                return Ok(0);
            }
    
            // CEI pattern: Emit event before transfer
            events::emit_excess_swept(
                env,
                recipient.clone(),
                ExcessSwept {
                    to: recipient.clone(),
                    amount: excess,
                },
            );
    
            // Acquire reentrancy lock before token transfer
            acquire_reentrancy_lock(env)?;
    
            // Transfer excess tokens to recipient
            let transfer_result = push_token(env, &recipient, excess);
    
            // Release reentrancy lock
            release_reentrancy_lock(env);
    
            // Propagate any transfer errors
            transfer_result?;
    
            Ok(excess)
        
}

