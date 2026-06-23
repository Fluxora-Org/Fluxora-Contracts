use fluxora_stream::{
    ContractError, DataKey, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, Env,
};

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

    /// Advance the ledger sequence far enough to clear the pause/resume cooldown
    /// (`MIN_PAUSE_INTERVAL_LEDGERS`) before toggling pause state.
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
}

#[test]
fn paused_stream_count_tracks_sender_pause_resume() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    assert_eq!(ctx.client.get_paused_stream_count(), 0);
    assert!(!ctx.client.get_global_emergency_paused());

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

#[test]
fn paused_stream_count_tracks_admin_pause_resume() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    ctx.clear_pause_cooldown();
    ctx.client.resume_stream_as_admin(&stream_id);
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

#[test]
fn paused_stream_count_ignores_failed_idempotent_calls() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    ctx.clear_pause_cooldown();
    let pause_again = ctx
        .client
        .try_pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(pause_again, Err(Ok(ContractError::StreamAlreadyPaused)));
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.client.get_paused_stream_count(), 0);

    ctx.clear_pause_cooldown();
    let resume_again = ctx.client.try_resume_stream(&stream_id);
    assert_eq!(resume_again, Err(Ok(ContractError::StreamNotPaused)));
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
}

#[test]
fn paused_stream_count_decrements_on_cancel_from_paused() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    ctx.client.cancel_stream(&stream_id);
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
}

#[test]
fn paused_stream_count_decrements_on_terminal_completion_from_paused() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(10);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    ctx.env.ledger().with_mut(|ledger| ledger.timestamp += 11);
    let withdrawn = ctx.client.withdraw(&stream_id);

    assert_eq!(withdrawn, 10);
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );
}

#[test]
fn paused_stream_count_never_underflows_when_upgrade_key_is_missing() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    ctx.env.as_contract(&ctx.contract_id, || {
        ctx.env
            .storage()
            .instance()
            .remove(&DataKey::PausedStreamCount);
    });

    assert_eq!(ctx.client.get_paused_stream_count(), 0);

    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

#[test]
fn paused_stream_count_is_initialised_to_zero() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
    assert_eq!(ctx.admin, ctx.client.get_config().admin);
}
