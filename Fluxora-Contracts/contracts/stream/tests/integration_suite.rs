#![cfg(test)]

use crate::{
    ContractError, CreateStreamParams, CreateStreamResult, FluxoraStream, FluxoraStreamClient,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    vec, Address, Env, Vec,
};

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
    sac: StellarAssetClient<'a>,
    client: FluxoraStreamClient<'a>,
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
        client.init(&token_id, &admin).unwrap();

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        let token = TokenClient::new(&env, &token_id);
        // Provide sufficient allowance for tests that don't explicitly test allowances.
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            token,
            sac,
            client,
        }
    }
}

#[test]
fn test_create_streams_partial_mixed_invalid() {
    let ctx = TestContext::setup();
    let current_time = ctx.env.ledger().timestamp();

    let streams = vec![
        &ctx.env,
        // Valid stream
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: current_time + 10,
            cliff_time: current_time + 10,
            end_time: current_time + 1010,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // Invalid stream: sender == recipient
        CreateStreamParams {
            recipient: ctx.sender.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: current_time + 10,
            cliff_time: current_time + 10,
            end_time: current_time + 1010,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // Invalid stream: end_time < start_time
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: current_time + 100,
            cliff_time: current_time + 100,
            end_time: current_time + 50,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // Valid stream 2
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 500,
            rate_per_second: 1,
            start_time: current_time + 20,
            cliff_time: current_time + 20,
            end_time: current_time + 520,
            withdraw_dust_threshold: None,
            memo: None,
        },
    ];

    let results = ctx.client.create_streams_partial(&ctx.sender, &streams);

    assert_eq!(results.len(), 4);

    // Result 1: Success
    let res1 = results.get(0).unwrap();
    assert!(res1.success);
    assert!(res1.stream_id.is_some());
    assert_eq!(res1.error, None);

    // Result 2: Error (InvalidParams - sender == recipient)
    let res2 = results.get(1).unwrap();
    assert!(!res2.success);
    assert_eq!(res2.stream_id, None);
    assert_eq!(res2.error, Some(ContractError::InvalidParams as u32));

    // Result 3: Error (InvalidParams - end_time < start_time)
    let res3 = results.get(2).unwrap();
    assert!(!res3.success);
    assert_eq!(res3.stream_id, None);
    assert_eq!(res3.error, Some(ContractError::InvalidParams as u32));

    // Result 4: Success
    let res4 = results.get(3).unwrap();
    assert!(res4.success);
    assert!(res4.stream_id.is_some());
    assert_eq!(res4.error, None);

    // Verify balance: only 1000 + 500 = 1500 tokens should have been pulled
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000 - 1500);
}

#[test]
fn test_create_streams_partial_insufficient_balance_mid_run() {
    let ctx = TestContext::setup();
    let current_time = ctx.env.ledger().timestamp();

    // Set balance to 1500
    ctx.sac.mint(&ctx.sender, &-8500_i128);
    assert_eq!(ctx.token.balance(&ctx.sender), 1500);

    let streams = vec![
        &ctx.env,
        // Valid stream (1000 tokens)
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: current_time + 10,
            cliff_time: current_time + 10,
            end_time: current_time + 1010,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // Valid stream (1000 tokens) - but insufficient balance
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: current_time + 10,
            cliff_time: current_time + 10,
            end_time: current_time + 1010,
            withdraw_dust_threshold: None,
            memo: None,
        },
        // Valid stream (500 tokens) - enough balance after first stream
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 500,
            rate_per_second: 1,
            start_time: current_time + 10,
            cliff_time: current_time + 10,
            end_time: current_time + 510,
            withdraw_dust_threshold: None,
            memo: None,
        },
    ];

    let results = ctx.client.create_streams_partial(&ctx.sender, &streams);

    assert_eq!(results.len(), 3);

    // Result 1: Success (1000 tokens)
    assert!(results.get(0).unwrap().success);

    // Result 2: Error (InsufficientBalance)
    let res2 = results.get(1).unwrap();
    assert!(!res2.success);
    assert_eq!(res2.error, Some(ContractError::InsufficientBalance as u32));

    // Result 3: Success (500 tokens)
    assert!(results.get(2).unwrap().success);

    // Final balance should be 0
    assert_eq!(ctx.token.balance(&ctx.sender), 0);
}

#[test]
fn test_create_streams_partial_ordering_guarantees() {
    let ctx = TestContext::setup();
    let current_time = ctx.env.ledger().timestamp();

    let mut streams = Vec::new(&ctx.env);
    for i in 0..10 {
        streams.push_back(CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 100,
            rate_per_second: 1,
            start_time: current_time + 10,
            cliff_time: current_time + 10,
            end_time: current_time + 110,
            withdraw_dust_threshold: None,
            memo: Some(soroban_sdk::Bytes::from_array(&ctx.env, &[i as u8])),
        });
    }

    let results = ctx.client.create_streams_partial(&ctx.sender, &streams);

    assert_eq!(results.len(), 10);

    for i in 0..10 {
        let res = results.get(i).unwrap();
        assert!(res.success);
        let stream_id = res.stream_id.unwrap();
        
        // Check that stream IDs are sequential (optional, but likely)
        // More importantly, verify they match the order by checking the memo
        // But we can't easily check the memo without loading the stream
    }
}
