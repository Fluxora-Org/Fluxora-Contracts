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

pub(crate) fn calculate_accrued(
    env: &Env,
    stream_id: u64,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
    
            if stream.status == StreamStatus::Completed {
                return Ok(stream.deposit_amount);
            }
    
            let now = if stream.status == StreamStatus::Cancelled {
                stream.cancelled_at.ok_or(ContractError::InvalidState)?
            } else {
                current_accrual_timestamp(env)?
            };
    
            Ok(accrual::calculate_accrued_amount_checkpointed(
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
            ))
        
}

pub(crate) fn set_lookback_window(
    env: &Env,
    stream_id: u64,
    sender: Address,
    max_lookback_ledgers: Option<u32>,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let stream = load_stream(env, stream_id)?;
            sender.require_auth();
            if sender != stream.sender {
                return Err(ContractError::Unauthorized);
            }
            if stream.status == StreamStatus::Cancelled {
                return Err(ContractError::InvalidState);
            }
            set_max_lookback_ledgers(env, stream_id, max_lookback_ledgers)
        
}

pub(crate) fn get_lookback_window(
    env: &Env,
    stream_id: u64,
) -> Result<Option<u32>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_stream(env, stream_id)?;
            Ok(max_lookback_ledgers(env, stream_id))
        
}

pub(crate) fn get_withdrawable(
    env: &Env,
    stream_id: u64,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
    
            // If the stream is completed or paused, withdrawals are not allowed.
            if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Paused {
                return Ok(0);
            }
    
            let accrued = crate::ops::views::calculate_accrued(env, stream_id)?;
            let mut withdrawable = accrued - stream.withdrawn_amount;
            withdrawable = apply_lookback_cap(
                env,
                &stream,
                env.ledger().timestamp(),
                accrued,
                withdrawable,
            );
    
            // Cap by contract balance for consistency with withdraw() (#39)
            let token_address = get_token(env)?;
            let contract_balance =
                token::Client::new(env, &token_address).balance(&env.current_contract_address());
            withdrawable = withdrawable.min(contract_balance);
    
            // Fallback max(0) just in case, though accrual is strictly monotonic
            Ok(if withdrawable > 0 { withdrawable } else { 0 })
        
}

pub(crate) fn get_claimable_at(
    env: &Env,
    stream_id: u64,
    timestamp: u64,
) -> Result<i128, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
    
            if stream.status == StreamStatus::Completed {
                return Ok(0);
            }
    
            let effective_time = match stream.status {
                StreamStatus::Cancelled => {
                    let at = stream.cancelled_at.ok_or(ContractError::InvalidState)?;
                    timestamp.min(at)
                }
                StreamStatus::Active | StreamStatus::Paused => timestamp,
                StreamStatus::Completed => unreachable!("returned above"),
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
                effective_time,
            );
    
            let claimable = accrued - stream.withdrawn_amount;
            let claimable = apply_lookback_cap(env, &stream, effective_time, accrued, claimable);
            Ok(if claimable > 0 { claimable } else { 0 })
        
}

pub(crate) fn get_config(
    env: &Env,
) -> Result<Config, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            crate::storage::get_config(env)
        
}

pub(crate) fn get_global_emergency_paused(
    env: &Env,
) -> bool {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            is_global_emergency_paused(env)
        
}

pub(crate) fn get_stream_state(
    env: &Env,
    stream_id: u64,
) -> Result<Stream, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_stream(env, stream_id)
        
}

pub(crate) fn get_cliff_status(
    env: &Env,
    stream_id: u64,
) -> Result<accrual::CliffStatus, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
    
            let now = if stream.status == StreamStatus::Cancelled {
                stream.cancelled_at.ok_or(ContractError::InvalidState)?
            } else {
                current_accrual_timestamp(env)?
            };
    
            Ok(accrual::cliff_status(now, stream.cliff_time))
        
}

pub(crate) fn get_paused_duration(
    env: &Env,
    stream_id: u64,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
            let total = if stream.status == StreamStatus::Paused {
                let current = env.ledger().timestamp().saturating_sub(stream.paused_at_timestamp);
                stream.cumulative_paused_duration.saturating_add(current)
            } else {
                stream.cumulative_paused_duration
            };
            Ok(total)
        
}

pub(crate) fn get_stream_health(
    env: &Env,
    stream_id: u64,
) -> Result<StreamHealth, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            bump_instance_ttl(env);
            let stream = load_stream(env, stream_id)?;
            let current_time = env.ledger().timestamp();
    
            let accrued_to_date_i128 = crate::ops::views::calculate_accrued(env, stream_id)?;
            let accrued_to_date = accrued_to_date_i128 as u128;
    
            let remaining_deposit = stream
                .deposit_amount
                .saturating_sub(stream.withdrawn_amount) as u128;
    
            let is_expired = current_time >= stream.end_time
                && stream.status != StreamStatus::Completed
                && stream.status != StreamStatus::Cancelled;
    
            // Underfunded check: will it run out before end_time?
            let duration = stream.end_time.saturating_sub(stream.checkpointed_at) as i128;
            let potential_additional = stream.rate_per_second.checked_mul(duration);
            let is_underfunded = match potential_additional {
                Some(added) => stream.checkpointed_amount.saturating_add(added) > stream.deposit_amount,
                None => true, // Overflow means it definitely exceeds deposit
            };
    
            // Seconds until depletion logic
            let mut seconds_until_depletion = None;
            if stream.rate_per_second > 0 {
                let total_to_accrue = stream
                    .deposit_amount
                    .saturating_sub(stream.checkpointed_amount);
                let seconds_to_deplete = (total_to_accrue / stream.rate_per_second) as u64;
                let depletion_time = stream.checkpointed_at.saturating_add(seconds_to_deplete);
    
                if depletion_time < stream.end_time {
                    seconds_until_depletion = Some(depletion_time.saturating_sub(current_time));
                } else {
                    seconds_until_depletion = Some(stream.end_time.saturating_sub(current_time));
                }
            } else if stream.checkpointed_amount >= stream.deposit_amount {
                seconds_until_depletion = Some(0);
            }
    
            Ok(StreamHealth {
                is_underfunded,
                is_expired,
                accrued_to_date,
                remaining_deposit,
                seconds_until_depletion,
            })
        
}

pub(crate) fn get_stream_memo(
    env: &Env,
    stream_id: u64,
) -> Result<Option<soroban_sdk::Bytes>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
            Ok(stream.memo)
        
}

pub(crate) fn get_stream_metadata(
    env: &Env,
    stream_id: u64,
) -> Result<Option<Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let stream = load_stream(env, stream_id)?;
            Ok(stream.metadata)
        
}

pub(crate) fn get_stream_count(
    env: &Env,
) -> u64 {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            read_stream_count(env)
        
}

pub(crate) fn get_protocol_fees_accrued(
    env: &Env,
) -> i128 {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            read_total_keeper_fees_paid(env)
        
}

pub(crate) fn get_total_liabilities(
    env: &Env,
) -> i128 {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            read_total_liabilities(env)
        
}

pub(crate) fn get_paused_stream_count(
    env: &Env,
) -> u64 {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            read_paused_stream_count(env)
        
}

pub(crate) fn get_recipient_streams(
    env: &Env,
    recipient: Address,
) -> soroban_sdk::Vec<u64> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let all = load_recipient_streams(env, &recipient);
            let cap = RECIPIENT_STREAMS_PAGE_LIMIT;
            if all.len() <= cap {
                return all;
            }
            // Cap applied before Vec is built to avoid materialising an oversized buffer.
            let mut out = soroban_sdk::Vec::new(env);
            for i in 0..cap {
                out.push_back(all.get(i).unwrap());
            }
            out
        
}

pub(crate) fn get_recipient_streams_paginated(
    env: &Env,
    recipient: Address,
    cursor: u64,
    limit: u32,
) -> Page {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let streams = load_recipient_streams(env, &recipient);
            let total = streams.len();
    
            // Apply limit cap
            let effective_limit = limit.min(RECIPIENT_STREAMS_PAGE_LIMIT);
    
            // Find starting position.
            //
            // `next_cursor` (below) is produced as the ID of the first
            // *not-yet-returned* item — i.e. "resume starting AT this ID",
            // inclusive. The lookup here must match that producer semantics:
            // when the cursor ID is found, start AT its position, not after it.
            // (Starting after it — `pos + 1` — silently dropped exactly one
            // stream per page boundary crossed; caught by
            // `test_get_recipient_streams_paginated_basic` and
            // `test_paginated_covers_all_streams`.)
            let start_idx = if cursor == 0 {
                0
            } else {
                match streams.binary_search(cursor) {
                    Ok(pos) => pos,  // Start at the cursor (inclusive)
                    Err(pos) => pos, // Insert position if not found
                }
            };
    
            // Calculate end position
            let end_idx = (start_idx as u32 + effective_limit).min(total as u32);
    
            let mut next_cursor = 0u64;
            if (end_idx as usize) < total as usize {
                next_cursor = streams.get(end_idx).unwrap();
            }
    
            let mut page_streams = soroban_sdk::Vec::new(env);
            for i in start_idx..end_idx {
                page_streams.push_back(streams.get(i).unwrap());
            }
    
            Page {
                stream_ids: page_streams,
                next_cursor,
            }
        
}

pub(crate) fn get_recipient_stream_count(
    env: &Env,
    recipient: Address,
) -> u64 {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_recipient_streams(env, &recipient).len() as u64
        
}

pub(crate) fn get_sender_portfolio_health(
    env: &Env,
    sender: Address,
    cursor: u64,
    limit: u32,
) -> PortfolioHealthPage {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            bump_instance_ttl(env);
    
            let streams = load_sender_streams(env, &sender);
            let total = streams.len();
    
            // Clamp limit to MAX_PAGE_SIZE; treat 0 as "use the maximum".
            let effective_limit = if limit == 0 {
                RECIPIENT_STREAMS_PAGE_LIMIT
            } else {
                limit.min(RECIPIENT_STREAMS_PAGE_LIMIT)
            };
    
            // Find starting position (inclusive cursor semantics — mirrors
            // get_recipient_streams_paginated).
            let start_idx = if cursor == 0 {
                0u32
            } else {
                match streams.binary_search(cursor) {
                    Ok(pos) => pos,  // start AT the cursor stream (inclusive)
                    Err(pos) => pos, // gap: start at the next higher stream
                }
            };
    
            let end_idx = (start_idx.saturating_add(effective_limit)).min(total as u32);
    
            let mut next_cursor = 0u64;
            if (end_idx as usize) < total as usize {
                next_cursor = streams.get(end_idx).unwrap();
            }
    
            let now = env.ledger().timestamp();
            let mut underfunded_count: u32 = 0;
            let mut expired_count: u32 = 0;
            let mut healthy_count: u32 = 0;
            let mut page_stream_ids = soroban_sdk::Vec::new(env);
    
            for i in start_idx..end_idx {
                let stream_id = streams.get(i).unwrap();
                page_stream_ids.push_back(stream_id);
    
                // Load the stream. If it was removed between index write and query
                // (e.g. by a concurrent close), skip it gracefully.
                let stream = match load_stream(env, stream_id) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
    
                // Terminal streams don't count toward any health bucket.
                if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled
                {
                    continue;
                }
    
                // Expired: past end_time but not yet closed.
                let is_expired = now >= stream.end_time;
    
                // Underfunded: deposit cannot cover obligations at current rate.
                let (is_underfunded, _, _) = compute_stream_health(&stream, now);
    
                // Expiry takes priority: a stream that has elapsed cannot be topped up.
                if is_expired {
                    expired_count = expired_count.saturating_add(1);
                } else if is_underfunded {
                    underfunded_count = underfunded_count.saturating_add(1);
                } else {
                    healthy_count = healthy_count.saturating_add(1);
                }
            }
    
            PortfolioHealthPage {
                underfunded_count,
                expired_count,
                healthy_count,
                next_cursor,
                stream_ids: page_stream_ids,
            }
        
}

pub(crate) fn get_streams_by_id_range(
    env: &Env,
    start_id: u64,
    end_id: u64,
    limit: u64,
) -> soroban_sdk::Vec<Stream> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            // Enforce DoS protection limit
            let page_size = limit.min(MAX_PAGE_SIZE);
            let mut result = soroban_sdk::Vec::new(env);
    
            // Handle invalid range
            if start_id > end_id || page_size == 0 {
                return result;
            }
    
            let total_count = read_stream_count(env);
            let effective_end = end_id.min(total_count);
    
            let mut current_id = start_id;
            while current_id <= effective_end && result.len() < page_size as u32 {
                // Try to load stream, skip if not found (closed/archived)
                if let Ok(stream) = load_stream(env, current_id) {
                    result.push_back(stream);
                }
                current_id += 1;
            }
    
            result
        
}

pub(crate) fn is_paused(
    env: &Env,
) -> bool {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            is_protocol_paused(env)
        
}

pub(crate) fn get_pause_info(
    env: &Env,
) -> PauseInfo {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let is_paused = is_protocol_paused(env);
            if is_paused {
                PauseInfo {
                    is_paused: true,
                    reason: get_pause_reason(env),
                    paused_at: get_pause_timestamp(env),
                    paused_by: get_pause_admin(env),
                }
            } else {
                PauseInfo {
                    is_paused: false,
                    reason: None,
                    paused_at: None,
                    paused_by: None,
                }
            }
        
}

