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
use crate::{apply_lookback_cap, SECONDS_PER_LEDGER, MIN_WITHDRAW_INTERVAL_LEDGERS};

pub(crate) fn withdraw(
    env: &Env,
    stream_id: u64,
    min_expected_amount: Option<i128>,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            // Enforce claim owner or recipient authorization
            if let Some(owner) = &stream.claim_owner {
                owner.require_auth();
            } else {
                stream.recipient.require_auth();
            }
    
            // Enforce withdrawal frequency limit to prevent excessive ledger I/O.
            // Use saturating_sub to prevent underflow from backward timestamp skew
            // (if current_ledger < last_withdraw_ledger, elapsed=0, withdrawal blocked).
            // First withdrawal (last_withdraw_ledger == 0) always succeeds.
            let current_ledger = env.ledger().sequence();
            if stream.last_withdraw_ledger != 0
                && current_ledger.saturating_sub(stream.last_withdraw_ledger)
                    < MIN_WITHDRAW_INTERVAL_LEDGERS
            {
                return Err(ContractError::WithdrawalTooFrequent);
            }
    
            if stream.status == StreamStatus::Completed {
                return Err(ContractError::InvalidState);
            }
    
            if stream.status == StreamStatus::Paused && !is_terminal_state(env, &stream) {
                return Err(ContractError::InvalidState);
            }
    
            let accrued = crate::ops::views::calculate_accrued(env, stream_id)?;
            let mut withdrawable = accrued - stream.withdrawn_amount;
            let effective_time = stream
                .cancelled_at
                .unwrap_or_else(|| env.ledger().timestamp());
            withdrawable = apply_lookback_cap(env, &stream, effective_time, accrued, withdrawable);
    
            // Cap by contract balance for safety (#39)
            let token_address = get_token(env)?;
            let contract_balance =
                token::Client::new(env, &token_address).balance(&env.current_contract_address());
            withdrawable = withdrawable.min(contract_balance);
    
            if let Some(min) = min_expected_amount {
                if withdrawable < min {
                    return Err(ContractError::BelowMinimumAmount);
                }
            }
    
            if withdrawable <= 0 {
                return Ok(0);
            }
    
            // Enforce dust threshold unless terminal state or final drain (#423)
            if withdrawable < stream.withdraw_dust_threshold
                && !is_terminal_state(env, &stream)
                && stream.withdrawn_amount + withdrawable < stream.deposit_amount
            {
                return Ok(0);
            }
    
            // CEI: update state before external token transfer to reduce reentrancy risk.
            // Cross-entrypoint idempotency: state is persisted BEFORE push_token so
            // repeated calls produce the same result (withdrawable will be 0 after
            // the first successful withdrawal).
            stream.withdrawn_amount += withdrawable;
            stream.last_withdraw_ledger = current_ledger; // Update withdrawal timestamp
            let completed_now = (stream.status == StreamStatus::Active
                || stream.status == StreamStatus::Paused)
                && stream.withdrawn_amount == stream.deposit_amount;
            let previous_status = stream.status;
            if completed_now {
                stream.status = StreamStatus::Completed;
            }
            save_stream(env, &stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            // Reduce liabilities as tokens leave the contract to the recipient.
            let liabilities = read_total_liabilities(env)
                .checked_sub(withdrawable)
                .unwrap_or(0);
            write_total_liabilities(env, liabilities);
    
            acquire_reentrancy_lock(env)?;
            let transfer_result = push_token(env, &stream.recipient, withdrawable);
            release_reentrancy_lock(env);
            transfer_result?;
    
            events::emit_withdrawal(
                env,
                stream_id,
                Withdrawal {
                    stream_id,
                    recipient: stream.recipient.clone(),
                    amount: withdrawable,
                },
            );
    
            if completed_now {
                events::emit_stream_completed(env, stream_id);
            }
    
            Ok(withdrawable)
        
}

pub(crate) fn withdraw_to(
    env: &Env,
    stream_id: u64,
    destination: Address,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
    
            // Enforce claim owner or recipient authorization for source of funds
            if let Some(owner) = &stream.claim_owner {
                owner.require_auth();
            } else {
                stream.recipient.require_auth();
            }
    
            if destination == env.current_contract_address() {
                return Err(ContractError::InvalidParams);
            }
    
            if stream.status == StreamStatus::Completed {
                return Err(ContractError::InvalidState);
            }
    
            if stream.status == StreamStatus::Paused && !is_terminal_state(env, &stream) {
                return Err(ContractError::InvalidState);
            }
    
            let accrued = crate::ops::views::calculate_accrued(env, stream_id)?;
            let mut withdrawable = accrued - stream.withdrawn_amount;
            let effective_time = stream
                .cancelled_at
                .unwrap_or_else(|| env.ledger().timestamp());
            withdrawable = apply_lookback_cap(env, &stream, effective_time, accrued, withdrawable);
    
            // Cap by contract balance for safety (#39)
            let token_address = get_token(env)?;
            let contract_balance =
                token::Client::new(env, &token_address).balance(&env.current_contract_address());
            withdrawable = withdrawable.min(contract_balance);
    
            if withdrawable <= 0 {
                return Ok(0);
            }
    
            // Enforce dust threshold unless terminal state or final drain (#423)
            if withdrawable < stream.withdraw_dust_threshold
                && !is_terminal_state(env, &stream)
                && stream.withdrawn_amount + withdrawable < stream.deposit_amount
            {
                return Ok(0);
            }
    
            stream.withdrawn_amount += withdrawable;
            let completed_now = (stream.status == StreamStatus::Active
                || stream.status == StreamStatus::Paused)
                && stream.withdrawn_amount == stream.deposit_amount;
            let previous_status = stream.status;
            if completed_now {
                stream.status = StreamStatus::Completed;
            }
            save_stream(env, &stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            // Reduce liabilities as tokens leave the contract.
            let liabilities = read_total_liabilities(env)
                .checked_sub(withdrawable)
                .unwrap_or(0);
            write_total_liabilities(env, liabilities);
    
            acquire_reentrancy_lock(env)?;
            let transfer_result = push_token(env, &destination, withdrawable);
            release_reentrancy_lock(env);
            transfer_result?;
    
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
    
            if completed_now {
                events::emit_stream_completed(env, stream_id);
            }
    
            Ok(withdrawable)
        
}

pub(crate) fn batch_withdraw(
    env: &Env,
    recipient: Address,
    stream_ids: soroban_sdk::Vec<u64>,
) -> Result<soroban_sdk::Vec<BatchWithdrawResult>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let mut withdrawals = soroban_sdk::Vec::new(env);
            for id in stream_ids.iter() {
                withdrawals.push_back(WithdrawToParam {
                    stream_id: id,
                    destination: recipient.clone(),
                });
            }
            crate::ops::withdraw::batch_withdraw_to(env, recipient, withdrawals)
        
}

pub(crate) fn batch_withdraw_to(
    env: &Env,
    recipient: Address,
    withdrawals: soroban_sdk::Vec<WithdrawToParam>,
) -> Result<soroban_sdk::Vec<BatchWithdrawResult>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            recipient.require_auth();
    
            // --- Batch validation: reject duplicate stream IDs (O(n)) ---
            let mut stream_ids = soroban_sdk::Vec::new(env);
            for param in withdrawals.iter() {
                stream_ids.push_back(param.stream_id);
            }
            reject_duplicate_ids(env, &stream_ids)?;
    
            // Validate destinations
            for param in withdrawals.iter() {
                if param.destination == env.current_contract_address() {
                    return Err(ContractError::InvalidParams);
                }
            }
    
            // Fetch initial contract balance and track remaining safety buffer
            let token_address = get_token(env)?;
            let mut contract_balance =
                token::Client::new(env, &token_address).balance(&env.current_contract_address());
            let mut results = soroban_sdk::Vec::new(env);
    
            // Cache ledger timestamp once — it is constant within a single transaction.
            let now = current_accrual_timestamp(env)?;
            let mut total_liabilities = read_total_liabilities(env);
            let mut liabilities_changed = false;
    
            for param in withdrawals.iter() {
                let mut stream = load_stream(env, param.stream_id)?;
    
                let current_owner = stream
                    .claim_owner
                    .clone()
                    .unwrap_or(stream.recipient.clone());
                if current_owner != recipient {
                    return Err(ContractError::Unauthorized);
                }
    
                let current_ledger = env.ledger().sequence();
                if stream.last_withdraw_ledger != 0
                    && current_ledger.saturating_sub(stream.last_withdraw_ledger)
                        < MIN_WITHDRAW_INTERVAL_LEDGERS
                {
                    return Err(ContractError::WithdrawalTooFrequent);
                }
    
                if stream.status == StreamStatus::Paused && !is_terminal_state(env, &stream) {
                    return Err(ContractError::InvalidState);
                }
    
                let mut withdrawable = if stream.status == StreamStatus::Completed {
                    0
                } else {
                    let effective_now = if stream.status == StreamStatus::Cancelled {
                        stream.cancelled_at.ok_or(ContractError::InvalidState)?
                    } else {
                        now
                    };
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
                        effective_now,
                    );
                    apply_lookback_cap(
                        env,
                        &stream,
                        effective_now,
                        accrued,
                        (accrued - stream.withdrawn_amount).max(0),
                    )
                };
    
                // Cap by running contract balance for safety
                withdrawable = withdrawable.min(contract_balance);
    
                // Enforce dust threshold unless terminal state or final drain
                if withdrawable > 0
                    && withdrawable < stream.withdraw_dust_threshold
                    && !is_terminal_state(env, &stream)
                    && stream.withdrawn_amount + withdrawable < stream.deposit_amount
                {
                    withdrawable = 0;
                }
    
                if withdrawable > 0 {
                    // Decrement running balance before the transfer to ensure atomicity
                    contract_balance -= withdrawable;
    
                    stream.withdrawn_amount += withdrawable;
                    let current_ledger = env.ledger().sequence();
                    stream.last_withdraw_ledger = current_ledger; // Update withdrawal timestamp
                    let completed_now = (stream.status == StreamStatus::Active
                        || stream.status == StreamStatus::Paused)
                        && stream.withdrawn_amount == stream.deposit_amount;
                    let previous_status = stream.status;
                    if completed_now {
                        stream.status = StreamStatus::Completed;
                    }
                    save_stream(env, &stream);
                    reconcile_paused_stream_count(env, previous_status, stream.status);
    
                    // Reduce liabilities locally as tokens leave the contract, then
                    // flush the shared TotalLiabilities slot once after the batch.
                    total_liabilities = total_liabilities.checked_sub(withdrawable).unwrap_or(0);
                    liabilities_changed = true;
    
                    acquire_reentrancy_lock(env)?;
                    let transfer_result = push_token(env, &param.destination, withdrawable);
                    release_reentrancy_lock(env);
                    transfer_result?;
    
                    events::emit_withdrawal_to(
                        env,
                        param.stream_id,
                        WithdrawalTo {
                            stream_id: param.stream_id,
                            recipient: stream.recipient.clone(),
                            destination: param.destination.clone(),
                            amount: withdrawable,
                        },
                    );
    
                    if completed_now {
                        events::emit_stream_completed(env, param.stream_id);
                    }
                }
    
                results.push_back(BatchWithdrawResult {
                    stream_id: param.stream_id,
                    amount: withdrawable,
                });
            }
    
            if liabilities_changed {
                write_total_liabilities(env, total_liabilities);
            }
    
            Ok(results)
        
}

