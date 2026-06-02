# PR #580 — Per-stream metadata TLV key-value extension

## Summary

Adds an optional, bounded, immutable `metadata: Option<Map<Bytes, Bytes>>` field to every
Fluxora stream. Integrations that need structured data beyond the 64-byte `memo` field (e.g.
invoice IDs, project codes, external reference URIs) can now attach up to 8 key-value pairs
with a 512-byte aggregate cap directly at stream-creation time.

## What changed

### `contracts/stream/src/lib.rs`

| Area | Change |
|---|---|
| **Constants** | Added `MAX_METADATA_KEYS = 8`, `MAX_METADATA_BYTES = 512`, `MAX_METADATA_KEY_BYTES = 32`, `MAX_METADATA_VALUE_BYTES = 128`; also backfilled previously-referenced but missing constants: `MAX_MEMO_BYTES`, `MAX_PAUSE_REASON_BYTES`, `MAX_TEMPLATES_PER_OWNER`, `MAX_GLOBAL_TEMPLATES`. |
| **`ContractError`** | Added `MetadataTooLarge = 21` (returned when any metadata bound is violated); backfilled `TemplateNotFound = 17`, `TemplateLimitExceeded = 18`, `TemplateUnauthorized = 19`, `PauseReasonTooLong = 20`. |
| **`DataKey`** | Appended `MaxRatePerSecond` and `LastPauseRecord(PauseKind)` to the end (preserves all existing discriminants). |
| **`Stream` struct** | Fixed keyword from `enum` to `struct` (was a pre-existing typo); added `metadata: Option<Map<Bytes, Bytes>>` field. |
| **`StreamCreated` event** | Added `metadata` field — indexers now receive the full metadata at creation time. |
| **`CreateStreamParams`** | Added `metadata` field so `create_streams` / `create_streams_partial` can carry metadata per entry. |
| **`CreateStreamRelativeParams`** | Added `metadata` field; propagated into the `CreateStreamParams` conversion inside `create_streams_relative`. |
| **`validate_metadata()`** | New standalone helper function — validates key count, per-key bytes, per-value bytes, and aggregate bytes; returns `ContractError::MetadataTooLarge` on any violation. Called before any stream ID is allocated. |
| **`persist_new_stream()`** | Added `metadata` parameter; calls `validate_metadata`, stores field in `Stream`, emits field in `StreamCreated` event. |
| **`persist_new_stream_skip_index()`** | Same additions as above (used by batched `create_streams`). |
| **`create_stream()`** | Added `metadata: Option<Map<Bytes, Bytes>>` as last positional parameter. |
| **`create_stream_relative_inner()`** | Passes `params.metadata` through to `create_stream`. |
| **`create_streams()`** | Passes `params.metadata` through to `persist_new_stream_skip_index`. |
| **`create_streams_relative()`** | Propagates `rel.metadata` into the `CreateStreamParams` conversion. |
| **`create_streams_partial()`** | Passes `params.metadata` through to `persist_new_stream`. |
| **`create_stream_from_template()`** | Added `metadata` parameter; passes into `CreateStreamRelativeParams`. |
| **`get_stream_metadata()`** | New permissionless query entrypoint: returns `Option<Map<Bytes, Bytes>>` for a given stream ID, or `ContractError::StreamNotFound`. |
| **`CONTRACT_VERSION`** | Bumped `3 → 4`. |

### `contracts/stream/tests/metadata_extension.rs` _(new file)_

34 tests covering:

- `None` metadata stored and returned as `None`
- Empty `Some({})` map is valid
- Single and multiple key-value pairs round-trip correctly
- `MAX_METADATA_KEYS` entries accepted; `MAX_METADATA_KEYS + 1` rejected with `MetadataTooLarge`
- Key length exactly at limit accepted; one byte over rejected
- Value length exactly at limit accepted; one byte over rejected
- Aggregate bytes exactly at limit accepted; one byte over rejected
- Metadata is unchanged after pause/resume, cancel, and partial withdraw (immutability)
- `StreamCreated` event is emitted (event presence verified)
- `create_streams` batch — each entry stores its own independent metadata
- `create_streams_relative` passes metadata through
- `create_streams_partial` records per-entry failure for invalid metadata
- **Security**: stream ID counter does not advance on validation failure
- **Security**: sender token balance is unchanged on validation failure
- `get_stream_metadata` returns `ContractError::StreamNotFound` for unknown IDs
- Two streams store independent metadata (no cross-contamination)
- `version()` returns `4`

### `docs/streaming.md`

Added a **"Per-stream metadata (TLV extension, CONTRACT_VERSION 4)"** section documenting:
- API table (`create_stream`, `get_stream_metadata`)
- Bound table (`MAX_METADATA_KEYS`, `MAX_METADATA_BYTES`, `MAX_METADATA_KEY_BYTES`, `MAX_METADATA_VALUE_BYTES`)
- Immutability invariants and security guarantees
- Rust client example
- Updated entrypoint index to include `get_stream_metadata`

### Existing test files

All existing test files updated to pass `&None` for the new `metadata` positional parameter
in `create_stream` / `try_create_stream`, and `metadata: None` in `CreateStreamParams` /
`CreateStreamRelativeParams` struct literals. No existing test behavior was changed.

## Security notes

1. **Fail-before-allocate**: `validate_metadata` is called before `read_stream_count` /
   `set_stream_count`, so a malformed metadata submission never wastes a stream ID and
   triggers no token transfer.
2. **Immutable post-creation**: The `metadata` field is written once (in `persist_new_stream`)
   and has no setter. All state-mutation entry-points (pause, resume, cancel, withdraw,
   rate-change, top-up) leave it untouched.
3. **DoS bounds**: The aggregate 512-byte cap and 8-key count cap prevent unbounded ledger-
   entry growth. Each validation check short-circuits as soon as a bound is exceeded.
4. **No new auth surface**: `get_stream_metadata` is permissionless and read-only. No
   new privileged operations were introduced.
5. **Arithmetic safety**: The aggregate byte accumulation uses `checked_add`; overflow
   returns `ContractError::MetadataTooLarge` rather than wrapping.

## Breaking changes

- `create_stream` gains a new positional parameter `metadata` (last position). Callers must
  pass `None` or a validated map. All in-tree test files have been updated.
- `create_stream_from_template` gains a new `metadata` parameter.
- `StreamCreated` event shape gains the `metadata` field — indexers must update parsers.
- `Stream` storage struct gains the `metadata` field — existing on-chain entries (from
  previous contract instances) will not have this field; redeploy required per the standard
  Soroban migration path.
- `CONTRACT_VERSION` is now `4`.

## Test plan

```bash
cargo test -p fluxora_stream --test metadata_extension
cargo test -p fluxora_stream
```

Expected: all existing tests pass with `&None` metadata; all 34 new metadata tests pass.
