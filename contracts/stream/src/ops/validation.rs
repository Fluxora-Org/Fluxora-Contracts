//! Internal validation and persistence helpers for stream creation.
//!
//! Moved out of `lib.rs` (issue #1520, slice 1). These were private
//! associated functions on `FluxoraStream`; they are now `pub(crate)` free
//! functions so they can be called from `ops::create` and any other `ops`
//! module that needs them, without being part of the contract's public ABI.
//!
//! Move-only: bodies are byte-for-byte identical to the originals except
//! `Self::foo(...)` call sites (in callers) become `validation::foo(...)`.

use soroban_sdk::xdr::ToXdr;
use soroban_sdk::{Address, Env, Map};

use crate::events;
use crate::storage::{
    add_stream_to_recipient_index, add_stream_to_sender_index, get_max_rate_per_second,
    next_stream_id_for, read_total_liabilities, save_stream, validate_metadata,
    write_total_liabilities,
};
use crate::{ContractError, Stream, StreamCreated, StreamKind, StreamStatus, MAX_MEMO_BYTES};

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_stream_params(
    env: &Env,
    sender: &Address,
    recipient: &Address,
    deposit_amount: i128,
    rate_per_second: i128,
    current_ledger_timestamp: u64,
    start_time: u64,
    cliff_time: u64,
    end_time: u64,
    kind: StreamKind,
) -> Result<(), ContractError> {
    validate_stream_params_with_self_policy(
        env,
        sender,
        recipient,
        deposit_amount,
        rate_per_second,
        current_ledger_timestamp,
        start_time,
        cliff_time,
        end_time,
        kind,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_stream_params_with_self_policy(
    env: &Env,
    sender: &Address,
    recipient: &Address,
    deposit_amount: i128,
    rate_per_second: i128,
    current_ledger_timestamp: u64,
    start_time: u64,
    cliff_time: u64,
    end_time: u64,
    kind: StreamKind,
    allow_self_recipient: bool,
) -> Result<(), ContractError> {
    // Validate positive amounts (#35)
    if deposit_amount <= 0 {
        return Err(ContractError::InvalidParams);
    }

    match kind {
        StreamKind::Linear | StreamKind::CliffSlope => {
            if rate_per_second <= 0 {
                return Err(ContractError::InvalidParams);
            }

            // Enforce governance-controlled maximum rate per second cap.
            let max_rate = get_max_rate_per_second(env);
            if rate_per_second > max_rate {
                return Err(ContractError::InvalidParams);
            }
        }
        StreamKind::CliffOnly => {
            if rate_per_second != 0 {
                return Err(ContractError::InvalidParams);
            }
        }
    }

    // Validate sender != recipient (#35). Pooled streams intentionally use
    // the sender as the aggregate stream recipient while member shares live
    // in `DataKey::PooledStreamShares`.
    if !allow_self_recipient && sender == recipient {
        return Err(ContractError::InvalidParams);
    }

    // Validate time constraints
    if start_time >= end_time {
        return Err(ContractError::InvalidParams);
    }
    if start_time < current_ledger_timestamp {
        return Err(ContractError::StartTimeInPast);
    }
    if cliff_time < start_time || cliff_time > end_time {
        return Err(ContractError::InvalidParams);
    }

    match kind {
        StreamKind::Linear => {
            // Validate deposit covers the full streamable amount from start to end.
            let duration = (end_time - start_time) as i128;
            let total_streamable = rate_per_second
                .checked_mul(duration)
                .ok_or(ContractError::InvalidParams)?;

            if deposit_amount < total_streamable {
                return Err(ContractError::InsufficientDeposit);
            }
        }
        StreamKind::CliffSlope => {
            // CliffSlope accrues only after the cliff, so the deposit must cover the
            // post-cliff portion of the schedule.
            let post_cliff_duration = (end_time.saturating_sub(cliff_time)) as i128;
            let post_cliff_streamable = rate_per_second
                .checked_mul(post_cliff_duration)
                .ok_or(ContractError::InvalidParams)?;

            if deposit_amount < post_cliff_streamable {
                return Err(ContractError::InsufficientDeposit);
            }
        }
        StreamKind::CliffOnly => {}
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn persist_new_stream(
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
) -> Result<u64, ContractError> {
    // Validate memo length before allocating a stream ID.
    if let Some(ref m) = memo {
        if m.len() as usize > MAX_MEMO_BYTES {
            return Err(ContractError::InvalidParams);
        }
    }

    // Validate metadata if present (fail-before-allocate).
    if let Some(ref meta) = metadata {
        validate_metadata(meta)?;
    }
    // Validate metadata size bounds before allocating a stream ID.
    if let Some(ref md) = metadata {
        validate_metadata(md)?;
    }

    let stream_id = next_stream_id_for(env, &sender);

    let stream = Stream {
        stream_id,
        sender: sender.clone(),
        recipient: recipient.clone(),
        claim_owner: None,
        deposit_amount,
        rate_per_second,
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
        metadata: metadata.clone(),
        last_rate_change_ledger: 0,
        is_pooled: None,
        irrevocable,
        witness,
        delegation_depth: 0,
        parent_stream_id: None,
        decommissioned: None,
        paused_at_timestamp: 0,
        cumulative_paused_duration: 0,
    };

    save_stream(env, &stream);

    // Add stream to recipient's index (maintains sorted order by stream_id)
    add_stream_to_recipient_index(env, &recipient, stream_id, Some(end_time));
    // Add stream to sender's portfolio index.
    add_stream_to_sender_index(env, &sender, stream_id, Some(end_time));

    // Track liability: the full deposit is owed to the recipient until withdrawn/refunded.
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
            recipient,
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            withdraw_dust_threshold,
            memo,
            metadata: stream.metadata,
        },
    );

    Ok(stream_id)
}

/// Like `persist_new_stream` but skips the per-call recipient index update.
///
/// Used by `create_streams` to batch index writes: the caller collects all
/// (recipient → stream_ids) pairs and flushes them once per unique recipient,
/// reducing ledger I/O from O(n) to O(1) per recipient.
#[allow(clippy::too_many_arguments)]
pub(crate) fn persist_new_stream_skip_index(
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
) -> Result<u64, ContractError> {
    if let Some(ref m) = memo {
        if m.len() as usize > MAX_MEMO_BYTES {
            return Err(ContractError::InvalidParams);
        }
    }

    // Validate metadata bounds before allocating a stream ID.
    if let Some(ref meta) = metadata {
        validate_metadata(meta)?;
    }

    let stream_id = next_stream_id_for(env, &sender);

    let stream = Stream {
        stream_id,
        sender: sender.clone(),
        recipient: recipient.clone(),
        claim_owner: None,
        deposit_amount,
        rate_per_second,
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
        metadata: metadata.clone(),
        last_rate_change_ledger: 0,
        is_pooled: None,
        irrevocable,
        witness,
        delegation_depth: 0,
        parent_stream_id: None,
        decommissioned: None,
        paused_at_timestamp: 0,
        cumulative_paused_duration: 0,
    };

    save_stream(env, &stream);

    // Index update is intentionally skipped here; caller must flush the cache.

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
            recipient,
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            withdraw_dust_threshold,
            memo,
            metadata: stream.metadata,
        },
    );

    Ok(stream_id)
}

/// Extract the ed25519 public key bytes from an `Address` that is known
/// to be an account-type address (G... strkey).
///
/// Uses `Address::to_xdr` which serializes via the host function
/// `serialize_to_bytes`. The XDR encoding of an account `Address` is:
///   - bytes  0..3: ScVal tag (Address = 18, big-endian u32)
///   - bytes  4..7: ScAddress tag (Account = 0, big-endian u32)
///   - bytes  8..11: PublicKey tag (PublicKeyTypeEd25519 = 0, big-endian u32)
///   - bytes 12..43: ed25519 public key (32 bytes)
pub(crate) fn ed25519_pubkey_from_address(env: &Env, addr: &Address) -> [u8; 32] {
    let xdr = addr.to_xdr(env);
    let pk_bytes = xdr.slice(12..44);
    let mut pk = [0u8; 32];
    pk_bytes.copy_into_slice(&mut pk);
    pk
}
