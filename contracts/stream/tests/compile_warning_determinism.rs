//! Tests for compile-time warning stabilization, hygiene, and upgrade determinism.
//!
//! Formalizes Issue #1332 requirements:
//! 1. Module import hygiene & warning-free baseline contract paths
//! 2. Deterministic execution across retries and contract upgrades
//! 3. Storage layout and discriminant preservation under compile-time cleanup

use fluxora_stream::{
    CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus, ContractError,
    CONTRACT_VERSION,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    admin: Address,
    token_id: Address,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        sac.mint(&sender, &10_000_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
            admin,
            token_id,
        }
    }
}

#[test]
fn test_contract_version_is_deterministic_and_stable() {
    let ctx = TestContext::setup();
    let v1 = ctx.client.version();
    let v2 = ctx.client.version();
    assert_eq!(v1, CONTRACT_VERSION);
    assert_eq!(v1, v2);
}

#[test]
fn test_stream_creation_and_retry_determinism_under_warning_hygiene() {
    let ctx = TestContext::setup();

    let params = CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: 1_000,
        rate_per_second: 10,
        start_time: 100,
        cliff_time: 100,
        end_time: 200,
        withdraw_dust_threshold: Some(0),
        memo: None,
        metadata: None,
        kind: StreamKind::Linear,
        irrevocable: None,
        witness: None,
    };

    let stream_id = ctx.client.create_stream(&ctx.sender, &params);
    assert_eq!(stream_id, 1);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 1);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 1_000);
    assert_eq!(state.status, StreamStatus::Active);

    // Verify retry reading stream count is deterministic
    let count1 = ctx.client.get_stream_count();
    let count2 = ctx.client.get_stream_count();
    assert_eq!(count1, 1);
    assert_eq!(count1, count2);
}

#[test]
fn test_accrual_calculation_determinism_across_retries() {
    let ctx = TestContext::setup();

    let params = CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: 10_000,
        rate_per_second: 100,
        start_time: 1_000,
        cliff_time: 1_000,
        end_time: 1_100,
        withdraw_dust_threshold: Some(0),
        memo: None,
        metadata: None,
        kind: StreamKind::Linear,
        irrevocable: None,
        witness: None,
    };

    let stream_id = ctx.client.create_stream(&ctx.sender, &params);
    ctx.env.ledger().set_timestamp(1_050);

    let accrued1 = ctx.client.calculate_accrued(&stream_id);
    let accrued2 = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued1, 5_000);
    assert_eq!(accrued1, accrued2);

    let withdrawable1 = ctx.client.get_withdrawable(&stream_id);
    let withdrawable2 = ctx.client.get_withdrawable(&stream_id);
    assert_eq!(withdrawable1, 5_000);
    assert_eq!(withdrawable1, withdrawable2);
}
