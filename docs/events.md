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
| StreamPaused     | `["paused", stream_id: u64]`    | `StreamPaused { stream_id: u64, reason: PauseReason }`                                                                                                    | When a stream is paused by the sender (`pause_stream`) or admin (`pause_stream_as_admin`). The `reason` field carries the operational context code.         |
| StreamResumed    | `["resumed", stream_id: u64]`   | `StreamEvent::Resumed(stream_id: u64)`                                                                                                                    | When a paused stream is resumed by the sender (`resume_stream`) or admin (`resume_stream_as_admin`).                    |
| StreamCancelled  | `["cancelled", stream_id: u64]` | `StreamEvent::StreamCancelled(stream_id: u64)`                                                                                                            | When a stream is cancelled by the sender (`cancel_stream`) or admin (`cancel_stream_as_admin`). `status` is persisted as `Cancelled` and `cancelled_at` is set before this event is emitted. |
| StreamCompleted  | `["completed", stream_id: u64]` | `StreamEvent::StreamCompleted(stream_id: u64)`                                                                                                            | When `withdrawn_amount` reaches `deposit_amount` during a `withdraw` or `batch_withdraw` call. Emitted after Withdrawal. |
| StreamClosed     | `["closed", stream_id: u64]`    | `StreamEvent::StreamClosed(stream_id: u64)`                                                                                                               | When a completed stream's storage is removed via `close_completed_stream`. Emitted before the storage entry is deleted.  |
| RateUpdated      | `["rate_upd", stream_id: u64]`  | `RateUpdated { stream_id: u64, old_rate_per_second: i128, new_rate_per_second: i128, effective_time: u64 }`                                               | When `update_rate_per_second` successfully changes a stream's rate.                                                     |
| RateCapEnforced  | `["rate_cap", stream_id: u64]`  | `RateCapEnforced { stream_id: u64, attempted_rate: i128, max_rate_per_second: i128 }`                                                                     | When a rate update is rejected due to exceeding the governance-controlled maximum rate per second cap.                 |
| StreamEndShortened | `["end_shrt", stream_id: u64]` | `StreamEndShortened { stream_id: u64, old_end_time: u64, new_end_time: u64, refund_amount: i128 }`                                                       | When `shorten_stream_end_time` successfully shortens a stream.                                                           |
| StreamEndExtended | `["end_ext", stream_id: u64]`  | `StreamEndExtended { stream_id: u64, old_end_time: u64, new_end_time: u64 }`                                                                              | When `extend_stream_end_time` successfully extends a stream.                                                             |
| StreamToppedUp   | `["top_up", stream_id: u64]`    | `StreamToppedUp { stream_id: u64, top_up_amount: i128, new_deposit_amount: i128 }`                                                                        | When `top_up_stream` successfully increases a stream's deposit.                                                          |
| RecipientUpdated | `["recp_upd", stream_id: u64]` | `RecipientUpdated { stream_id: u64, old_recipient: Address, new_recipient: Address }`                                                                     | When `update_recipient` successfully rotates a stream's receiving address.                                             |
| AdminUpdated     | `["AdminUpdated"]`              | `(old_admin: Address, new_admin: Address)`                                                                                                                | When the contract admin is rotated via `set_admin`.                                                                     |
| ContractPaused   | `["paused_ctl"]`                | `bool`                                                                                                                                                    | When the global contract pause state is toggled via `set_contract_paused`.                                              |
| ProtocolPaused   | `["pr_pause", admin: Address]`  | `ProtocolPaused { reason: String, paused_at: u64 }`                                                                                                       | When `pause_protocol` successfully pauses the protocol. Not emitted on idempotent calls.                               |
| ProtocolResumed  | `["pr_resume", admin: Address]` | `ProtocolResumed { resumed_at: u64 }`                                                                                                                     | When `resume_protocol` successfully resumes the protocol. Not emitted on idempotent calls.                             |
| SenderTransferred | `["sndr_xfr", stream_id: u64]` | `SenderTransferred { stream_id: u64, old_sender: Address, new_sender: Address }`                                                                          | When `transfer_sender` successfully rotates the stream sender. Emitted after state is persisted. Not emitted on failure. |
| DelegatedWithdrawal | `["dlg_wdraw", stream_id: u64]` | `DelegatedWithdrawal { stream_id: u64, recipient: Address, destination: Address, relayer: Address, amount: i128 }` | When a relayer successfully executes a recipient-signed delegated withdrawal via `delegated_withdraw_to`. Only emitted when `amount > 0`. |
| StreamHealthChanged | `["hlth_chg", stream_id: u64]` | `StreamHealthChanged { stream_id: u64, is_underfunded: bool, remaining_balance: i128, seconds_remaining: u64 }` | When a stream transitions between adequately funded and underfunded. Emitted by `decrease_rate_per_second`, `shorten_stream_end_time`, `top_up_stream`, and `cancel_stream`. Only emitted on actual health transitions, not on every mutation. |

**Additional topics (validator):** `gl_pause`, `gl_resume`, `rate_dec`, `tmpl_def`, `hlth_chg`.

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

Emitted by `persist_new_stream` after a successful `create_stream`, `create_streams`, or `create_streams_partial` call.

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

**StreamPaused** uses the new `StreamPaused` struct (introduced in `CONTRACT_VERSION = 3`):

```rust
#[contracttype]
pub struct StreamPaused {
    pub stream_id: u64,
    pub reason: PauseReason,
}

#[contracttype]
pub enum PauseReason {
    Operational   = 0,  // Routine sender-initiated pause
    Emergency     = 1,  // Security-related pause
    Compliance    = 2,  // Regulatory/compliance hold
    Administrative = 3, // Admin-initiated pause
}
```

| Function(s)                                                  | Topic         | Data                               |
| ------------------------------------------------------------ | ------------- | ---------------------------------- |
| `pause_stream`, `pause_stream_as_admin`                      | `"paused"`    | `StreamPaused { stream_id, reason }` |
| `resume_stream`, `resume_stream_as_admin`                    | `"resumed"`   | `StreamEvent::Resumed(id)`         |
| `cancel_stream`, `cancel_stream_as_admin`                    | `"cancelled"` | `StreamEvent::StreamCancelled(id)` |
| `withdraw`, `batch_withdraw` (final drain on Active streams) | `"completed"` | `StreamEvent::StreamCompleted(id)` |
| `close_completed_stream`                                     | `"closed"`    | `StreamEvent::StreamClosed(id)`    |

> **Breaking change (v3):** The `"paused"` event data changed from `StreamEvent::Paused(stream_id)`
> to `StreamPaused { stream_id, reason }`. Indexers must update their pause event parsers.
> `CONTRACT_VERSION` was bumped to `3` to signal this incompatibility.

Example (paused with reason):

```json
{
  "topics": ["paused", 0],
  "data": { "stream_id": 0, "reason": "Operational" }
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

Example JSON:

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

## On-chain Pause Audit Trail

In addition to events, the contract maintains an on-chain audit trail of the last pause action for each pause kind. This is queryable via `get_last_pause_record(kind: PauseKind)`.

### PauseKind

- `GlobalEmergency`: Toggled via `set_global_emergency_paused`.
- `Protocol`: Toggled via `pause_protocol`.
- `Stream`: Toggled via `pause_stream_as_admin`.

### PauseRecord

```rust
pub struct PauseRecord {
    pub actor: Address,
    pub timestamp: u64,
    pub reason: String,
}
```

### 14) StreamHealthChanged

Emitted by `decrease_rate_per_second`, `shorten_stream_end_time`, `top_up_stream`,
and `cancel_stream` when the stream's funding health status transitions between
adequately funded and underfunded.

A stream is **underfunded** when `remaining_balance < rate_per_second × seconds_remaining`.
Terminal streams (`Completed`, `Cancelled`) have `seconds_remaining = 0` and are never underfunded.

This event is only emitted when the `is_underfunded` flag actually changes, not on every mutation.

```
topics: ["hlth_chg", <stream_id: u64>]
data:   StreamHealthChanged {
          stream_id:         u64,
          is_underfunded:    bool,
          remaining_balance: i128,
          seconds_remaining: u64,
        }
```

Example (stream became underfunded after rate decrease):

```json
{
  "topics": ["hlth_chg", 0],
  "data": {
    "stream_id": 0,
    "is_underfunded": true,
    "remaining_balance": 500,
    "seconds_remaining": 800
  }
}
```

Example (stream became adequately funded after top-up):

```json
{
  "topics": ["hlth_chg", 0],
  "data": {
    "stream_id": 0,
    "is_underfunded": false,
    "remaining_balance": 1200,
    "seconds_remaining": 800
  }
}
```

Indexers should use this event to surface underfunded streams proactively.
The `remaining_balance` and `seconds_remaining` fields allow precise monitoring dashboards.

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
| `update_recipient`                                           | `"recp_upd"`    |
| `decrease_rate_per_second`, `shorten_stream_end_time`, `top_up_stream`, `cancel_stream` | `"hlth_chg"` |

If you change event topics or payloads in the contract, update this document and
include updated example snapshots in the PR.



ContractError: User-Facing Mapping for Clients
Summary
This document provides a comprehensive mapping of ContractError variants to their semantic meaning,
trigger conditions, affected roles, and recommended client actions. Integrators (wallets, indexers,
treasury tooling) can use this reference to handle protocol exceptions correctly.

| Error Code            | Value  | Description                                                                           | Functions Returning It                                                                                                                                                                                                                                              |
| --------------------- | ------ | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `StreamNotFound`      | 1      | The specified stream does not exist                                                   | `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `calculate_accrued`, `get_stream_state`, admin overrides                                                                                                                                              |
| `InvalidState`        | 2      | Operation attempted in an invalid state                                               | `cancel_stream`, `withdraw`, `withdraw_to`, `batch_withdraw`, `get_claimable_at`, admin overrides                                                                                                                                                                   |
| `InvalidParams`       | 3      | Function input parameters are invalid                                                 | `create_stream`, `withdraw_to`, `update_rate_per_second`, `top_up_stream`, `extend_stream_end_time`, `shorten_stream_end_time`, `batch_create_streams`                                                                                                              |
| `ContractPaused`      | 4      | Global emergency pause or creation pause is active                                    | `create_stream`, `create_streams`, `create_streams_partial`, `withdraw`, `withdraw_to`, `batch_withdraw`, `cancel_stream`, `top_up_stream`, `update_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time`, `update_recipient`, `trigger_auto_claim` |
| `StartTimeInPast`     | 5      | `start_time` is before the current ledger timestamp                                   | `create_stream`, `create_streams`, `create_streams_partial`                                                                                                                                                                                                         |
| `ArithmeticOverflow`  | 6      | Arithmetic overflow in stream calculations                                            | `create_stream`, `create_streams`, `create_streams_partial`, `update_rate_per_second`, `top_up_stream`, `shorten_stream_end_time`, `extend_stream_end_time`                                                                                                         |
| `Unauthorized`        | 7      | Caller is not authorized to perform this operation                                    | `init`, `set_admin`, `cancel_stream`, `top_up_stream`, `withdraw` (recipient check)                                                                                                                                                                                 |
| `AlreadyInitialised`  | 8      | Contract has already been initialized                                                 | `init`                                                                                                                                                                                                                                                              |
| `InsufficientBalance` | 9      | Token transfer failed due to insufficient balance or allowance                        | `create_stream`, `create_streams_partial`, `cancel_stream`, `withdraw`, `top_up_stream`                                                                                                                                                                             |
| `InsufficientDeposit` | 10     | Deposit amount does not cover the planned duration at the specified rate              | `create_stream`, `create_streams`, `update_rate_per_second`, `extend_stream_end_time`                                                                                                                                                                               |
| `StreamAlreadyPaused` | 11     | Stream is already in `Paused` state                                                   | `pause_stream`, `pause_stream_as_admin`                                                                                                                                                                                                                             |
| `StreamNotPaused`     | 12     | Stream is not `Paused`; cannot resume an `Active` stream                              | `resume_stream`, `resume_stream_as_admin`                                                                                                                                                                                                                           |
| `StreamTerminalState` | 13     | Stream is `Completed` or `Cancelled`; modification blocked                            | `pause_stream`, `resume_stream`, admin overrides                                                                                                                                                                                                                    |
| `DuplicateStreamId`   | 14     | Duplicate stream IDs supplied to a batch operation                                    | `batch_withdraw`                                                                                                                                                                                                                                                    |
| `InvalidSignature`    | 15     | Delegated withdrawal signature is invalid, expired, or nonce mismatch                 | `delegated_withdraw`                                                                                                                                                                                                                                                |
| `BelowMinimumAmount`  | 16     | Withdrawable amount is below the `expected_minimum_amount` committed in the signature | `delegated_withdraw`                                                                                                                                                                                                                                                |
| **`RateTooLow`**      | **17** | **`rate_per_second < MIN_RATE_PER_SECOND` (dust-attack prevention)**                  | **`create_stream`, `create_streams`, `create_stream_relative`, `create_streams_relative`, `create_stream_from_template`**                                                                                                                                           || `GlobalEmergency` | 19 | Global emergency pause has been activated | `create_stream`, `withdraw`, `cancel_stream`, `update_rate_per_second` |
| `RateTooLow` | 20 | The specified rate is below the minimum threshold | `create_stream`, `create_streams`, `update_rate_per_second` |



RateTooLow (17) — Dust-Attack Prevention
Introduced: Issue #576
Discriminant: 17
Affected entrypoints: All stream creation functions
Trigger
rate_per_second < MIN_RATE_PER_SECOND  // MIN_RATE_PER_SECOND = 100

Rationale
Streams with very low rate_per_second (e.g. 1 stroop/second) accrue imperceptibly slowly while occupying persistent ledger storage for years. This creates several problems:
Ledger bloat: Each stream consumes a persistent storage entry (~200+ bytes) that must be kept alive via TTL bumps.
Recipient index pollution: Low-value streams inflate RecipientStreams indices, increasing query costs for legitimate users.
Griefing vector: An attacker could create thousands of 1-stroop streams to a victim's address, making their recipient index unusable.
Indexer overhead: Indexers must track and process these streams indefinitely.
MIN_RATE_PER_SECOND = 100
At 100 stroops/second (~0.00001 USDC/sec at 7 decimals):
1 day accrual = 8,640,000 stroops = 0.864 USDC
1 year accrual = ~3,154,000,000 stroops = ~315 USDC
This is low enough for legitimate micro-streams (testnet faucets, small grants) while high enough to prevent state-bloat attacks.

Fix
Increase rate_per_second to at least 100:
// Before (fails with RateTooLow)
rate_per_second: 1,

// After (succeeds)
rate_per_second: 100,  // At minimum threshold
Integration Notes
Frontends should validate rate_per_second >= 100 client-side before submitting transactions.
Treasury tools should reject stream proposals with rates below the minimum.
The minimum is a compile-time constant; changing it requires a contract redeployment.
Spikes in RateTooLow errors in logs may indicate a dust-attack attempt.

Detailed Error Semantics
StreamNotFound (1)
Definition: The requested stream ID does not exist in contract storage.
Trigger Conditions:
stream_id is 0 or exceeds the current stream counter
Stream was never created
Stream was closed via close_completed_stream
Client Action:
match client.try_get_stream_state(&stream_id) {
    Ok(state) => { /* stream exists, use state */ }
    Err(ContractError::StreamNotFound) => {
        // Stream doesn't exist - check stream_id validity
        // Notify user or refresh stream list
    }
    Err(e) => { /* handle other errors */ }
}

InvalidState (2)
Definition: Operation attempted in a state where it is not allowed.
Trigger Conditions:| Scenario                                 | Description                    |
| ---------------------------------------- | ------------------------------ |
| Withdraw from Completed stream           | All funds already withdrawn    |
| Withdraw from non-terminal Paused stream | Must resume first              |
| Cancel Completed stream                  | Already terminal               |
| Top-up Completed/Cancelled stream        | Cannot modify terminal streams |

InvalidParams (3)
Definition: One or more input parameters are invalid.
Trigger Conditions:| Parameter                         | Invalid When                                                |
| --------------------------------- | ----------------------------------------------------------- |
| `sender == recipient`             | Sender and recipient addresses are identical                |
| `deposit_amount <= 0`             | Deposit must be positive                                    |
| `rate_per_second <= 0`            | Rate must be positive (caught by RateTooLow first if < 100) |
| `start_time >= end_time`          | Start must be before end                                    |
| `cliff_time < start_time`         | Cliff cannot precede start                                  |
| `cliff_time > end_time`           | Cliff cannot follow end                                     |
| `destination == contract_address` | Cannot withdraw to contract                                 |
| `new_rate_per_second <= old_rate` | Rate can only increase                                      |
| `top_up_amount <= 0`              | Top-up must be positive                                     |



ContractPaused (4)
Definition: The protocol is globally paused. No new streams may be created.
Trigger Conditions:
Admin called set_global_emergency_paused(true) or set_contract_paused(true)
Client Action:match client.try_create_stream(...) {
    Ok(stream_id) => { /* success */ }
    Err(ContractError::ContractPaused) => {
        let info = client.get_pause_info();
        if let Some(ref reason) = info.reason {
            println!("Pause reason: {}", reason);
        }
        // Retry later or contact admin
    }
    Err(e) => { /* handle other errors */ }
}

StartTimeInPast (5)
Definition: start_time is before the current ledger timestamp.
Fix: Use current_time + delay or create_stream_relative for offset-based timing.

ArithmeticOverflow (6)
Definition: Arithmetic overflow in stream calculations.
Trigger Conditions:
| Calculation                 | Overflow Condition         |
| --------------------------- | -------------------------- |
| `rate * duration`           | Result exceeds `i128::MAX` |
| `deposit + amount` (top-up) | Result exceeds `i128::MAX` |

Unauthorized (7)
Definition: Caller is not authorized to perform this operation.
Trigger Conditions:| Operation       | Authorization Requirement |
| --------------- | ------------------------- |
| `cancel_stream` | Caller is sender or admin |
| `top_up_stream` | Caller is sender or admin |
| `withdraw`      | Caller is recipient       |
| `set_admin`     | Current admin only        |


AlreadyInitialised (8)
Definition: Contract has already been initialized.
Fix: Call get_config to verify existing configuration.

InsufficientBalance (9)
Definition: Token transfer failed due to insufficient balance or allowance.
Fix: Check token balance and increase allowance before retrying.

InsufficientDeposit (10)
Definition: Deposit amount does not cover the planned duration at the specified rate.
Formula: deposit >= rate_per_second * (end_time - start_time)
Fix: Increase deposit, reduce rate, or shorten duration.


StreamAlreadyPaused (11)
Definition: Stream is already in Paused state.
Fix: Check get_stream_state before calling pause_stream.


StreamNotPaused (12)
Definition: Stream is not in Paused state.
Fix: Check get_stream_state before calling resume_stream.

StreamTerminalState (13)
Definition: Stream is in a terminal state (Completed or Cancelled).
Blocked Operations: pause_stream, resume_stream, cancel_stream, top_up_stream, update_rate_per_second

DuplicateStreamId (14)
Definition: Duplicate stream IDs were supplied to a batch operation.
Fix: Deduplicate stream_ids before calling batch_withdraw.

InvalidSignature (15)
Definition: Delegated withdrawal signature is invalid, expired, or nonce mismatch.
Fix: Request a fresh signature from the recipient with the current nonce.

BelowMinimumAmount (16)
Definition: Withdrawable amount is below the expected_minimum_amount committed in the signature.
Fix: Wait for more accrual or request a new signature with a lower minimum.

Previously Panicking Paths (Now Structured Errors)| Former Panic                                                         | Now Returns                         | Functions                                                                                                                                   |
| -------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `panic_with_error!(ContractPaused)` in `require_not_globally_paused` | `ContractError::ContractPaused`     | `withdraw`, `withdraw_to`, `batch_withdraw`, `cancel_stream`, `update_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time` |
| `panic_with_error!(ArithmeticOverflow)` in batch deposit sum         | `ContractError::ArithmeticOverflow` | `create_streams`                                                                                                                            |
| `assert!("batch_withdraw stream_ids must be unique")`                | `ContractError::DuplicateStreamId`  | `batch_withdraw`                                                                                                                            |


Role-Based Error Matrix
| Operation            | Recipient                                  | Sender                                                                        | Admin                        | Anyone                                                             |
| -------------------- | ------------------------------------------ | ----------------------------------------------------------------------------- | ---------------------------- | ------------------------------------------------------------------ |
| `create_stream`      | -                                          | InvalidParams, RateTooLow, InsufficientBalance, InsufficientDeposit           | -                            | -                                                                  |
| `pause_stream`       | -                                          | StreamNotFound, Unauthorized, StreamAlreadyPaused, StreamTerminalState        | Same + StreamNotFound        | StreamNotFound                                                     |
| `resume_stream`      | -                                          | StreamNotFound, Unauthorized, StreamNotPaused, StreamTerminalState            | Same + StreamNotFound        | StreamNotFound                                                     |
| `cancel_stream`      | -                                          | StreamNotFound, Unauthorized, InvalidState                                    | StreamNotFound, Unauthorized | -                                                                  |
| `withdraw`           | StreamNotFound, Unauthorized, InvalidState | -                                                                             | -                            | -                                                                  |
| `delegated_withdraw` | -                                          | -                                                                             | -                            | InvalidSignature, BelowMinimumAmount, StreamNotFound, InvalidState |
| `top_up_stream`      | -                                          | StreamNotFound, Unauthorized, InvalidParams, InvalidState, ArithmeticOverflow | StreamNotFound               | -                                                                  |
| `calculate_accrued`  | StreamNotFound                             | StreamNotFound                                                                | StreamNotFound               | StreamNotFound                                                     |
| `get_stream_state`   | StreamNotFound                             | StreamNotFound                                                                | StreamNotFound               | StreamNotFound                                                     |


Edge Cases: Time-Driven Errors| Edge Case                 | Error        | Condition                         |
| ------------------------- | ------------ | --------------------------------- |
| Stream past end\_time     | InvalidState | `withdraw` on completed stream    |
| Stream at exact end\_time | Success      | Full withdrawal allowed           |
| Stream before cliff       | InvalidState | `withdraw` returns 0              |
| Stream at exact cliff     | Success      | Accrual begins (from start\_time) |
| Future start\_time        | Success      | Stream created but no accrual yet |
| Cancel before cliff       | Success      | Full refund (accrued = 0)         |
| Cancel after end\_time    | InvalidState | No refund (accrued = deposit)     |


Testing Coverage
Error handling is verified by tests in contracts/stream/src/test.rs and contracts/stream/tests/rate_bounds.rs:| Error               | Test Pattern                                                         |
| ------------------- | -------------------------------------------------------------------- |
| StreamNotFound      | `try_get_stream_state` with invalid ID                               |
| InvalidParams       | `try_create_stream` with `sender == recipient`, `deposit <= 0`, etc. |
| RateTooLow          | `try_create_stream` with `rate_per_second < 100`                     |
| ContractPaused      | Global pause then create                                             |
| Unauthorized        | Wrong recipient `try_withdraw`                                       |
| InsufficientBalance | Sender with no tokens                                                |
| InsufficientDeposit | `deposit < rate * duration`                                          |
| StreamTerminalState | Pause/complete then modify                                           |
| DuplicateStreamId   | `batch_withdraw` with repeated stream IDs                            |
| InvalidSignature    | `delegated_withdraw` with invalid or expired signature               |
| BelowMinimumAmount  | `delegated_withdraw` when accrued < expected\_minimum                |
Discriminant stability is verified by test_contract_error_discriminants_are_stable in contracts/stream/src/test.rs, which asserts the exact u32 value of every ContractError variant and will fail at compile time if any value is changed.

FactoryError Reference
The factory contract (contracts/factory) uses a separate FactoryError enum.| Error                     | Description                                                            |
| ------------------------- | ---------------------------------------------------------------------- |
| `AlreadyInitialized`      | Factory has already been initialized; `init` may only be called once   |
| `NotInitialized`          | Factory has not been initialized; call `init` first                    |
| `Unauthorized`            | Caller is not the factory admin                                        |
| `RecipientNotAllowlisted` | Recipient address is not on the factory allowlist                      |
| `DepositExceedsCap`       | Requested deposit exceeds the per-stream cap configured in the factory |
| `DurationTooShort`        | Stream duration is below the factory-enforced minimum                  |


Scope
Included
All 17 ContractError variants (including RateTooLow)
Role-based error mapping
Success/failure semantics for each operation
Time-driven edge cases
Client action recommendations
Dust-attack prevention guidance

Excluded| Exclusion                    | Rationale                   | Residual Risk                         |
| ---------------------------- | --------------------------- | ------------------------------------- |
| Token-specific errors        | Delegated to token contract | Low - caught by `InsufficientBalance` |
| Gas budget errors            | Soroban runtime errors      | Low - indicates contract size issues  |
| Storage serialization errors | Runtime infrastructure      | Very Low                              |


Residual Risks
| Risk                | Likelihood | Impact | Mitigation                                         |
| ------------------- | ---------- | ------ | -------------------------------------------------- |
| Error code changes  | Low        | High   | Versioning in client SDKs; `version()` entrypoint  |
| Missing error cases | Low        | Medium | Comprehensive test coverage (target: 95%+)         |
| Client mishandling  | Medium     | Medium | This documentation + SDK helpers                   |
| Dust-attack bypass  | Very Low   | High   | `MIN_RATE_PER_SECOND` enforced at validation layer |


| Event name       | Topic(s)                        | Data (shape & types)                                                                                  |
|------------------|---------------------------------|-------------------------------------------------------------------------------------------------------|
| AutoClaimSet     | `["ac_set", stream_id: u64]`    | `AutoClaimSet { stream_id: u64, enabled: bool }`                                                     |
| AutoClaimTriggered | `["ac_trig", stream_id: u64]`  | `AutoClaimTriggered { stream_id: u64, amount: i128 }`                                                |
| ExcessWithdrawn  | `["ex_swept", stream_id: u64]` | `ExcessWithdrawn { stream_id: u64, amount: i128 }`                                                   |
| StreamMigrated   | `["migrated", stream_id: u64]` | `StreamMigrated { stream_id: u64, old_contract: Address, new_contract: Address }`                   |

| `set_auto_claim`, `trigger_auto_claim`                       | `"ac_set"`, `"ac_trig"`  |
| `sweep_excess`                                               | `"ex_swept"`             |
| `migrate_stream`                                             | `"migrated"`             |

