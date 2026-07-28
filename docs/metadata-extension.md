# Per-Stream Metadata Extension

Standalone reference for the optional bounded key-value `metadata` map attached to
streams at creation time (issue #580, `CONTRACT_VERSION` 4+).

**Implementation:** `validate_metadata` in `contracts/stream/src/storage.rs`; creation
paths in `contracts/stream/src/lib.rs`.

**Related:** [streaming.md § Per-stream metadata](./streaming.md#per-stream-metadata-tlv-extension-contract_version-4)

---

## Overview

Each stream may store `metadata: Option<Map<Bytes, Bytes>>` — a Soroban map of
opaque byte keys and values for integrator correlation (invoice IDs, project codes,
external URIs). Metadata is validated once at creation, persisted on the `Stream`
struct, emitted in `StreamCreated`, and exposed read-only via `get_stream_metadata`.

Metadata is **immutable** after the stream is created. No entry-point mutates or
clears the map.

---

## Bounds

| Bound | Constant | Value | Error on violation |
|-------|----------|-------|-------------------|
| Maximum key-value pairs | `MAX_METADATA_KEYS` | 8 | `MetadataTooLarge` |
| Maximum aggregate bytes (all keys + values) | `MAX_METADATA_BYTES` | 512 | `MetadataTooLarge` |
| Maximum single key length | `MAX_METADATA_KEY_BYTES` | 32 | `MetadataTooLarge` |
| Maximum single value length | `MAX_METADATA_VALUE_BYTES` | 128 | `MetadataTooLarge` |

Empty keys and empty values are allowed. `Some(empty map)` is valid.

---

## Validation order

`validate_metadata` runs **before** stream ID allocation and token transfer:

1. Reject if `metadata.len() > MAX_METADATA_KEYS`
2. For each `(key, value)` pair (iteration order):
   - Reject if `key.len() > MAX_METADATA_KEY_BYTES`
   - Reject if `value.len() > MAX_METADATA_VALUE_BYTES`
   - Accumulate `key.len() + value.len()` into running total (saturating add)
   - Reject if running total `> MAX_METADATA_BYTES`

On failure: `ContractError::MetadataTooLarge`; no stream ID consumed; no tokens moved.

Creation paths that call validation: `create_stream`, `create_streams`,
`create_streams_relative`, `create_streams_partial`, `create_stream_from_template`,
`create_stream_offer`.

---

## Compatibility matrix

Metadata is written at creation and **never mutated**. Operations either ignore
metadata or copy it to a new stream.

| Operation | Metadata behavior |
|-----------|-------------------|
| `create_stream` / `create_streams` / relative / partial | **Set** — validated and stored |
| `create_stream_from_template` | **Set** — caller-supplied map validated |
| `create_stream_offer` | **Set on offer** — stored on `StreamOffer`; copied on accept |
| `accept_stream_offer` | **Copied** — offer metadata becomes stream metadata |
| `reject_stream_offer` / `cancel_stream_offer` | Unaffected — offer removed; no stream yet |
| `get_stream_metadata` | **Read** — permissionless; works on active, paused, cancelled streams |
| `pause_stream` / `resume_stream` | Unchanged |
| `cancel_stream` / `keeper_cancel` / `witnessed_cancel_stream` | Unchanged — still readable |
| `withdraw` / `withdraw_to` / `batch_withdraw` / `batch_withdraw_to` | Unchanged |
| `delegated_withdraw` | Unchanged |
| `top_up_stream` | Unchanged |
| `update_rate_per_second` / `decrease_rate_per_second` | Unchanged |
| `extend_stream_end_time` / `shorten_stream_end_time` | Unchanged |
| `transfer_sender` / recipient rotation | Unchanged |
| `set_auto_renew` / `get_auto_renew` | Unchanged — auto-renew flag only |
| `clone_stream` | **Inherited** — clone receives `source.metadata.clone()` |
| `close_completed_stream` / `close_cancelled_stream` | Entry removed — metadata no longer queryable |

---

## Upgrade path (V5 → current)

V5 `Stream` entries had no `metadata` field. Soroban XDR decoding is forward-compatible:
V5-encoded streams decode with `metadata == None`. No migration is required.

---

## Regression surface

Executable coverage lives in `contracts/stream/tests/metadata_extension.rs`:

- Bounds validation (keys, values, aggregate)
- Immutability across pause/resume/cancel/withdraw/top-up
- Batch and template creation paths
- Offer accept round-trip
- Post-`shorten_stream_end_time` readability
- `get_stream_metadata` on cancelled streams

Run:

```bash
cargo test -p fluxora_stream --test metadata_extension --features testutils
```
