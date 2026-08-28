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
use crate::{apply_lookback_cap};

pub(crate) fn set_auto_claim(
    env: &Env,
    stream_id: u64,
    destination: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
            stream.recipient.require_auth();
    
            // Validate destination
            if !crate::ops::clone::is_valid_destination(env, &destination) {
                return Err(ContractError::InvalidParams);
            }
    
            // Store destination
            let key = DataKey::AutoClaimDestination(stream_id);
            env.storage().persistent().set(&key, &destination);
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_LIFETIME_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
    
            // Emit event
            events::emit_auto_claim_set(
                env,
                stream_id,
                AutoClaimSet {
                    stream_id,
                    destination: destination.clone(),
                },
            );
    
            Ok(())
        
}

pub(crate) fn revoke_auto_claim(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
            stream.recipient.require_auth();
    
            // Remove destination
            let key = DataKey::AutoClaimDestination(stream_id);
            env.storage().persistent().remove(&key);
    
            // Emit event
            events::emit_auto_claim_revoked(env, stream_id, AutoClaimRevoked { stream_id });
    
            Ok(())
        
}

pub(crate) fn trigger_auto_claim(
    env: &Env,
    stream_id: u64,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
    
            let mut stream = load_stream(env, stream_id)?;
    
            // Check stream is not terminal
            if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
                return Err(ContractError::InvalidState);
            }
    
            // Check we're at or past end_time
            let now = current_accrual_timestamp(env)?;
            if now < stream.end_time {
                return Err(ContractError::InvalidState);
            }
    
            // Load auto-claim destination
            let key = DataKey::AutoClaimDestination(stream_id);
            let destination: Address = env
                .storage()
                .persistent()
                .get(&key)
                .ok_or(ContractError::InvalidParams)?;
    
            // Validate destination before proceeding
            if !crate::ops::clone::is_valid_destination(env, &destination) {
                return Err(ContractError::InvalidParams);
            }
    
            // Bump TTL on destination
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_LIFETIME_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
    
            // Calculate withdrawable amount (same logic as withdraw)
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
    
            let withdrawable = apply_lookback_cap(
                env,
                &stream,
                now,
                accrued,
                accrued.saturating_sub(stream.withdrawn_amount).max(0),
            );
    
            // Early return if nothing to withdraw
            if withdrawable == 0 {
                return Ok(0);
            }
    
            // Update stream state (CEI pattern)
            stream.withdrawn_amount = stream
                .withdrawn_amount
                .checked_add(withdrawable)
                .unwrap_or(i128::MAX);
    
            // Check if stream is now completed
            let previous_status = stream.status;
            if stream.withdrawn_amount >= stream.deposit_amount {
                stream.status = StreamStatus::Completed;
            }
    
            save_stream(env, &stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            // Reduce liabilities as tokens leave the contract.
            let liabilities = read_total_liabilities(env)
                .checked_sub(withdrawable)
                .unwrap_or(0);
            write_total_liabilities(env, liabilities);
    
            // Emit auto-claim triggered event
            events::emit_auto_claim_triggered(
                env,
                stream_id,
                AutoClaimTriggered {
                    stream_id,
                    destination: destination.clone(),
                    amount: withdrawable,
                },
            );
    
            // Emit withdrawal event (for consistency with withdraw_to)
            events::emit_withdrawal_to(
                env,
                stream_id,
                WithdrawalTo {
                    stream_id,
                    recipient: stream.recipient.clone(),
                    destination: destination.clone(),
                    amount: withdrawable,
                },
            );
    
            // Emit completed event if applicable
            if stream.status == StreamStatus::Completed {
                events::emit_stream_completed(env, stream_id);
            }
    
            // Acquire reentrancy lock
            acquire_reentrancy_lock(env)?;
    
            // Transfer tokens to destination
            let transfer_result = push_token(env, &destination, withdrawable);
    
            // Release reentrancy lock
            release_reentrancy_lock(env);
    
            // Propagate any transfer errors
            transfer_result?;
    
            Ok(withdrawable)
        
}

pub(crate) fn get_auto_claim_status(
    env: &Env,
    stream_id: u64,
) -> Result<AutoClaimStatus, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
    
            // Check if auto-claim destination is set
            let key = DataKey::AutoClaimDestination(stream_id);
            let destination_opt: Option<Address> = env.storage().persistent().get(&key);
    
            match destination_opt {
                None => Ok(AutoClaimStatus::NotSet),
                Some(destination) => {
                    // Check if destination is valid
                    if !crate::ops::clone::is_valid_destination(env, &destination) {
                        return Ok(AutoClaimStatus::InvalidDestination(
                            AutoClaimInvalidPayload { destination },
                        ));
                    }
    
                    // Calculate claimable amount
                    let now = current_accrual_timestamp(env)?;
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
    
                    let claimable = accrued.saturating_sub(stream.withdrawn_amount).max(0);
                    let claimable = apply_lookback_cap(env, &stream, now, accrued, claimable);
    
                    Ok(AutoClaimStatus::ValidDestination(AutoClaimValidPayload {
                        destination,
                        claimable,
                    }))
                }
            }
        
}

pub(crate) fn get_auto_claim_destination(
    env: &Env,
    stream_id: u64,
) -> Result<Option<Address>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let _stream = load_stream(env, stream_id)?;
            let key = DataKey::AutoClaimDestination(stream_id);
            Ok(env.storage().persistent().get(&key))
        
}

