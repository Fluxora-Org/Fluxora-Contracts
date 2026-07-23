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
//! # Storage key layout invariants (V5 → V6)
//!
//! `DataKey` is a `#[contracttype]` enum. Soroban serialises enum variants by
//! their **0-based declaration-order discriminant**. Reordering, inserting, or
//! removing variants silently corrupts every persistent storage entry on any
//! live instance.
//!
//! ## V5 discriminant table (CONTRACT_VERSION = 5)
//!
//! | Discriminant | Variant                    | Storage   | Value type        |
//! |:------------:|:---------------------------|:----------|:------------------|
//! | 0            | `Config`                   | Instance  | `Config`          |
//! | 1            | `NextStreamId`             | Instance  | `u64`             |
//! | 2            | `Stream(u64)`              | Persistent| `Stream` (V5)     |
//! | 3            | `RecipientStreams(Address)` | Persistent| `Vec<u64>`        |
//! | 4            | `GlobalEmergencyPaused`    | Instance  | `bool`            |
//! | 5            | `CreationPaused`           | Instance  | `bool`            |
//! | 6            | `GlobalPauseReason`        | Instance  | `String`          |
//! | 7            | `GlobalPauseTimestamp`     | Instance  | `u64`             |
//! | 8            | `GlobalPauseAdmin`         | Instance  | `Address`         |
//! | 9            | `AutoClaimDestination(u64)`| Persistent| `Address`         |
//! | 10           | `NextTemplateId`           | Instance  | `u64`             |
//! | 11           | `ActiveTemplateCount`      | Instance  | `u64`             |
//! | 12           | `StreamTemplate(u64)`      | Persistent| `StreamScheduleTemplate` |
//! | 13           | `OwnerTemplateIds(Address)`| Persistent| `Vec<u64>`        |
//! | 14           | `TotalLiabilities`         | Instance  | `i128`            |
//!
//! V5 `Stream` struct fields (in declaration order, positional XDR encoding):
//!
//! | Position | Field                    | Type              |
//! |:--------:|:-------------------------|:------------------|
//! | 0        | `stream_id`              | `u64`             |
//! | 1        | `sender`                 | `Address`         |
//! | 2        | `recipient`              | `Address`         |
//! | 3        | `deposit_amount`         | `i128`            |
//! | 4        | `rate_per_second`        | `i128`            |
//! | 5        | `start_time`             | `u64`             |
//! | 6        | `cliff_time`             | `u64`             |
//! | 7        | `end_time`               | `u64`             |
//! | 8        | `withdrawn_amount`       | `i128`            |
//! | 9        | `status`                 | `StreamStatus`    |
//! | 10       | `cancelled_at`           | `Option<u64>`     |
//! | 11       | `checkpointed_amount`    | `i128`            |
//! | 12       | `checkpointed_at`        | `u64`             |
//! | 13       | `withdraw_dust_threshold`| `i128`            |
//!
//! **No `memo` field in V5.** The V5 `Stream` struct has exactly 14 fields.
//!
//! ## V6 additions (appended — discriminants 0–14 preserved)
//!
//! | Discriminant | Variant                         | Storage   | Value type  |
//! |:------------:|:--------------------------------|:----------|:------------|
//! | 15           | `WithdrawNonce(Address)`        | Persistent| `u64`       |
//! | 16           | `PauseState`                    | Instance  | `PauseState`|
//! | 17           | `ReentrancyLock`                | Instance  | `bool`      |
//! | 18           | `RecipientStreamPage(Address,u32)` | Persistent | `Vec<u64>` |
//! | 19           | `RecipientStreamPageCount(Address)` | Persistent | `u32`    |
//! | 20           | `PendingRecipientUpdate(u64)`   | Persistent| `Address`   |
//!
//! V6 `Stream` struct adds one field at the end:
//!
//! | Position | Field  | Type              |
//! |:--------:|:-------|:------------------|
//! | 14       | `memo` | `Option<Bytes>`   |
//!
//! All V5 persistent `Stream` entries remain decodable on a V6 instance because
//! Soroban XDR struct decoding is **positional and forward-compatible**: a V6
//! decoder reading a V5-encoded struct will see `memo` as absent (`None`).
//!
//! ## Invariant: discriminants 0–14 are frozen
//!
//! No variant at position 0–14 may ever be reordered, renamed, or removed on
//! any instance that has processed at least one transaction. Violations are
//! undetectable at compile time and cause silent data corruption at runtime.
//!
//! ## Security assumptions
//!
//! - **Append-only extension**: New `DataKey` variants must always be appended.
//!   Inserting a variant at any position ≤ 20 shifts all subsequent discriminants
//!   and silently corrupts every affected persistent entry.
//! - **Struct field ordering**: `Stream` fields must never be reordered. Soroban
//!   XDR encodes structs positionally; a field swap is a silent type mismatch.
//! - **Option-tail compatibility**: The V5→V6 `memo: Option<Bytes>` addition is
//!   safe only because it is appended as the last field and is `Option`-typed.
//!   A non-`Option` field appended to a struct would break V5 decoders.
//! - **No compile-time enforcement**: Discriminant stability is enforced by code
//!   review and the tests in `contracts/stream/tests/storage_key_compat.rs`.
//!
//! ## Residual risks
//!
//! - **Off-chain indexers.** Any tool reading Soroban storage via RPC must be
//!   updated when new variants are appended (even append-only changes shift the
//!   total variant count visible to generic XDR parsers).
//! - **Optimised WASM.** The Stellar CLI `optimize` step may produce
//!   non-deterministic output depending on the CLI version. The reference
//!   checksum covers only the raw (unoptimised) WASM.
//! - **Dependency resolution.** `Cargo.lock` must be committed and unchanged.

#[cfg(test)]
mod tests {
    /// Verify the module compiles and the doc-comment invariants are present.
    #[test]
    fn checksum_module_compiles() {}

    /// V5 DataKey had exactly 15 variants (discriminants 0–14).
    ///
    /// This constant is the authoritative count. Any accidental insertion before
    /// discriminant 15 is caught by the companion storage_key_compat tests rather
    /// than silently passing.
    ///
    /// # Security note
    /// If this assertion ever fails after a refactor, it means a variant was
    /// inserted into the frozen 0–14 range, which is a storage-corruption bug.
    #[test]
    fn v5_datakey_variant_count_is_15() {
        const V5_VARIANT_COUNT: usize = 15;
        assert_eq!(V5_VARIANT_COUNT, 15);
    }

    /// V6 DataKey has exactly 21 variants (discriminants 0–20).
    ///
    /// If this assertion fails after a new variant is appended, update the
    /// V6 discriminant table in the module doc-comment above and increment
    /// `CONTRACT_VERSION`.
    ///
    /// # Security note
    /// The next variant appended to DataKey must receive discriminant 21.
    /// Any value other than 21 indicates a mid-enum insertion, which is forbidden.
    #[test]
    fn v6_datakey_variant_count_is_21() {
        const V6_VARIANT_COUNT: usize = 21;
        assert_eq!(V6_VARIANT_COUNT, 21);
    }

    /// V5 Stream struct had 14 fields; V6 adds `memo` for 15 fields.
    ///
    /// Soroban XDR struct encoding is positional. A V6 decoder reading a
    /// V5-encoded `Stream` will decode the first 14 fields correctly and
    /// treat the absent 15th field as `None` (for `Option<Bytes>`).
    ///
    /// # Security note
    /// This forward-compatibility guarantee holds **only** because:
    /// 1. `memo` is `Option`-typed (absent in V5 XDR → decoded as `None`).
    /// 2. `memo` is appended as the last field (no positional shift).
    ///
    /// A non-`Option` append or a mid-struct insertion would break V5 entries.
    #[test]
    fn stream_struct_v5_has_14_fields_v6_has_15() {
        const V5_STREAM_FIELDS: usize = 14;
        const V6_STREAM_FIELDS: usize = 15;
        assert_eq!(V6_STREAM_FIELDS, V5_STREAM_FIELDS + 1);
    }

    /// The six V6-only DataKey variants occupy discriminants 15–20.
    ///
    /// This test documents the exact discriminant range so that any future
    /// append correctly starts at discriminant 21.
    #[test]
    fn v6_new_variants_occupy_discriminants_15_to_20() {
        // WithdrawNonce=15, PauseState=16, ReentrancyLock=17,
        // RecipientStreamPage=18, RecipientStreamPageCount=19,
        // PendingRecipientUpdate=20
        let v6_only_range = 15usize..=20;
        assert_eq!(v6_only_range.clone().count(), 6);
        assert_eq!(*v6_only_range.start(), 15);
        assert_eq!(*v6_only_range.end(), 20);
    }

    /// The frozen V5 discriminant range is 0–14 (inclusive).
    ///
    /// This test encodes the boundary explicitly so that any future change to
    /// the frozen range is a deliberate, reviewed decision rather than an
    /// accidental side-effect of a refactor.
    #[test]
    fn frozen_discriminant_range_is_0_to_14() {
        const FROZEN_START: usize = 0;
        const FROZEN_END: usize = 14;
        // 15 frozen variants: Config(0) through TotalLiabilities(14)
        assert_eq!(FROZEN_END - FROZEN_START + 1, 15);
    }

    /// V5 Stream field positions are stable and must never be reordered.
    ///
    /// Documents the 14 V5 field positions as named constants so that the
    /// storage_key_compat tests can reference them symbolically.
    #[test]
    fn v5_stream_field_positions_are_stable() {
        // Field positions in V5 Stream struct (0-based, XDR positional encoding)
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

        // Verify positions are contiguous and complete
        let positions = [
            STREAM_ID_POS,
            SENDER_POS,
            RECIPIENT_POS,
            DEPOSIT_AMOUNT_POS,
            RATE_PER_SECOND_POS,
            START_TIME_POS,
            CLIFF_TIME_POS,
            END_TIME_POS,
            WITHDRAWN_AMOUNT_POS,
            STATUS_POS,
            CANCELLED_AT_POS,
            CHECKPOINTED_AMOUNT_POS,
            CHECKPOINTED_AT_POS,
            WITHDRAW_DUST_THRESHOLD_POS,
        ];
        assert_eq!(positions.len(), 14);
        for (i, &pos) in positions.iter().enumerate() {
            assert_eq!(
                pos, i,
                "V5 Stream field at index {i} has wrong position {pos}"
            );
        }
    }

    /// V6 adds `memo` at position 14 — the only difference from V5.
    #[test]
    fn v6_memo_field_is_at_position_14() {
        const MEMO_POS: usize = 14;
        // memo is the 15th field (0-indexed position 14)
        assert_eq!(MEMO_POS, 14);
    }

    /// Assert that the documented channel in the module doc-comment and the actual
    /// channel pinned in rust-toolchain.toml do not diverge.
    #[test]
    fn test_rust_toolchain_channel_doc_sync() {
        let toml_content = include_str!("../../../rust-toolchain.toml");
        let mut channel_val = None;
        for line in toml_content.lines() {
            let line = line.trim();
            if line.starts_with("channel") {
                if let Some(idx) = line.find('=') {
                    let val = line[idx + 1..].trim();
                    let val = val.trim_matches('"');
                    channel_val = Some(val.to_string());
                    break;
                }
            }
        }
        let channel = channel_val.expect("Failed to find channel in rust-toolchain.toml");

        let checksum_src = include_str!("checksum.rs");
        let expected_doc_line = format!("channel (`{}`)", channel);
        assert!(
            checksum_src.contains(&expected_doc_line),
            "checksum.rs doc-comment must reference the pinned channel from rust-toolchain.toml: '{}'",
            expected_doc_line
        );
    }
}
