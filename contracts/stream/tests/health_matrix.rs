#![cfg(test)]

use fluxora_stream::{
    FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, Env,
};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
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

        Self {
            env,
            client,
            sender,
            recipient,
            token,
        }
    }
}

#[test]
fn test_health_matrix_active_fully_funded_before_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &100u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(50);
    let health = ctx.client.get_stream_health(&stream_id);
    
    assert_eq!(health.is_underfunded, false);
    assert_eq!(health.is_expired, false);
    assert_eq!(health.accrued_to_date, 0); 
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(950)); 
}

#[test]
fn test_health_matrix_active_underfunded_mid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &2_i128,
        &0u64,
        &100u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(300);
    let health = ctx.client.get_stream_health(&stream_id);
    
    assert_eq!(health.is_underfunded, true);
    assert_eq!(health.is_expired, false);
    assert_eq!(health.accrued_to_date, 600);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(200)); 
}

#[test]
fn test_health_matrix_paused_underfunded_mid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &2_i128,
        &0u64,
        &100u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_sequence(100);
    ctx.env.ledger().set_timestamp(300);
    ctx.client.pause_stream(&stream_id, &PauseReason::Operational);
    
    let health = ctx.client.get_stream_health(&stream_id);
    
    assert_eq!(health.is_underfunded, true);
    assert_eq!(health.is_expired, false);
    assert_eq!(health.accrued_to_date, 600);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(200));
}

#[test]
fn test_health_matrix_expired_not_fully_withdrawn() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &100u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1200);
    let health = ctx.client.get_stream_health(&stream_id);
    
    assert_eq!(health.is_underfunded, false);
    assert_eq!(health.is_expired, true); 
    assert_eq!(health.accrued_to_date, 1000);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(0));
}

#[test]
fn test_health_matrix_completed_after_end() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.env.ledger().set_sequence(1);
    
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &100u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1200);
    ctx.env.ledger().set_sequence(100);
    ctx.client.withdraw(&stream_id);
    
    let health = ctx.client.get_stream_health(&stream_id);
    
    assert_eq!(health.is_underfunded, false);
    assert_eq!(health.is_expired, false); 
    assert_eq!(health.accrued_to_date, 1000);
    assert_eq!(health.remaining_deposit, 0);
    assert_eq!(health.seconds_until_depletion, Some(0));
}

#[test]
fn test_health_matrix_cancelled_mid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &100u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    ctx.client.cancel_stream(&stream_id);
    
    let health = ctx.client.get_stream_health(&stream_id);
    
    assert_eq!(health.is_underfunded, false);
    assert_eq!(health.is_expired, false);
    assert_eq!(health.accrued_to_date, 500);
    // Cancellation does not adjust deposit_amount in state, so remaining_deposit stays 1000 until withdraw.
    assert_eq!(health.remaining_deposit, 1000); 
    // Seconds until depletion still returns the time remaining if it wasn't cancelled, 
    // since the rate_per_second is unmodified.
    assert_eq!(health.seconds_until_depletion, Some(500)); 
}
