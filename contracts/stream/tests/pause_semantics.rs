//! Regression tests for per-stream pause semantics (issue #1329).
//!
//! Covers the edge-case behaviors documented in `docs/pause-semantics.md` that
//! are not already pinned by `tests/paused_stream_count.rs` or
//! `tests/bulk_resume_as_admin.rs`:
//!
//! - Cooldown applies symmetrically to both pause and resume
//! - Withdrawal is blocked on a non-time-terminal Paused stream
//! - Withdrawal is allowed on a Paused stream past `end_time` (time-terminal override)
//! - A time-terminal withdrawal from a Paused stream transitions status to Completed
//! - `pause_stream` is rejected on a time-terminal stream (even if status is Active)
//! - `resume_stream` is rejected on a time-terminal Paused stream
//! - `pause_stream` is rejected on Cancelled and Completed streams
//! - Accrual continues during a pause (tokens accrue by wall-clock time, not status)
//! - `cancel_stream` from Paused refunds the correct unstreamed amount
//! - All four `PauseReason` variants are accepted without error
//! - Global emergency pause blocks `withdraw` but does not block `pause_stream`
//! - Admin-pause writes a `LastPauseRecord` while sender-pause does not
//! - `batch_withdraw_to` honors the same per-stream Paused gate as the other
//!   withdrawal entrypoints (issue #1327)
//! - `delegate_recipient_share` is unaffected by a per-stream Paused status,
//!   consistent with other non-withdrawal mutations (issue #1327)

#![cfg(test)]

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus, WithdrawToParam,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, Env,
};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    client: FluxoraStreamClient<'a>,
    admin: Address,
    sender: Address,
    recipient: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        stellar_asset.mint(&sender, &1_000_000_000);
        client.init(&token_id, &admin);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            contract_id,
            client,
            admin,
            sender,
            recipient,
            token,
        }
    }

    /// Advance ledger sequence past the pause/resume cooldown window (17 ledgers).
    fn clear_pause_cooldown(&self) {
        self.env.ledger().with_mut(|l| l.sequence_number += 32);
    }

    /// Create a linear stream with `deposit_amount == duration` (1 token/sec).
    fn create_stream(&self, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: duration as i128,
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: now + duration,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    /// Create a linear stream at a caller-chosen `rate_per_second`, sized so
    /// `delegate_recipient_share`'s integer-division math (`rate * share_bps
    /// / 10000`) stays non-zero for a 50% split. `create_stream` above fixes
    /// `rate_per_second == 1`, which underflows to a zero child rate.
    fn create_stream_with_rate(&self, rate_per_second: i128, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: rate_per_second * duration as i128,
                rate_per_second,
                start_time: now,
                cliff_time: now,
                end_time: now + duration,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    /// Pause a stream as the sender (clears cooldown first).
    fn sender_pause(&self, stream_id: u64) {
        self.clear_pause_cooldown();
        self.client
            .pause_stream(&stream_id, &PauseReason::Operational);
    }

    /// Pause a stream as the admin (clears cooldown first).
    fn admin_pause(&self, stream_id: u64) {
        self.clear_pause_cooldown();
        self.client
            .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    }
}

// ---------------------------------------------------------------------------
// Cooldown symmetry
// ---------------------------------------------------------------------------

/// The cooldown blocks a re-pause attempt before 17 ledgers have elapsed,
/// even if the sender immediately resumes and tries to re-pause.
#[test]
fn pause_cooldown_blocks_rapid_retoggle() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    // First pause — advance past cooldown.
    ctx.clear_pause_cooldown();
    ctx.client.pause_stream(&id, &PauseReason::Operational);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );

    // Advance past cooldown again, resume.
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Active
    );

    // Attempt immediate re-pause (no cooldown advance) — must be rejected.
    let err = ctx.client.try_pause_stream(&id, &PauseReason::Operational);
    assert_eq!(err, Err(Ok(ContractError::PauseCooldownActive)));
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Active
    );
}

/// The cooldown also applies to resume: immediately re-resuming after a
/// pause-then-resume cycle is rejected.
#[test]
fn resume_cooldown_blocks_rapid_re_resume() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    // Pause, then immediately try to resume without waiting — blocked.
    ctx.clear_pause_cooldown();
    ctx.client.pause_stream(&id, &PauseReason::Operational);

    let err = ctx.client.try_resume_stream(&id);
    assert_eq!(err, Err(Ok(ContractError::PauseCooldownActive)));
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );
}

// ---------------------------------------------------------------------------
// Withdrawal gate
// ---------------------------------------------------------------------------

/// `withdraw` is blocked when a stream is Paused and not yet past end_time.
#[test]
fn withdraw_blocked_while_paused() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    // Advance time so there is something to withdraw.
    ctx.env.ledger().with_mut(|l| l.timestamp += 100);

    ctx.sender_pause(id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );

    let err = ctx.client.try_withdraw(&id);
    assert_eq!(err, Err(Ok(ContractError::InvalidState)));
}

/// `batch_withdraw_to` is blocked on a Paused, non-time-terminal stream —
/// same gate as `withdraw`/`withdraw_to`/`batch_withdraw`/`withdraw_from_pool`,
/// just exercised through the keeper/auto-claim batch path. Regression guard
/// for the per-stream Paused check at the `batch_withdraw_to` call site,
/// which previously had no dedicated test (only global-pause coverage
/// existed in `src/test.rs`).
#[test]
fn batch_withdraw_to_blocked_while_paused() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    ctx.env.ledger().with_mut(|l| l.timestamp += 100);
    ctx.sender_pause(id);

    let destination = Address::generate(&ctx.env);
    let param = WithdrawToParam {
        stream_id: id,
        destination,
    };
    let err = ctx
        .client
        .try_batch_withdraw_to(&ctx.recipient, &soroban_sdk::vec![&ctx.env, param]);
    assert_eq!(err, Err(Ok(ContractError::InvalidState)));
}

/// `withdraw_to` is blocked on a Paused, non-time-terminal stream.
#[test]
fn withdraw_to_blocked_while_paused() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    ctx.env.ledger().with_mut(|l| l.timestamp += 100);
    ctx.sender_pause(id);

    let dest = Address::generate(&ctx.env);
    let err = ctx.client.try_withdraw_to(&id, &dest);
    assert_eq!(err, Err(Ok(ContractError::InvalidState)));
}

/// Once `current_time >= end_time`, withdrawal is allowed even if status is Paused.
#[test]
fn withdraw_allowed_on_paused_stream_past_end_time() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(100);

    ctx.sender_pause(id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );

    // Advance past end_time — time-terminal override kicks in.
    ctx.env.ledger().with_mut(|l| l.timestamp += 101);

    let withdrawn = ctx.client.withdraw(&id);
    assert_eq!(withdrawn, 100); // full deposit
}

/// A time-terminal withdrawal from a Paused stream transitions status to Completed
/// and decrements PausedStreamCount.
#[test]
fn time_terminal_withdraw_from_paused_completes_stream() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(50);

    ctx.sender_pause(id);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    // Advance past end_time.
    ctx.env.ledger().with_mut(|l| l.timestamp += 51);

    let withdrawn = ctx.client.withdraw(&id);
    assert_eq!(withdrawn, 50);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Completed
    );
    // Counter must be decremented: Paused → Completed.
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
}

// ---------------------------------------------------------------------------
// Time-terminal blocks pause / resume
// ---------------------------------------------------------------------------

/// `pause_stream` on a stream that is Active but has already reached end_time
/// must return `StreamTerminalState`.
#[test]
fn pause_rejected_on_time_terminal_stream() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(10);

    // Advance past end_time before attempting to pause.
    ctx.env.ledger().with_mut(|l| l.timestamp += 11);
    ctx.clear_pause_cooldown();

    let err = ctx.client.try_pause_stream(&id, &PauseReason::Operational);
    assert_eq!(err, Err(Ok(ContractError::StreamTerminalState)));
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Active
    );
}

/// `resume_stream` on a Paused stream that has since passed end_time must
/// return `StreamTerminalState` — the time-terminal override takes precedence.
#[test]
fn resume_rejected_on_time_terminal_paused_stream() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(100);

    ctx.sender_pause(id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );

    // Advance past end_time while stream is still Paused.
    ctx.env.ledger().with_mut(|l| l.timestamp += 101);
    ctx.clear_pause_cooldown();

    let err = ctx.client.try_resume_stream(&id);
    assert_eq!(err, Err(Ok(ContractError::StreamTerminalState)));
    // Status must remain Paused — no state change on error.
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );
}

// ---------------------------------------------------------------------------
// Terminal-status guards
// ---------------------------------------------------------------------------

/// `pause_stream` on a Cancelled stream must return `StreamTerminalState`.
#[test]
fn pause_rejected_on_cancelled_stream() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    ctx.client.cancel_stream(&id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Cancelled
    );

    ctx.clear_pause_cooldown();
    let err = ctx.client.try_pause_stream(&id, &PauseReason::Operational);
    assert_eq!(err, Err(Ok(ContractError::StreamTerminalState)));
}

/// `pause_stream` on a Completed stream must return `StreamTerminalState`.
#[test]
fn pause_rejected_on_completed_stream() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(50);

    // Advance past end_time and withdraw everything to reach Completed.
    ctx.env.ledger().with_mut(|l| l.timestamp += 51);
    ctx.client.withdraw(&id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Completed
    );

    ctx.clear_pause_cooldown();
    let err = ctx.client.try_pause_stream(&id, &PauseReason::Operational);
    // Completed is caught by the terminal-state check (status == Completed).
    assert_eq!(err, Err(Ok(ContractError::StreamTerminalState)));
}

// ---------------------------------------------------------------------------
// Accrual during pause
// ---------------------------------------------------------------------------

/// Tokens continue to accrue while a stream is Paused.
/// After resume, the recipient can withdraw all tokens that accrued during
/// both the active period and the pause window.
#[test]
fn accrual_continues_while_paused() {
    let ctx = Ctx::setup();
    // 1000-second stream, 1 token/sec.
    let id = ctx.create_stream(1_000);

    // Advance 200 seconds, then pause.
    ctx.env.ledger().with_mut(|l| l.timestamp += 200);
    ctx.sender_pause(id);

    // Advance another 300 seconds while paused — accrual should continue.
    ctx.env.ledger().with_mut(|l| l.timestamp += 300);

    // Resume the stream.
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&id);

    // After resume, withdrawable should reflect 500 seconds of accrual
    // (200 pre-pause + 300 during pause).
    let withdrawable = ctx.client.get_withdrawable(&id);
    assert_eq!(
        withdrawable, 500,
        "accrual must have continued for the 300 seconds the stream was paused"
    );

    let withdrawn = ctx.client.withdraw(&id);
    assert_eq!(withdrawn, 500);
}

// ---------------------------------------------------------------------------
// Cancel from Paused
// ---------------------------------------------------------------------------

/// Cancelling from a Paused state correctly refunds the unstreamed portion
/// and leaves the accrued portion claimable by the recipient.
#[test]
fn cancel_from_paused_refunds_unstreamed() {
    let ctx = Ctx::setup();
    // 1000-second, 1-token/sec stream. Deposit = 1000.
    let id = ctx.create_stream(1_000);

    // Advance 400 seconds, then pause.
    ctx.env.ledger().with_mut(|l| l.timestamp += 400);
    ctx.sender_pause(id);

    // Cancel from Paused state.
    ctx.client.cancel_stream(&id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Cancelled
    );

    // Counter must be 0 — Paused → Cancelled decrements it.
    assert_eq!(ctx.client.get_paused_stream_count(), 0);

    // Recipient should be able to withdraw the 400 accrued tokens.
    let withdrawn = ctx.client.withdraw(&id);
    assert_eq!(
        withdrawn, 400,
        "recipient must be able to claim accrued amount after cancel-from-paused"
    );
}

// ---------------------------------------------------------------------------
// PauseReason variants
// ---------------------------------------------------------------------------

/// All four PauseReason variants must be accepted without error.
#[test]
fn all_pause_reason_variants_accepted() {
    let ctx = Ctx::setup();

    for reason in [
        PauseReason::Operational,
        PauseReason::Administrative,
        PauseReason::Emergency,
        PauseReason::Compliance,
    ] {
        let id = ctx.create_stream(10_000);
        ctx.clear_pause_cooldown();
        ctx.client.pause_stream(&id, &reason);
        assert_eq!(
            ctx.client.get_stream_state(&id).status,
            StreamStatus::Paused
        );
        // Resume to leave a clean state for the next iteration.
        ctx.clear_pause_cooldown();
        ctx.client.resume_stream(&id);
    }
}

// ---------------------------------------------------------------------------
// Global emergency pause interaction
// ---------------------------------------------------------------------------

/// `withdraw` is blocked by the global emergency pause even on Active streams.
#[test]
fn global_pause_blocks_withdraw() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    ctx.env.ledger().with_mut(|l| l.timestamp += 100);
    ctx.client.set_global_emergency_paused(&true);

    let err = ctx.client.try_withdraw(&id);
    assert_eq!(err, Err(Ok(ContractError::ContractPaused)));
}

/// `pause_stream` (sender path) is NOT blocked by the global emergency pause —
/// a sender can still freeze their own stream while the protocol is halted.
#[test]
fn global_pause_does_not_block_sender_pause() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    ctx.client.set_global_emergency_paused(&true);
    ctx.clear_pause_cooldown();

    // Must succeed.
    ctx.client.pause_stream(&id, &PauseReason::Operational);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );
}

/// `resume_stream` (sender path) is NOT blocked by the global emergency pause.
#[test]
fn global_pause_does_not_block_sender_resume() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    ctx.sender_pause(id);
    ctx.client.set_global_emergency_paused(&true);
    ctx.clear_pause_cooldown();

    // Resume must succeed even while globally paused.
    ctx.client.resume_stream(&id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Active
    );
}

/// After `global_resume`, `withdraw` is unblocked again.
#[test]
fn global_resume_unblocks_withdraw() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    ctx.env.ledger().with_mut(|l| l.timestamp += 100);
    ctx.client.set_global_emergency_paused(&true);

    // Blocked while paused.
    assert_eq!(
        ctx.client.try_withdraw(&id),
        Err(Ok(ContractError::ContractPaused))
    );

    // Lift the global pause via the explicit global_resume entrypoint.
    ctx.client.global_resume();
    assert!(!ctx.client.get_global_emergency_paused());

    // Now withdraw should succeed.
    let withdrawn = ctx.client.withdraw(&id);
    assert_eq!(withdrawn, 100);
}

// ---------------------------------------------------------------------------
// Admin-pause PauseRecord side-effect
// ---------------------------------------------------------------------------

/// `pause_stream` (sender path) does NOT store `DataKey::LastPauseRecord(PauseKind::Stream)`.
/// `pause_stream_as_admin` DOES store it.
///
/// `get_pause_info` is the **protocol-wide** (global) pause query — it is not
/// the right accessor here, since `LastPauseRecord(PauseKind::Stream)` is a
/// separate, per-mechanism instance-storage key with no public getter. We
/// inspect it directly via `env.as_contract`, the same pattern already used
/// by `paused_stream_count.rs` and `storage_key_compat.rs` for other
/// storage-only keys.
#[test]
fn admin_pause_stores_pause_record_sender_pause_does_not() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(10_000);

    // Sender pause — no PauseRecord expected.
    ctx.sender_pause(id);
    let has_record_after_sender_pause = ctx.env.as_contract(&ctx.contract_id, || {
        ctx.env
            .storage()
            .instance()
            .has(&fluxora_stream::DataKey::LastPauseRecord(
                fluxora_stream::PauseKind::Stream,
            ))
    });
    assert!(
        !has_record_after_sender_pause,
        "sender pause must not write a LastPauseRecord(Stream)"
    );

    // Resume, then admin-pause — PauseRecord expected.
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&id);
    ctx.admin_pause(id);

    let record_after_admin_pause = ctx.env.as_contract(&ctx.contract_id, || {
        ctx.env
            .storage()
            .instance()
            .get::<_, fluxora_stream::PauseRecord>(&fluxora_stream::DataKey::LastPauseRecord(
                fluxora_stream::PauseKind::Stream,
            ))
    });
    assert!(
        record_after_admin_pause.is_some(),
        "admin pause must write a LastPauseRecord(Stream) accessible from instance storage"
    );
    assert_eq!(record_after_admin_pause.unwrap().actor, ctx.admin);
}

// ---------------------------------------------------------------------------
// Idempotency guards (cross-check with paused_stream_count.rs)
// ---------------------------------------------------------------------------

/// `pause_stream` on an already-Paused stream returns `StreamAlreadyPaused`
/// without touching the counter.
#[test]
fn double_pause_returns_stream_already_paused() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);

    ctx.sender_pause(id);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    ctx.clear_pause_cooldown();
    let err = ctx.client.try_pause_stream(&id, &PauseReason::Operational);
    assert_eq!(err, Err(Ok(ContractError::StreamAlreadyPaused)));
    // Counter must be unchanged.
    assert_eq!(ctx.client.get_paused_stream_count(), 1);
}

/// `resume_stream` on an Active stream returns `StreamNotPaused`
/// without touching the counter.
#[test]
fn resume_active_stream_returns_stream_not_paused() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(1_000);
    // Stream is Active — no pause has occurred.
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Active
    );

    ctx.clear_pause_cooldown();
    let err = ctx.client.try_resume_stream(&id);
    assert_eq!(err, Err(Ok(ContractError::StreamNotPaused)));
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
}

// ---------------------------------------------------------------------------
// Non-withdrawal operations on a Paused stream
// ---------------------------------------------------------------------------

/// `delegate_recipient_share` is NOT blocked by a per-stream Paused status
/// (only by the terminal-state and end_time guards it already checks).
/// This mirrors the existing "rate/schedule changes are allowed on Paused"
/// behavior documented for `update_rate_per_second` / `top_up_stream` — pause
/// only gates withdrawals, not configuration changes. Pinned here so a future
/// refactor that adds a `status == Active` guard to this entrypoint is a
/// deliberate, reviewed behavior change rather than an accidental regression.
#[test]
fn delegate_recipient_share_allowed_while_paused() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream_with_rate(10_000, 1_000);

    ctx.sender_pause(id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );

    let new_recipient = Address::generate(&ctx.env);
    let child_id =
        ctx.client
            .delegate_recipient_share(&id, &ctx.recipient, &5_000u32, &new_recipient);

    // Parent stream remains Paused; child is created Active.
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.client.get_stream_state(&child_id).status,
        StreamStatus::Active
    );
}
