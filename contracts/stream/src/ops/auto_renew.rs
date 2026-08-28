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

pub(crate) fn set_auto_renew(
    env: &Env,
    stream_id: u64,
    sender: Address,
    enabled: bool,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let stream = load_stream(env, stream_id)?;
            sender.require_auth();
            if stream.status == StreamStatus::Cancelled {
                return Err(ContractError::InvalidState);
            }
            if sender != stream.sender {
                return Err(ContractError::Unauthorized);
            }
    
            set_auto_renew_enabled(env, stream_id, enabled);
            Ok(())
        
}

pub(crate) fn get_auto_renew(
    env: &Env,
    stream_id: u64,
) -> Result<bool, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_stream(env, stream_id)?;
            Ok(auto_renew_enabled(env, stream_id))
        
}

pub(crate) fn renew_stream(
    env: &Env,
    stream_id: u64,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let stream = load_stream(env, stream_id)?;
    
            if stream.status != StreamStatus::Completed || !auto_renew_enabled(env, stream_id) {
                return Err(ContractError::InvalidState);
            }
    
            let now = current_accrual_timestamp(env)?;
            let duration = stream
                .end_time
                .checked_sub(stream.start_time)
                .ok_or(ContractError::InvalidState)?;
            let cliff_offset = stream
                .cliff_time
                .checked_sub(stream.start_time)
                .ok_or(ContractError::InvalidState)?;
            let new_end_time = now
                .checked_add(duration)
                .ok_or(ContractError::ArithmeticOverflow)?;
            let new_cliff_time = now
                .checked_add(cliff_offset)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            ops::validation::validate_stream_params(
                env,
                &stream.sender,
                &stream.recipient,
                stream.deposit_amount,
                stream.rate_per_second,
                now,
                now,
                new_cliff_time,
                new_end_time,
                stream.kind,
            )?;
    
            let token_address = get_token(env)?;
            let token_client = token::Client::new(env, &token_address);
            let contract_address = env.current_contract_address();
            if token_client.balance(&stream.sender) < stream.deposit_amount
                || token_client.allowance(&stream.sender, &contract_address) < stream.deposit_amount
            {
                return Err(ContractError::AutoRenewFundingUnavailable);
            }
    
            // Disable the consumed opt-in before the external call. Atomic
            // transaction rollback restores it if token transfer or persistence fails.
            set_auto_renew_enabled(env, stream_id, false);
            pull_token(env, &stream.sender, stream.deposit_amount)?;
    
            // Inherit irrevocable and witness settings from the source stream.
            // If a stream was designated irrevocable or assigned a compliance witness
            // originally, auto-renewal carries forward these safety and governance
            // protections so that sender-side cancellation rules and witness attestations
            // remain in force for the renewed stream period rather than silently lapsing.
            let new_stream_id = ops::validation::persist_new_stream(
                env,
                stream.sender.clone(),
                stream.recipient.clone(),
                stream.deposit_amount,
                stream.rate_per_second,
                now,
                new_cliff_time,
                new_end_time,
                stream.withdraw_dust_threshold,
                stream.memo.clone(),
                stream.kind,
                stream.metadata.clone(),
                stream.irrevocable,
                stream.witness.clone(),
            )?;
            set_auto_renew_enabled(env, new_stream_id, true);
    
            env.events().publish(
                (symbol_short!("renewed"), stream_id, new_stream_id),
                StreamRenewed {
                    old_stream_id: stream_id,
                    new_stream_id,
                },
            );
    
            Ok(new_stream_id)
        
}

pub(crate) fn close_completed_stream(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
    
            // Only explicitly terminal streams (Completed or Cancelled) can be closed.
            if stream.status != StreamStatus::Completed && stream.status != StreamStatus::Cancelled {
                return Err(ContractError::InvalidState);
            }
    
            // For Cancelled streams, prove no claimable balance remains before removing.
            // Accrual is frozen at cancelled_at; the recipient may still withdraw the frozen amount.
            // Closing before full settlement would destroy recipient funds.
            if stream.status == StreamStatus::Cancelled {
                let cancelled_at = stream.cancelled_at.ok_or(ContractError::InvalidState)?;
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
                    cancelled_at,
                );
                let claimable = accrued.saturating_sub(stream.withdrawn_amount).max(0);
                if claimable > 0 {
                    return Err(ContractError::InvalidState);
                }
            }
    
            events::emit_stream_closed(env, stream_id);
    
            // Remove stream from recipient's index before deleting the stream
            remove_stream_from_recipient_index(env, &stream.recipient, stream_id);
            env.storage()
                .persistent()
                .remove(&DataKey::AutoRenewEnabled(stream_id));
            env.storage()
                .persistent()
                .remove(&DataKey::MaxLookbackLedgers(stream_id));
            // Remove stream from sender's portfolio index.
            remove_stream_from_sender_index(env, &stream.sender, stream_id);
            remove_stream(env, stream_id);
    
            Ok(())
        
}

pub(crate) fn close_cancelled_stream(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
    
            // Only allow explicit cancelled streams here.
            if stream.status != StreamStatus::Cancelled {
                return Err(ContractError::InvalidState);
            }
    
            // Ensure recipient has fully withdrawn the frozen accrued amount at cancel time.
            let cancelled_at = stream.cancelled_at.ok_or(ContractError::InvalidState)?;
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
                cancelled_at,
            );
            let claimable = accrued.saturating_sub(stream.withdrawn_amount).max(0);
            if claimable > 0 {
                return Err(ContractError::InvalidState);
            }
    
            events::emit_stream_closed(env, stream_id);
    
            // Remove from recipient index and delete stream storage.
            remove_stream_from_recipient_index(env, &stream.recipient, stream_id);
            env.storage()
                .persistent()
                .remove(&DataKey::AutoRenewEnabled(stream_id));
            env.storage()
                .persistent()
                .remove(&DataKey::MaxLookbackLedgers(stream_id));
            // Remove stream from sender's portfolio index.
            remove_stream_from_sender_index(env, &stream.sender, stream_id);
            remove_stream(env, stream_id);
    
            Ok(())
        
}

