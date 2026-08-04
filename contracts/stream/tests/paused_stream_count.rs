use fluxora_stream::{
    ContractError, CreateStreamParams, DataKey, FluxoraStream, FluxoraStreamClient, PauseReason,
    StreamKind, StreamStatus,
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
    let withdrawn = ctx.client.withdraw(&stream_id, &None);

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

/// `get_paused_stream_count` tracks **only** individually-paused streams — it is
/// **not** affected by the protocol-wide `GlobalEmergencyPaused` circuit breaker.
///
/// When the global flag is raised (`set_global_emergency_paused(true)`) with zero
/// individually-paused streams, the counter correctly returns `0`.  Callers that
/// need full pause-state awareness must also query `get_global_emergency_paused()`.
/// This is intentional: the two mechanisms are orthogonal (per-stream status vs.
/// protocol-wide gate) and the counter deliberately reflects only the former.
#[test]
fn paused_stream_count_is_zero_during_global_emergency_pause() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    // Baseline: no individually-paused streams, no global emergency pause.
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
    assert!(!ctx.client.get_global_emergency_paused());

    // Raise the global emergency pause — all mutations are now blocked, but the
    // per-stream counter still reflects zero individually-paused streams.
    ctx.client.set_global_emergency_paused(&true);
    assert!(ctx.client.get_global_emergency_paused());
    assert_eq!(
        ctx.client.get_paused_stream_count(),
        0,
        "counter must remain 0: global pause does not increment PausedStreamCount"
    );

    // Verify the stream is still Active (not Paused) under the global flag.
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Active
    );

    // Clear the global pause — counter still 0.
    ctx.client.set_global_emergency_paused(&false);
    assert_eq!(
        ctx.client.get_paused_stream_count(),
        0,
        "clearing global pause must not change the counter either"
    );
}

/// Confirm that `get_paused_stream_count` is completely independent of the
/// `GlobalEmergencyPaused` toggle: the counter reflects only individually-paused
/// streams and does not move when the global flag is raised or cleared.
#[test]
fn paused_stream_count_unaffected_by_global_emergency_toggle() {
    let ctx = Ctx::setup();
    let stream_a = ctx.create_stream(100);
    let stream_b = ctx.create_stream(100);

    // Individually pause stream_a → count == 1.
    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&stream_a, &PauseReason::Operational);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    // Raise global emergency pause — count must remain 1 (unchanged).
    ctx.client.set_global_emergency_paused(&true);
    assert!(ctx.client.get_global_emergency_paused());
    assert_eq!(
        ctx.client.get_paused_stream_count(),
        1,
        "global pause on must not alter the per-stream count"
    );

    // Pause stream_b while global pause is active (admin bypass).
    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream_as_admin(&stream_b, &PauseReason::Administrative);
    assert_eq!(
        ctx.client.get_paused_stream_count(),
        2,
        "admin pause should still increment the counter during global pause"
    );

    // Clear the global emergency pause — count must remain 2.
    ctx.client.set_global_emergency_paused(&false);
    assert!(!ctx.client.get_global_emergency_paused());
    assert_eq!(
        ctx.client.get_paused_stream_count(),
        2,
        "clearing global pause must not change the per-stream count"
    );

    // Resume stream_a individually → count back to 1.
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream(&stream_a);
    assert_eq!(ctx.client.get_paused_stream_count(), 1);

    // Resume stream_b individually → count back to 0.
    ctx.clear_pause_cooldown();
    ctx.client.resume_stream_as_admin(&stream_b);
    assert_eq!(ctx.client.get_paused_stream_count(), 0);
}
