extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, CreateStreamResult, FluxoraStream, FluxoraStreamClient, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

#[allow(dead_code)]
struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }
}

#[test]
fn test_create_streams_partial_mixed_success() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let streams = vec![
        &ctx.env,
        // 1. Valid stream
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 1000,
            cliff_time: 1000,
            end_time: 2000,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // 2. Invalid stream (end_time < start_time)
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 2000,
            cliff_time: 2000,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // 3. Valid stream
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 2000,
            rate_per_second: 2,
            start_time: 1000,
            cliff_time: 1000,
            end_time: 2000,
            withdraw_dust_threshold: None,
            memo: None,
        },
    ];

    let results = ctx.client().create_streams_partial(&ctx.sender, &streams);

    assert_eq!(results.len(), 3);

    // First entry: success
    let res0 = results.get(0).unwrap();
    assert!(res0.success);
    assert_eq!(res0.stream_id, 0);
    assert!(res0.error_code.is_none());

    // Second entry: failure (InvalidParams)
    let res1 = results.get(1).unwrap();
    assert!(!res1.success);
    assert_eq!(res1.error_code, Some(ContractError::InvalidParams as u32));

    // Third entry: success
    let res2 = results.get(2).unwrap();
    assert!(res2.success);
    assert_eq!(res2.stream_id, 1);
    assert!(res2.error_code.is_none());

    // Verify side effects
    assert_eq!(ctx.token.balance(&ctx.sender), 7_000); // 10000 - 1000 - 2000
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);
    assert_eq!(ctx.client().get_stream_count(), 2);
}

#[test]
fn test_create_streams_partial_insufficient_balance_mid_run() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    // Initial balance is 10,000.
    let streams = vec![
        &ctx.env,
        // 1. Valid stream (uses 6,000)
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 6000,
            rate_per_second: 6,
            start_time: 1000,
            cliff_time: 1000,
            end_time: 2000,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // 2. Insufficient balance (needs 5,000, only 4,000 left)
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 5000,
            rate_per_second: 5,
            start_time: 1000,
            cliff_time: 1000,
            end_time: 2000,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // 3. Valid stream (needs 1,000, succeeds)
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 1000,
            cliff_time: 1000,
            end_time: 2000,
            withdraw_dust_threshold: None,
            memo: None,
        },
    ];

    let results = ctx.client().create_streams_partial(&ctx.sender, &streams);

    assert_eq!(results.len(), 3);

    // First entry: success
    assert!(results.get(0).unwrap().success);

    // Second entry: failure (likely caught by token client or our pre-check)
    let res1 = results.get(1).unwrap();
    assert!(!res1.success);
    // Note: error code depends on pull_token implementation, but should be failure.

    // Third entry: success
    assert!(results.get(2).unwrap().success);

    // Verify side effects
    assert_eq!(ctx.token.balance(&ctx.sender), 3_000); // 10000 - 6000 - 1000
    assert_eq!(ctx.client().get_stream_count(), 2);
}

#[test]
fn test_create_streams_partial_ordering_guarantees() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let streams = vec![
        &ctx.env,
        CreateStreamParams {
            recipient: Address::generate(&ctx.env),
            deposit_amount: 100,
            rate_per_second: 1,
            start_time: 1000,
            cliff_time: 1000,
            end_time: 1100,
            withdraw_dust_threshold: None,
            memo: None,
        },
        CreateStreamParams {
            recipient: Address::generate(&ctx.env),
            deposit_amount: 200,
            rate_per_second: 2,
            start_time: 1000,
            cliff_time: 1000,
            end_time: 1100,
            withdraw_dust_threshold: None,
            memo: None,
        },
    ];

    let results = ctx.client().create_streams_partial(&ctx.sender, &streams);

    assert_eq!(results.get(0).unwrap().stream_id, 0);
    assert_eq!(results.get(1).unwrap().stream_id, 1);
}
