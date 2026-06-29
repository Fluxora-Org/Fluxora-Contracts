// contracts/stream/tests/upgrade_path.rs
#![cfg(test)]

use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, BytesN, Env,
};

/// Test context for upgrade tests
struct UpgradeTestCtx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    admin: Address,
    token: Address,
    sender: Address,
    recipient: Address,
}

impl<'a> UpgradeTestCtx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token, &admin);

        Self {
            env,
            client,
            admin,
            token,
            sender,
            recipient,
        }
    }

    fn create_test_stream(&self) -> u64 {
        let amount = 1000_i128;
        let rate = 1_i128;
        let start_time = 1_000_000u64;
        let cliff_time = 1_000_000u64;
        let end_time = 2_000_000u64;

        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &amount,
            &rate,
            &start_time,
            &cliff_time,
            &end_time,
            &0,
            &None,
            &fluxora_stream::StreamKind::Linear,
        )
    }

    fn get_wasm_hash(&self) -> BytesN<32> {
        BytesN::from_array(&self.env, &[0u8; 32])
    }
}

/// Test that unauthorized users cannot upgrade the contract
#[test]
fn test_upgrade_unauthorized_fails() {
    let ctx = UpgradeTestCtx::setup();

    let new_hash = ctx.get_wasm_hash();
    let unauthorized = Address::generate(&ctx.env);

    let result = ctx
        .client
        .try_upgrade(&unauthorized, &new_hash);

    assert_eq!(result, Err(Ok(fluxora_stream::ContractError::Unauthorized)));
}

/// Test that admin can upgrade the contract
#[test]
fn test_upgrade_succeeds_for_admin() {
    let ctx = UpgradeTestCtx::setup();

    let new_hash = ctx.get_wasm_hash();

    let result = ctx.client.try_upgrade(&ctx.admin, &new_hash);
    assert!(result.is_ok());

    let events = ctx.env.events().all();
    assert!(events.iter().any(|e| e.0.topic0 == "upgraded"));
}

/// Test that upgrade preserves existing stream state
#[test]
fn test_upgrade_preserves_stream_state() {
    let ctx = UpgradeTestCtx::setup();

    let stream_id = ctx.create_test_stream();

    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.sender, ctx.sender);
    assert_eq!(stream.recipient, ctx.recipient);

    let new_hash = ctx.get_wasm_hash();
    ctx.client.upgrade(&ctx.admin, &new_hash);

    let stream_after = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream_after.sender, ctx.sender);
    assert_eq!(stream_after.recipient, ctx.recipient);
    assert_eq!(stream_after.deposit_amount, 1000);
}

/// Test that upgrade emits the ContractUpgraded event
#[test]
fn test_upgrade_emits_event() {
    let ctx = UpgradeTestCtx::setup();

    let new_hash = ctx.get_wasm_hash();

    ctx.env.events().clear();

    ctx.client.upgrade(&ctx.admin, &new_hash);

    let events = ctx.env.events().all();
    let upgrade_events: Vec<_> = events
        .iter()
        .filter(|e| e.0.topic0 == "upgraded")
        .collect();

    assert_eq!(upgrade_events.len(), 1);
}

/// Test that upgrade works when admin is a governance contract
#[test]
fn test_upgrade_with_governance_as_admin() {
    let ctx = UpgradeTestCtx::setup();

    let governance_addr = Address::generate(&ctx.env);
    
    let token = ctx.token.clone();
    ctx.client.init(&token, &governance_addr);

    let new_hash = ctx.get_wasm_hash();
    let result = ctx.client.try_upgrade(&governance_addr, &new_hash);
    assert!(result.is_ok());
}

/// Test that upgrade fails if contract is not initialized
#[test]
fn test_upgrade_fails_if_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_hash = BytesN::from_array(&env, &[0u8; 32]);

    let result = client.try_upgrade(&admin, &new_hash);
    assert!(result.is_err());
}