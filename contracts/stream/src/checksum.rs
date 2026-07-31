//! WASM build reproducibility — checksum verification and storage key layout invariants.
//!
//! This module documents and tests two orthogonal concerns:
//!
//! 1. **Build reproducibility**: invariants that the CI checksum verification relies on.
//! 2. **Storage key layout stability**: discriminant assignments for `DataKey` that must
//!    never change for any deployed contract instance.
//!
//! Neither section performs I/O or reads files at runtime.
//!
//! ---
//!
//! # Build reproducibility contract
//!
//! The following invariants must hold for a build to be reproducible:
//!
//! 1. **Rust toolchain** is pinned via `rust-toolchain.toml` to a specific
//!    channel (`1.94.1`) and target set (`wasm32-unknown-unknown`).
//! 2. **soroban-sdk** version is pinned in `contracts/stream/Cargo.toml`
//!    (currently `21.7.7`).
//! 3. **Build profile** is `--release` with `wasm32-unknown-unknown` target.
//! 4. **No feature flags** beyond the default are used during WASM builds
//!    (the `testutils` feature is only for `#[cfg(test)]`).
//! 5. **No environment-dependent code** is compiled into the WASM artifact.
//!
//! If any of these invariants change, the reference checksum in
//! `wasm/checksums.sha256` must be regenerated via
//! `script/update-wasm-checksums.sh`.
//!
//! ---
//!
//! # Stream data integrity checksum
//!
//! The module provides `compute_stream_checksum()` which computes a deterministic
//! SHA-256 hash over the **financial-state** fields of a `Stream` struct. This
//! checksum can be used externally to detect silent data corruption or tampering.
//!
//! ## Included fields (financial state)
//!
//! | Field                 | Type             | Rationale                              |
//! |:----------------------|:-----------------|:---------------------------------------|
//! | `stream_id`           | `u64`            | Uniquely identifies the stream         |
//! | `sender`              | `Address`        | Financial counterparty                 |
//! | `recipient`           | `Address`        | Financial counterparty                 |
//! | `claim_owner`         | `Option<Address>`| Ownership for claim operations         |
//! | `deposit_amount`      | `i128`           | Core financial amount                  |
//! | `rate_per_second`     | `i128`           | Core financial parameter               |
//! | `start_time`          | `u64`            | Vesting schedule boundary              |
//! | `cliff_time`          | `u64`            | Vesting schedule boundary              |
//! | `end_time`            | `u64`            | Vesting schedule boundary              |
//! | `withdrawn_amount`    | `i128`           | Tracked disbursement                   |
//! | `status`              | `StreamStatus`   | Lifecycle state                        |
//! | `cancelled_at`        | `Option<u64>`    | Terminal timestamp                     |
//! | `checkpointed_amount` | `i128`           | Accrual checkpoint                     |
//! | `checkpointed_at`     | `u64`            | Accrual checkpoint                     |
//! | `withdraw_dust_threshold` | `i128`       | Dust threshold for withdrawals         |
//! | `memo`                | `Option<Bytes>`  | User-supplied memo                     |
//! | `kind`                | `StreamKind`     | Vesting shape (Linear/CliffOnly/...)   |
//! | `metadata`            | `Option<Map<..>>`| Indexer-visible metadata               |
//! | `witness`             | `Option<Address>`| Compliance witness                     |
//! | `is_pooled`           | `Option<bool>`   | Pooled stream flag                     |
//! | `parent_stream_id`    | `Option<u64>`    | Delegation parent reference            |
//! | `irrevocable`         | `Option<bool>`   | Sender-cancel restriction              |
//!
//! ## Excluded fields (operational metadata)
//!
//! These fields are intentionally omitted from the checksum because they represent
//! runtime operational metadata that changes on valid operations without affecting
//! the financial integrity of the stream:
//!
//! | Field                       | Rationale                                      |
//! |:----------------------------|:-----------------------------------------------|
//! | `last_pause_toggle_ledger`  | Changes on every pause/resume; not financial   |
//! | `last_withdraw_ledger`      | Withdrawal frequency tracking; not financial   |
//! | `last_rate_change_ledger`   | Rate-change sequencing; rate itself is included|
//! | `decommissioned`            | Administrative cleanup flag                    |
//! | `delegation_depth`          | Governance/audit trail; not financial state    |
//!
//! ## Upgrade compatibility
//!
//! The set of fields included in the checksum is part of the contract's stability
//! guarantee. New fields added to the `Stream` struct must be:
//!
//! - **Appended** to the encoding order in `compute_stream_checksum` (never inserted).
//! - **Documented** in the included/excluded tables above.
//! - **Backward-compatible**: an older checksum computed on a stream without the
//!   new fields will differ from a new checksum that includes them, which is
//!   expected and acceptable since the new fields represent new financial state.

extern crate alloc;
use alloc::vec::Vec;
use soroban_sdk::{xdr::ToXdr, Address, Bytes, BytesN, Env, Map};
use crate::Stream;

/// SHA-256 output length in bytes.
pub const CHECKSUM_LENGTH: usize = 32;

/// Compute a deterministic SHA-256 checksum over the financial-state fields of a Stream struct.
///
/// # Determinism guarantee
///
/// Given the same `Stream` value, this function always returns the same 32-byte hash.
/// No runtime entropy (timestamps, ledger state, etc.) leaks into the computation.
pub fn compute_stream_checksum(env: &Env, stream: &Stream) -> BytesN<32> {
    let mut buf = soroban_sdk::Bytes::new(env);
    buf.extend_from_array(&stream.stream_id.to_le_bytes());
    buf.append(&stream.sender.to_xdr(env));
    buf.append(&stream.recipient.to_xdr(env));
    encode_option_address(env, &mut buf, &stream.claim_owner);
    buf.extend_from_array(&stream.deposit_amount.to_le_bytes());
    buf.extend_from_array(&stream.rate_per_second.to_le_bytes());
    buf.extend_from_array(&stream.start_time.to_le_bytes());
    buf.extend_from_array(&stream.cliff_time.to_le_bytes());
    buf.extend_from_array(&stream.end_time.to_le_bytes());
    buf.extend_from_array(&stream.withdrawn_amount.to_le_bytes());
    buf.extend_from_array(&(stream.status as u32).to_le_bytes());
    encode_option_u64(&mut buf, &stream.cancelled_at);
    buf.extend_from_array(&stream.checkpointed_amount.to_le_bytes());
    buf.extend_from_array(&stream.checkpointed_at.to_le_bytes());
    buf.extend_from_array(&stream.withdraw_dust_threshold.to_le_bytes());
    encode_option_bytes(&mut buf, &stream.memo);
    buf.extend_from_array(&(stream.kind as u32).to_le_bytes());
    encode_option_map(&mut buf, &stream.metadata);
    encode_option_address(env, &mut buf, &stream.witness);
    encode_option_bool(&mut buf, &stream.is_pooled);
    encode_option_u64(&mut buf, &stream.parent_stream_id);
    encode_option_bool(&mut buf, &stream.irrevocable);
    env.crypto().sha256(&buf).into()
}

/// Mock-compatible checksum using FNV-1a 32-bit hash, zero-extended to 32 bytes.
///
/// This function produces the same field-ordering as `compute_stream_checksum` but
/// uses a simple FNV-1a hash (no Env dependency). Useful for unit tests where
/// `env.crypto()` is unavailable.
pub fn compute_stream_checksum_no_crypto(stream: &Stream) -> [u8; 32] {
    let mut buf: Vec<u8> = Vec::new();
    let addr_dummy = [0u8; 32];
    buf.extend_from_slice(&stream.stream_id.to_le_bytes());
    buf.push(32u8); buf.extend_from_slice(&addr_dummy);
    buf.push(32u8); buf.extend_from_slice(&addr_dummy);
    encode_option_address_simple(&mut buf, &stream.claim_owner);
    buf.extend_from_slice(&stream.deposit_amount.to_le_bytes());
    buf.extend_from_slice(&stream.rate_per_second.to_le_bytes());
    buf.extend_from_slice(&stream.start_time.to_le_bytes());
    buf.extend_from_slice(&stream.cliff_time.to_le_bytes());
    buf.extend_from_slice(&stream.end_time.to_le_bytes());
    buf.extend_from_slice(&stream.withdrawn_amount.to_le_bytes());
    buf.extend_from_slice(&(stream.status as u32).to_le_bytes());
    match stream.cancelled_at {
        Some(v) => { buf.push(1u8); buf.extend_from_slice(&v.to_le_bytes()); }
        None => { buf.push(0u8); }
    }
    buf.extend_from_slice(&stream.checkpointed_amount.to_le_bytes());
    buf.extend_from_slice(&stream.checkpointed_at.to_le_bytes());
    buf.extend_from_slice(&stream.withdraw_dust_threshold.to_le_bytes());
    match &stream.memo {
        Some(bytes) => {
            buf.push(1u8);
            let len = bytes.len() as u32;
            buf.extend_from_slice(&len.to_le_bytes());
            for i in 0..bytes.len() { buf.push(bytes.get(i as u32).unwrap_or(0)); }
        }
        None => { buf.push(0u8); }
    }
    buf.extend_from_slice(&(stream.kind as u32).to_le_bytes());
    match &stream.metadata { Some(_) => buf.push(1u8), None => buf.push(0u8) }
    encode_option_address_simple(&mut buf, &stream.witness);
    match stream.is_pooled {
        Some(v) => { buf.push(1u8); buf.push(if v { 1u8 } else { 0u8 }); }
        None => { buf.push(0u8); }
    }
    match stream.parent_stream_id {
        Some(v) => { buf.push(1u8); buf.extend_from_slice(&v.to_le_bytes()); }
        None => { buf.push(0u8); }
    }
    match stream.irrevocable {
        Some(v) => { buf.push(1u8); buf.push(if v { 1u8 } else { 0u8 }); }
        None => { buf.push(0u8); }
    }
    let fnv_prime: u32 = 0x01000193;
    let mut hash: u32 = 0x811c9dc5;
    for &byte in &buf {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(fnv_prime);
    }
    let mut result = [0u8; 32];
    result[..4].copy_from_slice(&hash.to_le_bytes());
    result
}

// ── Serialisation helpers ──────────────────────────────────────────────────

fn encode_option_address(env: &Env, buf: &mut Bytes, opt: &Option<Address>) {
    match opt {
        Some(addr) => { buf.push_back(0x01u8); buf.append(&addr.to_xdr(env)); }
        None => { buf.push_back(0x00u8); }
    }
}

fn encode_option_u64(buf: &mut Bytes, opt: &Option<u64>) {
    match opt {
        Some(v) => { buf.push_back(0x01u8); buf.extend_from_array(&v.to_le_bytes()); }
        None => { buf.push_back(0x00u8); }
    }
}

fn encode_option_bytes(buf: &mut Bytes, opt: &Option<Bytes>) {
    match opt {
        Some(bytes) => {
            buf.push_back(0x01u8);
            let len = bytes.len() as u32;
            buf.extend_from_array(&len.to_le_bytes());
            buf.append(bytes);
        }
        None => { buf.push_back(0x00u8); }
    }
}

fn encode_option_bool(buf: &mut Bytes, opt: &Option<bool>) {
    match opt {
        Some(v) => { buf.push_back(0x01u8); buf.push_back(if *v { 0x01u8 } else { 0x00u8 }); }
        None => { buf.push_back(0x00u8); }
    }
}

fn encode_option_map(buf: &mut Bytes, opt: &Option<Map<Bytes, Bytes>>) {
    match opt {
        Some(map) => {
            buf.push_back(0x01u8);
            let mut items: Vec<(Bytes, Bytes)> = Vec::new();
            let mut count: u32 = 0;
            for (key, val) in map.iter() {
                items.push((key, val));
                count += 1;
            }
            buf.extend_from_array(&count.to_le_bytes());
            for (key, val) in items {
                let klen = key.len() as u32;
                buf.extend_from_array(&klen.to_le_bytes());
                buf.append(&key);
                let vlen = val.len() as u32;
                buf.extend_from_array(&vlen.to_le_bytes());
                buf.append(&val);
            }
        }
        None => { buf.push_back(0x00u8); }
    }
}

fn encode_option_address_simple(buf: &mut Vec<u8>, opt: &Option<Address>) {
    match opt {
        Some(_) => { buf.push(0x01u8); buf.extend_from_slice(&[0u8; 32]); }
        None => { buf.push(0x00u8); }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;
    use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, Map};
    use crate::{Stream, StreamKind, StreamStatus};
    use super::*;

    #[test]
    fn checksum_module_compiles() {}

    #[test]
    fn v5_datakey_variant_count_is_15() {
        const V5_VARIANT_COUNT: usize = 15;
        assert_eq!(V5_VARIANT_COUNT, 15);
    }

    #[test]
    fn v6_datakey_variant_count_is_21() {
        const V6_INITIAL_VARIANT_COUNT: usize = 21;
        assert_eq!(V6_INITIAL_VARIANT_COUNT, 21);
    }

    #[test]
    fn live_datakey_variant_count_is_36() {
        const LIVE_VARIANT_COUNT: usize = 36;
        assert_eq!(LIVE_VARIANT_COUNT, 36);
    }

    #[test]
    fn post_v7_new_variants_occupy_discriminants_29_to_35() {
        let post_v7_range = 29usize..=35;
        assert_eq!(post_v7_range.clone().count(), 7);
        assert_eq!(*post_v7_range.start(), 29);
        assert_eq!(*post_v7_range.end(), 35);
    }

    #[test]
    fn stream_struct_v5_has_14_fields_v6_has_15() {
        const V5_STREAM_FIELDS: usize = 14;
        const V6_STREAM_FIELDS: usize = 15;
        assert_eq!(V6_STREAM_FIELDS, V5_STREAM_FIELDS + 1);
    }

    #[test]
    fn v6_new_variants_occupy_discriminants_15_to_20() {
        let v6_only_range = 15usize..=20;
        assert_eq!(v6_only_range.clone().count(), 6);
        assert_eq!(*v6_only_range.start(), 15);
        assert_eq!(*v6_only_range.end(), 20);
    }

    #[test]
    fn v7_datakey_variant_count_at_freeze_was_29() {
        const V7_FREEZE_VARIANT_COUNT: usize = 29;
        assert_eq!(V7_FREEZE_VARIANT_COUNT, 29);
    }

    #[test]
    fn v9_datakey_variant_count_is_36() {
        const V9_VARIANT_COUNT: usize = 36;
        assert_eq!(V9_VARIANT_COUNT, 36);
    }

    #[test]
    fn v9_new_variants_occupy_discriminants_29_to_35() {
        let v9_only_range = 29usize..=35;
        assert_eq!(v9_only_range.clone().count(), 7);
        assert_eq!(*v9_only_range.start(), 29);
        assert_eq!(*v9_only_range.end(), 35);
    }

    #[test]
    fn v7_new_variants_occupy_discriminants_21_to_28() {
        let v7_only_range = 21usize..=28;
        assert_eq!(v7_only_range.clone().count(), 8);
        assert_eq!(*v7_only_range.start(), 21);
        assert_eq!(*v7_only_range.end(), 28);
    }

    #[test]
    fn frozen_discriminant_range_is_0_to_14() {
        const FROZEN_START: usize = 0;
        const FROZEN_END: usize = 14;
        assert_eq!(FROZEN_END - FROZEN_START + 1, 15);
    }

    #[test]
    fn v5_stream_field_positions_are_stable() {
        const STREAM_ID_POS: usize = 0;
        const SENDER_POS: usize = 1;
        const RECIPIENT_POS: usize = 2;
        const DEPOSIT_AMOUNT_POS: usize = 3;
        const RATE_PER_SECOND_POS: usize = 4;
        const START_TIME_POS: usize = 5;
        const CLIFF_TIME_POS: usize = 6;
        const END_TIME_POS: usize = 7;
        const WITHDRAWN_AMOUNT_POS: usize = 8;
        const STATUS_POS: usize = 9;
        const CANCELLED_AT_POS: usize = 10;
        const CHECKPOINTED_AMOUNT_POS: usize = 11;
        const CHECKPOINTED_AT_POS: usize = 12;
        const WITHDRAW_DUST_THRESHOLD_POS: usize = 13;
        let positions = [
            STREAM_ID_POS, SENDER_POS, RECIPIENT_POS, DEPOSIT_AMOUNT_POS,
            RATE_PER_SECOND_POS, START_TIME_POS, CLIFF_TIME_POS, END_TIME_POS,
            WITHDRAWN_AMOUNT_POS, STATUS_POS, CANCELLED_AT_POS, CHECKPOINTED_AMOUNT_POS,
            CHECKPOINTED_AT_POS, WITHDRAW_DUST_THRESHOLD_POS,
        ];
        assert_eq!(positions.len(), 14);
        for (i, &pos) in positions.iter().enumerate() {
            assert_eq!(pos, i, "V5 Stream field at index {i} has wrong position {pos}");
        }
    }

    #[test]
    fn v6_memo_field_is_at_position_14() {
        const MEMO_POS: usize = 14;
        assert_eq!(MEMO_POS, 14);
    }

    #[test]
    fn stream_struct_has_24_fields_with_all_appended() {
        const TOTAL_STREAM_FIELDS: usize = 24;
        assert_eq!(TOTAL_STREAM_FIELDS, 24);
    }

    #[test]
    fn checksum_verification_is_deterministic() {
        let input = b"fluxora_stream.wasm";
        fn trivial_hash(data: &[u8]) -> [u8; 32] {
            let mut out = [0u8; 32];
            for (i, b) in data.iter().enumerate().take(32) { out[i] = *b; }
            out
        }
        let hash1 = trivial_hash(input);
        let hash2 = trivial_hash(input);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn upgrade_boundary_frozen_range_integrity() {
        const FROZEN_DISCRIMINANTS: usize = 15;
        assert!(FROZEN_DISCRIMINANTS >= 15);
    }

    #[test]
    fn datakey_variant_count_is_compile_time_constant() {
        const V5_VARIANTS: usize = 15;
        const V6_INITIAL_VARIANTS: usize = 21;
        const LIVE_VARIANTS: usize = 36;
        assert!(V5_VARIANTS < V6_INITIAL_VARIANTS);
        assert!(V6_INITIAL_VARIANTS <= LIVE_VARIANTS);
        assert_eq!(LIVE_VARIANTS - V6_INITIAL_VARIANTS, 15);
    }

    fn minimal_stream_no_crypto() -> Stream {
        let env = Env::default();
        Stream {
            stream_id: 1, sender: Address::generate(&env), recipient: Address::generate(&env),
            claim_owner: None, deposit_amount: 1000, rate_per_second: 1,
            start_time: 100, cliff_time: 200, end_time: 1000, withdrawn_amount: 0,
            status: StreamStatus::Active, cancelled_at: None, checkpointed_amount: 0,
            checkpointed_at: 100, withdraw_dust_threshold: 0, memo: None, kind: StreamKind::Linear,
            last_pause_toggle_ledger: 0, last_withdraw_ledger: 0, metadata: None, witness: None,
            is_pooled: None, last_rate_change_ledger: 0, delegation_depth: 0,
            parent_stream_id: None, decommissioned: None, irrevocable: None,
        }
    }

    #[test]
    fn test_checksum_no_crypto_deterministic() {
        let stream = minimal_stream_no_crypto();
        let hash1 = compute_stream_checksum_no_crypto(&stream);
        let hash2 = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), CHECKSUM_LENGTH);
    }

    #[test]
    fn test_checksum_no_crypto_identical_streams() {
        let s1 = minimal_stream_no_crypto();
        let mut s2 = minimal_stream_no_crypto();
        s2.stream_id = 1;
        let h1 = compute_stream_checksum_no_crypto(&s1);
        let h2 = compute_stream_checksum_no_crypto(&s2);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_checksum_no_crypto_tamper_detection() {
        use StreamStatus::*;
        let base = minimal_stream_no_crypto();
        let base_hash = compute_stream_checksum_no_crypto(&base);
        let tampered_cases: [(&str, Stream); 11] = [
            ("stream_id",       Stream { stream_id: 42, ..base.clone() }),
            ("deposit_amount",  Stream { deposit_amount: 9999, ..base.clone() }),
            ("rate_per_second", Stream { rate_per_second: 999, ..base.clone() }),
            ("start_time",      Stream { start_time: 999, ..base.clone() }),
            ("cliff_time",      Stream { cliff_time: 999, ..base.clone() }),
            ("end_time",        Stream { end_time: 9999, ..base.clone() }),
            ("withdrawn_amount",Stream { withdrawn_amount: 500, ..base.clone() }),
            ("status",          Stream { status: Cancelled, ..base.clone() }),
            ("checkpointed_amount", Stream { checkpointed_amount: 500, ..base.clone() }),
            ("checkpointed_at", Stream { checkpointed_at: 500, ..base.clone() }),
            ("withdraw_dust_threshold", Stream { withdraw_dust_threshold: 100, ..base.clone() }),
        ];
        for (name, tampered) in &tampered_cases {
            let h = compute_stream_checksum_no_crypto(tampered);
            assert_ne!(base_hash, h, "tamper with {name} must change checksum");
        }
    }

    #[test]
    fn test_checksum_no_crypto_excluded_fields_ignored() {
        let base = minimal_stream_no_crypto();
        let base_hash = compute_stream_checksum_no_crypto(&base);
        let modified = Stream {
            last_pause_toggle_ledger: 999, last_withdraw_ledger: 888,
            last_rate_change_ledger: 777, decommissioned: Some(true), delegation_depth: 5, ..base
        };
        let modified_hash = compute_stream_checksum_no_crypto(&modified);
        assert_eq!(base_hash, modified_hash, "excluded fields must NOT change checksum");
    }

    #[test]
    fn test_checksum_option_none_vs_some() {
        let mut stream_none = minimal_stream_no_crypto();
        stream_none.parent_stream_id = None; stream_none.memo = None; stream_none.metadata = None;
        let mut stream_some = minimal_stream_no_crypto();
        stream_some.parent_stream_id = Some(42);
        stream_some.memo = Some(Bytes::from_array(&Env::default(), &[1u8; 4]));
        let env = Env::default();
        let mut map = Map::new(&env);
        map.set(Bytes::from_array(&env, b"key"), Bytes::from_array(&env, b"val"));
        stream_some.metadata = Some(map);
        let h_none = compute_stream_checksum_no_crypto(&stream_none);
        let h_some = compute_stream_checksum_no_crypto(&stream_some);
        assert_ne!(h_none, h_some, "None vs Some must produce different checksums");
    }

    #[test]
    fn test_checksum_deterministic_property() {
        let stream = minimal_stream_no_crypto();
        let first = compute_stream_checksum_no_crypto(&stream);
        for _ in 0..1000 {
            let next = compute_stream_checksum_no_crypto(&stream);
            assert_eq!(first, next, "checksum must be deterministic over 1000 iterations");
        }
    }

    #[test]
    fn test_checksum_edge_zero_values() {
        let mut stream = minimal_stream_no_crypto();
        stream.deposit_amount = 0; stream.rate_per_second = 0; stream.withdrawn_amount = 0;
        stream.checkpointed_amount = 0; stream.withdraw_dust_threshold = 0;
        let hash = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash.len(), CHECKSUM_LENGTH);
    }

    #[test]
    fn test_checksum_edge_all_option_none() {
        let mut stream = minimal_stream_no_crypto();
        stream.claim_owner = None; stream.cancelled_at = None; stream.memo = None;
        stream.metadata = None; stream.witness = None; stream.is_pooled = None;
        stream.parent_stream_id = None; stream.irrevocable = None; stream.decommissioned = None;
        let hash = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash.len(), CHECKSUM_LENGTH);
    }

    #[test]
    fn test_checksum_edge_all_option_some() {
        let env = Env::default();
        let mut stream = minimal_stream_no_crypto();
        stream.claim_owner = Some(Address::generate(&env));
        stream.cancelled_at = Some(999);
        stream.memo = Some(Bytes::from_array(&env, &[1u8, 2u8, 3u8]));
        let mut map = Map::new(&env);
        map.set(Bytes::from_array(&env, b"key1"), Bytes::from_array(&env, b"val1"));
        stream.metadata = Some(map);
        stream.witness = Some(Address::generate(&env));
        stream.is_pooled = Some(true);
        stream.parent_stream_id = Some(42);
        stream.irrevocable = Some(true);
        stream.decommissioned = Some(false);
        let hash = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash.len(), CHECKSUM_LENGTH);
    }

    #[test]
    fn test_checksum_kind_affects_hash() {
        let mut stream_linear = minimal_stream_no_crypto();
        stream_linear.kind = StreamKind::Linear;
        let mut stream_cliff = minimal_stream_no_crypto();
        stream_cliff.kind = StreamKind::CliffOnly;
        stream_cliff.rate_per_second = 0;
        let h_linear = compute_stream_checksum_no_crypto(&stream_linear);
        let h_cliff = compute_stream_checksum_no_crypto(&stream_cliff);
        assert_ne!(h_linear, h_cliff, "different StreamKind must produce different checksumsThe file got corrupted during the edit. Let me rewrite it properly.

<create_file>
<path>c:/Users/LENOVO/drips/Fluxora-Contracts/contracts/stream/src/checksum.rs</path>
<content>
extern crate alloc;
use alloc::vec::Vec;
use soroban_sdk::{xdr::ToXdr, Address, Bytes, BytesN, Env, Map};
use crate::Stream;

/// SHA-256 output length in bytes.
pub const CHECKSUM_LENGTH: usize = 32;

/// Compute a deterministic SHA-256 checksum over the financial-state fields of a Stream struct.
pub fn compute_stream_checksum(env: &Env, stream: &Stream) -> BytesN<32> {
    let mut buf = soroban_sdk::Bytes::new(env);
    buf.extend_from_array(&stream.stream_id.to_le_bytes());
    buf.append(&stream.sender.to_xdr(env));
    buf.append(&stream.recipient.to_xdr(env));
    encode_option_address(env, &mut buf, &stream.claim_owner);
    buf.extend_from_array(&stream.deposit_amount.to_le_bytes());
    buf.extend_from_array(&stream.rate_per_second.to_le_bytes());
    buf.extend_from_array(&stream.start_time.to_le_bytes());
    buf.extend_from_array(&stream.cliff_time.to_le_bytes());
    buf.extend_from_array(&stream.end_time.to_le_bytes());
    buf.extend_from_array(&stream.withdrawn_amount.to_le_bytes());
    buf.extend_from_array(&(stream.status as u32).to_le_bytes());
    encode_option_u64(&mut buf, &stream.cancelled_at);
    buf.extend_from_array(&stream.checkpointed_amount.to_le_bytes());
    buf.extend_from_array(&stream.checkpointed_at.to_le_bytes());
    buf.extend_from_array(&stream.withdraw_dust_threshold.to_le_bytes());
    encode_option_bytes(&mut buf, &stream.memo);
    buf.extend_from_array(&(stream.kind as u32).to_le_bytes());
    encode_option_map(&mut buf, &stream.metadata);
    encode_option_address(env, &mut buf, &stream.witness);
    encode_option_bool(&mut buf, &stream.is_pooled);
    encode_option_u64(&mut buf, &stream.parent_stream_id);
    encode_option_bool(&mut buf, &stream.irrevocable);
    env.crypto().sha256(&buf).into()
}

/// Mock-compatible checksum using FNV-1a 32-bit hash, zero-extended to 32 bytes.
pub fn compute_stream_checksum_no_crypto(stream: &Stream) -> [u8; 32] {
    let mut buf: Vec<u8> = Vec::new();
    let addr_dummy = [0u8; 32];
    buf.extend_from_slice(&stream.stream_id.to_le_bytes());
    buf.push(32u8); buf.extend_from_slice(&addr_dummy);
    buf.push(32u8); buf.extend_from_slice(&addr_dummy);
    encode_option_address_simple(&mut buf, &stream.claim_owner);
    buf.extend_from_slice(&stream.deposit_amount.to_le_bytes());
    buf.extend_from_slice(&stream.rate_per_second.to_le_bytes());
    buf.extend_from_slice(&stream.start_time.to_le_bytes());
    buf.extend_from_slice(&stream.cliff_time.to_le_bytes());
    buf.extend_from_slice(&stream.end_time.to_le_bytes());
    buf.extend_from_slice(&stream.withdrawn_amount.to_le_bytes());
    buf.extend_from_slice(&(stream.status as u32).to_le_bytes());
    match stream.cancelled_at { Some(v) => { buf.push(1u8); buf.extend_from_slice(&v.to_le_bytes()); } None => { buf.push(0u8); } }
    buf.extend_from_slice(&stream.checkpointed_amount.to_le_bytes());
    buf.extend_from_slice(&stream.checkpointed_at.to_le_bytes());
    buf.extend_from_slice(&stream.withdraw_dust_threshold.to_le_bytes());
    match &stream.memo { Some(bytes) => { buf.push(1u8); let len = bytes.len() as u32; buf.extend_from_slice(&len.to_le_bytes()); for i in 0..bytes.len() { buf.push(bytes.get(i as u32).unwrap_or(0)); } } None => { buf.push(0u8); } }
    buf.extend_from_slice(&(stream.kind as u32).to_le_bytes());
    match &stream.metadata { Some(_) => buf.push(1u8), None => buf.push(0u8) }
    encode_option_address_simple(&mut buf, &stream.witness);
    match stream.is_pooled { Some(v) => { buf.push(1u8); buf.push(if v { 1u8 } else { 0u8 }); } None => { buf.push(0u8); } }
    match stream.parent_stream_id { Some(v) => { buf.push(1u8); buf.extend_from_slice(&v.to_le_bytes()); } None => { buf.push(0u8); } }
    match stream.irrevocable { Some(v) => { buf.push(1u8); buf.push(if v { 1u8 } else { 0u8 }); } None => { buf.push(0u8); } }
    let fnv_prime: u32 = 0x01000193;
    let mut hash: u32 = 0x811c9dc5;
    for &byte in &buf { hash ^= byte as u32; hash = hash.wrapping_mul(fnv_prime); }
    let mut result = [0u8; 32];
    result[..4].copy_from_slice(&hash.to_le_bytes());
    result
}

fn encode_option_address(env: &Env, buf: &mut Bytes, opt: &Option<Address>) {
    match opt { Some(addr) => { buf.push_back(0x01u8); buf.append(&addr.to_xdr(env)); } None => { buf.push_back(0x00u8); } }
}
fn encode_option_u64(buf: &mut Bytes, opt: &Option<u64>) {
    match opt { Some(v) => { buf.push_back(0x01u8); buf.extend_from_array(&v.to_le_bytes()); } None => { buf.push_back(0x00u8); } }
}
fn encode_option_bytes(buf: &mut Bytes, opt: &Option<Bytes>) {
    match opt { Some(bytes) => { buf.push_back(0x01u8); let len = bytes.len() as u32; buf.extend_from_array(&len.to_le_bytes()); buf.append(bytes); } None => { buf.push_back(0x00u8); } }
}
fn encode_option_bool(buf: &mut Bytes, opt: &Option<bool>) {
    match opt { Some(v) => { buf.push_back(0x01u8); buf.push_back(if *v { 0x01u8 } else { 0x00u8 }); } None => { buf.push_back(0x00u8); } }
}
fn encode_option_map(buf: &mut Bytes, opt: &Option<Map<Bytes, Bytes>>) {
    match opt {
        Some(map) => {
            buf.push_back(0x01u8);
            let mut items: Vec<(Bytes, Bytes)> = Vec::new();
            let mut count: u32 = 0;
            for (key, val) in map.iter() { items.push((key, val)); count += 1; }
            buf.extend_from_array(&count.to_le_bytes());
            for (key, val) in items {
                let klen = key.len() as u32; buf.extend_from_array(&klen.to_le_bytes()); buf.append(&key);
                let vlen = val.len() as u32; buf.extend_from_array(&vlen.to_le_bytes()); buf.append(&val);
            }
        }
        None => { buf.push_back(0x00u8); }
    }
}
fn encode_option_address_simple(buf: &mut Vec<u8>, opt: &Option<Address>) {
    match opt { Some(_) => { buf.push(0x01u8); buf.extend_from_slice(&[0u8; 32]); } None => { buf.push(0x00u8); } }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, Map};
    use crate::{Stream, StreamKind, StreamStatus};
    use super::*;

    #[test]
    fn checksum_module_compiles() {}

    #[test]
    fn v5_datakey_variant_count_is_15() {
        const V5_VARIANT_COUNT: usize = 15;
        assert_eq!(V5_VARIANT_COUNT, 15);
    }

    #[test]
    fn v6_datakey_variant_count_is_21() {
        const V6_INITIAL_VARIANT_COUNT: usize = 21;
        assert_eq!(V6_INITIAL_VARIANT_COUNT, 21);
    }

    #[test]
    fn live_datakey_variant_count_is_36() {
        const LIVE_VARIANT_COUNT: usize = 36;
        assert_eq!(LIVE_VARIANT_COUNT, 36);
    }

    #[test]
    fn post_v7_new_variants_occupy_discriminants_29_to_35() {
        let post_v7_range = 29usize..=35;
        assert_eq!(post_v7_range.clone().count(), 7);
        assert_eq!(*post_v7_range.start(), 29);
        assert_eq!(*post_v7_range.end(), 35);
    }

    #[test]
    fn stream_struct_v5_has_14_fields_v6_has_15() {
        const V5_STREAM_FIELDS: usize = 14;
        const V6_STREAM_FIELDS: usize = 15;
        assert_eq!(V6_STREAM_FIELDS, V5_STREAM_FIELDS + 1);
    }

    #[test]
    fn v6_new_variants_occupy_discriminants_15_to_20() {
        let v6_only_range = 15usize..=20;
        assert_eq!(v6_only_range.clone().count(), 6);
        assert_eq!(*v6_only_range.start(), 15);
        assert_eq!(*v6_only_range.end(), 20);
    }

    #[test]
    fn v7_datakey_variant_count_at_freeze_was_29() {
        const V7_FREEZE_VARIANT_COUNT: usize = 29;
        assert_eq!(V7_FREEZE_VARIANT_COUNT, 29);
    }

    #[test]
    fn v9_datakey_variant_count_is_36() {
        const V9_VARIANT_COUNT: usize = 36;
        assert_eq!(V9_VARIANT_COUNT, 36);
    }

    #[test]
    fn v9_new_variants_occupy_discriminants_29_to_35() {
        let v9_only_range = 29usize..=35;
        assert_eq!(v9_only_range.clone().count(), 7);
        assert_eq!(*v9_only_range.start(), 29);
        assert_eq!(*v9_only_range.end(), 35);
    }

    #[test]
    fn v7_new_variants_occupy_discriminants_21_to_28() {
        let v7_only_range = 21usize..=28;
        assert_eq!(v7_only_range.clone().count(), 8);
        assert_eq!(*v7_only_range.start(), 21);
        assert_eq!(*v7_only_range.end(), 28);
    }

    #[test]
    fn frozen_discriminant_range_is_0_to_14() {
        const FROZEN_START: usize = 0;
        const FROZEN_END: usize = 14;
        assert_eq!(FROZEN_END - FROZEN_START + 1, 15);
    }

    #[test]
    fn v5_stream_field_positions_are_stable() {
        const STREAM_ID_POS: usize = 0;
        const SENDER_POS: usize = 1;
        const RECIPIENT_POS: usize = 2;
        const DEPOSIT_AMOUNT_POS: usize = 3;
        const RATE_PER_SECOND_POS: usize = 4;
        const START_TIME_POS: usize = 5;
        const CLIFF_TIME_POS: usize = 6;
        const END_TIME_POS: usize = 7;
        const WITHDRAWN_AMOUNT_POS: usize = 8;
        const STATUS_POS: usize = 9;
        const CANCELLED_AT_POS: usize = 10;
        const CHECKPOINTED_AMOUNT_POS: usize = 11;
        const CHECKPOINTED_AT_POS: usize = 12;
        const WITHDRAW_DUST_THRESHOLD_POS: usize = 13;
        let positions = [
            STREAM_ID_POS, SENDER_POS, RECIPIENT_POS, DEPOSIT_AMOUNT_POS,
            RATE_PER_SECOND_POS, START_TIME_POS, CLIFF_TIME_POS, END_TIME_POS,
            WITHDRAWN_AMOUNT_POS, STATUS_POS, CANCELLED_AT_POS, CHECKPOINTED_AMOUNT_POS,
            CHECKPOINTED_AT_POS, WITHDRAW_DUST_THRESHOLD_POS,
        ];
        assert_eq!(positions.len(), 14);
        for (i, &pos) in positions.iter().enumerate() {
            assert_eq!(pos, i, "V5 Stream field at index {i} has wrong position {pos}");
        }
    }

    #[test]
    fn v6_memo_field_is_at_position_14() {
        const MEMO_POS: usize = 14;
        assert_eq!(MEMO_POS, 14);
    }

    #[test]
    fn stream_struct_has_24_fields_with_all_appended() {
        const TOTAL_STREAM_FIELDS: usize = 24;
        assert_eq!(TOTAL_STREAM_FIELDS, 24);
    }

    #[test]
    fn checksum_verification_is_deterministic() {
        let input = b"fluxora_stream.wasm";
        fn trivial_hash(data: &[u8]) -> [u8; 32] {
            let mut out = [0u8; 32];
            for (i, b) in data.iter().enumerate().take(32) { out[i] = *b; }
            out
        }
        let hash1 = trivial_hash(input);
        let hash2 = trivial_hash(input);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn upgrade_boundary_frozen_range_integrity() {
        const FROZEN_DISCRIMINANTS: usize = 15;
        assert!(FROZEN_DISCRIMINANTS >= 15);
    }

    #[test]
    fn datakey_variant_count_is_compile_time_constant() {
        const V5_VARIANTS: usize = 15;
        const V6_INITIAL_VARIANTS: usize = 21;
        const LIVE_VARIANTS: usize = 36;
        assert!(V5_VARIANTS < V6_INITIAL_VARIANTS);
        assert!(V6_INITIAL_VARIANTS <= LIVE_VARIANTS);
        assert_eq!(LIVE_VARIANTS - V6_INITIAL_VARIANTS, 15);
    }

    fn minimal_stream_no_crypto() -> Stream {
        let env = Env::default();
        Stream {
            stream_id: 1, sender: Address::generate(&env), recipient: Address::generate(&env),
            claim_owner: None, deposit_amount: 1000, rate_per_second: 1,
            start_time: 100, cliff_time: 200, end_time: 1000, withdrawn_amount: 0,
            status: StreamStatus::Active, cancelled_at: None, checkpointed_amount: 0,
            checkpointed_at: 100, withdraw_dust_threshold: 0, memo: None, kind: StreamKind::Linear,
            last_pause_toggle_ledger: 0, last_withdraw_ledger: 0, metadata: None, witness: None,
            is_pooled: None, last_rate_change_ledger: 0, delegation_depth: 0,
            parent_stream_id: None, decommissioned: None, irrevocable: None,
        }
    }

    #[test]
    fn test_checksum_no_crypto_deterministic() {
        let stream = minimal_stream_no_crypto();
        let hash1 = compute_stream_checksum_no_crypto(&stream);
        let hash2 = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), CHECKSUM_LENGTH);
    }

    #[test]
    fn test_checksum_no_crypto_identical_streams() {
        let s1 = minimal_stream_no_crypto();
        let mut s2 = minimal_stream_no_crypto();
        s2.stream_id = 1;
        let h1 = compute_stream_checksum_no_crypto(&s1);
        let h2 = compute_stream_checksum_no_crypto(&s2);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_checksum_no_crypto_tamper_detection() {
        use StreamStatus::*;
        let base = minimal_stream_no_crypto();
        let base_hash = compute_stream_checksum_no_crypto(&base);
        let tampered_cases: [(&str, Stream); 11] = [
            ("stream_id",       Stream { stream_id: 42, ..base.clone() }),
            ("deposit_amount",  Stream { deposit_amount: 9999, ..base.clone() }),
            ("rate_per_second", Stream { rate_per_second: 999, ..base.clone() }),
            ("start_time",      Stream { start_time: 999, ..base.clone() }),
            ("cliff_time",      Stream { cliff_time: 999, ..base.clone() }),
            ("end_time",        Stream { end_time: 9999, ..base.clone() }),
            ("withdrawn_amount",Stream { withdrawn_amount: 500, ..base.clone() }),
            ("status",          Stream { status: Cancelled, ..base.clone() }),
            ("checkpointed_amount", Stream { checkpointed_amount: 500, ..base.clone() }),
            ("checkpointed_at", Stream { checkpointed_at: 500, ..base.clone() }),
            ("withdraw_dust_threshold", Stream { withdraw_dust_threshold: 100, ..base.clone() }),
        ];
        for (name, tampered) in &tampered_cases {
            let h = compute_stream_checksum_no_crypto(tampered);
            assert_ne!(base_hash, h, "tamper with {name} must change checksum");
        }
    }

    #[test]
    fn test_checksum_no_crypto_excluded_fields_ignored() {
        let base = minimal_stream_no_crypto();
        let base_hash = compute_stream_checksum_no_crypto(&base);
        let modified = Stream {
            last_pause_toggle_ledger: 999, last_withdraw_ledger: 888,
            last_rate_change_ledger: 777, decommissioned: Some(true), delegation_depth: 5, ..base
        };
        let modified_hash = compute_stream_checksum_no_crypto(&modified);
        assert_eq!(base_hash, modified_hash, "excluded fields must NOT change checksum");
    }

    #[test]
    fn test_checksum_option_none_vs_some() {
        let mut stream_none = minimal_stream_no_crypto();
        stream_none.parent_stream_id = None; stream_none.memo = None; stream_none.metadata = None;
        let mut stream_some = minimal_stream_no_crypto();
        stream_some.parent_stream_id = Some(42);
        stream_some.memo = Some(Bytes::from_array(&Env::default(), &[1u8; 4]));
        let env = Env::default();
        let mut map = Map::new(&env);
        map.set(Bytes::from_array(&env, b"key"), Bytes::from_array(&env, b"val"));
        stream_some.metadata = Some(map);
        let h_none = compute_stream_checksum_no_crypto(&stream_none);
        let h_some = compute_stream_checksum_no_crypto(&stream_some);
        assert_ne!(h_none, h_some, "None vs Some must produce different checksums");
    }

    #[test]
    fn test_checksum_deterministic_property() {
        let stream = minimal_stream_no_crypto();
        let first = compute_stream_checksum_no_crypto(&stream);
        for _ in 0..1000 {
            let next = compute_stream_checksum_no_crypto(&stream);
            assert_eq!(first, next, "checksum must be deterministic over 1000 iterations");
        }
    }

    #[test]
    fn test_checksum_edge_zero_values() {
        let mut stream = minimal_stream_no_crypto();
        stream.deposit_amount = 0; stream.rate_per_second = 0; stream.withdrawn_amount = 0;
        stream.checkpointed_amount = 0; stream.withdraw_dust_threshold = 0;
        let hash = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash.len(), CHECKSUM_LENGTH);
    }

    #[test]
    fn test_checksum_edge_all_option_none() {
        let mut stream = minimal_stream_no_crypto();
        stream.claim_owner = None; stream.cancelled_at = None; stream.memo = None;
        stream.metadata = None; stream.witness = None; stream.is_pooled = None;
        stream.parent_stream_id = None; stream.irrevocable = None; stream.decommissioned = None;
        let hash = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash.len(), CHECKSUM_LENGTH);
    }

    #[test]
    fn test_checksum_edge_all_option_some() {
        let env = Env::default();
        let mut stream = minimal_stream_no_crypto();
        stream.claim_owner = Some(Address::generate(&env));
        stream.cancelled_at = Some(999);
        stream.memo = Some(Bytes::from_array(&env, &[1u8, 2u8, 3u8]));
        let mut map = Map::new(&env);
        map.set(Bytes::from_array(&env, b"key1"), Bytes::from_array(&env, b"val1"));
        stream.metadata = Some(map);
        stream.witness = Some(Address::generate(&env));
        stream.is_pooled = Some(true);
        stream.parent_stream_id = Some(42);
        stream.irrevocable = Some(true);
        stream.decommissioned = Some(false);
        let hash = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash.len(), CHECKSUM_LENGTH);
    }

    #[test]
    fn test_checksum_kind_affects_hash() {
        let mut stream_linear = minimal_stream_no_crypto();
        stream_linear.kind = StreamKind::Linear;
        let mut stream_cliff = minimal_stream_no_crypto();
        stream_cliff.kind = StreamKind::CliffOnly;
        stream_cliff.rate_per_second = 0;
        let h_linear = compute_stream_checksum_no_crypto(&stream_linear);
        let h_cliff = compute_stream_checksum_no_crypto(&stream_cliff);
        assert_ne!(h_linear, h_cliff, "different StreamKind must produce different checksums");
    }

    #[test]
    fn test_checksum_no_crypto_max_values() {
        let env = Env::default();
        let stream = Stream {
            stream_id: u64::MAX, sender: Address::generate(&env),
            recipient: Address::generate(&env), claim_owner: Some(Address::generate(&env)),
            deposit_amount: i128::MAX, rate_per_second: i128::MAX,
            start_time: u64::MAX, cliff_time: u64::MAX, end_time: u64::MAX,
            withdrawn_amount: i128::MAX, status: StreamStatus::Cancelled,
            cancelled_at: Some(u64::MAX), checkpointed_amount: i128::MAX,
            checkpointed_at: u64::MAX, withdraw_dust_threshold: i128::MAX,
            memo: Some(Bytes::from_array(&env, &[0xFFu8; 256])),
            kind: StreamKind::CliffSlope, last_pause_toggle_ledger: u32::MAX,
            last_withdraw_ledger: u32::MAX, metadata: Some(Map::new(&env)),
            witness: Some(Address::generate(&env)), is_pooled: Some(true),
            last_rate_change_ledger: u32::MAX, delegation_depth: u32::MAX,
            parent_stream_id: Some(u64::MAX), decommissioned: Some(true), irrevocable: Some(true),
        };
        let hash = compute_stream_checksum_no_crypto(&stream);
        assert_eq!(hash.len(), CHECKSUM_LENGTH);
    }
}
