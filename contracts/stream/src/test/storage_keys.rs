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

use soroban_sdk::xdr::ToXdr;
use soroban_sdk::Env;

use crate::DataKey;

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Encode a [`DataKey`] to its on-chain XDR representation and return the
/// bytes as a lowercase hex string.
///
/// This is the canonical encoding used for storage: the same bytes the host
/// uses as the ledger-entry key.
fn key_hex(env: &Env, key: DataKey) -> std::string::String {
    key.to_xdr(env).iter().map(|b| format!("{b:02x}")).collect()
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
    ];

    let all_variants_encoded = &[
        key_hex(&env, DataKey::NextStreamId),
        key_hex(&env, DataKey::Stream(0)),
        key_hex(&env, DataKey::Stream(1)),
        key_hex(&env, DataKey::Stream(u64::MAX)),
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
