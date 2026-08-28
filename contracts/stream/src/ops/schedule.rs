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

pub(crate) fn create_stream_relative(
    env: &Env,
    sender: Address,
    params: CreateStreamRelativeParams,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            crate::ops::schedule::create_stream_relative_inner(env, sender, params)
        
}

pub(crate) fn create_stream_relative_inner(
    env: &Env,
    sender: Address,
    params: CreateStreamRelativeParams,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let current_time = env.ledger().timestamp();
    
            // Compute absolute times with overflow checks
            let start_time = current_time
                .checked_add(params.start_delay)
                .ok_or(ContractError::InvalidParams)?;
            let cliff_time = current_time
                .checked_add(params.cliff_delay)
                .ok_or(ContractError::InvalidParams)?;
            let end_time = start_time
                .checked_add(params.duration)
                .ok_or(ContractError::InvalidParams)?;
    
            // Delegate to the standard creation path so auth, pause checks,
            // validation, token transfer, and persistence remain identical.
            crate::ops::create::create_stream_internal(
                env,
                sender,
                params.recipient,
                params.deposit_amount,
                params.rate_per_second,
                start_time,
                cliff_time,
                end_time,
                params.withdraw_dust_threshold.unwrap_or(0),
                params.memo,
                params.kind,
                params.metadata,
                params.irrevocable,
                params.witness,
                None, // max_lookback_ledgers
            )
        
}

pub(crate) fn create_streams_relative(
    env: &Env,
    sender: Address,
    streams_relative: soroban_sdk::Vec<CreateStreamRelativeParams>,
) -> Result<soroban_sdk::Vec<u64>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            if streams_relative.is_empty() {
                return Ok(soroban_sdk::Vec::new(env));
            }
    
            let current_time = env.ledger().timestamp();
            let mut absolute_streams = soroban_sdk::Vec::new(env);
    
            // Convert relative parameters to absolute times
            for rel in streams_relative.iter() {
                let start_time = current_time
                    .checked_add(rel.start_delay)
                    .ok_or(ContractError::InvalidParams)?;
                let cliff_time = current_time
                    .checked_add(rel.cliff_delay)
                    .ok_or(ContractError::InvalidParams)?;
                let end_time = start_time
                    .checked_add(rel.duration)
                    .ok_or(ContractError::InvalidParams)?;
    
                let mut final_rate = rel.rate_per_second;
                if rel.kind == StreamKind::CliffOnly {
                    final_rate = 0;
                }
    
                absolute_streams.push_back(CreateStreamParams {
                    recipient: rel.recipient,
                    deposit_amount: rel.deposit_amount,
                    rate_per_second: final_rate,
                    start_time,
                    cliff_time,
                    end_time,
                    withdraw_dust_threshold: rel.withdraw_dust_threshold,
                    memo: rel.memo,
                    kind: rel.kind,
                    metadata: rel.metadata,
                    irrevocable: rel.irrevocable,
                    witness: rel.witness,
                });
            }
    
            // Delegate to standard create_streams with converted absolute times
            crate::ops::create::create_streams(env, sender, absolute_streams)
        
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_stream_from_template(
    env: &Env,
    sender: Address,
    template_id: u64,
    recipient: Address,
    deposit_amount: i128,
    rate_per_second: i128,
    withdraw_dust_threshold: i128,
    memo: Option<soroban_sdk::Bytes>,
    metadata: Option<Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>,
    kind: StreamKind,
    irrevocable: Option<bool>,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let tpl = load_stream_template(env, template_id)?;
            crate::ops::schedule::create_stream_relative(
                env,
                sender,
                CreateStreamRelativeParams {
                    recipient,
                    deposit_amount,
                    rate_per_second,
                    start_delay: tpl.start_delay,
                    cliff_delay: tpl.cliff_delay,
                    duration: tpl.duration,
                    withdraw_dust_threshold: Some(withdraw_dust_threshold),
                    memo,
                    kind,
                    metadata,
                    irrevocable,
                    witness: None,
                },
            )
        
}

