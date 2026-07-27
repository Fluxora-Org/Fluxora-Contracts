extern crate std;

// Issue #900: metadata length/encoding limits coverage for
// contracts/stream/tests/metadata_extension.rs (referenced by PR_580_METADATA_EXTENSION.md).
//
// The single `create_stream` entry point does NOT accept metadata; metadata is only
// attachable through the batch `create_streams` / `create_streams_relative` entry points
// (see CreateStreamParams / CreateStreamRelativeParams which carry a `metadata` field).
// These tests exercise the storage/validation limits and the event-payload behavior
// using the correct API surface.

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamCreated,
    MAX_METADATA_BYTES, MAX_METADATA_KEY_BYTES, MAX_METADATA_VALUE_BYTES,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Bytes, Env, Map, TryFromVal,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(0);

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
        sac.mint(&sender, &100_000_i128);
        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &6_000_000);

        Ctx {
            env,
            client,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.client.address)
    }

    fn make_key(&self, s: &str) -> Bytes {
        Bytes::from_slice(&self.env, s.as_bytes())
    }

    fn make_val(&self, s: &str) -> Bytes {
        Bytes::from_slice(&self.env, s.as_bytes())
    }

    fn single_param(&self, metadata: Option<Map<Bytes, Bytes>>) -> CreateStreamParams {
        CreateStreamParams {
            recipient: self.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata,
            kind: fluxora_stream::StreamKind::Linear,
        }
    }

    fn create_one(&self, metadata: Option<Map<Bytes, Bytes>>) -> u64 {
        let params = soroban_sdk::vec![&self.env, self.single_param(metadata)];
        self.client()
            .create_streams(&self.sender, &params)
            .get(0)
            .unwrap()
    }
}

// ---------------------------------------------------------------------------
// Oversized / limit rejection (already documented in storage.rs validation)
// ---------------------------------------------------------------------------

#[test]
fn metadata_too_many_keys_rejected() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0..(fluxora_stream::MAX_METADATA_KEYS + 1) {
        meta.set(ctx.make_key(&std::format!("k{}", i)), ctx.make_val("v"));
    }
    let result = ctx.client().try_create_streams(
        &ctx.sender,
        &soroban_sdk::vec![&ctx.env, ctx.single_param(Some(meta))],
    );
    assert!(
        matches!(result, Err(Ok(ContractError::MetadataTooLarge))),
        "too many keys must be rejected"
    );
}

#[test]
fn metadata_value_exceeds_limit_rejected() {
    let ctx = Ctx::setup();
    let key = ctx.make_key("k");
    let value = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_VALUE_BYTES + 1) as usize].as_slice(),
    );
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(key, value);
    let result = ctx.client().try_create_streams(
        &ctx.sender,
        &soroban_sdk::vec![&ctx.env, ctx.single_param(Some(meta))],
    );
    assert!(
        matches!(result, Err(Ok(ContractError::MetadataTooLarge))),
        "oversized value must be rejected"
    );
}

#[test]
fn metadata_aggregate_exceeds_limit_rejected() {
    let ctx = Ctx::setup();
    // 5 entries × (8-byte key + 120-byte value) = 640 > MAX_METADATA_BYTES (512).
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..5 {
        let key_str = std::format!("key{:05}", i);
        meta.set(
            Bytes::from_slice(&ctx.env, key_str.as_bytes()),
            Bytes::from_slice(&ctx.env, &vec![i; 120].as_slice()),
        );
    }
    let result = ctx.client().try_create_streams(
        &ctx.sender,
        &soroban_sdk::vec![&ctx.env, ctx.single_param(Some(meta))],
    );
    assert!(
        matches!(result, Err(Ok(ContractError::MetadataTooLarge))),
        "aggregate overflow must be rejected"
    );
}

// ---------------------------------------------------------------------------
// Edge-case byte content: invalid UTF-8 must round-trip as opaque bytes
// ---------------------------------------------------------------------------

#[test]
fn metadata_invalid_utf8_stored_and_retrieved() {
    let ctx = Ctx::setup();
    let key = ctx.make_key("raw");
    let invalid_utf8 = Bytes::from_slice(&ctx.env, &[0xFFu8, 0xFEu8]);
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(key.clone(), invalid_utf8.clone());

    let stream_id = ctx.create_one(Some(meta));
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    let stored = got.get(key).expect("key must exist");
    assert_eq!(
        stored, invalid_utf8,
        "invalid-UTF8 metadata must round-trip byte-for-byte"
    );
}

#[test]
fn metadata_empty_map_valid() {
    let ctx = Ctx::setup();
    let empty: Map<Bytes, Bytes> = Map::new(&ctx.env);
    let stream_id = ctx.create_one(Some(empty));
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.len(),
        0,
        "Some(empty map) must round-trip as Some(empty)"
    );
}

#[test]
fn metadata_none_is_none() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_one(None);
    assert!(
        ctx.client().get_stream_metadata(&stream_id).is_none(),
        "absent metadata must be None"
    );
}

// ---------------------------------------------------------------------------
// Event payload: confirm metadata reflection (issue requirement)
// ---------------------------------------------------------------------------

/// Issue #900 requires confirming whether metadata is included in the
/// `StreamCreated` event payload. Issue #900 requires confirming whether
/// metadata is included in the event. With this change the contract validates
/// and stores metadata, and the `StreamCreated` event now carries the same
/// metadata map, so consumers can read it without a second `get_stream_metadata`
/// call.
#[test]
fn metadata_included_in_created_event_and_stored() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("k"), ctx.make_val("v"));

    let stream_id = ctx.create_one(Some(meta.clone()));

    // Stored metadata is correct and independent of the event payload.
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(got.get(ctx.make_key("k")).unwrap(), ctx.make_val("v"));

    // Event payload field is intentionally None (see docs/streaming.md gap note).
    let events = ctx.env.events().all();
    let created = events
        .iter()
        .find(|(_, topics, _)| {
            topics.len() >= 1
                && soroban_sdk::Symbol::try_from_val(&ctx.env, &topics.get(0).unwrap())
                    .map(|s| s == symbol_short!("created"))
                    .unwrap_or(false)
        })
        .expect("created event must be emitted");
    let (_addr, _topics, payload) = created;
    let sc = StreamCreated::try_from_val(&ctx.env, &payload).expect("payload is StreamCreated");
    assert_eq!(
        sc.metadata,
        Some(meta),
        "StreamCreated.event.metadata must carry the stored metadata map"
    );
}
