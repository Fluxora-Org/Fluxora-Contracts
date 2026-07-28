#![cfg(test)]
extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Vec,
};

const MAX_POOL_RECIPIENTS: u32 = 100;

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    token: TokenClient<'a>,
    sender: Address,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|ledger| {
            ledger.timestamp = 1_000;
            ledger.sequence_number = 10;
        });

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        stellar_asset.mint(&sender, &1_000_000_000);
        token.approve(&sender, &contract_id, &1_000_000_000, &100_000);

        client.init(&token_id, &admin);

        Self {
            env,
            client,
            token,
            sender,
        }
    }

    fn create_pool(&self, recipients: &Vec<(Address, u32)>, deposit_amount: i128) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_pooled_stream(
            &self.sender,
            recipients,
            &deposit_amount,
            &(deposit_amount / 100),
            &now,
            &now,
            &(now + 100),
            &0,
            &None,
            &StreamKind::Linear,
        )
    }

    fn set_ledger(&self, timestamp: u64, sequence_number: u32) {
        self.env.ledger().with_mut(|ledger| {
            ledger.timestamp = timestamp;
            ledger.sequence_number = sequence_number;
        });
    }
}

fn two_recipient_pool(env: &Env, alice: &Address, bob: &Address) -> Vec<(Address, u32)> {
    vec![env, (alice.clone(), 7_000u32), (bob.clone(), 3_000u32)]
}

#[test]
fn pooled_stream_creation_indexes_and_pro_rata_withdrawal() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);
    let recipients = two_recipient_pool(&ctx.env, &alice, &bob);

    let stream_id = ctx.create_pool(&recipients, 1_000);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.is_pooled, Some(true));
    assert_eq!(state.recipient, ctx.sender);
    assert_eq!(state.deposit_amount, 1_000);

    let alice_index = ctx.client.get_recipient_streams(&alice);
    let bob_index = ctx.client.get_recipient_streams(&bob);
    assert!(alice_index.contains(stream_id));
    assert!(bob_index.contains(stream_id));

    let sender_health = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &100u32);
    assert!(sender_health.stream_ids.contains(stream_id));

    ctx.set_ledger(1_050, 20);
    assert_eq!(ctx.client.withdraw_from_pool(&stream_id, &alice), 350);
    assert_eq!(ctx.client.withdraw_from_pool(&stream_id, &bob), 150);
    assert_eq!(ctx.token.balance(&alice), 350);
    assert_eq!(ctx.token.balance(&bob), 150);

    ctx.set_ledger(1_100, 30);
    assert_eq!(ctx.client.withdraw_from_pool(&stream_id, &alice), 350);
    assert_eq!(ctx.client.withdraw_from_pool(&stream_id, &bob), 150);
    assert_eq!(ctx.token.balance(&alice), 700);
    assert_eq!(ctx.token.balance(&bob), 300);

    let final_state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(final_state.withdrawn_amount, 1_000);
    assert_eq!(final_state.status, StreamStatus::Completed);
}

#[test]
fn pooled_stream_rounds_down_each_member_share() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);
    let recipients = vec![&ctx.env, (alice.clone(), 1u32), (bob.clone(), 2u32)];

    let stream_id = ctx.create_pool(&recipients, 100);
    ctx.set_ledger(1_100, 20);

    assert_eq!(ctx.client.withdraw_from_pool(&stream_id, &alice), 33);
    assert_eq!(ctx.client.withdraw_from_pool(&stream_id, &bob), 66);
    assert_eq!(ctx.token.balance(&alice), 33);
    assert_eq!(ctx.token.balance(&bob), 66);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 99);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn pooled_stream_rejects_invalid_share_tables() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);

    let empty = Vec::new(&ctx.env);
    let err = ctx
        .client
        .try_create_pooled_stream(
            &ctx.sender,
            &empty,
            &1_000,
            &10,
            &1_000,
            &1_000,
            &1_100,
            &0,
            &None,
            &StreamKind::Linear,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);

    let zero_share = vec![&ctx.env, (alice.clone(), 0u32)];
    let err = ctx
        .client
        .try_create_pooled_stream(
            &ctx.sender,
            &zero_share,
            &1_000,
            &10,
            &1_000,
            &1_000,
            &1_100,
            &0,
            &None,
            &StreamKind::Linear,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);

    let duplicate = vec![
        &ctx.env,
        (alice.clone(), 6_000u32),
        (alice.clone(), 4_000u32),
    ];
    let err = ctx
        .client
        .try_create_pooled_stream(
            &ctx.sender,
            &duplicate,
            &1_000,
            &10,
            &1_000,
            &1_000,
            &1_100,
            &0,
            &None,
            &StreamKind::Linear,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);

    let mut too_many = Vec::new(&ctx.env);
    for _ in 0..=MAX_POOL_RECIPIENTS {
        too_many.push_back((Address::generate(&ctx.env), 1u32));
    }
    let err = ctx
        .client
        .try_create_pooled_stream(
            &ctx.sender,
            &too_many,
            &1_000,
            &10,
            &1_000,
            &1_000,
            &1_100,
            &0,
            &None,
            &StreamKind::Linear,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);

    let valid = vec![&ctx.env, (alice, 1u32), (bob, 1u32)];
    assert_eq!(ctx.create_pool(&valid, 1_000), 0);
}

#[test]
fn pooled_stream_rejects_non_member_withdrawal() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let outsider = Address::generate(&ctx.env);
    let recipients = vec![&ctx.env, (alice, 10_000u32)];

    let stream_id = ctx.create_pool(&recipients, 1_000);
    ctx.set_ledger(1_050, 20);

    let err = ctx
        .client
        .try_withdraw_from_pool(&stream_id, &outsider)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::Unauthorized);
}
