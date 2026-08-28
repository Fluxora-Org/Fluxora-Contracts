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
use crate::{validate_lookback_window, MAX_POOL_RECIPIENTS};

pub(crate) fn create_stream(
    env: &Env,
    sender: Address,
    params: CreateStreamParams,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let withdraw_dust_threshold = params.withdraw_dust_threshold.unwrap_or(0);
            crate::ops::create::create_stream_internal(
                env,
                sender,
                params.recipient,
                params.deposit_amount,
                params.rate_per_second,
                params.start_time,
                params.cliff_time,
                params.end_time,
                withdraw_dust_threshold,
                params.memo,
                params.kind,
                params.metadata,
                params.irrevocable,
                params.witness,
                None,
            )
        
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_stream_internal(
    env: &Env,
    sender: Address,
    recipient: Address,
    deposit_amount: i128,
    rate_per_second: i128,
    start_time: u64,
    cliff_time: u64,
    end_time: u64,
    withdraw_dust_threshold: i128,
    memo: Option<soroban_sdk::Bytes>,
    kind: StreamKind,
    metadata: Option<Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>,
    irrevocable: Option<bool>,
    witness: Option<Address>,
    max_lookback_ledgers: Option<u32>,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            sender.require_auth();
            require_not_creation_paused(env)?;
            validate_lookback_window(max_lookback_ledgers)?;
    
            let mut final_rate = rate_per_second;
            if kind == StreamKind::CliffOnly {
                final_rate = 0;
            }
    
            ops::validation::validate_stream_params(
                env,
                &sender,
                &recipient,
                deposit_amount,
                final_rate,
                env.ledger().timestamp(),
                start_time,
                cliff_time,
                end_time,
                kind,
            )?;
    
            pull_token(env, &sender, deposit_amount)?;
    
            let stream_id = ops::validation::persist_new_stream(
                env,
                sender,
                recipient,
                deposit_amount,
                final_rate,
                start_time,
                cliff_time,
                end_time,
                withdraw_dust_threshold,
                memo,
                kind,
                metadata,
                irrevocable,
                witness,
            )?;
    
            if let Some(ledgers) = max_lookback_ledgers {
                set_max_lookback_ledgers(env, stream_id, Some(ledgers))?;
            }
    
            Ok(stream_id)
        
}

pub(crate) fn create_stream_with_lookback(
    env: &Env,
    sender: Address,
    params: CreateStreamParams,
    max_lookback_ledgers: Option<u32>,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let withdraw_dust_threshold = params.withdraw_dust_threshold.unwrap_or(0);
            crate::ops::create::create_stream_internal(
                env,
                sender,
                params.recipient,
                params.deposit_amount,
                params.rate_per_second,
                params.start_time,
                params.cliff_time,
                params.end_time,
                withdraw_dust_threshold,
                params.memo,
                params.kind,
                params.metadata,
                params.irrevocable,
                params.witness,
                max_lookback_ledgers,
            )
        
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_pooled_stream(
    env: &Env,
    sender: Address,
    recipients: soroban_sdk::Vec<(Address, u32)>,
    deposit_amount: i128,
    rate_per_second: i128,
    start_time: u64,
    cliff_time: u64,
    end_time: u64,
    withdraw_dust_threshold: i128,
    memo: Option<soroban_sdk::Bytes>,
    kind: StreamKind,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            sender.require_auth();
            require_not_creation_paused(env)?;
    
            if recipients.len() > MAX_POOL_RECIPIENTS {
                return Err(ContractError::InvalidParams);
            }
    
            let mut total_shares: u32 = 0;
            let mut seen_recipients = soroban_sdk::Vec::<Address>::new(env);
            for (recipient, share) in recipients.iter() {
                if share == 0 {
                    return Err(ContractError::InvalidParams);
                }
                for seen in seen_recipients.iter() {
                    if seen == recipient {
                        return Err(ContractError::InvalidParams);
                    }
                }
                seen_recipients.push_back(recipient);
                total_shares = total_shares
                    .checked_add(share)
                    .ok_or(ContractError::ArithmeticOverflow)?;
            }
            if total_shares == 0 {
                return Err(ContractError::InvalidParams);
            }
    
            let mut final_rate = rate_per_second;
            if kind == StreamKind::CliffOnly {
                final_rate = 0;
            }
    
            ops::validation::validate_stream_params_with_self_policy(
                env,
                &sender,
                &sender,
                deposit_amount,
                final_rate,
                env.ledger().timestamp(),
                start_time,
                cliff_time,
                end_time,
                kind,
                true,
            )?;
    
            pull_token(env, &sender, deposit_amount)?;
    
            if let Some(ref m) = memo {
                if m.len() as usize > MAX_MEMO_BYTES {
                    return Err(ContractError::InvalidParams);
                }
            }
    
            let stream_id = next_stream_id_for(env, &sender);
    
            let stream = Stream {
                stream_id,
                sender: sender.clone(),
                recipient: sender.clone(),
                claim_owner: None,
                deposit_amount,
                rate_per_second: final_rate,
                start_time,
                cliff_time,
                end_time,
                withdrawn_amount: 0,
                status: StreamStatus::Active,
                cancelled_at: None,
                checkpointed_amount: 0,
                checkpointed_at: start_time,
                withdraw_dust_threshold,
                memo: memo.clone(),
                kind,
                last_pause_toggle_ledger: 0,
                last_withdraw_ledger: 0,
                metadata: None,
                witness: None,
                is_pooled: Some(true),
                last_rate_change_ledger: 0,
                delegation_depth: 0,
                parent_stream_id: None,
                irrevocable: None,
                decommissioned: None,
                paused_at_timestamp: 0,
                cumulative_paused_duration: 0,
            };
    
            save_stream(env, &stream);
            save_pooled_stream_shares(env, stream_id, &recipients);
            add_stream_to_sender_index(env, &sender, stream_id, Some(end_time));
            for (recipient, _) in recipients.iter() {
                add_stream_to_recipient_index(env, &recipient, stream_id, Some(end_time));
            }
    
            let liabilities = read_total_liabilities(env)
                .checked_add(deposit_amount)
                .unwrap_or(i128::MAX);
            write_total_liabilities(env, liabilities);
    
            events::emit_stream_created(
                env,
                stream_id,
                StreamCreated {
                    stream_id,
                    sender,
                    recipient: stream.recipient.clone(),
                    deposit_amount,
                    rate_per_second: final_rate,
                    start_time,
                    cliff_time,
                    end_time,
                    withdraw_dust_threshold,
                    memo,
                    metadata: None,
                },
            );
    
            Ok(stream_id)
        
}

pub(crate) fn create_streams(
    env: &Env,
    sender: Address,
    streams: soroban_sdk::Vec<CreateStreamParams>,
) -> Result<soroban_sdk::Vec<u64>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            sender.require_auth();
    
            if streams.is_empty() {
                return Ok(soroban_sdk::Vec::new(env));
            }
    
            require_not_creation_paused(env)?;
    
            let current_time = env.ledger().timestamp();
            let mut total_deposit: i128 = 0;
    
            // First pass: validate all streams and calculate total deposit required
            for params in streams.iter() {
                let mut final_rate = params.rate_per_second;
                if params.kind == StreamKind::CliffOnly {
                    final_rate = 0;
                }
    
                ops::validation::validate_stream_params(
                    env,
                    &sender,
                    &params.recipient,
                    params.deposit_amount,
                    final_rate,
                    current_time,
                    params.start_time,
                    params.cliff_time,
                    params.end_time,
                    params.kind,
                )?;
                total_deposit = total_deposit
                    .checked_add(params.deposit_amount)
                    .ok_or(ContractError::ArithmeticOverflow)?;
    
                // Validate metadata if present (fail-before-allocate).
                if let Some(ref meta) = params.metadata {
                    storage::validate_metadata(meta)?;
                }
            }
    
            // Bulk transfer tokens from sender to this contract atomically to save gas.
            // Empty batch: total_deposit = 0, no transfer occurs.
            if total_deposit > 0 {
                pull_token(env, &sender, total_deposit)?;
            }
    
            // Second pass: generate IDs, persist state, and emit events iteratively
            let mut created_ids = soroban_sdk::Vec::new(env);
            let mut recipient_cache: soroban_sdk::Map<Address, soroban_sdk::Vec<u64>> =
                soroban_sdk::Map::new(env);
            for params in streams.iter() {
                let mut final_rate = params.rate_per_second;
                if params.kind == StreamKind::CliffOnly {
                    final_rate = 0;
                }
    
                let stream_id = ops::validation::persist_new_stream_skip_index(
                    env,
                    sender.clone(),
                    params.recipient.clone(),
                    params.deposit_amount,
                    final_rate,
                    params.start_time,
                    params.cliff_time,
                    params.end_time,
                    params.withdraw_dust_threshold.unwrap_or(0),
                    params.memo.clone(),
                    params.kind,
                    params.metadata.clone(),
                    params.irrevocable,
                    params.witness.clone(),
                )?;
                created_ids.push_back(stream_id);
    
                // Accumulate stream_id into the cache for this recipient.
                let mut ids = recipient_cache
                    .get(params.recipient.clone())
                    .unwrap_or_else(|| soroban_sdk::Vec::new(env));
                ids.push_back(stream_id);
                recipient_cache.set(params.recipient.clone(), ids);
            }
    
            // Flush: one read + one write per unique recipient.
            for (recipient, new_ids) in recipient_cache.iter() {
                let mut existing = load_recipient_streams(env, &recipient);
                for id in new_ids.iter() {
                    let insert_pos = match existing.binary_search(id) {
                        Ok(pos) => pos,
                        Err(pos) => pos,
                    };
                    existing.insert(insert_pos, id);
                }
                save_recipient_streams(env, &recipient, &existing, None);
            }
    
            // Flush sender index once for the whole batch (O(1) read + write for the sender).
            {
                let mut existing = load_sender_streams(env, &sender);
                for id in created_ids.iter() {
                    let insert_pos = match existing.binary_search(id) {
                        Ok(pos) => pos,
                        Err(pos) => pos,
                    };
                    existing.insert(insert_pos, id);
                }
                save_sender_streams(env, &sender, &existing, None);
            }
    
            Ok(created_ids)
        
}

pub(crate) fn create_streams_partial(
    env: &Env,
    sender: Address,
    streams: soroban_sdk::Vec<CreateStreamParams>,
) -> Result<soroban_sdk::Vec<CreateStreamResult>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            sender.require_auth();
    
            if streams.is_empty() {
                return Ok(soroban_sdk::Vec::new(env));
            }
    
            require_not_creation_paused(env)?;
    
            let current_time = env.ledger().timestamp();
            let mut results = soroban_sdk::Vec::new(env);
    
            for params in streams.iter() {
                let mut final_rate = params.rate_per_second;
                if params.kind == StreamKind::CliffOnly {
                    final_rate = 0;
                }
    
                // Validation first
                let validation = ops::validation::validate_stream_params(
                    env,
                    &sender,
                    &params.recipient,
                    params.deposit_amount,
                    final_rate,
                    current_time,
                    params.start_time,
                    params.cliff_time,
                    params.end_time,
                    params.kind,
                );
    
                if let Err(e) = validation {
                    results.push_back(CreateStreamResult {
                        success: false,
                        stream_id: None,
                        error: Some(e as u32),
                    });
                    continue;
                }
    
                // Validate metadata if present (fail-before-transfer).
                if let Some(ref meta) = params.metadata {
                    if let Err(e) = storage::validate_metadata(meta) {
                        results.push_back(CreateStreamResult {
                            success: false,
                            stream_id: None,
                            error: Some(e as u32),
                        });
                        continue;
                    }
                }
    
                // Attempt transfer (per-entry isolation)
                let transfer = pull_token(env, &sender, params.deposit_amount);
                if transfer.is_err() {
                    results.push_back(CreateStreamResult {
                        success: false,
                        stream_id: None,
                        error: Some(ContractError::InsufficientBalance as u32),
                    });
                    continue;
                }
    
                // Persist
                let stream_id = ops::validation::persist_new_stream(
                    env,
                    sender.clone(),
                    params.recipient,
                    params.deposit_amount,
                    final_rate,
                    params.start_time,
                    params.cliff_time,
                    params.end_time,
                    params.withdraw_dust_threshold.unwrap_or(0),
                    params.memo.clone(),
                    params.kind,
                    params.metadata.clone(),
                    params.irrevocable,
                    params.witness,
                );
    
                match stream_id {
                    Ok(id) => results.push_back(CreateStreamResult {
                        success: true,
                        stream_id: Some(id),
                        error: None,
                    }),
                    Err(e) => results.push_back(CreateStreamResult {
                        success: false,
                        stream_id: None,
                        error: Some(e as u32),
                    }),
                }
            }
    
            Ok(results)
        
}

pub(crate) fn withdraw_from_pool(
    env: &Env,
    stream_id: u64,
    caller: Address,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            caller.require_auth();
    
            let mut stream = load_stream(env, stream_id)?;
            if stream.is_pooled != Some(true) {
                return Err(ContractError::InvalidState);
            }
    
            if stream.status == StreamStatus::Completed {
                return Err(ContractError::InvalidState);
            }
            if stream.status == StreamStatus::Paused && !is_terminal_state(env, &stream) {
                return Err(ContractError::InvalidState);
            }
    
            let shares = read_pooled_stream_shares(env, stream_id)?;
            let mut caller_share: u32 = 0;
            let mut total_shares: u32 = 0;
            for (addr, share) in shares.iter() {
                if addr == caller {
                    caller_share = caller_share
                        .checked_add(share)
                        .ok_or(ContractError::ArithmeticOverflow)?;
                }
                total_shares = total_shares
                    .checked_add(share)
                    .ok_or(ContractError::ArithmeticOverflow)?;
            }
    
            if caller_share == 0 || total_shares == 0 {
                return Err(ContractError::Unauthorized);
            }
    
            let now = current_accrual_timestamp(env)?;
            let global_accrued = accrual::calculate_accrued_amount_checkpointed(
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
    
            // Round down after applying the share fraction. This prevents any
            // individual pool member from receiving more than their pro-rata claim;
            // residual rounding dust remains in the pool until swept/closed by
            // existing residual handling.
            let caller_accrued = (global_accrued as u128)
                .checked_mul(caller_share as u128)
                .and_then(|val| val.checked_div(total_shares as u128))
                .ok_or(ContractError::ArithmeticOverflow)? as i128;
    
            let caller_withdrawn = read_pooled_stream_withdrawn(env, stream_id, caller.clone());
            let mut withdrawable = caller_accrued - caller_withdrawn;
    
            let token_address = get_token(env)?;
            let contract_balance =
                token::Client::new(env, &token_address).balance(&env.current_contract_address());
            withdrawable = withdrawable.min(contract_balance);
    
            if withdrawable <= 0 {
                return Ok(0);
            }
    
            if withdrawable < stream.withdraw_dust_threshold
                && !is_terminal_state(env, &stream)
                && stream.withdrawn_amount + withdrawable < stream.deposit_amount
            {
                return Ok(0);
            }
    
            stream.withdrawn_amount += withdrawable;
            save_pooled_stream_withdrawn(
                env,
                stream_id,
                caller.clone(),
                caller_withdrawn + withdrawable,
            );
    
            let completed_now = (stream.status == StreamStatus::Active
                || stream.status == StreamStatus::Paused)
                && stream.withdrawn_amount >= stream.deposit_amount;
    
            let previous_status = stream.status;
            if completed_now {
                stream.status = StreamStatus::Completed;
            }
            save_stream(env, &stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            let liabilities = read_total_liabilities(env)
                .checked_sub(withdrawable)
                .unwrap_or(0);
            write_total_liabilities(env, liabilities);
    
            acquire_reentrancy_lock(env)?;
            let transfer_result = push_token(env, &caller, withdrawable);
            release_reentrancy_lock(env);
            transfer_result?;
    
            events::emit_withdrawal(
                env,
                stream_id,
                Withdrawal {
                    stream_id,
                    recipient: caller.clone(),
                    amount: withdrawable,
                },
            );
    
            if completed_now {
                events::emit_stream_completed(env, stream_id);
            }
    
            Ok(withdrawable)
        
}

