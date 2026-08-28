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
use crate::{MIN_PAUSE_INTERVAL_LEDGERS};

pub(crate) fn pause_stream(
    env: &Env,
    stream_id: u64,
    reason: PauseReason,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let mut stream = load_stream(env, stream_id)?;
    
            crate::ops::validation::require_stream_sender(&stream.sender);
    
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

pub(crate) fn resume_stream(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let mut stream = load_stream(env, stream_id)?;
            crate::ops::validation::require_stream_sender(&stream.sender);
    
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

