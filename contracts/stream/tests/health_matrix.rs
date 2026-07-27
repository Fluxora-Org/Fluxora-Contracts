#![cfg(test)]

use fluxora_stream::{
    CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
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

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        let token_admin_client = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
        token_admin_client.mint(&sender, &1_000_000_000);

        Self {
            env,
            client,
            sender,
            recipient,
        }
    }
}

#[test]
fn test_health_matrix_active_fully_funded_before_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(50);
    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(!health.is_expired);
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(300);
    let health = ctx.client.get_stream_health(&stream_id);

    assert!(health.is_underfunded);
    assert!(!health.is_expired);
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(300);
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);

    let health = ctx.client.get_stream_health(&stream_id);

    assert!(health.is_underfunded);
    assert!(!health.is_expired);
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(1200);
    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(health.is_expired);
    assert_eq!(health.accrued_to_date, 1000);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(0));
}

#[test]
fn test_health_matrix_completed_after_end() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(1200);
    ctx.client.withdraw(&stream_id);

    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(!health.is_expired);
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(500);
    ctx.client.cancel_stream(&stream_id);

    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(!health.is_expired);
    assert_eq!(health.accrued_to_date, 500);
    // Cancellation does not adjust deposit_amount in state, so remaining_deposit stays 1000 until withdraw.
    assert_eq!(health.remaining_deposit, 1000);
    // Seconds until depletion still returns the time remaining if it wasn't cancelled,
    // since the rate_per_second is unmodified.
    assert_eq!(health.seconds_until_depletion, Some(500));
}

#[test]
fn test_health_matrix_before_start() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // start_time=500, so ledger at t=0 is before the stream begins.
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 500u64,
            cliff_time: 500u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(0);
    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(!health.is_expired);
    assert_eq!(health.accrued_to_date, 0);
    assert_eq!(health.remaining_deposit, 1000);
    // No accrual yet, so depletion timer is undefined -> None.
    assert_eq!(health.seconds_until_depletion, None);
}
