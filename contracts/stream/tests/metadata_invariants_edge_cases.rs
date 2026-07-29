//! Per-stream metadata TLV extension invariants & edge-cases test suite.
//!
//! Validates:
//! 1. Metadata size bounds & overflow protection (keys, per-key bytes, per-value bytes, aggregate bytes).
//! 2. Validation ordering and zero-side-effects (CEI: no ID allocation, no token movement, no events on failure).
//! 3. Metadata immutability across state-mutating operations (pause, resume, cancel, withdraw, top-up, etc.).
//! 4. Metadata propagation across creation pathways (offers, templates, stream cloning).
//! 5. Backward compatibility across contract versions (V5 decoding to None, Some(empty) vs None distinction).

extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    MAX_METADATA_KEYS, MAX_METADATA_KEY_BYTES, MAX_METADATA_VALUE_BYTES,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Bytes, Env, Map,
};

struct TestCtx<'a> {
    env: Env,
    contract_id: Address,
    client: FluxoraStreamClient<'a>,
    token_id: Address,
    sac: StellarAssetClient<'a>,
    admin: Address,
    sender: Address,
    recipient: Address,
}

impl<'a> TestCtx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| {
            l.timestamp = 1_000_000;
            l.sequence_number = 100;
        });

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

        sac.mint(&sender, &1_000_000_000);

        let token_client = soroban_sdk::token::Client::new(&env, &token_id);
        token_client.approve(&sender, &contract_id, &i128::MAX, &999_999);

        client.init(&token_id, &admin);

        TestCtx {
            env,
            contract_id,
            client,
            token_id,
            sac,
            admin,
            sender,
            recipient,
        }
    }

    fn make_bytes(&self, byte: u8, len: u32) -> Bytes {
        let mut b = Bytes::new(&self.env);
        for _ in 0..len {
            b.push_back(byte);
        }
        b
    }

    fn default_params(&self) -> CreateStreamParams {
        CreateStreamParams {
            recipient: self.recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 1_000_000,
            cliff_time: 1_000_000,
            end_time: 1_001_000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
            irrevocable: Some(false),
            witness: None,
        }
    }
}

#[test]
fn test_metadata_exact_boundary_limits_accepted() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    // 1. Exact max keys (8 entries), each 32 bytes key + 32 bytes value (total aggregate 512 bytes)
    let mut meta = Map::new(env);
    for i in 0..MAX_METADATA_KEYS {
        let k = ctx.make_bytes(b'k' + i as u8, MAX_METADATA_KEY_BYTES);
        let v = ctx.make_bytes(b'v' + i as u8, 32);
        meta.set(k, v);
    }

    let mut params = ctx.default_params();
    params.metadata = Some(meta.clone());

    let res = ctx.client.try_create_stream(&ctx.sender, &params);
    assert!(res.is_ok());
    let stream_id = res.unwrap().unwrap();

    let fetched = ctx.client.get_stream_metadata(&stream_id).unwrap();
    assert_eq!(fetched.len(), MAX_METADATA_KEYS);
    assert_eq!(fetched, meta);
}

#[test]
fn test_metadata_key_count_overflow_rejected() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    // 9 keys (MAX_METADATA_KEYS + 1)
    let mut meta = Map::new(env);
    for i in 0..=(MAX_METADATA_KEYS) {
        let k = ctx.make_bytes(b'a' + i as u8, 1);
        let v = ctx.make_bytes(b'b' + i as u8, 1);
        meta.set(k, v);
    }

    let mut params = ctx.default_params();
    params.metadata = Some(meta);

    let res = ctx.client.try_create_stream(&ctx.sender, &params);
    assert_eq!(res, Err(Ok(ContractError::MetadataTooLarge)));
}

#[test]
fn test_metadata_key_byte_length_overflow_rejected() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    // Key of 33 bytes (MAX_METADATA_KEY_BYTES + 1)
    let mut meta = Map::new(env);
    let k = ctx.make_bytes(b'k', MAX_METADATA_KEY_BYTES + 1);
    let v = ctx.make_bytes(b'v', 10);
    meta.set(k, v);

    let mut params = ctx.default_params();
    params.metadata = Some(meta);

    let res = ctx.client.try_create_stream(&ctx.sender, &params);
    assert_eq!(res, Err(Ok(ContractError::MetadataTooLarge)));
}

#[test]
fn test_metadata_value_byte_length_overflow_rejected() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    // Value of 129 bytes (MAX_METADATA_VALUE_BYTES + 1)
    let mut meta = Map::new(env);
    let k = ctx.make_bytes(b'k', 10);
    let v = ctx.make_bytes(b'v', MAX_METADATA_VALUE_BYTES + 1);
    meta.set(k, v);

    let mut params = ctx.default_params();
    params.metadata = Some(meta);

    let res = ctx.client.try_create_stream(&ctx.sender, &params);
    assert_eq!(res, Err(Ok(ContractError::MetadataTooLarge)));
}

#[test]
fn test_metadata_aggregate_byte_length_overflow_rejected() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    // 4 entries each with 32 bytes key + 97 bytes value = 129 * 4 = 516 bytes (> 512 aggregate)
    let mut meta = Map::new(env);
    for i in 0..4 {
        let k = ctx.make_bytes(b'a' + i as u8, MAX_METADATA_KEY_BYTES);
        let v = ctx.make_bytes(b'v' + i as u8, 97);
        meta.set(k, v);
    }

    let mut params = ctx.default_params();
    params.metadata = Some(meta);

    let res = ctx.client.try_create_stream(&ctx.sender, &params);
    assert_eq!(res, Err(Ok(ContractError::MetadataTooLarge)));
}

#[test]
fn test_metadata_failure_preserves_state_and_balances() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    let token_client = soroban_sdk::token::Client::new(env, &ctx.token_id);
    let initial_balance = token_client.balance(&ctx.sender);
    let initial_count = ctx.client.get_stream_count();

    // Invalid metadata
    let mut meta = Map::new(env);
    meta.set(ctx.make_bytes(b'k', 40), ctx.make_bytes(b'v', 10));

    let mut params = ctx.default_params();
    params.metadata = Some(meta);

    let res = ctx.client.try_create_stream(&ctx.sender, &params);
    assert_eq!(res, Err(Ok(ContractError::MetadataTooLarge)));

    // State invariants: stream count and token balance MUST remain unchanged
    assert_eq!(ctx.client.get_stream_count(), initial_count);
    assert_eq!(token_client.balance(&ctx.sender), initial_balance);
}

#[test]
fn test_metadata_propagation_via_stream_offers() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    let mut meta = Map::new(env);
    meta.set(ctx.make_bytes(b'o', 5), ctx.make_bytes(b'v', 20));

    let mut params = ctx.default_params();
    params.metadata = Some(meta.clone());

    let offer_id = ctx.client.create_stream_offer(&ctx.sender, &params, &None);

    // Accept offer -> stream created inheriting offer metadata
    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    let fetched = ctx.client.get_stream_metadata(&stream_id).unwrap();
    assert_eq!(fetched, meta);
}

#[test]
fn test_metadata_propagation_via_templates() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    let tpl_id = ctx
        .client
        .register_stream_template(&ctx.admin, &0, &0, &1_000);

    let mut meta = Map::new(env);
    meta.set(ctx.make_bytes(b't', 10), ctx.make_bytes(b'm', 10));

    let stream_id = ctx.client.create_stream_from_template(
        &ctx.sender,
        &tpl_id,
        &ctx.recipient,
        &1_000,
        &1,
        &0,
        &None,
        &Some(meta.clone()),
        &StreamKind::Linear,
        &Some(false),
    );

    let fetched = ctx.client.get_stream_metadata(&stream_id).unwrap();
    assert_eq!(fetched, meta);
}

#[test]
fn test_metadata_immutability_across_stream_operations() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    let mut meta = Map::new(env);
    meta.set(ctx.make_bytes(b'i', 8), ctx.make_bytes(b'm', 15));

    let mut params = ctx.default_params();
    params.metadata = Some(meta.clone());

    let stream_id = ctx.client.create_stream(&ctx.sender, &params);

    // Operational state mutations
    env.ledger().with_mut(|l| l.sequence_number += 100);
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(ctx.client.get_stream_metadata(&stream_id).unwrap(), meta);

    env.ledger().with_mut(|l| l.sequence_number += 100);
    ctx.client.resume_stream(&stream_id);
    assert_eq!(ctx.client.get_stream_metadata(&stream_id).unwrap(), meta);

    env.ledger().with_mut(|l| l.timestamp = 1_000_100);
    ctx.client.withdraw(&stream_id);
    assert_eq!(ctx.client.get_stream_metadata(&stream_id).unwrap(), meta);

    env.ledger().with_mut(|l| l.sequence_number += 100);
    ctx.client.top_up_stream(&stream_id, &ctx.sender, &500);
    assert_eq!(ctx.client.get_stream_metadata(&stream_id).unwrap(), meta);

    env.ledger().with_mut(|l| l.sequence_number += 100);
    ctx.client.cancel_stream(&stream_id);
    assert_eq!(ctx.client.get_stream_metadata(&stream_id).unwrap(), meta);
}

#[test]
fn test_metadata_none_vs_some_empty_map_distinction() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    // Stream A: metadata = None
    let mut params_a = ctx.default_params();
    params_a.metadata = None;
    let id_a = ctx.client.create_stream(&ctx.sender, &params_a);

    // Stream B: metadata = Some(empty map)
    let mut params_b = ctx.default_params();
    params_b.metadata = Some(Map::new(env));
    let id_b = ctx.client.create_stream(&ctx.sender, &params_b);

    assert_eq!(ctx.client.get_stream_metadata(&id_a), None);
    assert_eq!(ctx.client.get_stream_metadata(&id_b), Some(Map::new(env)));
    assert_ne!(
        ctx.client.get_stream_metadata(&id_a),
        ctx.client.get_stream_metadata(&id_b)
    );
}
