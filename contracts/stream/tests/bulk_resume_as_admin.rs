//! Coverage for `bulk_resume_streams_as_admin` — atomic all-or-nothing batch resume.
//!
//! Complements the governance e2e suite in `governance_executor_e2e.rs` with
//! direct contract-level branch coverage (empty batch, happy path, duplicates,
//! missing IDs, active/cancelled members, cooldown).

#![cfg(test)]

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    vec, Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
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
            client,
            sender,
            recipient,
        }
    }

    fn clear_pause_cooldown(&self) {
        self.env
            .ledger()
            .with_mut(|ledger| ledger.sequence_number += 32);
    }

    fn create_stream(&self, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &(duration as i128),
            &1,
            &now,
            &now,
            &(now + duration),
            &0,
            &None,
            &StreamKind::Linear,
        )
    }

    fn pause_admin(&self, stream_id: u64) {
        self.clear_pause_cooldown();
        self.client
            .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    }
}

#[test]
fn bulk_resume_empty_batch_is_noop() {
    let ctx = Ctx::setup();
    ctx.client.bulk_resume_streams_as_admin(&vec![&ctx.env]);
}

#[test]
fn bulk_resume_all_paused_succeeds() {
    let ctx = Ctx::setup();
    let a = ctx.create_stream(10_000);
    let b = ctx.create_stream(10_000);
    ctx.pause_admin(a);
    ctx.pause_admin(b);

    ctx.clear_pause_cooldown();
    ctx.client
        .bulk_resume_streams_as_admin(&vec![&ctx.env, a, b]);

    assert_eq!(ctx.client.get_stream_state(&a).status, StreamStatus::Active);
    assert_eq!(ctx.client.get_stream_state(&b).status, StreamStatus::Active);
}

#[test]
fn bulk_resume_mixed_cancelled_is_atomic() {
    let ctx = Ctx::setup();
    let paused_a = ctx.create_stream(10_000);
    let cancelled = ctx.create_stream(10_000);
    let paused_b = ctx.create_stream(10_000);

    ctx.pause_admin(paused_a);
    ctx.pause_admin(paused_b);
    ctx.client.cancel_stream_as_admin(&cancelled);

    ctx.clear_pause_cooldown();
    let result = ctx
        .client
        .try_bulk_resume_streams_as_admin(&vec![&ctx.env, paused_a, cancelled, paused_b]);

    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
    assert_eq!(
        ctx.client.get_stream_state(&paused_a).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.client.get_stream_state(&paused_b).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.client.get_stream_state(&cancelled).status,
        StreamStatus::Cancelled
    );
}

#[test]
fn bulk_resume_rejects_duplicate_ids() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(10_000);
    ctx.pause_admin(id);

    ctx.clear_pause_cooldown();
    let result = ctx
        .client
        .try_bulk_resume_streams_as_admin(&vec![&ctx.env, id, id]);
    assert_eq!(result, Err(Ok(ContractError::DuplicateStreamId)));
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );
}

#[test]
fn bulk_resume_rejects_missing_stream() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(10_000);
    ctx.pause_admin(id);

    ctx.clear_pause_cooldown();
    let result = ctx
        .client
        .try_bulk_resume_streams_as_admin(&vec![&ctx.env, id, 999_999u64]);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );
}

#[test]
fn bulk_resume_rejects_active_member() {
    let ctx = Ctx::setup();
    let paused = ctx.create_stream(10_000);
    let active = ctx.create_stream(10_000);
    ctx.pause_admin(paused);

    ctx.clear_pause_cooldown();
    let result = ctx
        .client
        .try_bulk_resume_streams_as_admin(&vec![&ctx.env, paused, active]);
    assert_eq!(result, Err(Ok(ContractError::StreamNotPaused)));
    assert_eq!(
        ctx.client.get_stream_state(&paused).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.client.get_stream_state(&active).status,
        StreamStatus::Active
    );
}

#[test]
fn bulk_resume_rejects_pause_cooldown() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(10_000);
    // Pause without clearing cooldown afterward — last toggle is "now".
    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream_as_admin(&id, &PauseReason::Administrative);

    // Immediate bulk resume must hit PauseCooldownActive.
    let result = ctx
        .client
        .try_bulk_resume_streams_as_admin(&vec![&ctx.env, id]);
    assert_eq!(result, Err(Ok(ContractError::PauseCooldownActive)));
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );
}
