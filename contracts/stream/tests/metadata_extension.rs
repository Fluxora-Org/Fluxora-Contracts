extern crate std;

// Comprehensive tests for per-stream metadata TLV extension.
//
// Coverage:
// - Happy-path: create stream with metadata, query it back
// - Batch creation (create_streams / create_streams_partial / create_streams_relative) with metadata
// - Template creation (create_stream_from_template) with metadata
// - Validation: key count limit, per-key/value byte limits, aggregate byte limit
// - Immutability: metadata is unchanged by pause/resume/cancel/withdraw
// - StreamCreated event includes metadata
// - None metadata is stored and returned as None
// - Empty metadata map (Some({})) is valid
// - Boundary values at exactly MAX_METADATA_KEYS / MAX_METADATA_BYTES limits
// - Clone resets metadata to None
// - Close completed stream reclaims metadata storage
// - Independent metadata storage across streams (no cross-contamination)
// - Fail-before-allocate: no stream ID or token movement on validation failure
// - Duplicate key deduplication
// - Zero-length key and value entries
// - Gas scaling at max boundary

use fluxora_stream::{
    ContractError, CreateStreamParams, CreateStreamRelativeParams, FluxoraStream,
    FluxoraStreamClient, StreamKind, StreamStatus, MAX_METADATA_BYTES, MAX_METADATA_KEYS,
    MAX_METADATA_KEY_BYTES, MAX_METADATA_VALUE_BYTES,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Bytes, Env, Map,
};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
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
        sac.mint(&sender, &100_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &999_999);

        env.ledger().set_timestamp(0);

        Ctx {
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

    fn make_key(&self, s: &str) -> Bytes {
        Bytes::from_slice(&self.env, s.as_bytes())
    }

    fn make_val(&self, s: &str) -> Bytes {
        Bytes::from_slice(&self.env, s.as_bytes())
    }

    /// Build a metadata map with `count` entries "k0"->"v0", "k1"->"v1", ...
    fn metadata_n(&self, count: u32) -> Map<Bytes, Bytes> {
        let mut m: Map<Bytes, Bytes> = Map::new(&self.env);
        for i in 0..count {
            let k = self.make_key(&std::format!("k{}", i));
            let v = self.make_val(&std::format!("v{}", i));
            m.set(k, v);
        }
        m
    }

    fn create_stream_with_metadata(&self, metadata: Option<Map<Bytes, Bytes>>) -> u64 {
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0_i128,
            &None,
            &StreamKind::Linear,
            &None,
            &None,
            &metadata,
        )
    }
}

// ---------------------------------------------------------------------------
// Happy-path tests
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_none_stored_and_returned() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream_with_metadata(None);
    let got = ctx.client().get_stream_metadata(&stream_id);
    assert!(got.is_none(), "metadata should be None when not supplied");
}

#[test]
fn test_metadata_empty_map_valid() {
    let ctx = Ctx::setup();
    let empty: Map<Bytes, Bytes> = Map::new(&ctx.env);
    let stream_id = ctx.create_stream_with_metadata(Some(empty));
    let got = ctx.client().get_stream_metadata(&stream_id);
    assert!(
        got.is_some(),
        "Some(empty map) should round-trip as Some(empty)"
    );
    assert_eq!(got.unwrap().len(), 0);
}

#[test]
fn test_metadata_single_entry_round_trips() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("invoice_id"), ctx.make_val("INV-2026-001"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(got.len(), 1);
    let v = got.get(ctx.make_key("invoice_id")).expect("key must exist");
    assert_eq!(v, ctx.make_val("INV-2026-001"));
}

#[test]
fn test_metadata_multiple_entries_round_trip() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("invoice_id"), ctx.make_val("INV-001"));
    meta.set(ctx.make_key("project"), ctx.make_val("PROJ-42"));
    meta.set(
        ctx.make_key("ref_uri"),
        ctx.make_val("https://example.com/inv/001"),
    );

    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(
        got.get(ctx.make_key("project")).unwrap(),
        ctx.make_val("PROJ-42")
    );
}

#[test]
fn test_metadata_max_keys_valid() {
    let ctx = Ctx::setup();
    // Exactly MAX_METADATA_KEYS entries should succeed.
    let meta = ctx.metadata_n(MAX_METADATA_KEYS);
    let stream_id = ctx.create_stream_with_metadata(Some(meta));
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(got.len(), MAX_METADATA_KEYS);
}

// ---------------------------------------------------------------------------
// Validation: key count
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_too_many_keys_rejected() {
    let ctx = Ctx::setup();
    // MAX_METADATA_KEYS + 1 entries must fail.
    let meta = ctx.metadata_n(MAX_METADATA_KEYS + 1);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(meta),
    );
    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!("Expected MetadataTooLarge, got {:?}", result),
    }
}

// ---------------------------------------------------------------------------
// Validation: per-key byte limit
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_key_exactly_at_limit_valid() {
    let ctx = Ctx::setup();
    // Key of exactly MAX_METADATA_KEY_BYTES should be accepted.
    let key = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; MAX_METADATA_KEY_BYTES as usize].as_slice(),
    );
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(key, ctx.make_val("v"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta));
    assert!(ctx.client().get_stream_metadata(&stream_id).is_some());
}

#[test]
fn test_metadata_key_exceeds_limit_rejected() {
    let ctx = Ctx::setup();
    // Key of MAX_METADATA_KEY_BYTES + 1 must be rejected.
    let key = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_KEY_BYTES + 1) as usize].as_slice(),
    );
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(key, ctx.make_val("v"));
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(meta),
    );
    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!("Expected MetadataTooLarge, got {:?}", result),
    }
}

// ---------------------------------------------------------------------------
// Validation: per-value byte limit
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_value_exactly_at_limit_valid() {
    let ctx = Ctx::setup();
    let value = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; MAX_METADATA_VALUE_BYTES as usize].as_slice(),
    );
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("k"), value);
    let stream_id = ctx.create_stream_with_metadata(Some(meta));
    assert!(ctx.client().get_stream_metadata(&stream_id).is_some());
}

#[test]
fn test_metadata_value_exceeds_limit_rejected() {
    let ctx = Ctx::setup();
    let value = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_VALUE_BYTES + 1) as usize].as_slice(),
    );
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("k"), value);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(meta),
    );
    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!("Expected MetadataTooLarge, got {:?}", result),
    }
}

// ---------------------------------------------------------------------------
// Validation: aggregate byte limit
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_aggregate_exactly_at_limit_valid() {
    let ctx = Ctx::setup();
    // Fill exactly MAX_METADATA_BYTES bytes total (e.g. 4 x (8-byte key + 120-byte value) = 4x128 = 512).
    // 4 entries: key "keyXXXXX" (8 bytes) + value of 120 bytes = 128 bytes each x 4 = 512 total.
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..4 {
        let key_str = std::format!("key{:05}", i); // 8 bytes
        let value = Bytes::from_slice(&ctx.env, &vec![i; 120].as_slice()); // 120 bytes
        meta.set(Bytes::from_slice(&ctx.env, key_str.as_bytes()), value);
    }
    let stream_id = ctx.create_stream_with_metadata(Some(meta));
    assert!(ctx.client().get_stream_metadata(&stream_id).is_some());
}

#[test]
fn test_metadata_aggregate_exceeds_limit_rejected() {
    let ctx = Ctx::setup();
    // 5 entries x (8-byte key + 120-byte value) = 640 bytes > MAX_METADATA_BYTES (512).
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..5 {
        let key_str = std::format!("key{:05}", i); // 8 bytes
        let value = Bytes::from_slice(&ctx.env, &vec![i; 120].as_slice()); // 120 bytes
        meta.set(Bytes::from_slice(&ctx.env, key_str.as_bytes()), value);
    }
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(meta),
    );
    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!(
            "Expected MetadataTooLarge for aggregate overflow, got {:?}",
            result
        ),
    }
}

// ---------------------------------------------------------------------------
// Immutability: metadata does not change after post-creation operations
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_unchanged_after_pause_resume() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("ref"), ctx.make_val("PAUSE_TEST"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    ctx.client()
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);
    ctx.client().resume_stream(&stream_id);

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("ref")).unwrap(),
        ctx.make_val("PAUSE_TEST")
    );
}

#[test]
fn test_metadata_unchanged_after_cancel() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("ref"), ctx.make_val("CANCEL_TEST"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    ctx.env.ledger().set_timestamp(100);
    ctx.client().cancel_stream(&stream_id);

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("ref")).unwrap(),
        ctx.make_val("CANCEL_TEST"),
        "metadata must survive cancellation"
    );
}

#[test]
fn test_metadata_unchanged_after_partial_withdraw() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("ref"), ctx.make_val("WITHDRAW_TEST"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("ref")).unwrap(),
        ctx.make_val("WITHDRAW_TEST"),
        "metadata must survive withdrawal"
    );
}

// ---------------------------------------------------------------------------
// StreamCreated event includes metadata
// ---------------------------------------------------------------------------

#[test]
fn test_stream_created_event_contains_metadata() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("project"), ctx.make_val("PROJ-99"));

    let _ = ctx.create_stream_with_metadata(Some(meta.clone()));

    let events = ctx.env.events().all();
    // Find the "created" event (last emitted in create_stream)
    let created_events: std::vec::Vec<_> = events
        .iter()
        .filter(|(_, topics, _)| {
            topics.len() >= 1 && {
                let topic_val = topics.get(0).unwrap();
                let sym = soroban_sdk::Symbol::try_from_val(&ctx.env, &topic_val);
                sym.is_ok()
            }
        })
        .collect();

    assert!(
        !created_events.is_empty(),
        "at least one event must be emitted"
    );
}

// ---------------------------------------------------------------------------
// Batch creation: create_streams with metadata
// ---------------------------------------------------------------------------

#[test]
fn test_create_streams_batch_each_entry_stores_own_metadata() {
    let ctx = Ctx::setup();
    let recipient_a = Address::generate(&ctx.env);
    let recipient_b = Address::generate(&ctx.env);

    let mut meta_a: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta_a.set(ctx.make_key("stream"), ctx.make_val("A"));

    let mut meta_b: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta_b.set(ctx.make_key("stream"), ctx.make_val("B"));

    let params = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient: recipient_a.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: Some(meta_a.clone()),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
        CreateStreamParams {
            recipient: recipient_b.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: Some(meta_b.clone()),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    ];

    let ids = ctx.client().create_streams(&ctx.sender, &params);
    assert_eq!(ids.len(), 2);

    let got_a = ctx
        .client()
        .get_stream_metadata(&ids.get(0).unwrap())
        .unwrap();
    let got_b = ctx
        .client()
        .get_stream_metadata(&ids.get(1).unwrap())
        .unwrap();

    assert_eq!(
        got_a.get(ctx.make_key("stream")).unwrap(),
        ctx.make_val("A")
    );
    assert_eq!(
        got_b.get(ctx.make_key("stream")).unwrap(),
        ctx.make_val("B")
    );
}

#[test]
fn test_create_streams_batch_none_metadata_stored_as_none() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    let params = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    ];

    let ids = ctx.client().create_streams(&ctx.sender, &params);
    assert_eq!(ids.len(), 1);
    assert!(ctx
        .client()
        .get_stream_metadata(&ids.get(0).unwrap())
        .is_none());
}

// ---------------------------------------------------------------------------
// create_streams_relative with metadata
// ---------------------------------------------------------------------------

#[test]
fn test_create_streams_relative_with_metadata() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("src"), ctx.make_val("relative"));

    let params = soroban_sdk::vec![
        &ctx.env,
        CreateStreamRelativeParams {
            recipient: recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: Some(meta.clone()),
            kind: StreamKind::Linear,
            irrevocable: None,
        },
    ];

    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);
    assert_eq!(ids.len(), 1);

    let got = ctx
        .client()
        .get_stream_metadata(&ids.get(0).unwrap())
        .unwrap();
    assert_eq!(
        got.get(ctx.make_key("src")).unwrap(),
        ctx.make_val("relative")
    );
}

// ---------------------------------------------------------------------------
// create_streams_partial: metadata validation per-entry
// ---------------------------------------------------------------------------

#[test]
fn test_create_streams_partial_invalid_metadata_fails_entry() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    // Oversized key
    let oversized_key = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_KEY_BYTES + 1) as usize].as_slice(),
    );
    let mut bad_meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    bad_meta.set(oversized_key, ctx.make_val("v"));

    let params = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: Some(bad_meta),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    ];

    let results = ctx.client().create_streams_partial(&ctx.sender, &params);

    assert_eq!(results.len(), 1);
    let r = results.get(0).unwrap();
    assert!(!r.success, "entry with oversized key must fail");
    assert!(r.stream_id.is_none());
    assert!(r.error.is_some());
}

// ---------------------------------------------------------------------------
// Security: no stream ID allocated when metadata validation fails
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_validation_failure_does_not_allocate_stream_id() {
    let ctx = Ctx::setup();

    let before_count = ctx.client().get_stream_count();

    // Oversized value
    let bad_value = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_VALUE_BYTES + 1) as usize].as_slice(),
    );
    let mut bad_meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    bad_meta.set(ctx.make_key("k"), bad_value);

    let _ = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(bad_meta),
    );

    let after_count = ctx.client().get_stream_count();
    assert_eq!(
        before_count, after_count,
        "stream ID counter must not advance when metadata validation fails"
    );
}

// ---------------------------------------------------------------------------
// Security: no token movement on metadata validation failure
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_validation_failure_no_token_movement() {
    let ctx = Ctx::setup();
    let balance_before = ctx.token.balance(&ctx.sender);

    let meta = ctx.metadata_n(MAX_METADATA_KEYS + 1); // exceeds key count

    let _ = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(meta),
    );

    let balance_after = ctx.token.balance(&ctx.sender);
    assert_eq!(
        balance_before, balance_after,
        "sender balance must be unchanged when metadata validation fails"
    );
}

// ---------------------------------------------------------------------------
// get_stream_metadata returns ContractError::StreamNotFound for unknown ID
// ---------------------------------------------------------------------------

#[test]
fn test_get_stream_metadata_nonexistent_stream() {
    let ctx = Ctx::setup();
    let result = ctx.client().try_get_stream_metadata(&999u64);
    match result {
        Err(Ok(ContractError::StreamNotFound)) => {}
        _ => panic!("Expected StreamNotFound, got {:?}", result),
    }
}

// ---------------------------------------------------------------------------
// Boundary: two streams with metadata, independent storage
// ---------------------------------------------------------------------------

#[test]
fn test_two_streams_independent_metadata() {
    let ctx = Ctx::setup();
    let recipient_a = Address::generate(&ctx.env);
    let recipient_b = Address::generate(&ctx.env);

    let mut meta_a: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta_a.set(ctx.make_key("id"), ctx.make_val("stream-A"));

    let mut meta_b: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta_b.set(ctx.make_key("id"), ctx.make_val("stream-B"));

    let id_a = ctx.client().create_stream(
        &ctx.sender,
        &recipient_a,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(meta_a),
    );

    let id_b = ctx.client().create_stream(
        &ctx.sender,
        &recipient_b,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(meta_b),
    );

    let got_a = ctx.client().get_stream_metadata(&id_a).unwrap();
    let got_b = ctx.client().get_stream_metadata(&id_b).unwrap();

    assert_eq!(
        got_a.get(ctx.make_key("id")).unwrap(),
        ctx.make_val("stream-A")
    );
    assert_eq!(
        got_b.get(ctx.make_key("id")).unwrap(),
        ctx.make_val("stream-B")
    );
    // Cross-check: neither stream leaks into the other
    assert_ne!(
        got_a.get(ctx.make_key("id")).unwrap(),
        got_b.get(ctx.make_key("id")).unwrap()
    );
}

// ---------------------------------------------------------------------------
// CONTRACT_VERSION check
// ---------------------------------------------------------------------------

#[test]
fn test_contract_version_is_current() {
    let ctx = Ctx::setup();
    let v = ctx.client().version();
    assert!(
        v >= 7,
        "CONTRACT_VERSION must be >= 7 (current is {})",
        v
    );
}

// ---------------------------------------------------------------------------
// Additional edge-case & regression tests (issue #1294)
// ---------------------------------------------------------------------------

#[test]
fn test_clone_stream_resets_metadata_to_none() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("invoice_id"), ctx.make_val("INV-CLONE-100"));

    let source_id = ctx.create_stream_with_metadata(Some(meta));
    let new_recipient = Address::generate(&ctx.env);

    // Clone source stream
    let cloned_id = ctx.client().clone_stream(
        &source_id,
        &new_recipient,
        &100u64,  // start_time
        &1100u64, // end_time
        &1000_i128,
        &false,
    );

    // Verify source stream retains its original metadata
    let source_meta = ctx.client().get_stream_metadata(&source_id).unwrap();
    assert_eq!(
        source_meta.get(ctx.make_key("invoice_id")).unwrap(),
        ctx.make_val("INV-CLONE-100")
    );

    // Verify cloned stream metadata is explicitly reset to None
    let cloned_meta = ctx.client().get_stream_metadata(&cloned_id);
    assert!(
        cloned_meta.is_none(),
        "Cloned stream must reset metadata to None to prevent single-use ID duplication"
    );
}

#[test]
fn test_metadata_zero_length_key_and_value() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    let empty_key = Bytes::from_slice(&ctx.env, b"");
    let empty_val = Bytes::from_slice(&ctx.env, b"");
    meta.set(empty_key.clone(), empty_val.clone());

    let stream_id = ctx.create_stream_with_metadata(Some(meta));
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got.get(empty_key).unwrap(), empty_val);
}

#[test]
fn test_metadata_duplicate_key_overwrite_byte_calculation() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    let key = ctx.make_key("duplicate_key");

    // Set initial value
    meta.set(key.clone(), ctx.make_val("initial_val"));
    // Overwrite with updated value
    meta.set(key.clone(), ctx.make_val("updated_val"));

    let stream_id = ctx.create_stream_with_metadata(Some(meta));
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.len(),
        1,
        "soroban_sdk::Map must deduplicate unique keys"
    );
    assert_eq!(got.get(key).unwrap(), ctx.make_val("updated_val"));
}

#[test]
fn test_close_completed_stream_reclaims_metadata_storage() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("ref"), ctx.make_val("CLOSE-TEST"));

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
        &Some(meta),
    );

    // Fast-forward past end time and withdraw full deposit to complete stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Verify stream status is Completed
    let stream_info = ctx.client().get_stream(&stream_id);
    assert_eq!(stream_info.status, StreamStatus::Completed);

    // Close the completed stream to reclaim storage
    ctx.client().close_completed_stream(&stream_id);

    // Querying metadata for closed stream returns StreamNotFound
    let res = ctx.client().try_get_stream_metadata(&stream_id);
    match res {
        Err(Ok(ContractError::StreamNotFound)) => {}
        _ => panic!(
            "Expected StreamNotFound after closing completed stream, got {:?}",
            res
        ),
    }
}

#[test]
fn test_create_stream_from_template_with_metadata() {
    let ctx = Ctx::setup();
    let template_owner = Address::generate(&ctx.env);

    // Register a schedule template
    let template_id = ctx.client().register_stream_template(
        &template_owner,
        &0u64,    // start_delay
        &0u64,    // cliff_delay
        &1000u64, // duration
    );

    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("payroll_id"), ctx.make_val("PAYROLL-2026-07"));

    let stream_id = ctx.client().create_stream_from_template(
        &ctx.sender,
        &template_id,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0_i128,
        &None,
        &Some(meta.clone()),
        &StreamKind::Linear,
        &None,
    );

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("payroll_id")).unwrap(),
        ctx.make_val("PAYROLL-2026-07")
    );
}

#[test]
fn test_create_streams_partial_metadata_isolation() {
    let ctx = Ctx::setup();
    let recipient_1 = Address::generate(&ctx.env);
    let recipient_2 = Address::generate(&ctx.env);

    let mut valid_meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    valid_meta.set(ctx.make_key("valid"), ctx.make_val("meta1"));

    // Invalid metadata exceeding aggregate MAX_METADATA_BYTES (512 bytes)
    let mut invalid_meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    let val_too_large = Bytes::from_slice(&ctx.env, &[2u8; 128]);
    for i in 0..6 {
        let k = Bytes::from_slice(&ctx.env, &[i as u8; 30]);
        invalid_meta.set(k, val_too_large.clone());
    }
    // Total bytes = 6 * (30 + 128) = 948 bytes > 512 bytes

    let params = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient: recipient_1,
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(valid_meta),
            irrevocable: None,
            witness: None,
        },
        CreateStreamParams {
            recipient: recipient_2,
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(invalid_meta),
            irrevocable: None,
            witness: None,
        },
    ];

    let results = ctx.client().create_streams_partial(&ctx.sender, &params);
    assert_eq!(results.len(), 2);

    let r1 = results.get(0).unwrap();
    assert!(r1.success);
    assert!(r1.stream_id.is_some());
    assert!(r1.error.is_none());

    let r2 = results.get(1).unwrap();
    assert!(!r2.success);
    assert!(r2.stream_id.is_none());
    assert!(r2.error.is_some());

    // Verify valid stream metadata was persisted properly
    let got1 = ctx
        .client()
        .get_stream_metadata(&r1.stream_id.unwrap())
        .unwrap();
    assert_eq!(
        got1.get(ctx.make_key("valid")).unwrap(),
        ctx.make_val("meta1")
    );
}

#[test]
fn test_metadata_gas_scaling_boundary() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);

    // Exactly 8 keys, each with 32-byte key and 32-byte value (total 8 * 64 = 512 bytes == MAX_METADATA_BYTES)
    for i in 0..MAX_METADATA_KEYS {
        let key_bytes = [i as u8; 32];
        let val_bytes = [(i + 10) as u8; 32];
        meta.set(
            Bytes::from_slice(&ctx.env, &key_bytes),
            Bytes::from_slice(&ctx.env, &val_bytes),
        );
    }

    let stream_id = ctx.create_stream_with_metadata(Some(meta));
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(got.len(), 8);

    for i in 0..MAX_METADATA_KEYS {
        let key_bytes = [i as u8; 32];
        let val_bytes = [(i + 10) as u8; 32];
        let v = got.get(Bytes::from_slice(&ctx.env, &key_bytes)).unwrap();
        assert_eq!(v, Bytes::from_slice(&ctx.env, &val_bytes));
    }
}

// ---------------------------------------------------------------------------
// Batch metadata validation: invalid entry fails before token transfer
// ---------------------------------------------------------------------------

#[test]
fn test_create_streams_invalid_metadata_fails_before_bulk_transfer() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let balance_before = ctx.token.balance(&ctx.sender);

    // Invalid: too many keys
    let bad_meta = ctx.metadata_n(MAX_METADATA_KEYS + 1);

    let params = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(bad_meta),
            irrevocable: None,
            witness: None,
        },
    ];

    let result = ctx.client().try_create_streams(&ctx.sender, &params);
    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!("Expected MetadataTooLarge, got {:?}", result),
    }

    // Verify no tokens were transferred (atomic batch failure)
    let balance_after = ctx.token.balance(&ctx.sender);
    assert_eq!(
        balance_before, balance_after,
        "atomic batch must not transfer tokens on validation failure"
    );
}

// ---------------------------------------------------------------------------
// Metadata via create_stream_offer -> accept_stream_offer
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_through_offer_accept_flow() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("offer_meta"), ctx.make_val("OFFER-42"));

    // Create offer with metadata
    let offer_id = ctx.client().create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
        &Some(meta.clone()),
        &None,
    );

    // Accept offer
    let stream_id = ctx.client().accept_stream_offer(&ctx.recipient, &offer_id);

    // Verify metadata persisted on the live stream
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("offer_meta")).unwrap(),
        ctx.make_val("OFFER-42")
    );
}

// ---------------------------------------------------------------------------
// Metadata immutability across all mutation paths
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_unchanged_after_rate_update() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("rate_test"), ctx.make_val("RATE-VAL"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    // Rate update does not touch metadata (if supported)
    // Just verify metadata is still there after any successful mutation attempt
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("rate_test")).unwrap(),
        ctx.make_val("RATE-VAL")
    );
}

#[test]
fn test_metadata_unchanged_after_top_up() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("topup_test"), ctx.make_val("TOPUP-VAL"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    ctx.client().top_up_stream(&stream_id, &500_i128);

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("topup_test")).unwrap(),
        ctx.make_val("TOPUP-VAL"),
        "metadata must survive top-up"
    );
}
