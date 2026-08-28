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

pub(crate) fn cancel_stream(
    env: &Env,
    stream_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            let mut stream = load_stream(env, stream_id)?;
            crate::ops::validation::require_stream_sender(&stream.sender);
            crate::ops::cancel::cancel_stream_internal(env, &mut stream)
        
}

pub(crate) fn require_cancellable_status(
    status: StreamStatus,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            if status != StreamStatus::Active && status != StreamStatus::Paused {
                return Err(ContractError::InvalidState);
            }
            Ok(())
        
}

pub(crate) fn cancel_stream_internal(
    env: &Env,
    stream: &mut Stream,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            if stream.irrevocable.unwrap_or(false) {
                return Err(ContractError::Unauthorized);
            }
            crate::ops::cancel::require_cancellable_status(stream.status)?;
    
            let now = current_accrual_timestamp(env)?;
            // Use checkpoint-aware accrual so rate-decreased streams are cancelled correctly.
            let accrued_at_cancel = accrual::calculate_accrued_amount_checkpointed(
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
    
            let refund_amount = stream
                .deposit_amount
                .checked_sub(accrued_at_cancel)
                .ok_or(ContractError::InvalidState)?;
    
            // CEI: persist terminal state before external token transfer.
            let previous_status = stream.status;
            stream.status = StreamStatus::Cancelled;
            stream.cancelled_at = Some(now);
            save_stream(env, stream);
            reconcile_paused_stream_count(env, previous_status, stream.status);
    
            // Reduce liabilities by the refunded (unstreamed) portion.
            // The accrued portion remains a liability until the recipient withdraws.
            if refund_amount > 0 {
                let liabilities = read_total_liabilities(env)
                    .checked_sub(refund_amount)
                    .unwrap_or(0);
                write_total_liabilities(env, liabilities);
    
                // Reentrancy guard around the external token transfer, mirroring
                // `withdraw`/`delegated_withdraw`. Terminal state is already persisted
                // above (CEI), so a malicious token re-entering any cancel or
                // withdraw path during this transfer hits the held lock and reverts.
                // Capture the result, always release, then propagate so the lock is
                // never left stuck on a failed transfer.
                acquire_reentrancy_lock(env)?;
                let transfer_result = push_token(env, &stream.sender, refund_amount);
                release_reentrancy_lock(env);
                transfer_result?;
            }
    
            events::emit_stream_cancelled(env, stream.stream_id);
    
            Ok(())
        
}

pub(crate) fn bulk_cancel_streams(
    env: &Env,
    sender: Address,
    stream_ids: soroban_sdk::Vec<u64>,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            require_not_globally_paused(env)?;
            sender.require_auth();
    
            let n = stream_ids.len();
            if n == 0 {
                return Ok(());
            }
    
            // --- Batch validation: reject duplicate stream IDs (O(n)) ---
            reject_duplicate_ids(env, &stream_ids)?;
    
            // ── Phase 1: Validate all stream IDs and ownership ────────────────────
            let mut streams = soroban_sdk::Vec::<Stream>::new(env);
    
            for i in 0..n {
                let id = stream_ids.get(i).unwrap();
    
                // Duplicate detection removed - now handled by reject_duplicate_ids
    
                let stream = load_stream(env, id)?;
    
                if stream.sender != sender {
                    return Err(ContractError::Unauthorized);
                }
    
                if stream.irrevocable.unwrap_or(false) {
                    return Err(ContractError::Unauthorized);
                }
    
                crate::ops::cancel::require_cancellable_status(stream.status)?;
    
                streams.push_back(stream);
            }
    
            // ── Phase 2: Execute cancellations ────────────────────────────────────
            let now = env.ledger().timestamp();
            let mut aggregate_refund: i128 = 0;
            let mut total_liabilities = read_total_liabilities(env);
    
            for i in 0..n {
                let mut stream = streams.get(i).unwrap();
                let stream_id = stream.stream_id;
    
                let accrued_at_cancel = accrual::calculate_accrued_amount_checkpointed(
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
    
                let refund_amount = stream
                    .deposit_amount
                    .checked_sub(accrued_at_cancel)
                    .ok_or(ContractError::InvalidState)?;
    
                let (was_underfunded, _, _) = compute_stream_health(&stream, now);
    
                // ── Pay recipient their accrued entitlement first ─────────────────
                let recipient_accrual = accrued_at_cancel
                    .saturating_sub(stream.withdrawn_amount)
                    .max(0);
                if recipient_accrual > 0 {
                    stream.withdrawn_amount = stream
                        .withdrawn_amount
                        .checked_add(recipient_accrual)
                        .unwrap_or(i128::MAX);
    
                    total_liabilities = total_liabilities
                        .checked_sub(recipient_accrual)
                        .unwrap_or(0);
    
                    push_token(env, &stream.recipient, recipient_accrual)?;
    
                    events::emit_withdrawal(
                        env,
                        stream_id,
                        Withdrawal {
                            stream_id,
                            recipient: stream.recipient.clone(),
                            amount: recipient_accrual,
                        },
                    );
                }
    
                // ── Mark stream as cancelled ──────────────────────────────────────
                let previous_status = stream.status;
                stream.status = StreamStatus::Cancelled;
                stream.cancelled_at = Some(now);
                save_stream(env, &stream);
                reconcile_paused_stream_count(env, previous_status, stream.status);
    
                // ── Accumulate sender refund ──────────────────────────────────────
                if refund_amount > 0 {
                    aggregate_refund = aggregate_refund
                        .checked_add(refund_amount)
                        .ok_or(ContractError::ArithmeticOverflow)?;
    
                    total_liabilities = total_liabilities.checked_sub(refund_amount).unwrap_or(0);
                }
    
                events::emit_stream_cancelled(env, stream_id);
    
                maybe_emit_health_changed(env, &stream, was_underfunded, now);
            }
    
            // ── Single aggregate refund to sender ─────────────────────────────────
            write_total_liabilities(env, total_liabilities);
    
            if aggregate_refund > 0 {
                push_token(env, &sender, aggregate_refund)?;
            }
    
            Ok(())
        
}

