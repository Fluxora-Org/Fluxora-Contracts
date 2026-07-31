# Checksum Handling Refinement - TODO

## Step 1: Refactor `checksum.rs` to expose real API
- [x] Move `checksum` module from `#[cfg(test)]` to always-compiled in `lib.rs`
- [x] Add `compute_stream_checksum(env, stream) -> [u8; 32]` with explicit field inclusion
- [x] Document `CHECKSUM_INCLUDED_FIELDS` / `CHECKSUM_EXCLUDED_FIELDS` constants
- [x] Keep existing variant-count tests gated behind `#[cfg(test)]`

## Step 2: Update `checksum_tamper_tests.rs`
- [x] Import real `compute_stream_checksum` from `fluxora_stream`
- [x] Remove local ad-hoc helper function
- [x] Clean up orphaned code in test file


- [x] Edge cases: zero values, max values, all-Option None vs Some
- [x] Tamper detection for each included field
- [x] Excluded fields ignored
- [x] StreamKind affects hash
