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

pub(crate) fn clone_stream(
    env: &Env,
    stream_id: u64,
    new_recipient: Address,
    start_time: u64,
    end_time: u64,
    deposit: i128,
    force: bool,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            // ── 1. Pause guard ────────────────────────────────────────────────────
            require_not_creation_paused(env)?;
    
            // ── 2. Load source stream ─────────────────────────────────────────────
            let source = load_stream(env, stream_id)?;
    
            if source.decommissioned.unwrap_or(false) {
                return Err(ContractError::InvalidState);
            }
    
            // ── 2.1. Status guard ─────────────────────────────────────────────────
            // Reject cloning from a terminal-state source (Cancelled or Completed).
            if source.status == StreamStatus::Cancelled || source.status == StreamStatus::Completed {
                return Err(ContractError::StreamTerminalState);
            }
    
            // ── 3. Authorization: source sender ──────────────────────────────────
            // Only the source stream's original sender may clone it.
            // The contract admin can clone streams they created (admin == sender).
            // For streams created by others, the admin must coordinate with the sender
            // out-of-band; there is no admin-override path for clone_stream to prevent
            // privilege escalation (an admin should not be able to spend a sender's tokens).
            source.sender.require_auth();
    
            // ── 4. CliffOnly guard ────────────────────────────────────────────────
            // Streams with withdraw_dust_threshold == i128::MAX are treated as
            // "CliffOnly" sentinel streams. Cloning them without explicit opt-in
            // would silently propagate a degenerate configuration.
            if source.withdraw_dust_threshold == i128::MAX && !force {
                return Err(ContractError::InvalidParams);
            }
    
            // ── 5. Compute inherited cliff offset ─────────────────────────────────
            // Preserve the relative cliff position: cliff_offset = source.cliff_time - source.start_time.
            // Apply it to the new start_time.
            let cliff_offset = source.cliff_time.saturating_sub(source.start_time); // if cliff < start (degenerate), treat as no cliff
            let new_cliff_time = start_time
                .checked_add(cliff_offset)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            // ── 6. Validate new stream parameters ────────────────────────────────
            let current_time = env.ledger().timestamp();
            ops::validation::validate_stream_params(
                env,
                &source.sender,
                &new_recipient,
                deposit,
                source.rate_per_second,
                current_time,
                start_time,
                new_cliff_time,
                end_time,
                source.kind,
            )?;
    
            // ── 7. Pull deposit tokens from sender ────────────────────────────────
            pull_token(env, &source.sender, deposit)?;
    
            // ── 8. Persist the new stream ─────────────────────────────────────────
            let new_stream_id = ops::validation::persist_new_stream(
                env,
                source.sender.clone(),
                new_recipient.clone(),
                deposit,
                source.rate_per_second,
                start_time,
                new_cliff_time,
                end_time,
                source.withdraw_dust_threshold,
                source.memo.clone(),
                source.kind,
                None, // Clone resets metadata to prevent single-use ID duplication
                source.irrevocable,
                source.witness.clone(),
            )?;
    
            // ── 9. Emit clone-specific event for indexer correlation ──────────────
            events::emit_stream_cloned(
                env,
                new_stream_id,
                StreamCloned {
                    new_stream_id,
                    source_stream_id: stream_id,
                    sender: source.sender.clone(),
                    recipient: new_recipient,
                    deposit_amount: deposit,
                    rate_per_second: source.rate_per_second,
                    start_time,
                    cliff_time: new_cliff_time,
                    end_time,
                    withdraw_dust_threshold: source.withdraw_dust_threshold,
                },
            );
    
            Ok(new_stream_id)
        
}

pub(crate) fn is_valid_destination(
    env: &Env,
    destination: &Address,
) -> bool {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            if destination == &env.current_contract_address() {
                return false;
            }
            true
        
}

