use fluxora_stream::{
    ContractError, CreateStreamParams, DataKey, FluxoraStream, FluxoraStreamClient, PauseReason,
    Stream, StreamKind, StreamStatus,
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
    sender: Address,
    recipient: Address,
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
            sender,
            recipient,
            token,
        }
    }

    fn clear_pause_cooldown(&self) {
        self.env
            .ledger()
            .with_mut(|ledger| ledger.sequence_number += 32);
    }

    fn advance_timestamp(&self, seconds: u64) {
        self.env
            .ledger()
            .with_mut(|ledger| ledger.timestamp += seconds);
    }

    fn create_stream(&self, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: (duration as i128),
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: (now + duration),
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    fn set_cumulative_paused_duration(&self, stream_id: u64, value: u64) {
        self.env.as_contract(&self.contract_id, || {
            let mut stream: Stream = self
                .env
                .storage()
                .persistent()
                .get(&DataKey::Stream(stream_id))
                .unwrap();
            stream.cumulative_paused_duration = value;
            self.env
                .storage()
                .persistent()
                .set(&DataKey::Stream(stream_id), &stream);
        });
    }
}

#[test]
fn fresh_stream_has_zero_paused_duration() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    let duration = ctx.client.get_paused_duration(&stream_id);
    assert_eq!(duration, 0);
}

#[test]
fn single_pause_resume_accumulates_correctly() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);

    ctx.advance_timestamp(50);
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);

    let duration = ctx.client.get_paused_duration(&stream_id);
    assert_eq!(duration, 50);
}

#[test]
fn multiple_pause_resume_cycles_accumulate() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    ctx.advance_timestamp(30);
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.client.get_paused_duration(&stream_id), 30);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    ctx.advance_timestamp(70);
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.client.get_paused_duration(&stream_id), 100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    ctx.advance_timestamp(200);
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.client.get_paused_duration(&stream_id), 300);
}

#[test]
fn currently_paused_stream_includes_ongoing_duration() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    ctx.advance_timestamp(75);

    let duration = ctx.client.get_paused_duration(&stream_id);
    assert_eq!(duration, 75);
}

#[test]
fn overflow_returns_typed_error() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);

    ctx.set_cumulative_paused_duration(stream_id, u64::MAX - 10);

    ctx.advance_timestamp(20);
    ctx.clear_pause_cooldown();

    let result = ctx.client.try_resume_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

#[test]
fn get_paused_duration_stable_at_boundary() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);

    ctx.set_cumulative_paused_duration(stream_id, u64::MAX - 1000);

    let duration = ctx.client.get_paused_duration(&stream_id);
    assert!(duration >= u64::MAX - 1000);
}

#[test]
fn get_stream_state_returns_paused_duration_field() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    ctx.advance_timestamp(42);
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.cumulative_paused_duration, 42);
}

#[test]
fn admin_pause_resume_tracks_duration() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

    ctx.advance_timestamp(99);
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream_as_admin(&stream_id);

    let duration = ctx.client.get_paused_duration(&stream_id);
    assert_eq!(duration, 99);
}

#[test]
fn admin_pause_then_sender_resume_tracks_duration() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

    ctx.advance_timestamp(55);
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_id);

    let duration = ctx.client.get_paused_duration(&stream_id);
    assert_eq!(duration, 55);
}

#[test]
fn sender_pause_then_admin_resume_tracks_duration() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);

    ctx.advance_timestamp(33);
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream_as_admin(&stream_id);

    let duration = ctx.client.get_paused_duration(&stream_id);
    assert_eq!(duration, 33);
}

#[test]
fn calculate_accrued_unchanged_by_paused_duration() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    ctx.set_cumulative_paused_duration(stream_id, u64::MAX - 1);

    ctx.advance_timestamp(10);
    ctx.clear_pause_cooldown();

    let health = ctx.client.try_get_stream_health(&stream_id);
    assert!(health.is_ok());
}

#[test]
fn deterministic_without_excessive_cycles() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(1000);

    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);

    ctx.set_cumulative_paused_duration(stream_id, u64::MAX - 200);

    ctx.advance_timestamp(300);
    ctx.clear_pause_cooldown();

    let result = ctx.client.try_resume_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}
