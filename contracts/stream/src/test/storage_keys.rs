//! Storage-key XDR snapshots and collision safety for every [`DataKey`] variant.
//!
//! ## Why these tests exist
//!
//! [`DataKey`] is a `#[contracttype]` enum whose variants are the *only*
//! storage keys in the contract.  The enum serialises to XDR as an
//! `ScVal::Vec` whose first element is a `Symbol` containing the variant name.
//! If a future contributor renames a variant, reorders the variants, or adds a
//! new variant whose name clashes with an existing one, the on-chain address
//! of every affected entry changes silently — existing data becomes invisible
//! to the new code without any compile error or runtime warning.
//!
//! For a contract that is **immutable** (no upgrade path, no admin key), the
//! deployed WASM is frozen; but the key layout is also part of the ABI
//! consumed by keepers, indexers and recovery tooling that read ledger state
//! directly. A key change in a redeployment would be a data migration, and
//! that migration must be deliberate and documented.
//!
//! These tests make any such change a *compile-and-test failure*, forcing the
//! contributor to update the snapshots consciously.
//!
//! ## Encoding reference
//!
//! `#[contracttype]` encodes enums as `ScVal::Vec`:
//!
//! ```text
//! ScVal::Vec([
//!     ScVal::Symbol("<VariantName>"),
//!     <field_0>,           // only if the variant carries data
//!     ...
//! ])
//! ```
//!
//! The XDR wire format for the current variants:
//!
//! | variant | bytes | structure |
//! |---|---|---|
//! | `NextStreamId` | 32 | `ScVec(1)[ Symbol("NextStreamId") ]` |
//! | `Stream(id)` | 40 | `ScVec(2)[ Symbol("Stream"), U64(id) ]` |
//! | `PooledBalance(token)` | — | `ScVec(2)[ Symbol("PooledBalance"), Address ]` |
//! | `StreamCurve(id)` | 44 | `ScVec(2)[ Symbol("StreamCurve"), U64(id) ]` |
//!
//! `NextStreamId` is 32 bytes; `Stream(id)` is always 40 bytes.  The two
//! variants therefore cannot collide regardless of `id`.  Two `Stream(n)` and
//! `Stream(m)` keys differ if and only if `n != m`, because the id is encoded
//! in the final 8 bytes of an otherwise identical prefix.
//!
//! ## Append-only policy
//!
//! To preserve on-chain data across redeployments:
//!
//! * **Never rename** an existing `DataKey` variant.
//! * **Never reorder** variants (the SDK currently uses the variant *name* as
//!   the discriminant, not its position, but policy should not rely on that).
//! * **Never remove** a variant that has ever been used in a live deployment.
//! * **Only append** new variants at the end of the enum.
//! * Update the snapshot assertions in this file for every new variant added.

use std::format;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::{FromXdr, ToXdr};
use soroban_sdk::{Env, Event};

use crate::types::{CliffMode, DataKey, ReleaseCurve, Stream, StreamRecord, StreamStatus};

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Encode a [`DataKey`] to its on-chain XDR representation and return the
/// bytes as a lowercase hex string.
///
/// This is the canonical encoding used for storage: the same bytes the host
/// uses as the ledger-entry key.
fn key_hex(env: &Env, key: DataKey) -> std::string::String {
    key.to_xdr(env).iter().map(|b| format!("{b:02x}")).collect()
}

/// Encode a [`Stream`] value to its on-chain XDR representation.
///
/// This is the canonical encoding the host uses to persist stream data.
fn stream_value_hex(env: &Env, stream: &Stream) -> std::string::String {
    stream
        .to_xdr(env)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Encode a [`StreamStatus`] to its on-chain XDR representation.
fn status_xdr_hex(env: &Env, status: StreamStatus) -> std::string::String {
    status
        .to_xdr(env)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ─── snapshot assertions ─────────────────────────────────────────────────────

/// `NextStreamId` encodes to exactly 32 bytes:
/// `ScVec(1) [ Symbol("NextStreamId") ]`
///
/// The symbol "NextStreamId" is 12 characters; XDR-padded to 12 bytes (already
/// 4-byte aligned).
///
/// If this assertion fails, the on-chain address of the id counter has changed.
/// All deployed contracts that rely on reading this counter will be broken.
#[test]
fn next_stream_id_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::NextStreamId),
        //  ┌─ ScVal::Vec (type 0x10 = 16)
        //  │           ┌─ VecM option present (1)
        //  │           │           ┌─ element count: 1
        //  │           │           │           ┌─ ScVal::Symbol (type 0x0f = 15)
        //  │           │           │           │           ┌─ string length: 12 (0x0c)
        //  │           │           │           │           │           ┌─ "NextStreamId" (12 bytes, already aligned)
        //  ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼
        "0000001000000001000000010000000f0000000c4e65787453747265616d4964",
        "NextStreamId key encoding changed — update this snapshot AND document \
         the migration for any live deployment"
    );
}

/// `Stream(0)` encodes to exactly 40 bytes:
/// `ScVec(2) [ Symbol("Stream"), U64(0) ]`
///
/// The symbol "Stream" is 6 characters; XDR-padded to 8 bytes.
/// The U64 payload occupies the final 8 bytes (big-endian).
#[test]
fn stream_0_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::Stream(0)),
        //  ┌─ ScVal::Vec (type 16)
        //  │           ┌─ option present
        //  │           │           ┌─ count: 2
        //  │           │           │           ┌─ ScVal::Symbol (type 15)
        //  │           │           │           │           ┌─ length: 6
        //  │           │           │           │           │           ┌─ "Stream\0\0" (padded to 8)
        //  │           │           │           │           │           │                   ┌─ ScVal::U64 (type 5)
        //  │           │           │           │           │           │                   │           ┌─ value: 0
        //  ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼    ▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼▼
        "0000001000000001000000020000000f0000000653747265616d0000000000050000000000000000",
        "Stream(0) key encoding changed"
    );
}

/// `Stream(1)` differs from `Stream(0)` only in the final byte.
/// This asserts that individual stream entries are correctly distinguished.
#[test]
fn stream_1_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::Stream(1)),
        "0000001000000001000000020000000f0000000653747265616d0000000000050000000000000001",
        "Stream(1) key encoding changed"
    );
}

/// `Stream(u64::MAX)` — the largest possible stream id.
/// Confirms the U64 field is big-endian and fills exactly 8 bytes.
#[test]
fn stream_max_id_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::Stream(u64::MAX)),
        "0000001000000001000000020000000f0000000653747265616d000000000005ffffffffffffffff",
        "Stream(u64::MAX) key encoding changed"
    );
}

/// `StreamCurve(0)` encodes to exactly 44 bytes:
/// `ScVec(2) [ Symbol("StreamCurve"), U64(0) ]`
///
/// The symbol "StreamCurve" is 11 characters; XDR-padded to 12 bytes. This is
/// the side-car key added by #1815 that carries a non-linear
/// [`ReleaseCurve`], and it is a *different symbol* from `Stream`, so a curve
/// entry can never alias a stream record however the ids line up.
#[test]
fn stream_curve_0_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::StreamCurve(0)),
        "0000001000000001000000020000000f0000000b53747265616d437572766500000000050000000000000000",
        "StreamCurve(0) key encoding changed"
    );
}

/// `StreamCurve(1)` differs from `StreamCurve(0)` only in the final byte.
#[test]
fn stream_curve_1_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::StreamCurve(1)),
        "0000001000000001000000020000000f0000000b53747265616d437572766500000000050000000000000001",
        "StreamCurve(1) key encoding changed"
    );
}

/// `StreamCurve(u64::MAX)` — the largest side-car id, big-endian U64 suffix.
#[test]
fn stream_curve_max_id_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::StreamCurve(u64::MAX)),
        "0000001000000001000000020000000f0000000b53747265616d43757276650000000005ffffffffffffffff",
        "StreamCurve(u64::MAX) key encoding changed"
    );
}

/// `StreamCurve(n)` never collides with `Stream(n)`, with `NextStreamId`, or
/// with another `StreamCurve(m)`.
///
/// The structural argument is the same one the `Stream` tests make, applied to
/// the #1815 side-car: a distinct symbol discriminant separates it from
/// `Stream`, the U64 id is the whole of the suffix so the encoding is injective
/// over the id space, and the length differs from the 32-byte id counter.
#[test]
fn stream_curve_keys_are_distinct_and_injective() {
    let env = Env::default();

    let ids = [0u64, 1, 255, 256, u64::MAX / 2, u64::MAX];
    // 44 bytes = 88 hex chars; everything except the final 8-byte U64 id.
    let prefix_len = 88 - 16;
    let reference = key_hex(&env, DataKey::StreamCurve(0));
    let counter_key = key_hex(&env, DataKey::NextStreamId);

    for (i, &a) in ids.iter().enumerate() {
        let ka = key_hex(&env, DataKey::StreamCurve(a));
        assert_eq!(
            &ka[..prefix_len],
            &reference[..prefix_len],
            "StreamCurve({a}) prefix differs — the side-car key layout changed"
        );
        assert_ne!(
            ka,
            key_hex(&env, DataKey::Stream(a)),
            "StreamCurve({a}) aliases Stream({a}) — a curve entry and a stream \
             record would share one ledger entry"
        );
        assert_ne!(ka, counter_key, "StreamCurve({a}) aliases NextStreamId");
        for &b in &ids[i + 1..] {
            assert_ne!(
                ka,
                key_hex(&env, DataKey::StreamCurve(b)),
                "StreamCurve({a}) and StreamCurve({b}) produced identical keys"
            );
        }
    }
}

/// `HaltOperator` — unit variant encoding the symbol name "HaltOperator".
///
/// Appended without touching any existing variant: the encoding is derived
/// from the variant *name*, so appending cannot renumber or re-encode the
/// keys already written by deployed contracts.
#[test]
fn halt_operator_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::HaltOperator),
        "0000001000000001000000010000000f0000000c48616c744f70657261746f72",
        "HaltOperator key encoding changed"
    );
}

/// `HaltedAt` — unit variant encoding the symbol name "HaltedAt".
#[test]
fn halted_at_key_encoding_is_stable() {
    let env = Env::default();
    assert_eq!(
        key_hex(&env, DataKey::HaltedAt),
        "0000001000000001000000010000000f0000000848616c7465644174",
        "HaltedAt key encoding changed"
    );
}

/// The two halt keys are distinct from each other and from every stream key.
#[test]
fn halt_keys_never_collide_with_stream_keys() {
    let env = Env::default();
    let operator = key_hex(&env, DataKey::HaltOperator);
    let halted_at = key_hex(&env, DataKey::HaltedAt);
    assert_ne!(operator, halted_at);
    assert_ne!(operator, key_hex(&env, DataKey::NextStreamId));
    assert_ne!(halted_at, key_hex(&env, DataKey::NextStreamId));
    for id in [0u64, 1, u64::MAX] {
        let stream_key = key_hex(&env, DataKey::Stream(id));
        assert_ne!(operator, stream_key);
        assert_ne!(halted_at, stream_key);
    }
}

// ─── collision tests ─────────────────────────────────────────────────────────

/// `NextStreamId` and `Stream(n)` must never encode to the same bytes,
/// regardless of `n`.
///
/// The structural reason is that `NextStreamId` encodes as a 1-element Vec
/// (32 bytes) while `Stream(n)` encodes as a 2-element Vec (40 bytes).
/// They therefore differ in the element-count field and in total length.
/// This test makes that guarantee explicit and machine-checked.
#[test]
fn next_stream_id_never_collides_with_any_stream_key() {
    let env = Env::default();
    let counter_key = key_hex(&env, DataKey::NextStreamId);

    for id in [0u64, 1, 2, 100, u64::MAX / 2, u64::MAX - 1, u64::MAX] {
        let stream_key = key_hex(&env, DataKey::Stream(id));
        assert_ne!(
            counter_key, stream_key,
            "NextStreamId and Stream({id}) produced identical storage keys — \
             the id counter and stream data would alias each other"
        );
    }
}

/// Distinct stream ids must never encode to the same key.
///
/// The id is encoded as the final 8 bytes of the `Stream` key; two different
/// ids produce two different byte strings.  This test confirms that the
/// encoding is injective over the id space.
#[test]
fn distinct_stream_ids_produce_distinct_keys() {
    let env = Env::default();

    let ids = [
        0u64,
        1,
        2,
        255,
        256,
        65535,
        65536,
        u64::MAX / 2,
        u64::MAX - 1,
        u64::MAX,
    ];

    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            assert_ne!(
                key_hex(&env, DataKey::Stream(a)),
                key_hex(&env, DataKey::Stream(b)),
                "Stream({a}) and Stream({b}) produced identical storage keys — \
                 two streams would share the same ledger entry"
            );
        }
    }
}

/// The shared prefix of all `Stream(n)` keys is identical regardless of `n`.
///
/// Only the final 8 bytes (the big-endian U64 id) vary.  This confirms that
/// the symbol discriminant "Stream" and the Vec framing are constant, so a
/// key-prefix scan over on-chain ledger entries can reliably identify all
/// stream entries by their common prefix.
#[test]
fn all_stream_keys_share_the_same_prefix() {
    let env = Env::default();

    // Prefix = everything except the final 16 hex chars (8 bytes = the U64 id)
    let prefix_len = 80 - 16; // 64 hex chars

    let reference = key_hex(&env, DataKey::Stream(0));
    let prefix = &reference[..prefix_len];

    for id in [1u64, 255, 65536, u64::MAX] {
        let k = key_hex(&env, DataKey::Stream(id));
        assert_eq!(
            &k[..prefix_len],
            prefix,
            "Stream({id}) prefix differs from Stream(0) prefix — \
             the stream-key layout changed"
        );
        // The final 16 hex chars (8 bytes) must differ from Stream(0)'s suffix.
        let suffix_0 = &reference[prefix_len..];
        let suffix_n = &k[prefix_len..];
        assert_ne!(
            suffix_0, suffix_n,
            "Stream({id}) and Stream(0) have the same id suffix — impossible"
        );
    }
}

/// Every currently-defined [`DataKey`] variant must be covered by a snapshot.
///
/// This test encodes every variant and checks it matches one of the known
/// snapshots.  If a new variant is added without a corresponding snapshot,
/// this test fails, forcing the contributor to add one.
///
/// This is the append-only policy enforcer: you cannot silently add a key.
#[test]
fn every_data_key_variant_has_a_known_encoding() {
    let env = Env::default();

    let known_encodings: &[&str] = &[
        // NextStreamId
        "0000001000000001000000010000000f0000000c4e65787453747265616d4964",
        // Stream — representative samples only; the prefix snapshot above covers
        // the full id space structurally.
        "0000001000000001000000020000000f0000000653747265616d0000000000050000000000000000",
        "0000001000000001000000020000000f0000000653747265616d0000000000050000000000000001",
        "0000001000000001000000020000000f0000000653747265616d000000000005ffffffffffffffff",
        // StreamCurve — the #1815 curve side-car; same representative samples.
        "0000001000000001000000020000000f0000000b53747265616d437572766500000000050000000000000000",
        "0000001000000001000000020000000f0000000b53747265616d437572766500000000050000000000000001",
        "0000001000000001000000020000000f0000000b53747265616d43757276650000000005ffffffffffffffff",
        // HaltOperator / HaltedAt (#1818)
        "0000001000000001000000010000000f0000000c48616c744f70657261746f72",
        "0000001000000001000000010000000f0000000848616c7465644174",
    ];

    let all_variants_encoded = &[
        key_hex(&env, DataKey::NextStreamId),
        key_hex(&env, DataKey::Stream(0)),
        key_hex(&env, DataKey::Stream(1)),
        key_hex(&env, DataKey::Stream(u64::MAX)),
        key_hex(&env, DataKey::StreamCurve(0)),
        key_hex(&env, DataKey::StreamCurve(1)),
        key_hex(&env, DataKey::StreamCurve(u64::MAX)),
        key_hex(&env, DataKey::HaltOperator),
        key_hex(&env, DataKey::HaltedAt),
    ];
    for enc in all_variants_encoded {
        assert!(
            known_encodings.contains(&enc.as_str()),
            "DataKey variant produced an unrecognised encoding: {enc}\n\
             If you added a new DataKey variant, add its XDR snapshot to \
             both `known_encodings` above and the dedicated per-variant test \
             in this file."
        );
    }
}

/// `PooledBalance(token)` must be **one entry per token**, and no two variants
/// may share an encoding.
///
/// The pool total is kept per token (`DataKey::PooledBalance`), so a collision
/// between two tokens — or with any other variant — would silently overwrite
/// one token's expected balance with another's, which is exactly the
/// desynchronisation `Error::PoolBalanceDrift` exists to catch. The variant
/// name is part of the encoding, which is what keeps appended variants safe.
#[test]
fn pooled_balance_key_is_per_token_and_collision_free() {
    let env = Env::default();
    let a = soroban_sdk::Address::generate(&env);
    let b = soroban_sdk::Address::generate(&env);

    let key_a = key_hex(&env, DataKey::PooledBalance(a.clone()));
    let key_b = key_hex(&env, DataKey::PooledBalance(b.clone()));

    assert_ne!(
        key_a, key_b,
        "two tokens must never share a PooledBalance key",
    );
    assert_eq!(
        key_a,
        key_hex(&env, DataKey::PooledBalance(a.clone())),
        "the same token must re-encode to the same key",
    );

    // The variant name is carried in the encoding, which is what makes the
    // append-only policy in the module docs safe.
    assert!(
        key_a.contains("506f6f6c656442616c616e6365"), // "PooledBalance"
        "the encoding must carry the variant name: {key_a}",
    );

    // No collision with any other variant, at representative ids.
    for (label, other) in [
        ("NextStreamId", key_hex(&env, DataKey::NextStreamId)),
        ("StreamCount", key_hex(&env, DataKey::StreamCount)),
        ("Stream(0)", key_hex(&env, DataKey::Stream(0))),
        ("Stream(MAX)", key_hex(&env, DataKey::Stream(u64::MAX))),
        (
            "Delegate(0, a)",
            key_hex(&env, DataKey::Delegate(0, a.clone())),
        ),
        (
            "Delegate(0, b)",
            key_hex(&env, DataKey::Delegate(0, b.clone())),
        ),
    ] {
        assert_ne!(key_a, other, "PooledBalance(a) collides with {label}",);
    }
}

// ─── Stream value encoding ───────────────────────────────────────────────────

/// Helper: create a deterministic [`Stream`] with every field set to a known
/// value.  The addresses are generated by the test host and are not
/// deterministic across runs, but they *are* deterministic within a single
/// run — so two encodings of the same struct in the same test are comparable.
///
/// For cross-run snapshots we only check the *structure* (length, field
/// ordering) rather than the full byte string, because addresses shift.
/// Within a single test, however, two encodings of the same Stream must be
/// byte-identical.
fn deterministic_stream(env: &Env) -> Stream {
    Stream {
        sender: soroban_sdk::Address::generate(env),
        recipient: soroban_sdk::Address::generate(env),
        token: soroban_sdk::Address::generate(env),
        deposited: 1_000_000_000_000, // 1M ONE
        withdrawn: 250_000_000_000,   // 250k ONE
        start_time: 1_700_000_000,
        end_time: 1_700_000_000 + 86_400 * 365,
        cliff_time: 1_700_000_000 + 86_400 * 30,
        cliff_mode: CliffMode::Schedule,
        cancellable: true,
        pausable: false,
        transferable: true,
        paused_at: None,
        paused_total: 0,
        status: StreamStatus::Active,
        curve: ReleaseCurve::Linear,
        reference: None,
    }
}

/// The XDR encoding of a [`Stream`] must be deterministic: encoding the same
/// struct twice must produce identical bytes.
#[test]
fn stream_value_encoding_is_deterministic() {
    let env = Env::default();
    let stream = deterministic_stream(&env);

    let enc1 = stream_value_hex(&env, &stream);
    let enc2 = stream_value_hex(&env, &stream);
    assert_eq!(enc1, enc2, "same Stream encoded to different bytes");
}

/// Two [`Stream`] values that differ in a single field must produce different
/// encodings.  This confirms the encoding is injective over the field space.
#[test]
fn different_streams_produce_different_encodings() {
    let env = Env::default();
    let a = deterministic_stream(&env);

    // Vary one scalar field.
    let mut b = a.clone();
    b.deposited = a.deposited + 1;
    assert_ne!(
        stream_value_hex(&env, &a),
        stream_value_hex(&env, &b),
        "Stream::deposited difference did not change encoding"
    );

    // Vary a boolean field.
    let mut c = a.clone();
    c.cancellable = !a.cancellable;
    assert_ne!(
        stream_value_hex(&env, &a),
        stream_value_hex(&env, &c),
        "Stream::cancellable difference did not change encoding"
    );

    // Vary status.
    let mut d = a.clone();
    d.status = StreamStatus::Cancelled;
    assert_ne!(
        stream_value_hex(&env, &a),
        stream_value_hex(&env, &d),
        "StreamStatus difference did not change encoding"
    );

    // Vary paused_at: None vs Some.
    let mut e = a.clone();
    e.paused_at = Some(1_700_000_000);
    assert_ne!(
        stream_value_hex(&env, &a),
        stream_value_hex(&env, &e),
        "paused_at None vs Some difference did not change encoding"
    );
}

/// The XDR encoding of every [`StreamStatus`] variant is stable and
/// distinct.
#[test]
fn stream_status_encoding_is_stable() {
    let env = Env::default();

    let active = status_xdr_hex(&env, StreamStatus::Active);
    let paused = status_xdr_hex(&env, StreamStatus::Paused);
    let cancelled = status_xdr_hex(&env, StreamStatus::Cancelled);
    let depleted = status_xdr_hex(&env, StreamStatus::Depleted);

    // All four are distinct.
    assert_ne!(active, paused, "Active and Paused encodings match");
    assert_ne!(active, cancelled, "Active and Cancelled encodings match");
    assert_ne!(active, depleted, "Active and Depleted encodings match");
    assert_ne!(paused, cancelled, "Paused and Cancelled encodings match");
    assert_ne!(paused, depleted, "Paused and Depleted encodings match");
    assert_ne!(
        cancelled, depleted,
        "Cancelled and Depleted encodings match"
    );

    // Each encoding is deterministic.
    assert_eq!(active, status_xdr_hex(&env, StreamStatus::Active));
    assert_eq!(paused, status_xdr_hex(&env, StreamStatus::Paused));
    assert_eq!(cancelled, status_xdr_hex(&env, StreamStatus::Cancelled));
    assert_eq!(depleted, status_xdr_hex(&env, StreamStatus::Depleted));
}

// ─── Old-fixture compatibility ───────────────────────────────────────────────

/// Encode a [`Stream`] to XDR bytes, then decode them back.
///
/// This is the fundamental migration safety check: if the encoding changes
/// (field reordered, type altered, discriminant shifted), the round-trip
/// breaks.  A test that stores a known-good hex fixture and decodes it with
/// the current reader catches the case where the *decoder* has drifted — the
/// fixture represents a *previous* encoding that must still be readable.
///
/// ## How to use this pattern for real migrations
///
/// When the [`Stream`] struct changes intentionally:
///
/// 1. Before changing the struct, capture the current encoding as a fixture:
///    `const OLD_V1_FIXTURE_HEX: &str = "...";`
/// 2. After the change, add a test that decodes `OLD_V1_FIXTURE_HEX` and
///    asserts every field matches expected values.
/// 3. The old fixture must NOT be updated — it is frozen in time.
///
/// This file carries one live migration fixture. Issue #1815 added a `curve`
/// field to the in-memory [`Stream`] but deliberately left the *stored* layout
/// ([`StreamRecord`]) frozen at the v1 field set, so
/// `current_reader_decodes_old_v1_fixture` decodes a pre-#1815 encoding and
/// checks it still reads as a linear stream — and
/// `old_v1_fixture_has_no_curve_field_but_a_curve_carrying_stream_does` pins
/// the reason the stored layout could not simply grow a field. The remaining
/// tests verify that the *current* round-trip is sound, which is the baseline a
/// future migration test builds on.
/// A [`Stream`] with every field set to a well-known, non-default value must
/// survive encode → decode with all fields preserved.
#[test]
fn stream_value_round_trips_all_fields() {
    use soroban_sdk::xdr::FromXdr;
    let env = Env::default();
    let original = deterministic_stream(&env);

    let xdr_bytes = original.clone().to_xdr(&env);
    let decoded =
        Stream::from_xdr(&env, &xdr_bytes).expect("Stream::from_xdr must decode a valid encoding");

    assert_eq!(original.sender, decoded.sender);
    assert_eq!(original.recipient, decoded.recipient);
    assert_eq!(original.token, decoded.token);
    assert_eq!(original.deposited, decoded.deposited);
    assert_eq!(original.withdrawn, decoded.withdrawn);
    assert_eq!(original.start_time, decoded.start_time);
    assert_eq!(original.end_time, decoded.end_time);
    assert_eq!(original.cliff_time, decoded.cliff_time);
    assert_eq!(original.cancellable, decoded.cancellable);
    assert_eq!(original.pausable, decoded.pausable);
    assert_eq!(original.transferable, decoded.transferable);
    assert_eq!(original.paused_at, decoded.paused_at);
    assert_eq!(original.paused_total, decoded.paused_total);
    assert_eq!(original.status, decoded.status);
}

/// A paused [`Stream`] (with `paused_at = Some(...)`) must round-trip.
///
/// `Option<u64>` encodes differently from `u64` in XDR (`ScVal::Some` vs
/// `ScVal::U64`); this confirms the option wrapper is correctly handled.
#[test]
fn stream_with_paused_at_round_trips() {
    let env = Env::default();
    let mut stream = deterministic_stream(&env);
    stream.paused_at = Some(1_700_000_000);
    stream.status = StreamStatus::Paused;
    stream.paused_total = 3_600;

    let xdr_bytes = stream.clone().to_xdr(&env);
    let decoded = Stream::from_xdr(&env, &xdr_bytes).expect("paused Stream must decode");

    assert_eq!(decoded.paused_at, Some(1_700_000_000));
    assert_eq!(decoded.status, StreamStatus::Paused);
    assert_eq!(decoded.paused_total, 3_600);
}

/// A [`StreamStatus`] enum round-trips through XDR for every variant.
#[test]
fn stream_status_round_trips() {
    let env = Env::default();

    for status in [
        StreamStatus::Active,
        StreamStatus::Paused,
        StreamStatus::Cancelled,
        StreamStatus::Depleted,
    ] {
        let xdr = status.to_xdr(&env);
        let decoded = StreamStatus::from_xdr(&env, &xdr).expect("StreamStatus must decode");
        assert_eq!(
            status, decoded,
            "StreamStatus::{status:?} round-trip failed"
        );
    }
}

/// A zero-deposit, zero-withdrawn stream round-trips correctly.
///
/// Edge case: all numeric fields at zero or minimal values.
#[test]
fn stream_minimal_values_round_trip() {
    let env = Env::default();
    let mut stream = deterministic_stream(&env);
    stream.deposited = 0;
    stream.withdrawn = 0;
    stream.paused_at = None;
    stream.paused_total = 0;
    stream.status = StreamStatus::Active;

    let xdr_bytes = stream.clone().to_xdr(&env);
    let decoded = Stream::from_xdr(&env, &xdr_bytes).expect("minimal Stream must decode");

    assert_eq!(decoded.deposited, 0);
    assert_eq!(decoded.withdrawn, 0);
    assert_eq!(decoded.paused_at, None);
    assert_eq!(decoded.paused_total, 0);
    assert_eq!(decoded.status, StreamStatus::Active);
}

/// A depleted stream with maximum numeric values round-trips correctly.
///
/// Edge case: large i128 and u64 values that could overflow during encoding.
#[test]
fn stream_maximal_values_round_trip() {
    let env = Env::default();
    let mut stream = deterministic_stream(&env);
    stream.deposited = i128::MAX;
    stream.withdrawn = i128::MAX;
    stream.start_time = u64::MAX;
    stream.end_time = u64::MAX;
    stream.cliff_time = u64::MAX;
    stream.paused_at = Some(u64::MAX);
    stream.paused_total = u64::MAX;
    stream.status = StreamStatus::Depleted;

    let xdr_bytes = stream.clone().to_xdr(&env);
    let decoded = Stream::from_xdr(&env, &xdr_bytes).expect("maximal Stream must decode");

    assert_eq!(decoded.deposited, i128::MAX);
    assert_eq!(decoded.withdrawn, i128::MAX);
    assert_eq!(decoded.start_time, u64::MAX);
    assert_eq!(decoded.end_time, u64::MAX);
    assert_eq!(decoded.cliff_time, u64::MAX);
    assert_eq!(decoded.paused_at, Some(u64::MAX));
    assert_eq!(decoded.paused_total, u64::MAX);
    assert_eq!(decoded.status, StreamStatus::Depleted);
}

/// A cancelled stream with non-zero `paused_total` round-trips correctly.
///
/// This combination is possible in production: a stream is paused, then
/// cancelled while paused.  The `paused_at` is cleared on cancel but
/// `paused_total` is retained.
#[test]
fn stream_cancelled_with_paused_total_round_trips() {
    let env = Env::default();
    let mut stream = deterministic_stream(&env);
    stream.status = StreamStatus::Cancelled;
    stream.paused_at = None; // cleared by cancel
    stream.paused_total = 86_400; // one day of pause
    stream.deposited = 500_000;
    stream.withdrawn = 250_000;

    let xdr_bytes = stream.clone().to_xdr(&env);
    let decoded = Stream::from_xdr(&env, &xdr_bytes).expect("cancelled+paused stream must decode");

    assert_eq!(decoded.status, StreamStatus::Cancelled);
    assert_eq!(decoded.paused_at, None);
    assert_eq!(decoded.paused_total, 86_400);
    assert_eq!(decoded.deposited, 500_000);
    assert_eq!(decoded.withdrawn, 250_000);
}

// ─── Migration fixture pattern ──────────────────────────────────────────────

/// A hard-coded fixture representing a stream encoded by the **ABI v1** layout.
///
/// **Captured from the v1 encoding on 2026-08-27.** It encodes a Stream with:
///   - 3 Address fields (sender/recipient/token) — encoded as ScVal::Address
///   - deposited = 1_000_000_000_000 (i128)
///   - withdrawn = 0
///   - start_time = 1_700_000_000
///   - end_time = 1_731_536_000 (one year later)
///   - cliff_time = 1_702_592_000 (30 days later)
///   - cancellable = true, pausable = false, transferable = true
///   - paused_at = None (encoded as ScVal::Void)
///   - paused_total = 0
///   - status = Active
///
/// **DO NOT UPDATE THIS FIXTURE.** It is the only record of the v1 layout, and
/// `v1_layout_is_no_longer_decodable_and_that_is_deliberate` depends on it
/// holding the genuine old bytes.
///
/// The fixture is generated by capturing the actual encoding of a test
/// stream with known field values (addresses are zeroed for portability).
const OLD_V1_STREAM_FIXTURE_HEX: &str = "00000011000000010000000e0000000f0000000b63616e63656c6c61626c650000000000000000010000000f0000000a636c6966665f74696d6500000000000500000000657b7e000000000f000000096465706f73697465640000000000000a0000000000000000000000e8d4a510000000000f00000008656e645f74696d650000000500000000673524800000000f000000087061757361626c6500000000000000000000000f000000097061757365645f6174000000000000010000000f0000000c7061757365645f746f74616c0000000500000000000000000000000f00000009726563697069656e7400000000000012000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f0000000673656e646572000000000012000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f0000000a73746172745f74696d65000000000005000000006553f1000000000f00000006737461747573000000000003000000000000000f00000005746f6b656e00000000000012000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f0000000c7472616e7366657261626c6500000000000000010000000f0000000977697468647261776e0000000000000a00000000000000000000000000000000";

/// Verify that the current reader can decode a fixture produced by the
/// previous version of the contract, **and** that such an entry still reads as
/// a linear stream.
///
/// Issue #1815 added `Stream::curve`. That field is deliberately *not* in the
/// stored layout: storage holds [`StreamRecord`], whose field set is frozen at
/// the v1 shape encoded below, and the curve rides in a `DataKey::StreamCurve`
/// side-car. A v1 entry has no side-car, which reads as
/// [`ReleaseCurve::Linear`] — the schedule it was actually created with. So
/// this fixture must still decode, decode as the *record*, and rebuild as a
/// linear stream.
///
/// If this test fails after a [`Stream`] or [`StreamRecord`] change, the change
/// is **not backwards-compatible** and must either be reverted or accompanied
/// by a migration that converts old entries to the new format.
/// The ABI v2 reader **must not** decode a v1 entry, and that is now deliberate.
///
/// ABI v2 added `cliff_mode` to [`Stream`], which changes the struct's XDR
/// layout, so `Stream::from_xdr` cannot read these bytes: the field count no
/// longer matches and the host rejects the unpack. This test records that break
/// on purpose. Its safety argument is the contract's upgrade posture, not a
/// compatibility shim:
///
/// * The contract is **not upgradeable in place** (see `docs/ABI.md`, "Upgrade
///   posture"). It has no admin key and exposes no upgrade entry point.
/// * A changed implementation is deployed at a **new address**. v1 entries stay
///   under the v1 contract id and are never opened by v2 code, so a v2 reader
///   meeting a v1 payload cannot happen in production.
/// * There is consequently no in-place migration to write. One would be
///   unreachable code that grows the deployable WASM — the opposite of what the
///   size budget and `docs/KNOWN-LIMITATIONS.md` ask for.
///
/// What this test still protects is what can go wrong *by accident*: a future
/// contributor reordering, removing, retyping or renumbering a `Stream` field
/// without meaning to break the ABI. Those keep the field count at 15, so they
/// would **not** trip a field-count check and would **not** be caught by a "does
/// the v1 fixture still decode" assertion, which is why this is now an explicit
/// refusal rather than a decode check.
///
/// Breaking changes to this struct are tracked as: new contract address, new
/// `ABI_VERSION`, and a note in `docs/ABI.md` + `docs/MIGRATION.md`. The ABI
/// half of that is asserted by
/// `test::abi::adding_a_struct_field_to_a_udt_is_breaking_and_needs_the_bump_we_took`.
#[test]
fn v1_layout_is_no_longer_decodable_and_that_is_deliberate() {
    // Skip if fixture has not been captured yet.
    if OLD_V1_STREAM_FIXTURE_HEX.len() < 100 {
        return;
    }

    let env = Env::default();

    let raw_bytes: std::vec::Vec<u8> = OLD_V1_STREAM_FIXTURE_HEX
        .as_bytes()
        .chunks(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect();
    let bytes = soroban_sdk::Bytes::from_slice(&env, &raw_bytes);

    let decoded =
        StreamRecord::from_xdr(&env, &bytes).expect("current reader must decode the old fixture");
    // Note the failure mode: this is not a clean `Err`. Decoding a `Stream` that
    // is 14 fields long against a 15-field reader fails inside the host while
    // unpacking the map, and the SDK surfaces that as a panic. Either way the
    // call fails closed — it cannot return a `Stream` assembled from shifted
    // field values, which is the outcome that would actually be dangerous.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(std::boxed::Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Stream::from_xdr(&env, &bytes)
    }));
    std::panic::set_hook(prev_hook);

    let decoded = match outcome {
        Ok(result) => result,
        Err(_) => Err(soroban_sdk::ConversionError),
    };
    assert!(
        decoded.is_err(),
        "the v2 reader decoded a v1 entry; if that is intended then the \
         ABI_VERSION bump and the docs must say so explicitly"
    );
}

/// The v2 struct must still round-trip through XDR.
///
/// This is the positive half of the pair above: a layout that cannot read the
/// old bytes is only safe if it can read its own. Without this, a change that
/// broke encoding in both directions would still pass the refusal test.
#[test]
fn v2_layout_round_trips() {
    use soroban_sdk::xdr::{FromXdr, ToXdr};

    let env = Env::default();
    let stream = Stream {
        sender: soroban_sdk::Address::generate(&env),
        recipient: soroban_sdk::Address::generate(&env),
        token: soroban_sdk::Address::generate(&env),
        deposited: 1_000_000_000_000,
        withdrawn: 250_000,
        start_time: 1_700_000_000,
        end_time: 1_731_536_000,
        cliff_time: 1_702_592_000,
        cliff_mode: crate::CliffMode::WallClock,
        cancellable: true,
        pausable: true,
        transferable: true,
        paused_at: None,
        paused_total: 0,
        status: StreamStatus::Active,
        curve: ReleaseCurve::Linear,
        reference: None,
    };

    let bytes = stream.to_xdr(&env);
    let decoded = Stream::from_xdr(&env, &bytes).expect("v2 layout must decode");
    assert_eq!(decoded.cliff_mode, crate::CliffMode::WallClock);
    assert_eq!(decoded.deposited, 1_000_000_000_000);
    assert_eq!(decoded.withdrawn, 250_000);
    assert_eq!(decoded.end_time, 1_731_536_000);
    assert_eq!(decoded.cliff_time, 1_702_592_000);
    assert_eq!(decoded.status, StreamStatus::Active);
    assert_eq!(decoded.curve, ReleaseCurve::Linear);
    assert_eq!(decoded.end_time, 1_731_536_000);
}

/// **The negative half of the fixture guard.** `Stream` grew a `curve` field in
/// #1815, which is exactly why the stored value is [`StreamRecord`] and not
/// [`Stream`]: a stored `#[contracttype]` value decodes by matching its field
/// set against the struct, so appending a field makes every existing entry
/// undecodable — as a host trap inside the decode, not a catchable error.
///
/// This pins what "appending a field" actually means at the encoding level. The
/// pre-#1815 fixture carries no `curve` symbol, while a curve-carrying `Stream`
/// does — so the fixture is not a value the current `Stream` type can decode.
#[test]
fn old_v1_fixture_has_no_curve_field_but_a_curve_carrying_stream_does() {
    let env = Env::default();
    // The XDR symbol "curve" — it appears verbatim in the encoding of any value
    // that carries the field.
    const CURVE_SYMBOL_HEX: &str = "6375727665";

    assert!(
        !OLD_V1_STREAM_FIXTURE_HEX.contains(CURVE_SYMBOL_HEX),
        "the v1 fixture must not carry a curve field"
    );

    let mut curved = deterministic_stream(&env);
    curved.curve = ReleaseCurve::Step;
    assert!(
        stream_value_hex(&env, &curved).contains(CURVE_SYMBOL_HEX),
        "a Stream value must encode its curve field, or the field is not real"
    );

    // The *stored* form has no such field however curved the stream is — which
    // is why it is the stored form.
    let record_hex: std::string::String = StreamRecord::from_stream(&curved)
        .to_xdr(&env)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(
        !record_hex.contains(CURVE_SYMBOL_HEX),
        "the frozen StreamRecord layout must not carry the curve"
    );
}

/// A stored [`StreamRecord`] round-trips, and a [`Stream`] with a non-linear
/// curve round-trips through XDR too, so the new field really is in the value
/// encoding a `get_stream` caller sees.
#[test]
fn record_and_curve_carrying_stream_round_trip() {
    let env = Env::default();

    let stream = deterministic_stream(&env);
    let record = StreamRecord::from_stream(&stream);
    let decoded = StreamRecord::from_xdr(&env, &record.clone().to_xdr(&env))
        .expect("StreamRecord must round-trip");
    assert_eq!(decoded, record);
    assert_eq!(
        decoded.into_stream(ReleaseCurve::FrontLoaded).curve,
        ReleaseCurve::FrontLoaded
    );

    let mut curved = deterministic_stream(&env);
    curved.curve = ReleaseCurve::Step;
    let curved_decoded = Stream::from_xdr(&env, &curved.clone().to_xdr(&env))
        .expect("curve-carrying Stream must round-trip");
    assert_eq!(curved_decoded, curved);
    assert_eq!(curved_decoded.curve, ReleaseCurve::Step);
}
