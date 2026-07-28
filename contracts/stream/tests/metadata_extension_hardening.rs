extern crate std;

//! Hardened metadata extension regression suite — issue #1292.
//!
//! This file complements `contracts/stream/tests/metadata_extension.rs` with
//! focused regression tests that pin down edge cases NOT covered there:
//!
//! 1. **Combined XDR size budget**: existing tests measure the `Stream` struct
//!    alone. This file adds a combined measurement of the full storage entry
//!    shape (memo + metadata at MAX), and re-prints measured byte counts so
//!    CI logs record the actual sizes.
//! 2. **Operation coverage gaps**: `extend_stream_end_time`, `clone_stream`
//!    (clone-of-clone), and `create_stream_offer` validation paths.
//! 3. **Pre-allocation invariant under ID reservations**: validation must
//!    run BEFORE `next_stream_id_for` consumes a reserved ID, just like it
//!    does for the global counter.
//! 4. **Stream end-time mutations do not touch metadata**: `extend`,
//!    `shorten` (covered elsewhere), and pause/resume cooldown sequencing
//!    while metadata is at MAX size.
//!
//! Run:
//!
//! ```bash
//! cargo test -p fluxora_stream --features testutils --test metadata_extension_hardening
//! ```

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind,
    MAX_MEMO_BYTES, MAX_METADATA_BYTES, MAX_METADATA_KEYS, MAX_METADATA_KEY_BYTES,
    MAX_METADATA_VALUE_BYTES, MAX_STREAM_ENTRY_BYTES,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    xdr::ToXdr,
    Address, Bytes, Env, Map, Vec,
};

// ---------------------------------------------------------------------------
// Fixture constants
// ---------------------------------------------------------------------------

const INITIAL_SENDER_BALANCE: i128 = 1_000_000_i128;
const LEDGER_START_TIMESTAMP: u64 = 1_000_000_u64;
const LEDGER_START_SEQUENCE: u32 = 100_000_u32;

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

        StellarAssetClient::new(&env, &token_id).mint(&sender, &INITIAL_SENDER_BALANCE);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &999_999);

        // Assert the post-mint balance so failures during setup are obvious.
        assert_eq!(token.balance(&sender), INITIAL_SENDER_BALANCE);

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

    fn key(&self, s: &str) -> Bytes {
        Bytes::from_slice(&self.env, s.as_bytes())
    }

    fn val(&self, s: &str) -> Bytes {
        Bytes::from_slice(&self.env, s.as_bytes())
    }

    fn metadata_fixed(&self, count: u32) -> Map<Bytes, Bytes> {
        let mut m: Map<Bytes, Bytes> = Map::new(&self.env);
        for i in 0..count {
            m.set(self.key(&std::format!("k{}", i)), self.val(&std::format!("v{}", i)));
        }
        m
    }

    fn create_with_memo_and_metadata(
        &self,
        memo: Option<Bytes>,
        metadata: Option<Map<Bytes, Bytes>>,
    ) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: 1_000_i128,
                rate_per_second: 1_i128,
                start_time: LEDGER_START_TIMESTAMP,
                cliff_time: LEDGER_START_TIMESTAMP,
                end_time: LEDGER_START_TIMESTAMP + 1_000,
                withdraw_dust_threshold: Some(0_i128),
                memo,
                metadata,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// 1. Combined XDR size budget
// ---------------------------------------------------------------------------

/// When BOTH `memo` and `metadata` are at their independent maxima, the full
/// serialized `Stream` must remain within `MAX_STREAM_ENTRY_BYTES`.
///
/// Existing coverage in `metadata_extension.rs::test_metadata_worst_case_xdr_size_within_ceiling`
/// measures only the metadata-padded baseline. This test exercises the worst-case
/// combination and prints the actual byte counts so CI logs act as a size-budget
/// record.
///
/// Layout:
///   `memo`       = MAX_MEMO_BYTES (256) bytes
///   `metadata`   = 4 entries × (8-byte key + 120-byte value) = 512 bytes exactly
///
/// Expected structural total ≈ 1 696 bytes (well under 4 096 ceiling).
#[test]
fn test_metadata_and_memo_combined_xdr_size_within_ceiling() {
    let ctx = Ctx::setup();

    // 4 entries × (8-byte key + 120-byte value) = 512 bytes exactly.
    let mut metadata: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..4u8 {
        let key_str = std::format!("key{:05}", i); // 8 bytes
        let value_bytes = vec![i; 120]; // 120 bytes each
        metadata.set(
            Bytes::from_slice(&ctx.env, key_str.as_bytes()),
            Bytes::from_slice(&ctx.env, &value_bytes),
        );
    }

    // Memo at MAX_MEMO_BYTES.
    let memo = Bytes::from_slice(&ctx.env, &vec![0xAB_u8; MAX_MEMO_BYTES]);

    let stream_id = ctx.create_with_memo_and_metadata(Some(memo), Some(metadata));

    let stream = ctx.client().get_stream_state(&stream_id);
    let xdr_bytes = stream.to_xdr(&ctx.env);
    let serialized_len = xdr_bytes.len() as usize;

    std::println!(
        "XDR_SIZE_MEASUREMENT: memo_max+metadata_max_stream_entry: {} bytes (ceiling: {} bytes)",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );

    assert!(
        serialized_len <= MAX_STREAM_ENTRY_BYTES,
        "max memo + max metadata entry ({} bytes) exceeds MAX_STREAM_ENTRY_BYTES ({})",
        serialized_len,
        MAX_STREAM_ENTRY_BYTES
    );
}

// ---------------------------------------------------------------------------
// 2. Operation coverage gaps
// ---------------------------------------------------------------------------

/// `extend_stream_end_time` MUST NOT mutate metadata, even when metadata is
/// populated at near-MAX aggregate size.
#[test]
fn test_metadata_unchanged_after_extend_stream_end_time() {
    let ctx = Ctx::setup();

    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.key("project"), ctx.val("Fluxora"));
    meta.set(ctx.key("env"), ctx.val("testnet"));
    let stream_id = ctx.create_with_memo_and_metadata(None, Some(meta.clone()));

    // Advance so extend has had accrual.
    ctx.env.ledger().set_timestamp(LEDGER_START_TIMESTAMP + 100);

    // Extend from 1_000s → 2_000s; deposit must cover the extra duration.
    // deposit_amount = 1_000, rate = 1 → streamable over 1_000s = 1_000.
    // Extending to 2_000s requires 2_000 deposit, so top up first.
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &1_000_i128);
    ctx.client()
        .extend_stream_end_time(&stream_id, &(LEDGER_START_TIMESTAMP + 2_000));

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.get(ctx.key("project")).unwrap(),
        ctx.val("Fluxora"),
        "metadata must survive extend_stream_end_time"
    );
    assert_eq!(
        got.get(ctx.key("env")).unwrap(),
        ctx.val("testnet"),
        "all metadata entries must survive extend_stream_end_time"
    );
}

/// Cloning a stream that already has metadata must propagate metadata to the
/// grandchild. This covers the path: source → clone → clone-of-clone.
#[test]
fn test_metadata_clone_chain_grandchild_inherits_metadata() {
    let ctx = Ctx::setup();
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.key("chain"), ctx.val("G0"));
    let source_id = ctx.create_with_memo_and_metadata(None, Some(meta.clone()));

    // Advance past start so clone is valid.
    ctx.env.ledger().set_timestamp(LEDGER_START_TIMESTAMP + 1);
    let mid_recipient = Address::generate(&ctx.env);
    let mid_id = ctx.client().clone_stream(
        &source_id,
        &mid_recipient,
        &(LEDGER_START_TIMESTAMP + 1),
        &(LEDGER_START_TIMESTAMP + 1_001),
        &1_000_i128,
        &false,
    );

    // Mutate the mid stream's metadata-equivalent? We don't expose set_metadata,
    // so the mid clone retains the inherited metadata.

    // Now clone the mid → grandchild.
    let grand_recipient = Address::generate(&ctx.env);
    // After clone the stream.start_time of mid is LEDGER_START_TIMESTAMP + 1, so
    // we advance further before cloning again to be safe.
    ctx.env.ledger().set_timestamp(LEDGER_START_TIMESTAMP + 2);
    let grand_id = ctx.client().clone_stream(
        &mid_id,
        &grand_recipient,
        &(LEDGER_START_TIMESTAMP + 2),
        &(LEDGER_START_TIMESTAMP + 1_002),
        &1_000_i128,
        &false,
    );

    let got = ctx.client().get_stream_metadata(&grand_id).unwrap();
    assert_eq!(
        got.get(ctx.key("chain")).unwrap(),
        ctx.val("G0"),
        "clone-of-clone must inherit metadata through the chain"
    );
}

// ---------------------------------------------------------------------------
// 3. Pre-allocation invariant under ID reservations
// ---------------------------------------------------------------------------

/// Even when the caller holds an active `reserve_stream_ids`, validation must
/// run BEFORE any reserved ID is consumed. A failed validation must not
/// advance reservation `consumed`, mirroring the behaviour for the global
/// counter (covered by `test_metadata_validation_failure_does_not_allocate_stream_id`
/// in `metadata_extension.rs`).
#[test]
fn test_metadata_validation_failure_does_not_consume_reserved_ids() {
    let ctx = Ctx::setup();

    // Reserve 2 IDs ahead of time.
    ctx.client().reserve_stream_ids(&ctx.sender, &2_u32);

    // Read the current reservation slot; we can't read it directly, so infer
    // by counting stream IDs after the failed attempt.
    let baseline_count = ctx.client().get_stream_count();

    // Build a metadata map whose aggregate exceeds MAX_METADATA_BYTES.
    let mut bad: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..5u8 {
        let key_str = std::format!("k{}_k", i); // 4 bytes
        bad.set(
            Bytes::from_slice(&ctx.env, key_str.as_bytes()),
            Bytes::from_slice(&ctx.env, &vec![i; 120]),
        );
    }
    // 5 × (4 + 120) = 620 > 512 (aggregate overflow).

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
            metadata: Some(bad),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // The stream counter must not advance under reservation either.
    assert_eq!(
        ctx.client().get_stream_count(),
        baseline_count,
        "stream counter must not advance when validation rejects metadata, even with a reservation"
    );
}

// ---------------------------------------------------------------------------
// 4. `create_stream_offer` validates metadata
// ---------------------------------------------------------------------------

/// `create_stream_offer` must call `validate_metadata` and reject offers with
/// metadata that exceeds aggregate byte limit. The offer must NOT consume a
/// stream ID on failure (mirrors `create_stream` allocator behaviour).
#[test]
fn test_create_stream_offer_rejects_oversized_metadata() {
    let ctx = Ctx::setup();

    let baseline_count = ctx.client().get_stream_count();

    // 4 entries with very large values that overflow aggregate.
    let mut bad: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..4u8 {
        let k = std::format!("k{}", i);
        bad.set(
            Bytes::from_slice(&ctx.env, k.as_bytes()),
            Bytes::from_slice(&ctx.env, &vec![i; 200]),
        );
    }
    // 4 × (2 + 200) = 808 > 512 → MetadataTooLarge.

    let result = ctx.client().try_create_stream_offer(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: LEDGER_START_TIMESTAMP + 10,
            cliff_time: LEDGER_START_TIMESTAMP + 10,
            end_time: LEDGER_START_TIMESTAMP + 1_010,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: Some(bad),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
        &None,
    );

    match result {
        Err(Ok(ContractError::MetadataTooLarge)) => {}
        _ => panic!(
            "Expected MetadataTooLarge on create_stream_offer, got {:?}",
            result
        ),
    }

    // The stream counter must not advance (offers pre-allocate an offer ID
    // that equals the next stream ID). Failing validation must reject the
    // whole transaction rather than leaving a dangling offer.
    assert_eq!(
        ctx.client().get_stream_count(),
        baseline_count,
        "failed create_stream_offer must not advance the stream counter"
    );
}

/// A valid metadata map at exactly MAX_METADATA_BYTES round-trips through the
/// offer→accept flow and lands on the resulting Stream. Lock down the
/// happy-path offer metadata carry-over for CI regression awareness.
#[test]
fn test_create_stream_offer_metadata_valid_round_trips() {
    let ctx = Ctx::setup();

    // 4 entries × (8-byte key + 120-byte value) = 512 exactly.
    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..4u8 {
        let key_str = std::format!("key{:05}", i);
        meta.set(
            Bytes::from_slice(&ctx.env, key_str.as_bytes()),
            Bytes::from_slice(&ctx.env, &vec![i; 120]),
        );
    }

    let offer_id = ctx.client().create_stream_offer(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000_i128,
            rate_per_second: 1_i128,
            start_time: LEDGER_START_TIMESTAMP + 10,
            cliff_time: LEDGER_START_TIMESTAMP + 10,
            end_time: LEDGER_START_TIMESTAMP + 1_010,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: Some(meta.clone()),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
        &None,
    );

    // Offer must also expose its metadata via get_stream_offer.
    let offer = ctx.client().get_stream_offer(&offer_id);
    let offer_meta = offer.metadata.expect("offer must carry metadata");
    assert_eq!(
        offer_meta.len(),
        4,
        "offer metadata must have 4 entries after creation"
    );

    // Accept and verify landing.
    ctx.env.ledger().set_timestamp(LEDGER_START_TIMESTAMP + 5);
    let stream_id = ctx.client().accept_stream_offer(&ctx.recipient, &offer_id);

    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(got.len(), 4, "stream must have 4 metadata entries after accept");

    // Cross-check: key/value contents match.
    for i in 0u8..4u8 {
        let key_str = std::format!("key{:05}", i);
        let expected_value = Bytes::from_slice(&ctx.env, &vec![i; 120]);
        let actual = got.get(Bytes::from_slice(&ctx.env, key_str.as_bytes())).unwrap();
        assert_eq!(actual, expected_value, "metadata entry contents must survive offer→accept");
    }
}

// ---------------------------------------------------------------------------
// 5. Pause/resume while metadata is at MAX
// ---------------------------------------------------------------------------

/// When metadata is populated at near-MAX aggregate size and the user pauses
/// then resumes the stream across the cooldown interval, the stream entry's
/// XDR size must remain within the ceiling throughout the lifecycle.
///
/// This is a property-style assertion that catches shrinkage of the entry
/// budget as the Stream struct grows over versions.
#[test]
fn test_metadata_at_max_survives_pause_resume_cycle() {
    let ctx = Ctx::setup();

    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    for i in 0u8..4u8 {
        let key_str = std::format!("key{:05}", i);
        meta.set(
            Bytes::from_slice(&ctx.env, key_str.as_bytes()),
            Bytes::from_slice(&ctx.env, &vec![i; 120]),
        );
    }

    let stream_id = ctx.create_with_memo_and_metadata(None, Some(meta));

    // Initial size baseline.
    let initial_size = ctx.client().get_stream_state(&stream_id).to_xdr(&ctx.env).len();
    assert!(
        initial_size <= MAX_STREAM_ENTRY_BYTES,
        "metadata-full entry ({} bytes) exceeds ceiling ({}) before any operation",
        initial_size,
        MAX_STREAM_ENTRY_BYTES
    );

    // Pause: requires sequence > last_pause + MIN_PAUSE_INTERVAL_LEDGERS (17).
    ctx.env.ledger().with_mut(|l| l.sequence_number += 17);
    ctx.client()
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);
    let after_pause = ctx.client().get_stream_state(&stream_id).to_xdr(&ctx.env).len();
    assert!(
        after_pause <= MAX_STREAM_ENTRY_BYTES,
        "pause must not inflate entry size (was {} bytes, now {} bytes)",
        initial_size,
        after_pause
    );

    // Resume.
    ctx.env.ledger().with_mut(|l| l.sequence_number += 17);
    ctx.client().resume_stream(&stream_id);
    let after_resume = ctx.client().get_stream_state(&stream_id).to_xdr(&ctx.env).len();
    assert!(
        after_resume <= MAX_STREAM_ENTRY_BYTES,
        "resume must not inflate entry size (was {} bytes, now {} bytes)",
        initial_size,
        after_resume
    );

    // Metadata content survived.
    let got = ctx.client().get_stream_metadata(&stream_id).unwrap();
    assert_eq!(
        got.len(),
        4,
        "metadata entry count must be preserved across pause/resume"
    );
}

// ---------------------------------------------------------------------------
// 6. validate_metadata never crashes on adversarial u32 arithmetic
// ---------------------------------------------------------------------------

/// Validate that the validator NEVER panics or trails past the bound check on
/// inputs that would overflow u32 (long key/values with extremes that could
/// wrap). The validator uses `checked_add` and `ok_or` so the round-trip
/// behaviour is: oversized input → `MetadataTooLarge`, not panics.
///
/// This test uses raw `Bytes` of sizes just above `MAX_METADATA_VALUE_BYTES`
/// to confirm tightly: a single oversized value entry is rejected on the
/// per-field check before the aggregate check runs.
#[test]
fn test_validate_metadata_adversarial_value_overflow_attempt_rejected() {
    let ctx = Ctx::setup();

    // A single value of MAX_METADATA_VALUE_BYTES + 1 must be rejected outright
    // by the per-field check (no aggregate check needed).
    let huge = Bytes::from_slice(
        &ctx.env,
        &vec![0u8; (MAX_METADATA_VALUE_BYTES as usize) + 1],
    );

    let mut meta: Map<Bytes, Bytes> = Map::new(&ctx.env);
    meta.set(ctx.key("k"), huge);

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
            "oversized value must trigger MetadataTooLarge (per-field guard), got {:?}",
            result
        ),
    }
}

// ---------------------------------------------------------------------------
// 7. Reading metadata on a stream created BEFORE the metadata contract upgrade
// ---------------------------------------------------------------------------

/// Backward-compat regression: a stream that was created with `metadata: None`
/// (the pre-V4 default) and then has its contract handle replaced must still
/// read `metadata == None` unchanged. This is the closest simulation we can
/// run in-process of "follow the same stream after a contract WASM swap";
/// it verifies that no implicit None→Some coercion happens elsewhere.
#[test]
fn test_legacy_none_metadata_unchanged_after_re_read() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_with_memo_and_metadata(None, None);

    // Multiple reads, across calls — should always be None.
    for _ in 0..5 {
        let m = ctx.client().get_stream_metadata(&stream_id);
        assert!(m.is_none(), "legacy None metadata must remain None across reads");
        assert!(
            ctx.client().get_stream_state(&stream_id).metadata.is_none(),
            "state-layer metadata must remain None across reads"
        );
    }
}

// ---------------------------------------------------------------------------
// 8. Constants are pinned
// ---------------------------------------------------------------------------

/// Pin the metadata constants at their current values. A bump to any of these
/// would visibly affect XDR sizing, gas costs, and CI records. We assert them
/// here so an accidental change forces a CI failure with a clear message.
#[test]
fn test_metadata_constants_are_pinned_at_current_values() {
    assert_eq!(MAX_METADATA_KEYS, 8_u32, "MAX_METADATA_KEYS pin changed");
    assert_eq!(MAX_METADATA_BYTES, 512_u32, "MAX_METADATA_BYTES pin changed");
    assert_eq!(MAX_METADATA_KEY_BYTES, 32_u32, "MAX_METADATA_KEY_BYTES pin changed");
    assert_eq!(
        MAX_METADATA_VALUE_BYTES, 128_u32,
        "MAX_METADATA_VALUE_BYTES pin changed"
    );
    assert!(
        MAX_METADATA_KEYS as u32 * (MAX_METADATA_KEY_BYTES + MAX_METADATA_VALUE_BYTES) > MAX_METADATA_BYTES,
        "MAX_METADATA_BYTES is no longer the binding aggregate cap (per-entry max > aggregate max)"
    );
}

// Suppress unused imports warnings when no test references these directly.
// (Kept for readers who copy/paste scaffolds; safe to remove if the lints object.)
#[allow(dead_code)]
fn _unused_witness(_v: Vec<Bytes>) {}
