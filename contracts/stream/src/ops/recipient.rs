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
use crate::{MIN_WITHDRAW_INTERVAL_LEDGERS};

pub(crate) fn delegated_cancel(
    env: &Env,
    stream_id: u64,
    relayer: Address,
    sender_public_key: soroban_sdk::BytesN<32>,
    nonce: u64,
    deadline: u64,
    signature: soroban_sdk::BytesN<64>,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            relayer.require_auth();
    
            delegation::validate_delegated_cancel_params(env, stream_id, nonce, deadline)?;
    
            let mut stream = load_stream(env, stream_id)?;
    
            if crate::ops::validation::ed25519_pubkey_from_address(env, &stream.sender) != sender_public_key.to_array() {
                return Err(ContractError::InvalidSignature);
            }
    
            let mut msg = soroban_sdk::Bytes::new(env);
            msg.extend_from_slice(delegation::DELEGATED_CANCEL_DOMAIN);
            msg.extend_from_array(&stream_id.to_be_bytes());
            msg.extend_from_array(&nonce.to_be_bytes());
            msg.extend_from_array(&deadline.to_be_bytes());
    
            env.crypto()
                .ed25519_verify(&sender_public_key, &msg, &signature);
    
            crate::storage::increment_delegated_cancel_nonce(env, &stream.sender);
    
            crate::ops::cancel::cancel_stream_internal(env, &mut stream)
        
}

pub(crate) fn update_recipient(
    env: &Env,
    stream_id: u64,
    new_recipient: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let stream = load_stream(env, stream_id)?;
    
            crate::ops::validation::require_stream_sender(&stream.sender);
    
            if new_recipient == stream.recipient {
                return Err(ContractError::InvalidParams);
            }
    
            if crate::ops::recipient::get_pending_recipient_update(env, stream_id).is_some() {
                return Err(ContractError::InvalidState);
            }
    
            let key = DataKey::PendingRecipientUpdate(stream_id);
            env.storage().persistent().set(
                &key,
                &PendingRecipientUpdate {
                    stream_id,
                    proposed_recipient: new_recipient,
                },
            );
            env.storage().persistent().extend_ttl(
                &key,
                PERSISTENT_LIFETIME_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
    
            Ok(())
        
}

pub(crate) fn get_pending_recipient_update(
    env: &Env,
    stream_id: u64,
) -> Option<PendingRecipientUpdate> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            env.storage()
                .persistent()
                .get(&DataKey::PendingRecipientUpdate(stream_id))
        
}

pub(crate) fn accept_recipient_update(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let pending = crate::ops::recipient::get_pending_recipient_update(env, stream_id)
                .ok_or(ContractError::InvalidState)?;
            let mut stream = load_stream(env, stream_id)?;
    
            // Transition: propose → accept — only the current recipient may authorize.
            stream.recipient.require_auth();
            let old_recipient = stream.recipient.clone();
            remove_stream_from_recipient_index(env, &old_recipient, stream_id);
            add_stream_to_recipient_index(
                env,
                &pending.proposed_recipient,
                stream_id,
                Some(stream.end_time),
            );
    
            stream.recipient = pending.proposed_recipient.clone();
            save_stream(env, &stream);
            append_rotation_entry(
                env,
                stream_id,
                RotationEntry {
                    old_addr: old_recipient.clone(),
                    new_addr: pending.proposed_recipient.clone(),
                    ledger: env.ledger().sequence(),
                    role: RotationRole::Recipient,
                    authoriser: old_recipient.clone(),
                },
            );
            env.storage()
                .persistent()
                .remove(&DataKey::PendingRecipientUpdate(stream_id));
    
            events::emit_recipient_updated(
                env,
                stream_id,
                RecipientUpdated {
                    stream_id,
                    old_recipient,
                    new_recipient: pending.proposed_recipient,
                },
            );
    
            Ok(())
        
}

pub(crate) fn cancel_recipient_update(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let stream = load_stream(env, stream_id)?;
            crate::ops::validation::require_stream_sender(&stream.sender);
            if !env
                .storage()
                .persistent()
                .has(&DataKey::PendingRecipientUpdate(stream_id))
            {
                return Err(ContractError::InvalidState);
            }
            env.storage()
                .persistent()
                .remove(&DataKey::PendingRecipientUpdate(stream_id));
            Ok(())
        
}

pub(crate) fn transfer_claim_ownership(
    env: &Env,
    stream_id: u64,
    current_owner: Address,
    new_owner: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            let actual_current = stream
                .claim_owner
                .clone()
                .unwrap_or(stream.recipient.clone());
            if actual_current != current_owner {
                return Err(ContractError::Unauthorized);
            }
    
            current_owner.require_auth();
    
            let old_owner = stream.claim_owner.clone();
            stream.claim_owner = Some(new_owner.clone());
            save_stream(env, &stream);
    
            env.events().publish(
                (symbol_short!("claim_own"), stream_id),
                ClaimOwnershipTransferred {
                    stream_id,
                    old_owner,
                    new_owner,
                },
            );
    
            Ok(())
        
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn delegated_withdraw(
    env: &Env,
    stream_id: u64,
    relayer: Address,
    recipient_public_key: soroban_sdk::BytesN<32>,
    nonce: u64,
    deadline: u64,
    expected_minimum_amount: i128,
    relayer_fee: i128,
    signature: soroban_sdk::BytesN<64>,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
    
            // The relayer authorizes the transaction (pays gas); recipient auth is
            // replaced by the ed25519 signature check below.
            relayer.require_auth();
    
            // 1. Validate delegation parameters (deadline, nonce, & fee >= 0).
            delegation::validate_delegation_params(env, stream_id, nonce, deadline, relayer_fee)?;
    
            // 2. Load stream.
            let mut stream = load_stream(env, stream_id)?;
    
            // 3. Enforce withdrawal frequency limit to prevent excessive ledger I/O.
            let current_ledger = env.ledger().sequence();
            if stream.last_withdraw_ledger != 0
                && current_ledger.saturating_sub(stream.last_withdraw_ledger)
                    < MIN_WITHDRAW_INTERVAL_LEDGERS
            {
                return Err(ContractError::WithdrawalTooFrequent);
            }
    
            // 4. Verify the supplied public key matches the stream recipient.
            if crate::ops::validation::ed25519_pubkey_from_address(env, &stream.recipient)
                != recipient_public_key.to_array()
            {
                return Err(ContractError::InvalidSignature);
            }
    
            // 5. Build the signed message payload (56 bytes total):
            //    stream_id (8 bytes) | nonce (8 bytes) | deadline (8 bytes)
            //    | expected_minimum_amount (16 bytes) | relayer_fee (16 bytes)
            let mut msg = soroban_sdk::Bytes::new(env);
            msg.extend_from_array(&stream_id.to_be_bytes());
            msg.extend_from_array(&nonce.to_be_bytes());
            msg.extend_from_array(&deadline.to_be_bytes());
            msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
            msg.extend_from_array(&relayer_fee.to_be_bytes()); // Included in signed payload
    
            // Verify ed25519 signature
            env.crypto()
                .ed25519_verify(&recipient_public_key, &msg, &signature);
    
            // 6. State checks (same as withdraw).
            if stream.status == StreamStatus::Completed {
                return Err(ContractError::InvalidState);
            }
            if stream.status == StreamStatus::Paused && !is_terminal_state(env, &stream) {
                return Err(ContractError::InvalidState);
            }
    
            // 7. Compute gross withdrawable amount.
            let accrued = crate::ops::views::calculate_accrued(env, stream_id)?;
            let mut gross_withdrawable = accrued - stream.withdrawn_amount;
            gross_withdrawable = apply_lookback_cap(
                env,
                &stream,
                stream
                    .cancelled_at
                    .unwrap_or_else(|| env.ledger().timestamp()),
                accrued,
                gross_withdrawable,
            );
    
            // Cap by contract balance for safety.
            let token_address = get_token(env)?;
            let contract_balance =
                token::Client::new(env, &token_address).balance(&env.current_contract_address());
            gross_withdrawable = gross_withdrawable.min(contract_balance);
    
            if gross_withdrawable < relayer_fee {
                return Err(ContractError::InsufficientBalance);
            }
    
            // 8. Deduct relayer fee to get net payout for recipient
            let net_amount = gross_withdrawable - relayer_fee;
    
            // 9. Enforce minimum amount guard on NET amount
            if net_amount < expected_minimum_amount {
                return Err(ContractError::BelowMinimumAmount);
            }
    
            if gross_withdrawable <= 0 {
                return Ok(0);
            }
    
            // 10. CEI: update state before external token transfers.
            stream.withdrawn_amount += gross_withdrawable;
            stream.last_withdraw_ledger = current_ledger;
            let completed_now = (stream.status == StreamStatus::Active
                || stream.status == StreamStatus::Paused)
                && stream.withdrawn_amount == stream.deposit_amount;
            let previous_status = stream.status;
            if completed_now {
                stream.status = StreamStatus::Completed;
            }
            save_stream(env, &stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            // 11. Increment nonce to prevent replay.
            increment_delegated_nonce(env, &stream.recipient);
    
            // 12. Transfers via push_token: Net payout to RECIPIENT first, Fee to RELAYER second
            // Cross-entrypoint idempotency: reentrancy lock prevents nested token callbacks
            // from corrupting withdrawn_amount or liability tracking.
            acquire_reentrancy_lock(env)?;
            if net_amount > 0 {
                push_token(env, &stream.recipient, net_amount)?;
            }
            if relayer_fee > 0 {
                push_token(env, &relayer, relayer_fee)?;
            }
            release_reentrancy_lock(env);
    
            events::emit_withdrawal(
                env,
                stream_id,
                Withdrawal {
                    stream_id,
                    recipient: stream.recipient.clone(),
                    amount: net_amount,
                },
            );
    
            if completed_now {
                events::emit_stream_completed(env, stream_id);
            }
    
            Ok(net_amount)
        
}

pub(crate) fn get_delegated_nonce(
    env: &Env,
    recipient: Address,
) -> u64 {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_delegated_nonce(env, &recipient)
        
}

pub(crate) fn get_delegated_cancel_nonce(
    env: &Env,
    sender: Address,
) -> u64 {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            crate::storage::load_delegated_cancel_nonce(env, &sender)
        
}

pub(crate) fn delegate_recipient_share(
    env: &Env,
    stream_id: u64,
    recipient: Address,
    share_bps: u32,
    new_recipient: Address,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            if stream.kind == StreamKind::CliffOnly || stream.kind == StreamKind::CliffSlope {
                return Err(ContractError::UnsupportedStreamKind);
            }
    
            recipient.require_auth();
            if stream.recipient != recipient {
                return Err(ContractError::Unauthorized);
            }
    
            if share_bps == 0 || share_bps > 10000 {
                return Err(ContractError::InvalidParams);
            }
    
            if recipient == new_recipient {
                return Err(ContractError::CyclicDelegation);
            }
    
            if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
                return Err(ContractError::StreamTerminalState);
            }
    
            let now = current_accrual_timestamp(env)?;
            if now >= stream.end_time {
                return Err(ContractError::InvalidState);
            }
    
            if stream.delegation_depth >= MAX_DELEGATION_DEPTH {
                return Err(ContractError::DelegationDepthExceeded);
            }
    
            // Prevent cycles by walking only the bounded delegation chain. The
            // current stream is included so callers cannot delegate back to an
            // ancestor recipient or to the current recipient under another path.
            let mut current_stream_id = Some(stream_id);
            let mut checked_depth = 0u32;
            while let Some(candidate_id) = current_stream_id {
                if checked_depth > MAX_DELEGATION_DEPTH {
                    return Err(ContractError::DelegationDepthExceeded);
                }
    
                let candidate = if candidate_id == stream_id {
                    stream.clone()
                } else {
                    load_stream(env, candidate_id)?
                };
                if candidate.recipient == new_recipient {
                    return Err(ContractError::CyclicDelegation);
                }
                current_stream_id = candidate.parent_stream_id;
                checked_depth += 1;
            }
    
            let old_rate = stream.rate_per_second;
            let child_rate = old_rate
                .checked_mul(share_bps as i128)
                .ok_or(ContractError::ArithmeticOverflow)?
                / 10000;
    
            if child_rate <= 0 || child_rate >= old_rate {
                return Err(ContractError::InvalidParams);
            }
    
            let new_rate_per_second = old_rate - child_rate;
    
            // Checkpoint accrual under the old rate
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
    
            let remaining_seconds = (stream.end_time - now) as i128;
            let future_accrual_parent = new_rate_per_second
                .checked_mul(remaining_seconds)
                .ok_or(ContractError::ArithmeticOverflow)?;
            let new_deposit_parent = accrued_now
                .checked_add(future_accrual_parent)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            let child_deposit = stream
                .deposit_amount
                .checked_sub(new_deposit_parent)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            if child_deposit < 0 {
                return Err(ContractError::InvalidState);
            }
    
            // Persist parent state
            stream.checkpointed_amount = accrued_now;
            stream.checkpointed_at = now;
            stream.rate_per_second = new_rate_per_second;
            stream.deposit_amount = new_deposit_parent;
            save_stream(env, &stream);
    
            // Create child stream
            let child_stream_id = next_stream_id_for(env, &stream.sender);
    
            let child_stream = Stream {
                stream_id: child_stream_id,
                sender: stream.sender.clone(),
                recipient: new_recipient.clone(),
                claim_owner: None,
                deposit_amount: child_deposit,
                rate_per_second: child_rate,
                start_time: now,
                cliff_time: stream.cliff_time.max(now),
                end_time: stream.end_time,
                withdrawn_amount: 0,
                status: StreamStatus::Active,
                cancelled_at: None,
                checkpointed_amount: 0,
                checkpointed_at: now,
                withdraw_dust_threshold: stream.withdraw_dust_threshold,
                memo: stream.memo.clone(),
                kind: stream.kind,
                last_pause_toggle_ledger: 0,
                last_withdraw_ledger: 0,
                last_rate_change_ledger: 0,
                metadata: stream.metadata.clone(),
                witness: stream.witness.clone(),
                irrevocable: stream.irrevocable,
                is_pooled: None,
                parent_stream_id: Some(stream_id),
                delegation_depth: stream.delegation_depth + 1,
                decommissioned: None,
                paused_at_timestamp: 0,
                cumulative_paused_duration: 0,
            };
    
            save_stream(env, &child_stream);
            add_stream_to_recipient_index(env, &new_recipient, child_stream_id, Some(stream.end_time));
            add_stream_to_sender_index(env, &stream.sender, child_stream_id, Some(stream.end_time));
    
            env.events().publish(
                (symbol_short!("del_share"), stream_id),
                RecipientShareDelegated {
                    parent_stream_id: stream_id,
                    child_stream_id,
                    delegator: recipient,
                    delegatee: new_recipient,
                    share_bps,
                    new_parent_rate: new_rate_per_second,
                    child_rate,
                },
            );
    
            Ok(child_stream_id)
        
}

