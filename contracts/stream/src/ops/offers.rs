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

pub(crate) fn create_stream_offer(
    env: &Env,
    sender: Address,
    params: CreateStreamParams,
    expiry_time: Option<u64>,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            sender.require_auth();
            require_not_creation_paused(env)?;
            let now = env.ledger().timestamp();
    
            // Validate expiry is in the future if provided.
            if let Some(expiry) = expiry_time {
                if expiry <= now {
                    return Err(ContractError::InvalidParams);
                }
            }
    
            // For CliffOnly, rate must be 0.
            let final_rate = if params.kind == StreamKind::CliffOnly {
                0
            } else {
                params.rate_per_second
            };
    
            // Reuse stream parameter validation (same rules as create_stream).
            ops::validation::validate_stream_params(
                env,
                &sender,
                &params.recipient,
                params.deposit_amount,
                final_rate,
                now,
                params.start_time,
                params.cliff_time,
                params.end_time,
                params.kind,
            )?;
    
            // Validate memo length.
            if let Some(ref m) = params.memo {
                if m.len() as usize > MAX_MEMO_BYTES {
                    return Err(ContractError::InvalidParams);
                }
            }
    
            let withdraw_dust_threshold = params.withdraw_dust_threshold.unwrap_or(0);
            // Validate dust threshold.
            if withdraw_dust_threshold < 0 {
                return Err(ContractError::InvalidDustThreshold);
            }
            if withdraw_dust_threshold > params.deposit_amount {
                return Err(ContractError::InvalidDustThreshold);
            }
    
            // Validate metadata if present.
            if let Some(ref meta) = params.metadata {
                storage::validate_metadata(meta)?;
            }
    
            // ── CEI: state changes before token transfer ──────────────────────────
    
            // Allocate offer ID from the global stream counter.
            let offer_id = next_stream_id_for(env, &sender);
    
            let offer = StreamOffer {
                offer_id,
                sender: sender.clone(),
                recipient: params.recipient.clone(),
                deposit_amount: params.deposit_amount,
                rate_per_second: final_rate,
                start_time: params.start_time,
                cliff_time: params.cliff_time,
                end_time: params.end_time,
                withdraw_dust_threshold,
                memo: params.memo.clone(),
                kind: params.kind,
                metadata: params.metadata.clone(),
                expiry_time,
                created_at: now,
            };
    
            // Persist offer and update recipient index BEFORE pulling tokens.
            save_stream_offer(env, &offer);
            add_offer_to_recipient_pending(env, &params.recipient, offer_id);
    
            // ── CEI: token transfer ───────────────────────────────────────────────
            pull_token(env, &sender, params.deposit_amount)?;
    
            // ── Emit event ────────────────────────────────────────────────────────
            env.events().publish(
                (symbol_short!("offr_crt"), offer_id),
                StreamOfferCreated {
                    offer_id,
                    sender,
                    recipient: params.recipient,
                    deposit_amount: params.deposit_amount,
                    rate_per_second: final_rate,
                    start_time: params.start_time,
                    cliff_time: params.cliff_time,
                    end_time: params.end_time,
                    expiry_time,
                    created_at: now,
                },
            );
    
            Ok(offer_id)
        
}

pub(crate) fn accept_stream_offer(
    env: &Env,
    recipient: Address,
    offer_id: u64,
) -> Result<u64, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            recipient.require_auth();
            require_not_globally_paused(env)?;
    
            let now = env.ledger().timestamp();
    
            // ── Validate ──────────────────────────────────────────────────────────
            let offer = load_stream_offer(env, offer_id)?;
    
            if offer.recipient != recipient {
                return Err(ContractError::OfferWrongRecipient);
            }
    
            // Check expiry.
            if let Some(expiry) = offer.expiry_time {
                if now > expiry {
                    return Err(ContractError::OfferExpired);
                }
            }
    
            // Re-anchor start_time so the stream never starts in the past.
            let effective_start = offer.start_time.max(now);
    
            // Preserve cliff offset: cliff_offset = cliff_time - start_time.
            let cliff_offset = offer.cliff_time.saturating_sub(offer.start_time);
            let effective_cliff = effective_start
                .checked_add(cliff_offset)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            // Preserve duration: duration = end_time - start_time.
            let duration = offer.end_time.saturating_sub(offer.start_time);
            let effective_end = effective_start
                .checked_add(duration)
                .ok_or(ContractError::ArithmeticOverflow)?;
    
            // Re-validate the adjusted schedule (cliff and end must still be consistent).
            if effective_start >= effective_end {
                return Err(ContractError::InvalidParams);
            }
            if effective_cliff > effective_end {
                return Err(ContractError::InvalidParams);
            }
    
            // For Linear streams: re-validate deposit covers the preserved duration.
            if offer.kind == StreamKind::Linear {
                let total_streamable = offer
                    .rate_per_second
                    .checked_mul(duration as i128)
                    .ok_or(ContractError::ArithmeticOverflow)?;
                if offer.deposit_amount < total_streamable {
                    return Err(ContractError::InsufficientDeposit);
                }
            }
    
            // ── CEI: state changes (remove offer, create stream, update indices) ──
    
            // Remove offer from storage and recipient's pending index FIRST.
            remove_stream_offer(env, offer_id);
            remove_offer_from_recipient_pending(env, &offer.recipient, offer_id);
    
            // Construct and persist the Active stream (reusing the pre-allocated ID).
            let stream = Stream {
                stream_id: offer_id,
                sender: offer.sender.clone(),
                recipient: offer.recipient.clone(),
                claim_owner: None,
                deposit_amount: offer.deposit_amount,
                rate_per_second: offer.rate_per_second,
                start_time: effective_start,
                cliff_time: effective_cliff,
                end_time: effective_end,
                withdrawn_amount: 0,
                status: StreamStatus::Active,
                cancelled_at: None,
                checkpointed_amount: 0,
                checkpointed_at: effective_start,
                withdraw_dust_threshold: offer.withdraw_dust_threshold,
                memo: offer.memo.clone(),
                kind: offer.kind,
                last_pause_toggle_ledger: 0,
                last_withdraw_ledger: 0,
                metadata: offer.metadata.clone(),
                witness: None,
                is_pooled: None,
                last_rate_change_ledger: 0,
                delegation_depth: 0,
                parent_stream_id: None,
                irrevocable: None,
                decommissioned: None,
                paused_at_timestamp: 0,
                cumulative_paused_duration: 0,
            };
    
            save_stream(env, &stream);
    
            // Add to RecipientStreams index (the offer was intentionally excluded from it).
            add_stream_to_recipient_index(env, &offer.recipient, offer_id, Some(effective_end));
    
            // Track liability: the full deposit is now owed to the recipient.
            let liabilities = read_total_liabilities(env)
                .checked_add(offer.deposit_amount)
                .unwrap_or(i128::MAX);
            write_total_liabilities(env, liabilities);
    
            // ── Emit events ───────────────────────────────────────────────────────
            events::emit_stream_created(
                env,
                offer_id,
                StreamCreated {
                    stream_id: offer_id,
                    sender: offer.sender.clone(),
                    recipient: offer.recipient.clone(),
                    deposit_amount: offer.deposit_amount,
                    rate_per_second: offer.rate_per_second,
                    start_time: effective_start,
                    cliff_time: effective_cliff,
                    end_time: effective_end,
                    withdraw_dust_threshold: offer.withdraw_dust_threshold,
                    memo: offer.memo,
                    metadata: offer.metadata,
                },
            );
    
            env.events().publish(
                (symbol_short!("offr_acc"), offer_id),
                StreamOfferAccepted {
                    offer_id,
                    effective_start_time: effective_start,
                    recipient: offer.recipient,
                },
            );
    
            Ok(offer_id)
        
}

pub(crate) fn reject_stream_offer(
    env: &Env,
    recipient: Address,
    offer_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            recipient.require_auth();
    
            let offer = load_stream_offer(env, offer_id)?;
    
            if offer.recipient != recipient {
                return Err(ContractError::OfferWrongRecipient);
            }
    
            let sender = offer.sender.clone();
            let deposit = offer.deposit_amount;
    
            // ── CEI: remove state before token transfer ───────────────────────────
            remove_stream_offer(env, offer_id);
            remove_offer_from_recipient_pending(env, &offer.recipient, offer_id);
    
            // ── CEI: token transfer ───────────────────────────────────────────────
            push_token(env, &sender, deposit)?;
    
            env.events().publish(
                (symbol_short!("offr_cxl"), offer_id),
                StreamOfferCancelled {
                    offer_id,
                    by: recipient,
                    refund_amount: deposit,
                },
            );
    
            Ok(())
        
}

pub(crate) fn cancel_stream_offer(
    env: &Env,
    sender: Address,
    offer_id: u64,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            sender.require_auth();
    
            let offer = load_stream_offer(env, offer_id)?;
    
            if offer.sender != sender {
                return Err(ContractError::OfferWrongSender);
            }
    
            let deposit = offer.deposit_amount;
            let recipient = offer.recipient.clone();
    
            // ── CEI: remove state before token transfer ───────────────────────────
            remove_stream_offer(env, offer_id);
            remove_offer_from_recipient_pending(env, &recipient, offer_id);
    
            // ── CEI: token transfer ───────────────────────────────────────────────
            push_token(env, &sender, deposit)?;
    
            env.events().publish(
                (symbol_short!("offr_cxl"), offer_id),
                StreamOfferCancelled {
                    offer_id,
                    by: sender,
                    refund_amount: deposit,
                },
            );
    
            Ok(())
        
}

pub(crate) fn get_stream_offer(
    env: &Env,
    offer_id: u64,
) -> Result<StreamOffer, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_stream_offer(env, offer_id)
        
}

pub(crate) fn get_recipient_pending_offers(
    env: &Env,
    recipient: Address,
) -> soroban_sdk::Vec<u64> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_recipient_pending_offers(env, &recipient)
        
}

