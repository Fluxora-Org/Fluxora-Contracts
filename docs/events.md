# Contract event schema

This document lists all events emitted by the `FluxoraStream` contract, the exact
topics used, and the data schema (field names and Rust/Soroban types). Use this
as the canonical source of truth for indexers and backend parsers. The schemas
below are derived directly from the contract source `contracts/stream/src/lib.rs`.

Notes:
- Soroban events contain an ordered list of topics and a single `data` payload.
- Topics shown below are the literal values used in `env.events().publish(...)`.
- Types use the contract's Rust types (e.g. `u64`, `i128`, `Address`).
- Keep this file in sync with the contract when event shapes change.

## Event list

| Event name | Topic(s) | Data (shape & types) | When emitted |
|---|---:|---|---|
| StreamCreated | ["created", stream_id] | deposit_amount: i128 | When a stream is successfully created (after tokens transferred). The `stream_id` is the newly assigned stream id (u64). The event is published in `persist_new_stream`.
| Withdrawal | ["withdrew", stream_id] | withdraw_amount: i128 | When a recipient successfully withdraws accrued tokens. Only emitted when amount > 0.
| StreamPaused | ["paused", stream_id] | StreamEvent::Paused(stream_id) — enum wrapper containing the u64 stream id | When a stream is paused by the sender or admin.
| StreamResumed | ["resumed", stream_id] | StreamEvent::Resumed(stream_id) — enum wrapper containing the u64 stream id | When a paused stream is resumed by the sender or admin.
| StreamCancelled | ["cancelled", stream_id] | StreamEvent::Cancelled(stream_id) — enum wrapper containing the u64 stream id | When a stream is cancelled by the sender or admin.
| AdminUpdated | ["admin", "updated"] | (old_admin: Address, new_admin: Address) | When contract admin is rotated via `set_admin`.

## Exact Soroban event structure

Soroban events are represented as JSON in test snapshots; the general shape is:

- topics: array of topic items (symbols or values)
- data: a value (single item) which can be a primitive (i128/u64) or a tuple/contracttype

Examples below are taken from the contract's test environment and use the same
field names and types the contract publishes.

### 1) StreamCreated

- topics: ["created", <stream_id>]  // stream_id: u64
- data: <deposit_amount>             // i128

Example JSON (illustrative):

{
  "topics": ["created", 0],
  "data": 1000
}

In Soroban test snapshots the raw event object appears as:

{
  "event": {
    "topics": ["created", 0],
    "data": 1000
  }
}

### 2) Withdrawal

- topics: ["withdrew", <stream_id>]  // stream_id: u64
- data: <withdraw_amount>             // i128

Example:

{
  "topics": ["withdrew", 0],
  "data": 300
}

### 3) StreamPaused / StreamResumed / StreamCancelled

- topics: ["paused"|"resumed"|"cancelled", <stream_id>]  // stream_id: u64
- data: StreamEvent enum value which wraps the stream_id. The enum in the
  contract is defined as:

```rust
#[contracttype]
#[derive(Clone, Debug)]
pub enum StreamEvent {
    Paused(u64),
    Resumed(u64),
    Cancelled(u64),
}
```

Example (paused):

{
  "topics": ["paused", 0],
  "data": { "Paused": 0 }
}

Indexers should accept either an enum-wrapped object (as above) or simply read
the topics for the stream id and treat the data as the same u64 id. The contract
always publishes the stream id both as the second topic and inside the enum payload.

### 4) AdminUpdated

- topics: ["admin", "updated"]  // both symbols
- data: (old_admin: Address, new_admin: Address)

Example (illustrative):

{
  "topics": ["admin", "updated"],
  "data": ["G...OLD_ADDRESS...", "G...NEW_ADDRESS..."]
}

Addresses are Soroban `Address` values and appear as their string representation
in test JSON (Stellar-like or contract address string). Indexers should parse
them using the same Address decoding used elsewhere in the backend.

## Parsing recommendations for indexers

- Use topics to quickly filter events by type and stream id: topics[0] is the
  canonical event name symbol ("created", "withdrew", "paused", "resumed", "cancelled", "admin").
- For stream-level events prefer reading stream id from topics[1] (u64) rather
  than relying solely on decoding the enum in `data` (redundant).
- For `created` and `withdrew` events, `data` is a signed 128-bit integer
  (`i128`): parse using a big-int library supporting 128-bit signed integers.
- For admin updates, `data` is a 2-tuple of `Address` values: parse as addresses.

## Keeping this doc in sync

This file is derived from `contracts/stream/src/lib.rs` emit calls:

- `persist_new_stream` publishes `(symbol_short!("created"), stream_id), deposit_amount`
- `withdraw` publishes `(symbol_short!("withdrew"), stream_id), withdrawable`
- `pause_stream` / `pause_stream_as_admin` publish `(symbol_short!("paused"), stream_id), StreamEvent::Paused(stream_id)`
- `resume_stream` / `resume_stream_as_admin` publish `(symbol_short!("resumed"), stream_id), StreamEvent::Resumed(stream_id)`
- `cancel_stream` / `cancel_stream_as_admin` publish `(symbol_short!("cancelled"), stream_id), StreamEvent::Cancelled(stream_id)`
- `set_admin` publishes `(symbol_short!("admin"), symbol_short!("updated")), (old_admin, new_admin)`

If you change event topics or payloads in the contract, please update this
document to match and include example snapshots.

---
Commit message suggestion: `docs: add event schema and topics for indexers`
