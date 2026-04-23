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

| Event name       | Topic(s)                        | Data (shape & types)                                                                                                                                      | When emitted                                                                                                            |
|------------------|---------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------|
| StreamCreated    | `["created", stream_id: u64]`   | `StreamCreated { stream_id: u64, sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64, memo: Option<Bytes> }` | After a stream is successfully created and deposit tokens transferred. Not emitted on any validation failure.           |
| Withdrawal       | `["withdrew", stream_id: u64]`  | `Withdrawal { stream_id: u64, recipient: Address, amount: i128 }`                                                                                         | When a recipient successfully withdraws accrued tokens. Only emitted when `amount > 0`.                                |
| WithdrawalTo     | `["wdraw_to", stream_id: u64]`  | `WithdrawalTo { stream_id: u64, recipient: Address, destination: Address, amount: i128 }`                                                                 | When a recipient calls `withdraw_to` or `batch_withdraw_to` and `amount > 0`. Destination may differ from recipient.                          |
| StreamPaused     | `["paused", stream_id: u64]`    | `StreamEvent::Paused(stream_id: u64)`                                                                                                                     | When a stream is paused by the sender (`pause_stream`) or admin (`pause_stream_as_admin`).                              |
| StreamResumed    | `["resumed", stream_id: u64]`   | `StreamEvent::Resumed(stream_id: u64)`                                                                                                                    | When a paused stream is resumed by the sender (`resume_stream`) or admin (`resume_stream_as_admin`).                    |
| StreamCancelled  | `["cancelled", stream_id: u64]` | `StreamEvent::StreamCancelled(stream_id: u64)`                                                                                                            | When a stream is cancelled by the sender (`cancel_stream`) or admin (`cancel_stream_as_admin`). `status` is persisted as `Cancelled` and `cancelled_at` is set before this event is emitted. |
| StreamCompleted  | `["completed", stream_id: u64]` | `StreamEvent::StreamCompleted(stream_id: u64)`                                                                                                            | When `withdrawn_amount` reaches `deposit_amount` during a `withdraw` or `batch_withdraw` call. Emitted after Withdrawal. |
| StreamClosed     | `["closed", stream_id: u64]`    | `StreamEvent::StreamClosed(stream_id: u64)`                                                                                                               | When a completed stream's storage is removed via `close_completed_stream`. Emitted before the storage entry is deleted.  |
| RateUpdated      | `["rate_upd", stream_id: u64]`  | `RateUpdated { stream_id: u64, old_rate_per_second: i128, new_rate_per_second: i128, effective_time: u64 }`                                               | When `update_rate_per_second` successfully changes a stream's rate.                                                     |
| StreamEndShortened | `["end_shrt", stream_id: u64]` | `StreamEndShortened { stream_id: u64, old_end_time: u64, new_end_time: u64, refund_amount: i128 }`                                                       | When `shorten_stream_end_time` successfully shortens a stream.                                                           |
| StreamEndExtended | `["end_ext", stream_id: u64]`  | `StreamEndExtended { stream_id: u64, old_end_time: u64, new_end_time: u64 }`                                                                              | When `extend_stream_end_time` successfully extends a stream.                                                             |
| StreamToppedUp   | `["top_up", stream_id: u64]`    | `StreamToppedUp { stream_id: u64, top_up_amount: i128, new_deposit_amount: i128 }`                                                                        | When `top_up_stream` successfully increases a stream's deposit.                                                          |
| AdminUpdated     | `["AdminUpdated"]`              | `(old_admin: Address, new_admin: Address)`                                                                                                                | When the contract admin is rotated via `set_admin`.                                                                     |
| ContractPaused   | `["paused_ctl"]`                | `bool`                                                                                                                                                    | When the global contract pause state is toggled via `set_contract_paused`.                                              |
| ProtocolPaused   | `["pr_pause", admin: Address]`  | `ProtocolPaused { reason: String, paused_at: u64 }`                                                                                                       | When `pause_protocol` successfully pauses the protocol. Not emitted on idempotent calls.                               |
| ProtocolResumed  | `["pr_resume", admin: Address]` | `ProtocolResumed { resumed_at: u64 }`                                                                                                                     | When `resume_protocol` successfully resumes the protocol. Not emitted on idempotent calls.                             |
| SenderTransferred | `["sndr_xfr", stream_id: u64]` | `SenderTransferred { stream_id: u64, old_sender: Address, new_sender: Address }`                                                                          | When `transfer_sender` successfully rotates the stream sender. Emitted after state is persisted. Not emitted on failure. |

---
| Event name | Topic(s) | Data (shape & types) | When emitted |
|---|---:|---|---|
| StreamCreated | ["created", stream_id] | StreamCreated { stream_id: u64, sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64 } | When a stream is successfully created (after tokens transferred). The `stream_id` is the newly assigned stream id (u64). The event is published in `persist_new_stream`. Not emitted on failed creation (e.g., `StartTimeInPast`).
| Withdrawal | ["withdrew", stream_id] | Withdrawal { stream_id: u64, recipient: Address, amount: i128 } | When a recipient successfully withdraws accrued tokens. Only emitted when amount > 0.
| StreamPaused | ["paused", stream_id] | StreamEvent::Paused(stream_id) — enum wrapper containing the u64 stream id | When a stream is paused by the sender or admin.
| StreamResumed | ["resumed", stream_id] | StreamEvent::Resumed(stream_id) — enum wrapper containing the u64 stream id | When a paused stream is resumed by the sender or admin.
| StreamCancelled | ["cancelled", stream_id] | StreamEvent::StreamCancelled(stream_id) — enum wrapper containing the u64 stream id | When a stream is cancelled by the sender or admin.
| AdminUpdated | ["admin", "updated"] | (old_admin: Address, new_admin: Address) | When contract admin is rotated via `set_admin`.
| ContractPaused | ["paused_ctl"] | bool | When global pause is set to true or false.

## Exact Soroban event structure

Soroban events are represented as JSON in test snapshots; the general shape is:

- **topics**: array of topic items (symbols or values)
- **data**: a value (single item) which can be a primitive, a struct, or a tuple

### 1) StreamCreated

Emitted by `persist_new_stream` after a successful `create_stream` or `create_streams` call.

```
topics: ["created", <stream_id: u64>]
data:   StreamCreated {
          stream_id:       u64,
          sender:          Address,
          recipient:       Address,
          deposit_amount:  i128,
          rate_per_second: i128,
          start_time:      u64,
          cliff_time:      u64,
          end_time:        u64,
          memo:            Option<Bytes>,  // None when not supplied; max 64 bytes
        }
```

Example JSON (illustrative):

```json
{
  "topics": ["created", 0],
  "data": {
    "stream_id": 0,
    "sender": "G...SENDER...",
    "recipient": "G...RECIPIENT...",
    "deposit_amount": 1000,
    "rate_per_second": 1,
    "start_time": 0,
    "cliff_time": 0,
    "end_time": 1000
  }
}
```

### 2) Withdrawal

Emitted by `withdraw` and each stream in `batch_withdraw` when `withdrawable > 0`.

```
topics: ["withdrew", <stream_id: u64>]
data:   Withdrawal {
          stream_id: u64,
          recipient: Address,
          amount:    i128,
        }
```

Example:

```json
{
  "topics": ["withdrew", 0],
  "data": { "stream_id": 0, "recipient": "G...RECIPIENT...", "amount": 300 }
}
```

### 3) WithdrawalTo

Emitted by `withdraw_to` when `withdrawable > 0`. The `destination` field holds the
address that actually receives the tokens; `recipient` is the stream's registered
recipient (the authorised caller).

```
topics: ["wdraw_to", <stream_id: u64>]
data:   WithdrawalTo {
          stream_id:   u64,
          recipient:   Address,
          destination: Address,
          amount:      i128,
        }
```

### 4) StreamPaused / StreamResumed / StreamCancelled / StreamCompleted / StreamClosed

These all use the `StreamEvent` enum as their data payload:

```rust
#[contracttype]
pub enum StreamEvent {
    Paused(u64),
    Resumed(u64),
    StreamCancelled(u64),
    StreamCompleted(u64),
    StreamClosed(u64),
}
```

| Function(s)                                                  | Topic         | Data enum variant                  |
| ------------------------------------------------------------ | ------------- | ---------------------------------- |
| `pause_stream`, `pause_stream_as_admin`                      | `"paused"`    | `StreamEvent::Paused(id)`          |
| `resume_stream`, `resume_stream_as_admin`                    | `"resumed"`   | `StreamEvent::Resumed(id)`         |
| `cancel_stream`, `cancel_stream_as_admin`                    | `"cancelled"` | `StreamEvent::StreamCancelled(id)` |
| `withdraw`, `batch_withdraw` (final drain on Active streams) | `"completed"` | `StreamEvent::StreamCompleted(id)` |
| `close_completed_stream`                                     | `"closed"`    | `StreamEvent::StreamClosed(id)`    |

Example (cancelled):

```json
{
  "topics": ["cancelled", 0],
  "data": { "StreamCancelled": 0 }
}
```

`StreamCancelled` does not embed refund or timestamp fields in the payload.
Indexers should read `get_stream_state(stream_id)` to obtain `cancelled_at` and derive refund
from state plus accrual (`refund = deposit_amount - accrued_at_cancelled_at`).

Example (completed — emitted after the Withdrawal event on the same call):

```json
{
  "topics": ["completed", 0],
  "data": { "StreamCompleted": 0 }
}
```

> **Indexers:** the `stream_id` appears both as the second topic and inside the
> enum payload. Read it from the topic for efficiency; use the payload only for
> cross-checking.

### 5) RateUpdated

```
topics: ["rate_upd", <stream_id: u64>]
data:   RateUpdated {
          stream_id:           u64,
          old_rate_per_second: i128,
          new_rate_per_second: i128,
          effective_time:      u64,
        }
```

### 6) StreamEndShortened

```
topics: ["end_shrt", <stream_id: u64>]
data:   StreamEndShortened {
          stream_id:     u64,
          old_end_time:  u64,
          new_end_time:  u64,
          refund_amount: i128,
        }
```

Emission guarantees:
- Emitted exactly once on successful `shorten_stream_end_time`.
- Not emitted on failed shorten calls (`InvalidParams`, `InvalidState`, auth failure).

### 7) StreamEndExtended

```
topics: ["end_ext", <stream_id: u64>]
data:   StreamEndExtended {
          stream_id:    u64,
          old_end_time: u64,
          new_end_time: u64,
        }
```

### 8) StreamToppedUp

This event is emitted only after the top-up has succeeded. Validation failures,
authorization failures, arithmetic overflow, or failed token pulls emit no
`top_up` contract event.

```
topics: ["top_up", <stream_id: u64>]
data:   StreamToppedUp {
          stream_id:          u64,
          top_up_amount:      i128,
          new_deposit_amount: i128,
        }
```

### 9) AdminUpdated

Emitted by `set_admin`.

```
topics: ["AdminUpdated"]
data:   (old_admin: Address, new_admin: Address)
```

Example:

```json
{
  "topics": ["AdminUpdated"],
  "data": ["G...OLD_ADDRESS...", "G...NEW_ADDRESS..."]
}
```

### 10) ProtocolPaused

Emitted by `pause_protocol` when the protocol is successfully paused.
**Not emitted** on idempotent calls (when already paused).

```
topics: ["pr_pause", admin: Address]
data:   ProtocolPaused {
          reason: String,
          paused_at: u64,
        }
```

Example:

```json
{
  "topics": ["pr_pause", "G...ADMIN_ADDRESS..."],
  "data": {
    "reason": "security incident",
    "paused_at": 1234567
  }
}
```

### 11) ProtocolResumed

Emitted by `resume_protocol` when the protocol is successfully resumed.
**Not emitted** on idempotent calls (when not paused).

```
topics: ["pr_resume", admin: Address]
data:   ProtocolResumed {
          resumed_at: u64,
        }
```

Example:

```json
{
  "topics": ["pr_resume", "G...ADMIN_ADDRESS..."],
  "data": {
    "resumed_at": 2345678
  }
}
```

### 12) SenderTransferred

Emitted by `transfer_sender` when the stream sender is successfully rotated.

```
topics: ["sndr_xfr", <stream_id: u64>]
data:   SenderTransferred {
          stream_id:  u64,
          old_sender: Address,
          new_sender: Address,
        }
```

Example:

```json
{
  "topics": ["sndr_xfr", 0],
  "data": {
    "stream_id": 0,
    "old_sender": "G...OLD_SENDER...",
    "new_sender": "G...NEW_SENDER..."
  }
}
```

Indexers should update their sender reference for the stream on receipt of this event.
The `old_sender` field allows indexers to correlate the previous treasury key.

---

## Parsing recommendations for indexers

- Use `topics[0]` to filter by event type; use `topics[1]` to get the `stream_id`
  for all stream-level events.
- For `Withdrawal` and `WithdrawalTo`, the `amount` field is `i128` — use a
  big-int library that supports 128-bit signed integers.
- `StreamCompleted` is emitted on the **same call** as the final `Withdrawal` that drains
  an `Active` stream. Cancelled streams do not transition to `Completed`.
- `StreamClosed` signals that the stream's on-chain storage has been removed.
  After this event, `get_stream_state` returns `StreamNotFound` for that ID.
- `AdminUpdated` has a single-element topic list (no stream_id).

> **See [docs/indexer-derivation.md](./indexer-derivation.md)** for the complete
> specification of how to derive stream state from events, when to call
> `get_stream_state`, and worked examples for each lifecycle path (including
> cancellation, rate changes, and completion).

---

## Keeping this doc in sync

This file is derived from `contracts/stream/src/lib.rs` emit calls:

- `persist_new_stream` publishes `(symbol_short!("created"), stream_id), StreamCreated { ... }`
- `withdraw` publishes `(symbol_short!("withdrew"), stream_id), Withdrawal { stream_id, recipient, amount }`
- `pause_stream` / `pause_stream_as_admin` publish `(symbol_short!("paused"), stream_id), StreamEvent::Paused(stream_id)`
- `resume_stream` / `resume_stream_as_admin` publish `(symbol_short!("resumed"), stream_id), StreamEvent::Resumed(stream_id)`
- `cancel_stream` / `cancel_stream_as_admin` publish `(symbol_short!("cancelled"), stream_id), StreamEvent::StreamCancelled(stream_id)`
- `set_admin` publishes `(symbol_short!("admin"), symbol_short!("updated")), (old_admin, new_admin)`

If you change event topics or payloads in the contract, please update this
document to match and include example snapshots.

---

Commit message suggestion: `docs: add event schema and topics for indexers`
| Source location | Symbol emitted |
|--------------------------------------------------------------|-----------------|
| `persist_new_stream`                                         | `"created"`     |
| `withdraw`, `batch_withdraw`                                 | `"withdrew"`    |
| `withdraw_to`, `batch_withdraw_to`                           | `"wdraw_to"`    |
| `withdraw`, `batch_withdraw`, `batch_withdraw_to` (completion) | `"completed"`   |
| `pause_stream`, `pause_stream_as_admin`                      | `"paused"`      |
| `resume_stream`, `resume_stream_as_admin`                    | `"resumed"`     |
| `cancel_stream`, `cancel_stream_as_admin`                    | `"cancelled"`   |
| `close_completed_stream`                                     | `"closed"`      |
| `update_rate_per_second`                                     | `"rate_upd"`    |
| `shorten_stream_end_time`                                    | `"end_shrt"`    |
| `extend_stream_end_time`                                     | `"end_ext"`     |
| `top_up_stream`                                              | `"top_up"`      |
| `set_admin`                                                  | `"AdminUpdated"`|
| `set_contract_paused`                                        | `"paused_ctl"`  |
| `pause_protocol`                                             | `"pr_pause"`    |
| `resume_protocol`                                            | `"pr_resume"`   |

If you change event topics or payloads in the contract, update this document and
include updated example snapshots in the PR.
