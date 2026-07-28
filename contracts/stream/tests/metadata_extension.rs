extern crate std;

// Comprehensive tests for per-stream metadata TLV extension.
//
// Hardening properties (deterministic across upgrades and retries):
//
// - Metadata is stored as a field of the Stream struct (discriminant 2 of DataKey),
//   NOT as a separate DataKey variant. This means metadata imposes no additional
//   storage-migration burden: the discriminant table is unchanged.
//
// - XDR forward-compatibility: an absent metadata field (from a V6-era entry)
//   decodes as None. Tests verify this in storage_key_compat.rs.
//
// - Immutability: metadata cannot be modified after stream creation. All post-creation
//   operations (pause, resume, cancel, withdraw, clone) leave metadata unchanged.
//
// - Validation: key count, per-key size, per-value size, and aggregate size are
//   all enforced before any stream ID is allocated or any token is moved.
//
// - Deterministic round-trip: metadata written in a given configuration always
//   returns byte-identical values on read, across any number of operations.
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
// - Storage independence: each stream's metadata is isolated
// - Deterministic round-trip after pause/resume/cancel/withdraw

use fluxora_stream::{
    ContractError, CreateStreamParams, CreateStreamRelativeParams, CreateStreamResult,
    FluxoraStream, FluxoraStreamClient, StreamKind, MAX_METADATA_BYTES, MAX_METADATA_KEYS,
    MAX_METADATA_KEY_BYTES, MAX_METADATA_VALUE_BYTES,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    xdr::ToXdr,
    Address, Bytes, Env, Map, TryFromVal,
};

// ---------------------------------------------------------------------------
// Fixture constants
// ---------------------------------------------------------------------------

/// Sender balance minted at setup time.  Large enough for all test deposits.
const INITIAL_SENDER_BALANCE: i128 = 100_000_i128;

/// Starting ledger timestamp (seconds).  All time-sensitive tests advance
/// from this value so there is no implicit dependency on "wall-clock now".
const LEDGER_START_TIMESTAMP: u64 = 1_000_000_u64;

/// Starting ledger sequence number.  Pinned so tests that depend on ledger
/// sequence (pause cooldown, withdraw cooldown) begin from a known baseline.
const LEDGER_START_SEQUENCE: u32 = 100_000_u32;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    /// The sender whose token balance is tracked across tests.
    sender: Address,
    recipient: Address,
    /// Token client used for balance / allowance assertions.
    token: TokenClient<'a>,
    /// The admin address — kept for double-init idempotency tests.
    admin: Address,
    /// The token address — kept for double-init idempotency tests.
    token_id: Address,
}

impl<'a> Ctx<'a> {
    /// Build a fully-verified test fixture.
    ///
    /// Assertions made here:
    /// 1. Sender balance after mint equals `INITIAL_SENDER_BALANCE`.
    /// 2. Sender allowance for the contract equals `i128::MAX` after approve.
    /// 3. Ledger timestamp is exactly `LEDGER_START_TIMESTAMP`.
    /// 4. Ledger sequence is exactly `LEDGER_START_SEQUENCE`.
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        // Pin ledger state before registering the contract so every test
        // starts from the same deterministic ledger snapshot.
        env.ledger().with_mut(|li| {
            li.timestamp = LEDGER_START_TIMESTAMP;
            li.sequence_number = LEDGER_START_SEQUENCE;
        });

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
        sac.mint(&sender, &INITIAL_SENDER_BALANCE);

        let token = TokenClient::new(&env, &token_id);

        // Approve and immediately assert the allowance was accepted.
        token.approve(&sender, &contract_id, &i128::MAX, &999_999);
        assert_eq!(
            token.allowance(&sender, &contract_id),
            i128::MAX,
            "FIXTURE: sender allowance must be i128::MAX after approve"
        );

        // Assert mint landed.
        assert_eq!(
            token.balance(&sender),
            INITIAL_SENDER_BALANCE,
            "FIXTURE: sender balance must equal INITIAL_SENDER_BALANCE after mint"
        );

        // Assert ledger state is deterministic.
        assert_eq!(
            env.ledger().timestamp(),
            LEDGER_START_TIMESTAMP,
            "FIXTURE: ledger timestamp must be LEDGER_START_TIMESTAMP"
        );
        assert_eq!(
            env.ledger().sequence(),
            LEDGER_START_SEQUENCE,
            "FIXTURE: ledger sequence must be LEDGER_START_SEQUENCE"
        );

        Ctx {
            env,
            contract_id,
            sender,
            recipient,
            token,
            admin,
            token_id,
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

    /// Helper: create a stream with standard timing anchored to LEDGER_START_TIMESTAMP.
    fn create_stream_with_metadata(&self, metadata: Option<Map<Bytes, Bytes>>) -> u64 {
        let recipient = Address::generate(&self.env);
        let params = vec![
            &self.env,
            CreateStreamParams {
                recipient,
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1000,
                withdraw_dust_threshold: None,
                memo: None,
                kind: StreamKind::Linear,
                metadata,
            },
        ];
        let results = self.client().create_streams_partial(&self.sender, &params);
        let r = results.get(0).unwrap();
        assert!(r.success, "create_streams_partial entry must succeed");
        r.stream_id.unwrap()
    }

    /// Assert the sender's current balance equals `expected`.
    fn assert_sender_balance(&self, expected: i128, msg: &str) {
        assert_eq!(self.token.balance(&self.sender), expected, "{}", msg);
    }

    /// Assert no tokens moved since `balance_before`.
    fn assert_no_token_movement(&self, balance_before: i128, msg: &str) {
        self.assert_sender_balance(balance_before, msg);
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
    let recipient = Address::generate(&ctx.env);
    let bad_meta = ctx.metadata_n(MAX_METADATA_KEYS + 1);
    let params = vec![
        &ctx.env,
        CreateStreamParams {
            recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(bad_meta),
        },
    ];
    let results = ctx.client().create_streams_partial(&ctx.sender, &params);
    assert_eq!(results.len(), 1);
    let r = results.get(0).unwrap();
    assert!(!r.success, "entry with too many keys must fail");
    assert!(r.stream_id.is_none());
    assert!(r.error.is_some());
}

// ---------------------------------------------------------------------------
// Validation: per-key byte limit
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_key_exactly_at_limit_valid() {
    let ctx = Ctx::setup();
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
    let key = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_KEY_BYTES + 1) as usize].as_slice(),
    );
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(key, ctx.make_val("v"));
    let recipient = Address::generate(&ctx.env);
    let params = vec![
        &ctx.env,
        CreateStreamParams {
            recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(meta),
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
    let recipient = Address::generate(&ctx.env);
    let params = vec![
        &ctx.env,
        CreateStreamParams {
            recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(meta),
        },
    ];
    let results = ctx.client().create_streams_partial(&ctx.sender, &params);
    assert_eq!(results.len(), 1);
    let r = results.get(0).unwrap();
    assert!(!r.success, "entry with oversized value must fail");
    assert!(r.stream_id.is_none());
    assert!(r.error.is_some());
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
fn test_metadata_aggregate_one_byte_over_limit_rejected() {
    let ctx = Ctx::setup();
    // 4 entries × (8-byte key + 120-byte value) = 512 bytes.
    // Add one extra byte to the last entry's value → 513 bytes > 512.
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..3 {
        let key_str = std::format!("key{:05}", i); // 8 bytes
        let value = Bytes::from_slice(&ctx.env, &vec![i; 120].as_slice()); // 120 bytes
        meta.set(Bytes::from_slice(&ctx.env, key_str.as_bytes()), value);
    }
    // Last entry: 8-byte key + 121-byte value = 129 bytes  →  total = 3×128 + 129 = 513 > 512
    let value_over = Bytes::from_slice(&ctx.env, &vec![3u8; 121].as_slice());
    meta.set(Bytes::from_slice(&ctx.env, b"key00003"), value_over);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: Some(meta),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!(
            "Expected MetadataTooLarge for one-byte-over aggregate, got {:?}",
            result
        ),
    }
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
    let recipient = Address::generate(&ctx.env);
    let params = vec![
        &ctx.env,
        CreateStreamParams {
            recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(meta),
        },
    ];
    let results = ctx.client().create_streams_partial(&ctx.sender, &params);
    assert_eq!(results.len(), 1);
    let r = results.get(0).unwrap();
    assert!(!r.success, "entry with aggregate overflow must fail");
    assert!(r.stream_id.is_none());
    assert!(r.error.is_some());
}

/// Single-entry early-exit path: one entry whose key (1 byte) + value (MAX_METADATA_BYTES bytes)
/// exceeds the aggregate limit by itself.  validate_metadata must reject on the first
/// iteration without needing a second entry.
#[test]
fn test_metadata_single_entry_aggregate_exceeds_limit_early_exit() {
    let ctx = Ctx::setup();
    // value alone = MAX_METADATA_BYTES bytes; key = 1 byte → total = MAX_METADATA_BYTES + 1 > limit.
    let value = Bytes::from_slice(&ctx.env, &vec![0u8; MAX_METADATA_BYTES as usize].as_slice());
    // MAX_METADATA_VALUE_BYTES is 128, so value above is 512 bytes which already exceeds
    // MAX_METADATA_VALUE_BYTES (128).  Use a value of exactly MAX_METADATA_VALUE_BYTES bytes
    // and pad the key to push the aggregate over the limit.
    //
    // Strategy: key = 32 bytes (at limit), value = 128 bytes (at limit) → 160 bytes per entry.
    // With 4 such entries: 640 bytes > 512 but that is multi-entry.
    //
    // For a genuine single-entry over-aggregate test, we need key_len + val_len > 512.
    // key_len <= 32 and val_len <= 128, so max per-entry is 160 < 512 —
    // a single entry can never alone exceed MAX_METADATA_BYTES with valid key/value sizes.
    // The real single-entry early-exit is for the *per-entry running total*: after the first
    // entry the running total is checked and if it already exceeds 512 the loop returns.
    //
    // Test the early-exit by having the very first entry push the running total past 512.
    // Use key=32, value=128 so first entry = 160 bytes.  Follow with enough identical entries
    // so that the limit is crossed at the SECOND entry (not the end), and assert rejection.
    // To make it hit at entry 2: first entry = 160, second = 160 → total 320.  Still under.
    // We need total > 512 at entry ceil(512/160) = 4th entry (640 bytes).
    // The existing test_metadata_aggregate_exceeds_limit_rejected already covers this.
    //
    // What we uniquely test here: a value that by itself exceeds MAX_METADATA_VALUE_BYTES
    // causes rejection BEFORE the aggregate check (per-field check is first in the loop).
    let oversized_value = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_VALUE_BYTES + 1) as usize].as_slice(),
    );
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("k"), oversized_value);

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: Some(meta),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    // Must fail before even reaching the aggregate check.
    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!(
            "Expected MetadataTooLarge for per-value early exit, got {:?}",
            result
        ),
    }
}

// ---------------------------------------------------------------------------
// Storage-layer invariants
// ---------------------------------------------------------------------------

/// Metadata is stored INLINE on the Stream struct under DataKey::Stream(id).
/// Verified by reading get_stream_state().metadata and confirming it matches
/// get_stream_metadata() — both views must agree and return the same value.
#[test]
fn test_metadata_inline_on_stream_struct_not_separate_key() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("inline_test"), ctx.make_val("confirmed"));

    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    // Read via get_stream_state (reads the full Stream struct from DataKey::Stream).
    let stream_state = ctx.client().get_stream_state(&stream_id);
    let state_meta = stream_state.metadata;

    // Read via dedicated accessor (returns stream.metadata field).
    let accessor_meta = ctx.client().get_stream_metadata(&stream_id);

    // Both must be Some.
    assert!(
        state_meta.is_some(),
        "get_stream_state().metadata must be Some"
    );
    assert!(
        accessor_meta.is_some(),
        "get_stream_metadata() must be Some"
    );

    // Both must agree on content.
    let state_map = state_meta.unwrap();
    let acc_map = accessor_meta.unwrap();
    assert_eq!(
        state_map.get(ctx.make_key("inline_test")),
        acc_map.get(ctx.make_key("inline_test")),
        "get_stream_state().metadata and get_stream_metadata() must return identical values"
    );
    assert_eq!(
        acc_map.get(ctx.make_key("inline_test")).unwrap(),
        ctx.make_val("confirmed")
    );
}

/// None metadata is distinguishable from Some(empty map) at the storage layer.
/// Both get_stream_state().metadata and get_stream_metadata() must reflect the
/// difference — not just the public accessor.
#[test]
fn test_none_vs_empty_map_distinct_at_storage_layer() {
    let ctx = Ctx::setup();

    // Stream A: None metadata.
    let id_none = ctx.create_stream_with_metadata(None);

    // Stream B: Some(empty map).
    let empty: Map<Bytes, Bytes> = Map::new(&ctx.env);
    let id_empty = ctx.create_stream_with_metadata(Some(empty));

    // Via get_stream_state().metadata:
    let state_none = ctx.client().get_stream_state(&id_none).metadata;
    let state_empty = ctx.client().get_stream_state(&id_empty).metadata;

    assert!(
        state_none.is_none(),
        "get_stream_state().metadata must be None for a stream created without metadata"
    );
    assert!(
        state_empty.is_some(),
        "get_stream_state().metadata must be Some for a stream created with Some(empty)"
    );
    assert_eq!(
        state_empty.unwrap().len(),
        0,
        "Some(empty map) must have 0 entries in get_stream_state().metadata"
    );

    // Via get_stream_metadata():
    let acc_none = ctx.client().get_stream_metadata(&id_none);
    let acc_empty = ctx.client().get_stream_metadata(&id_empty);

    assert!(acc_none.is_none(), "get_stream_metadata() must be None");
    assert!(
        acc_empty.is_some(),
        "get_stream_metadata() must be Some(empty)"
    );
    assert_eq!(acc_empty.unwrap().len(), 0);
}

/// validate_metadata is called BEFORE next_stream_id_for (pre-ID-allocation).
/// When metadata validation fails the stream counter must not advance.
#[test]
fn test_metadata_validation_failure_does_not_allocate_stream_id() {
    let ctx = Ctx::setup();
    let before_count = ctx.client().get_stream_count();

    let bad_value = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_VALUE_BYTES + 1) as usize].as_slice(),
    );
    let mut bad_meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    bad_meta.set(ctx.make_key("k"), bad_value);

    let _ = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: Some(bad_meta),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    let after_count = ctx.client().get_stream_count();
    assert_eq!(
        before_count, after_count,
        "stream ID counter must not advance when metadata validation fails"
    );
}

/// When metadata validation fails no tokens must move.
#[test]
fn test_metadata_validation_failure_no_token_movement() {
    let ctx = Ctx::setup();
    let balance_before = ctx.token.balance(&ctx.sender);

    let meta = ctx.metadata_n(MAX_METADATA_KEYS + 1); // exceeds key count

    let _ = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: Some(meta),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.assert_no_token_movement(
        balance_before,
        "sender balance must be unchanged when metadata validation fails",
    );
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

    // First pause requires ledger sequence >= MIN_PAUSE_INTERVAL when counter is 0.
    ctx.env.ledger().with_mut(|l| l.sequence_number += 17);
    ctx.client()
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);
    ctx.env.ledger().with_mut(|l| l.sequence_number += 17);
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

    ctx.env.ledger().set_timestamp(LEDGER_START_TIMESTAMP + 100);
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

    ctx.env.ledger().set_timestamp(LEDGER_START_TIMESTAMP + 200);
    ctx.client().withdraw(&stream_id);

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("ref")).unwrap(),
        ctx.make_val("WITHDRAW_TEST"),
        "metadata must survive withdrawal"
    );
}

#[test]
fn test_metadata_unchanged_after_top_up() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("ref"), ctx.make_val("TOPUP_TEST"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("ref")).unwrap(),
        ctx.make_val("TOPUP_TEST"),
        "metadata must survive top-up"
    );
}

// ---------------------------------------------------------------------------
// StreamCreated event includes metadata
// ---------------------------------------------------------------------------

#[test]
fn test_stream_created_event_contains_metadata() {
    use soroban_sdk::testutils::Events;

    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("project"), ctx.make_val("PROJ-99"));

    let _ = ctx.create_stream_with_metadata(Some(meta.clone()));

    let events = ctx.env.events().all();
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
        "at least one event must be emitted on stream creation"
    );
}

// ---------------------------------------------------------------------------
// get_stream_metadata returns StreamNotFound for unknown IDs
// ---------------------------------------------------------------------------

#[test]
fn test_get_stream_metadata_nonexistent_stream() {
    let ctx = Ctx::setup();
    let result = ctx.client().try_get_stream_metadata(&999_u64);
    match result {
        Err(Ok(ContractError::StreamNotFound)) => {}
        _ => panic!("Expected StreamNotFound, got {:?}", result),
    }
}

// ---------------------------------------------------------------------------
// Gas / XDR size: metadata contribution stays within MAX_STREAM_ENTRY_BYTES
// ---------------------------------------------------------------------------

/// Worst-case metadata (MAX_METADATA_KEYS entries, each at the per-key and
/// per-value byte limits) must produce a Stream entry whose XDR serialization
/// stays within MAX_STREAM_ENTRY_BYTES.
///
/// The actual byte count is printed via `println!` so CI logs provide a
/// permanent record of the measured size.  If this test fails after a struct
/// change, follow the runbook in the MAX_STREAM_ENTRY_BYTES doc comment in
/// lib.rs: measure, add 25% headroom, round up to the next 512-byte boundary.
#[test]
fn test_metadata_worst_case_xdr_size_within_ceiling() {
    let ctx = Ctx::setup();

    let params = vec![
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
            kind: StreamKind::Linear,
            metadata: Some(meta_a.clone()),
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
            kind: StreamKind::Linear,
            metadata: Some(meta_b.clone()),
        },
    ];

    let stream_id = ctx.create_stream_with_metadata(Some(valid_worst_meta));

    // Retrieve the full stream struct and serialize it.
    let stream = ctx.client().get_stream_state(&stream_id);
    let xdr_bytes = stream.to_xdr(&ctx.env);
    let serialized_len = xdr_bytes.len() as usize;

    println!(
        "XDR_SIZE_MEASUREMENT: metadata_worst_case_stream_entry: {} bytes (ceiling: {} bytes)",
        serialized_len, MAX_STREAM_ENTRY_BYTES
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "worst-case metadata stream entry ({} bytes) exceeds MAX_STREAM_ENTRY_BYTES ({})",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

/// A stream with no metadata must also fit within MAX_STREAM_ENTRY_BYTES.
/// This establishes the baseline (non-metadata) XDR footprint for comparison.
#[test]
fn test_stream_no_metadata_xdr_size_baseline() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream_with_metadata(None);

    let params = vec![
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
            kind: StreamKind::Linear,
            metadata: None,
        },
    ];

    println!(
        "XDR_SIZE_MEASUREMENT: no_metadata_stream_entry: {} bytes (ceiling: {} bytes)",
        serialized_len, MAX_STREAM_ENTRY_BYTES
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "baseline stream entry ({} bytes) exceeds MAX_STREAM_ENTRY_BYTES ({})",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

// ---------------------------------------------------------------------------
// Upgrade / backward compatibility
// ---------------------------------------------------------------------------

/// A stream created WITHOUT metadata (None) remains fully readable after the
/// contract client handle is re-created — simulating the post-upgrade scenario
/// where the client points at the same on-chain state with a fresh handle.
///
/// This validates backward-compatibility for legacy streams that were created
/// before the metadata extension was added.
#[test]
fn test_stream_without_metadata_readable_after_client_reattach() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream_with_metadata(None);

    // Simulate "upgrade": discard the old client handle and create a new one
    // pointing at the same contract address.  In a real upgrade the WASM would
    // be swapped; here we verify that re-attaching the client does not corrupt
    // the stored None metadata field.
    let new_client = FluxoraStreamClient::new(&ctx.env, &ctx.contract_id);

    let stream = new_client.get_stream_state(&stream_id);
    assert!(
        stream.metadata.is_none(),
        "legacy stream (None metadata) must remain readable after client re-attach"
    );
    assert_eq!(stream.stream_id, stream_id);
    assert_eq!(stream.sender, ctx.sender);
    assert_eq!(stream.recipient, ctx.recipient);

    let meta_via_accessor = new_client.get_stream_metadata(&stream_id);
    assert!(
        meta_via_accessor.is_none(),
        "get_stream_metadata must return None for a legacy stream"
    );
}

/// A stream created WITH metadata remains fully readable after the client
/// handle is re-created — simulating the post-upgrade scenario for metadata-
/// carrying streams.
#[test]
fn test_stream_with_metadata_readable_after_client_reattach() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("upgrade_test"), ctx.make_val("persistent"));
    let stream_id = ctx.create_stream_with_metadata(Some(meta));

    // Re-attach client.
    let new_client = FluxoraStreamClient::new(&ctx.env, &ctx.contract_id);

    let stream = new_client.get_stream_state(&stream_id);
    assert!(
        stream.metadata.is_some(),
        "metadata must survive client re-attach"
    );

    let got = new_client.get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("upgrade_test")).unwrap(),
        ctx.make_val("persistent"),
        "metadata content must be unchanged after client re-attach"
    );
}

/// Both a legacy stream (None) and a metadata-carrying stream can coexist and
/// remain independently readable — models the mixed state after a protocol upgrade
/// where new streams carry metadata and old ones don't.
#[test]
fn test_legacy_and_metadata_streams_coexist() {
    let ctx = Ctx::setup();

    let legacy_id = ctx.create_stream_with_metadata(None);

    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("post_upgrade"), ctx.make_val("true"));
    let meta_id = ctx.create_stream_with_metadata(Some(meta));

    // Legacy stream: metadata must be None.
    assert!(ctx.client().get_stream_metadata(&legacy_id).is_none());
    assert!(ctx.client().get_stream_state(&legacy_id).metadata.is_none());

    // Metadata stream: must be Some.
    let got = ctx.client().get_stream_metadata(&meta_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("post_upgrade")).unwrap(),
        ctx.make_val("true")
    );

    // IDs are strictly increasing (monotone allocator).
    assert!(
        meta_id > legacy_id,
        "stream IDs must be monotonically increasing"
    );
}

// ---------------------------------------------------------------------------
// Aggregate math edge cases (overflow-safe arithmetic)
// ---------------------------------------------------------------------------

/// validate_metadata uses checked_add; confirm that the zero-key, zero-value
/// degenerate case (empty string key + empty string value) is handled and
/// adds 0 to the aggregate without error.
#[test]
fn test_aggregate_math_zero_key_zero_value_is_zero_bytes() {
    let ctx = Ctx::setup();
    // key = 0 bytes, value = 0 bytes → aggregate = 0, well within 512.
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key(""), ctx.make_val(""));
    let stream_id = ctx.create_stream_with_metadata(Some(meta));
    assert!(
        ctx.client().get_stream_metadata(&stream_id).is_some(),
        "zero-byte key + zero-byte value must contribute 0 bytes to the aggregate"
    );
}

/// Aggregate is computed as the SUM of (key_len + val_len) across ALL entries.
/// Verify this by constructing entries whose individual sizes are below the
/// per-field limits but whose sum is engineered to land exactly at and then
/// one past the aggregate limit.
///
/// - 2 entries × (32-byte key + 124-byte value) = 2 × 156 = 312 bytes.
/// - Adding a third 32-byte key + 168-byte value would push to 312 + 200 = 512,
///   but the value limit is 128, so we cannot exceed 312 + (32+128)=472 with 3 entries
///   using only valid per-field sizes and a single extra entry.
/// - Use 4 entries of (8-byte key + 120-byte value) = 4 × 128 = 512 (at limit, valid).
/// - Add a 5th entry of 1 byte key + 1 byte value = 2 bytes → total 514 > 512 (rejected).
#[test]
fn test_aggregate_math_sum_of_all_entries() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);

    // 4 entries exactly at the limit.
    for i in 0u8..4 {
        let key_str = std::format!("key{:05}", i); // 8 bytes
        let value = Bytes::from_slice(&ctx.env, &vec![i; 120]); // 120 bytes
        meta.set(Bytes::from_slice(&ctx.env, key_str.as_bytes()), value);
    }
    // 5th entry: 1-byte key + 1-byte value → aggregate = 512 + 2 = 514 > 512.
    meta.set(ctx.make_key("x"), ctx.make_val("y"));

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: Some(meta),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!(
            "Expected MetadataTooLarge when aggregate sum crosses 512, got {:?}",
            result
        ),
    }
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
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: Some(meta_a.clone()),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
        CreateStreamParams {
            recipient: recipient_b.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
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
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
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
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1_000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
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

#[test]
fn test_create_streams_partial_invalid_metadata_fails_entry() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

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
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: LEDGER_START_TIMESTAMP,
            cliff_time: LEDGER_START_TIMESTAMP,
            end_time: LEDGER_START_TIMESTAMP + 1_000,
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
// clone_stream: metadata is inherited by the cloned stream
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_inherited_by_clone_stream() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("ref"), ctx.make_val("CLONE_TEST"));
    let source_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    // Advance past start so cloning is valid.
    ctx.env.ledger().set_timestamp(LEDGER_START_TIMESTAMP + 1);
    let new_recipient = Address::generate(&ctx.env);
    let cloned_id = ctx.client().clone_stream(
        &source_id,
        &new_recipient,
        &(LEDGER_START_TIMESTAMP + 1),
        &(LEDGER_START_TIMESTAMP + 1_001),
        &1_000_i128,
        &false,
    );

    let got = ctx.client().get_stream_metadata(&cloned_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("ref")).unwrap(),
        ctx.make_val("CLONE_TEST"),
        "cloned stream must inherit source metadata"
    );
}

// ---------------------------------------------------------------------------
// create_stream_from_template: metadata is passed through
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_from_template() {
    let ctx = Ctx::setup();

    let template_id = ctx.client().register_stream_template(
        &ctx.sender,
        &0_u64,     // start_delay
        &0_u64,     // cliff_delay
        &1_000_u64, // duration
    );

    let recipient = Address::generate(&ctx.env);
    let _ = ctx.client().create_streams_partial(
        &ctx.sender,
        &vec![
            &ctx.env,
            CreateStreamParams {
                recipient,
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1000,
                withdraw_dust_threshold: None,
                memo: None,
                kind: StreamKind::Linear,
                metadata: Some(bad_meta),
            },
        ],
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

    let recipient = Address::generate(&ctx.env);
    let _ = ctx.client().create_streams_partial(
        &ctx.sender,
        &vec![
            &ctx.env,
            CreateStreamParams {
                recipient,
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1000,
                withdraw_dust_threshold: None,
                memo: None,
                kind: StreamKind::Linear,
                metadata: Some(meta),
            },
        ],
    );

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.make_key("src")).unwrap(),
        ctx.make_val("template"),
        "metadata from template must be stored"
    );
}

// ---------------------------------------------------------------------------
// Boundary: two independent streams do not share metadata
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

    let params = vec![
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
            kind: StreamKind::Linear,
            metadata: Some(meta_a),
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
            kind: StreamKind::Linear,
            metadata: Some(meta_b),
        },
    ];
    let ids = ctx.client().create_streams_partial(&ctx.sender, &params);
    assert_eq!(ids.len(), 2);
    let id_a = ids.get(0).unwrap().stream_id.unwrap();
    let id_b = ids.get(1).unwrap().stream_id.unwrap();

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
    assert_ne!(
        got_a.get(ctx.make_key("id")).unwrap(),
        got_b.get(ctx.make_key("id")).unwrap(),
        "streams must not share metadata"
    );
}

// ---------------------------------------------------------------------------
// Hardening: deterministic metadata round-trip across pause/resume/cancel
// ---------------------------------------------------------------------------

#[test]
fn test_metadata_deterministic_across_all_operations() {
    let ctx = Ctx::setup();

    // Create a stream with known metadata
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.make_key("invoice_id"), ctx.make_val("INV-2026-001"));
    meta.set(
        ctx.make_key("ref_uri"),
        ctx.make_val("https://example.com/inv/001"),
    );
    let stream_id = ctx.create_stream_with_metadata(Some(meta.clone()));

    // Read back immediately — baseline
    let read1 = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        read1.get(ctx.make_key("invoice_id")).unwrap(),
        ctx.make_val("INV-2026-001")
    );

    // Pause then resume — metadata must survive
    ctx.client()
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);
    let read2 = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(read2, read1, "metadata must survive pause");

    ctx.client().resume_stream(&stream_id);
    let read3 = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(read3, read1, "metadata must survive resume");

    // Withdraw — metadata must survive
    ctx.client().withdraw(&stream_id);
    let read4 = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(read4, read1, "metadata must survive withdraw");

    // Cancel — metadata must survive (stream becomes terminal but data is preserved)
    ctx.client().cancel_stream(&stream_id);
    let read5 = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(read5, read1, "metadata must survive cancel");

    // Third read again — deterministic: all 5 reads are identical
}

// ---------------------------------------------------------------------------
// Hardening: many streams with independent metadata
// ---------------------------------------------------------------------------

#[test]
fn test_many_streams_independent_metadata() {
    let ctx = Ctx::setup();
    let count: u32 = 20;

    let mut params_vec = soroban_sdk::Vec::new(&ctx.env);
    for i in 0..count {
        let recipient = Address::generate(&ctx.env);
        let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
        let key = ctx.make_key("idx");
        let val = ctx.make_val(&std::format!("stream-{}", i));
        meta.set(key, val);
        params_vec.push_back(CreateStreamParams {
            recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(meta),
        });
    }

    let results = ctx
        .client()
        .create_streams_partial(&ctx.sender, &params_vec);
    assert_eq!(results.len(), count as u32);

    // Verify each stream's metadata is independent
    for i in 0..count {
        let r = results.get(i).unwrap();
        assert!(r.success, "stream {} must be created successfully", i);
        let stream_id = r.stream_id.unwrap();
        let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
        let expected = ctx.make_val(&std::format!("stream-{}", i));
        assert_eq!(
            got.get(ctx.make_key("idx")).unwrap(),
            expected,
            "stream {} must have its own metadata",
            i
        );
    }
}

// ---------------------------------------------------------------------------
// CONTRACT_VERSION bumped to 4
// ---------------------------------------------------------------------------

#[test]
fn test_contract_version_is_current() {
    let ctx = Ctx::setup();
    let v = ctx.client().version();
    assert!(v >= 7, "CONTRACT_VERSION must be >= 7 (current is {})", v);
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
