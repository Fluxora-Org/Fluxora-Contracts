#![cfg(test)]

use fluxora_stream::{
    ContractError, CreateStreamParams, CreateStreamRelativeParams, FluxoraStream,
    FluxoraStreamClient, StreamKind,
};
use soroban_sdk::{
    testutils::Address as _, testutils::Ledger as _, token::Client as TokenClient, Address, Env,
    Vec,
};

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

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
        }
    }
}

#[test]
fn test_persist_new_stream_irrevocable_some_true() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 100,
            cliff_time: 100,
            end_time: 1100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
            irrevocable: Some(true),
            witness: None,
        },
    );

    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.irrevocable, Some(true));

    // Assert that cancel_stream fails because stream is irrevocable
    let res = ctx.client.try_cancel_stream(&stream_id);
    assert_eq!(res, Err(Ok(ContractError::Unauthorized)));
}

#[test]
fn test_persist_new_stream_irrevocable_some_false() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 100,
            cliff_time: 100,
            end_time: 1100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
            irrevocable: Some(false),
            witness: None,
        },
    );

    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.irrevocable, Some(false));

    // Assert cancel_stream succeeds when irrevocable is Some(false)
    let res = ctx.client.try_cancel_stream(&stream_id);
    assert!(res.is_ok());
}

#[test]
fn test_persist_new_stream_irrevocable_none() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 100,
            cliff_time: 100,
            end_time: 1100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
            irrevocable: None,
            witness: None,
        },
    );

    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.irrevocable, None);

    // Assert cancel_stream succeeds when irrevocable is None
    let res = ctx.client.try_cancel_stream(&stream_id);
    assert!(res.is_ok());
}

#[test]
fn test_create_stream_relative_irrevocable() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client.create_stream_relative(
        &ctx.sender,
        &CreateStreamRelativeParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
            irrevocable: Some(true),
        },
    );

    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.irrevocable, Some(true));
}

#[test]
fn test_create_streams_batch_irrevocable() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let mut params_vec = Vec::new(&ctx.env);
    params_vec.push_back(CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 100,
        cliff_time: 100,
        end_time: 1100,
        withdraw_dust_threshold: Some(0),
        memo: None,
        kind: StreamKind::Linear,
        metadata: None,
        irrevocable: Some(true),
        witness: None,
    });

    let stream_ids = ctx.client.create_streams(&ctx.sender, &params_vec);
    assert_eq!(stream_ids.len(), 1);
    let stream_id = stream_ids.get(0).unwrap();

    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.irrevocable, Some(true));
}

#[test]
fn test_renew_stream_inherits_irrevocable() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 100,
            cliff_time: 100,
            end_time: 1100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
            irrevocable: Some(true),
            witness: None,
        },
    );

    ctx.client.set_auto_renew(&stream_id, &ctx.sender, &true);
    ctx.env.ledger().set_timestamp(1100);

    let new_stream_id = ctx.client.renew_stream(&stream_id);
    let renewed_stream = ctx.client.get_stream_state(&new_stream_id);
    assert_eq!(renewed_stream.irrevocable, Some(true));
}

#[test]
fn test_clone_stream_inherits_irrevocable() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 100,
            cliff_time: 100,
            end_time: 1100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
            irrevocable: Some(true),
            witness: None,
        },
    );

    let new_recipient = Address::generate(&ctx.env);
    let new_stream_id = ctx.client.clone_stream(
        &stream_id,
        &new_recipient,
        &0u64,
        &1000u64,
        &1000_i128,
        &false,
    );
    let cloned_stream = ctx.client.get_stream_state(&new_stream_id);
    assert_eq!(cloned_stream.irrevocable, Some(true));
}
