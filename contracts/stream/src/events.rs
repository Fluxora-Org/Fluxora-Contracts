//! Event emission helpers for the Fluxora stream contract.
//!
//! Each function wraps a single `env.events().publish(...)` call, keeping
//! the `symbol_short!` topic definitions co-located with the payload struct.
//! This makes ABI review trivial: every event topic is in one file.

use crate::types::*;
use soroban_sdk::{symbol_short, Env};

/// Emit the `created` event when a new stream is persisted.
pub(crate) fn emit_stream_created(env: &Env, stream_id: u64, payload: StreamCreated) {
    env.events()
        .publish((symbol_short!("created"), stream_id), payload);
}

/// Emit the `withdrew` event when a token withdrawal is processed.
pub(crate) fn emit_withdrawal(env: &Env, stream_id: u64, payload: Withdrawal) {
    env.events()
        .publish((symbol_short!("withdrew"), stream_id), payload);
}

/// Emit the `withdrew` event for a `withdraw_to` destination.
pub(crate) fn emit_withdrawal_to(env: &Env, stream_id: u64, payload: WithdrawalTo) {
    env.events()
        .publish((symbol_short!("withdrew"), stream_id), payload);
}

/// Emit the `cancelled` event when a stream is cancelled.
pub(crate) fn emit_stream_cancelled(env: &Env, stream_id: u64) {
    env.events().publish(
        (symbol_short!("cancelled"), stream_id),
        StreamEvent::StreamCancelled(stream_id),
    );
}

/// Emit the `completed` event when a stream reaches terminal Completed state.
pub(crate) fn emit_stream_completed(env: &Env, stream_id: u64) {
    env.events().publish(
        (symbol_short!("complete"), stream_id),
        StreamEvent::StreamCompleted(stream_id),
    );
}

/// Emit the `closed` event when storage is reclaimed.
pub(crate) fn emit_stream_closed(env: &Env, stream_id: u64) {
    env.events().publish(
        (symbol_short!("closed"), stream_id),
        StreamEvent::StreamClosed(stream_id),
    );
}

/// Emit the `paused` event.
pub(crate) fn emit_stream_paused(env: &Env, stream_id: u64, payload: StreamPaused) {
    env.events()
        .publish((symbol_short!("paused"), stream_id), payload);
}

/// Emit the `resumed` event.
pub(crate) fn emit_stream_resumed(env: &Env, stream_id: u64) {
    env.events().publish(
        (symbol_short!("resumed"), stream_id),
        StreamEvent::Resumed(stream_id),
    );
}

/// Emit the `rate_upd` event when a rate is updated.
pub(crate) fn emit_rate_updated(env: &Env, stream_id: u64, payload: RateUpdated) {
    env.events()
        .publish((symbol_short!("rate_upd"), stream_id), payload);
}

/// Emit the `rate_dec` event for a safe rate decrease.
pub(crate) fn emit_rate_decreased(env: &Env, stream_id: u64, payload: RateDecreased) {
    env.events()
        .publish((symbol_short!("rate_dec"), stream_id), payload);
}

/// Emit the `rate_cap` event when a rate cap is enforced.
pub(crate) fn emit_rate_cap_enforced(env: &Env, stream_id: u64, payload: RateCapEnforced) {
    env.events()
        .publish((symbol_short!("rate_cap"), stream_id), payload);
}

/// Emit the `end_shrt` event when end time is shortened.
pub(crate) fn emit_stream_end_shortened(env: &Env, stream_id: u64, payload: StreamEndShortened) {
    env.events()
        .publish((symbol_short!("end_shrt"), stream_id), payload);
}

/// Emit the `end_ext` event when end time is extended.
pub(crate) fn emit_stream_end_extended(env: &Env, stream_id: u64, payload: StreamEndExtended) {
    env.events()
        .publish((symbol_short!("end_ext"), stream_id), payload);
}

/// Emit the `topped_up` event when a stream is topped up.
pub(crate) fn emit_stream_topped_up(env: &Env, stream_id: u64, payload: StreamToppedUp) {
    env.events()
        .publish((symbol_short!("toppedup"), stream_id), payload);
}

/// Emit the `sndr_xfr` event when sender is transferred.
pub(crate) fn emit_sender_transferred(env: &Env, stream_id: u64, payload: SenderTransferred) {
    env.events()
        .publish((symbol_short!("sndr_xfr"), stream_id), payload);
}

/// Emit the `hlth_chg` event when stream health changes.
pub(crate) fn emit_stream_health_changed(env: &Env, stream_id: u64, payload: StreamHealthChanged) {
    env.events()
        .publish((symbol_short!("hlth_chg"), stream_id), payload);
}

/// Emit the `rcpt_upd` event when recipient is updated.
pub(crate) fn emit_recipient_updated(env: &Env, stream_id: u64, payload: RecipientUpdated) {
    env.events()
        .publish((symbol_short!("rcpt_upd"), stream_id), payload);
}

/// Emit the `g_paused` event for global emergency pause change.
pub(crate) fn emit_global_emergency_pause_changed(env: &Env, payload: GlobalEmergencyPauseChanged) {
    env.events().publish((symbol_short!("g_paused"),), payload);
}

/// Emit the `g_resume` event when global pause is lifted.
pub(crate) fn emit_global_resumed(env: &Env, payload: GlobalResumed) {
    env.events().publish((symbol_short!("g_resume"),), payload);
}

/// Emit the `ct_pause` event when creation pause changes.
pub(crate) fn emit_contract_pause_changed(env: &Env, payload: ContractPauseChanged) {
    env.events().publish((symbol_short!("ct_pause"),), payload);
}

/// Emit `pr_pause` event when protocol is paused.
pub(crate) fn emit_protocol_paused(env: &Env, payload: ProtocolPaused) {
    env.events().publish((symbol_short!("pr_pause"),), payload);
}

/// Emit `pr_rsm` event when protocol is resumed.
pub(crate) fn emit_protocol_resumed(env: &Env, payload: ProtocolResumed) {
    env.events().publish((symbol_short!("pr_rsm"),), payload);
}

/// Emit `ac_set` event when auto-claim destination is set.
pub(crate) fn emit_auto_claim_set(env: &Env, stream_id: u64, payload: AutoClaimSet) {
    env.events()
        .publish((symbol_short!("ac_set"), stream_id), payload);
}

/// Emit `ac_rev` event when auto-claim destination is revoked.
pub(crate) fn emit_auto_claim_revoked(env: &Env, stream_id: u64, payload: AutoClaimRevoked) {
    env.events()
        .publish((symbol_short!("ac_rev"), stream_id), payload);
}

/// Emit `ac_trig` event when auto-claim is triggered.
pub(crate) fn emit_auto_claim_triggered(env: &Env, stream_id: u64, payload: AutoClaimTriggered) {
    env.events()
        .publish((symbol_short!("ac_trig"), stream_id), payload);
}

/// Emit `excess_sw` event when admin sweeps excess tokens.
pub(crate) fn emit_excess_swept(env: &Env, payload: ExcessSwept) {
    env.events().publish((symbol_short!("excess_sw"),), payload);
}

/// Emit the `cloned` event when a stream is cloned.
pub(crate) fn emit_stream_cloned(env: &Env, stream_id: u64, payload: StreamCloned) {
    env.events()
        .publish((symbol_short!("cloned"), stream_id), payload);
}

/// Emit the `kp_cncl` event for keeper-initiated cancellations.
pub(crate) fn emit_keeper_cancelled(env: &Env, stream_id: u64, payload: KeeperCancelled) {
    env.events()
        .publish((symbol_short!("kp_cncl"), stream_id), payload);
}
