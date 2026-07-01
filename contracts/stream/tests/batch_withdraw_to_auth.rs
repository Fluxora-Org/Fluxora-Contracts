extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, StreamKind, WithdrawToParam,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
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

        TestContext {
            env,
            contract_id,
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
fn test_batch_withdraw_to_requires_recipient_auth() {
    let ctx = TestContext::setup();
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    let destination = Address::generate(&ctx.env);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id,
            destination: destination.clone(),
        },
    ];

    let results = ctx.client().batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().amount, 500);
    assert_eq!(ctx.token.balance(&destination), 500);
}

#[test]
fn test_batch_withdraw_to_mixed_recipients_reverts_atomically() {
    let ctx = TestContext::setup();
    let other_recipient = Address::generate(&ctx.env);

    let stream_id_a = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );
    let stream_id_b = ctx.client().create_stream(
        &ctx.sender,
        &other_recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id: stream_id_a,
            destination: Address::generate(&ctx.env),
        },
        WithdrawToParam {
            stream_id: stream_id_b,
            destination: Address::generate(&ctx.env),
        },
    ];

    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
    assert_eq!(
        ctx.client().get_stream_state(&stream_id_a).withdrawn_amount,
        0
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id_b).withdrawn_amount,
        0
    );
}

#[test]
fn test_batch_withdraw_to_duplicate_destinations_aggregate_transfers() {
    let ctx = TestContext::setup();
    let stream_id_a = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );
    let stream_id_b = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    let destination = Address::generate(&ctx.env);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id: stream_id_a,
            destination: destination.clone(),
        },
        WithdrawToParam {
            stream_id: stream_id_b,
            destination: destination.clone(),
        },
    ];

    let results = ctx.client().batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().amount, 500);
    assert_eq!(results.get(1).unwrap().amount, 500);
    assert_eq!(ctx.token.balance(&destination), 1000);
}

#[test]
fn test_batch_withdraw_to_rejects_contract_destination() {
    let ctx = TestContext::setup();
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id,
            destination: ctx.contract_id.clone(),
        },
    ];

    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        0
    );
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1000);
}
