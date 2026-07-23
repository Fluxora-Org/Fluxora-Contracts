//! TTL verification tests for the factory stream registry (issue #954).
//!
//! Confirms:
//! - A single `append_stream_id` call extends the registry TTL to [`PERSISTENT_BUMP_AMOUNT`].
//! - A batched `append_stream_ids_batch` call extends the registry TTL to the same target
//!   (the O(1) bump-per-batch path does not under-bump relative to the O(1) single-item path).
//! - The instance TTL is extended to [`INSTANCE_BUMP_AMOUNT`] after `init`.

extern crate std;

use fluxora_factory::{
    DataKey, FluxoraFactory, FluxoraFactoryClient, INSTANCE_BUMP_AMOUNT, PERSISTENT_BUMP_AMOUNT,
};
use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_env_host::StorageType;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, IntoVal, Val, Vec,
};

const MAX_DEPOSIT: i128 = 10_000_000;
const MIN_DURATION: u64 = 86_400;
const DEPOSIT_AMOUNT: i128 = 200_000;
const RATE_PER_SECOND: i128 = 1;
const STREAM_DURATION: u64 = 200_000;
const SENDER_FUNDING: i128 = 1_000_000_000;
const LEDGER_TIMESTAMP: u64 = 1_000_000_000;

struct Ctx {
    env: Env,
    factory: FluxoraFactoryClient<'static>,
    sender: Address,
    recipient: Address,
    factory_id: Address,
}

impl Ctx {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(LEDGER_TIMESTAMP);

        let stream_contract_id = env.register_contract(None, FluxoraStream);
        let factory_contract_id = env.register_contract(None, FluxoraFactory);

        let stream = FluxoraStreamClient::new(&env, &stream_contract_id);
        let factory = FluxoraFactoryClient::new(&env, &factory_contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        stellar_asset.mint(&sender, &SENDER_FUNDING);
        token.approve(&sender, &stream_contract_id, &SENDER_FUNDING, &100_000);

        stream.init(&token_id, &admin);
        factory.init(&admin, &stream_contract_id, &MAX_DEPOSIT, &MIN_DURATION);
        factory.set_allowlist(&recipient, &true);

        Self {
            env,
            factory,
            sender,
            recipient,
            factory_id: factory_contract_id,
        }
    }

    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }
}

/// Helper: query the persistent TTL (`live_until_ledger`) for the factory's
/// `DataKey::FactoryStreamIds` registry entry.
fn registry_live_until(env: &Env, factory_id: &Address) -> u32 {
    env.as_contract(factory_id, || {
        let key_val: Val = DataKey::FactoryStreamIds.into_val(env);
        env.host()
            .get_contract_data_live_until_ledger(key_val, StorageType::Persistent)
            .unwrap()
    })
}

/// A single `create_stream` call goes through `append_stream_id` which bumps
/// the registry entry's persistent TTL to `PERSISTENT_BUMP_AMOUNT`.
#[test]
fn test_registry_ttl_single_push() {
    let ctx = Ctx::setup();
    let seq = ctx.env.ledger().sequence();
    let expected = seq + PERSISTENT_BUMP_AMOUNT;

    let start = ctx.now();
    ctx.factory.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &DEPOSIT_AMOUNT,
        &RATE_PER_SECOND,
        &start,
        &start,
        &(start + STREAM_DURATION),
        &0,
        &StreamKind::Linear,
        &None,
    );

    let live_until = registry_live_until(&ctx.env, &ctx.factory_id);
    assert_eq!(
        live_until, expected,
        "single push must set registry TTL to PERSISTENT_BUMP_AMOUNT"
    );
}

/// A batched `create_streams` call goes through `append_stream_ids_batch` which
/// bumps the registry's persistent TTL **once** for the whole batch. The TTL
/// target must match the single-item path.
#[test]
fn test_registry_ttl_batch_push() {
    let ctx = Ctx::setup();
    let seq = ctx.env.ledger().sequence();
    let expected = seq + PERSISTENT_BUMP_AMOUNT;

    let r = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&r, &true);

    let now = ctx.now();
    let batch_size = 10u32;
    let mut streams: Vec<CreateStreamParams> = Vec::new(&ctx.env);
    for i in 0..batch_size {
        let start = now + (i as u64) * 10_000;
        streams.push_back(CreateStreamParams {
            recipient: r.clone(),
            deposit_amount: DEPOSIT_AMOUNT,
            rate_per_second: RATE_PER_SECOND,
            start_time: start,
            cliff_time: start,
            end_time: start + STREAM_DURATION,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
        });
    }

    let created_ids = ctx.factory.create_streams(&ctx.sender, &streams);
    assert_eq!(created_ids.len(), batch_size);

    let live_until = registry_live_until(&ctx.env, &ctx.factory_id);
    assert_eq!(
        live_until, expected,
        "batch push must set registry TTL to PERSISTENT_BUMP_AMOUNT (O(1) path)"
    );
}

/// The factory's instance TTL (contract instance + code) is bumped to
/// `INSTANCE_BUMP_AMOUNT` by the `bump_instance` call in `init`.
#[test]
fn test_instance_ttl() {
    let ctx = Ctx::setup();
    let seq = ctx.env.ledger().sequence();
    let expected = seq + INSTANCE_BUMP_AMOUNT;

    let live_until = ctx
        .env
        .host()
        .get_contract_instance_live_until_ledger(ctx.factory_id.to_object())
        .unwrap();
    assert_eq!(
        live_until, expected,
        "factory instance TTL must be INSTANCE_BUMP_AMOUNT after init"
    );
}
