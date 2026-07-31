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
| StreamCreated    | `["created", stream_id: u64]`   | `StreamCreated { stream_id: u64, sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64, withdraw_dust_threshold: i128, memo: Option<Bytes>, metadata: Option<Map<Bytes,Bytes>> }` | After a stream is successfully created and deposit tokens transferred. Not emitted on any validation failure.           |
| Withdrawal       | `["withdrew", stream_id: u64]`  | `Withdrawal { stream_id: u64, recipient: Address, amount: i128 }`                                                                                         | When a recipient successfully withdraws accrued tokens. Only emitted when `amount > 0`.                                |
| WithdrawalTo     | `["wdraw_to", stream_id: u64]`  | `WithdrawalTo { stream_id: u64, recipient: Address, destination: Address, amount: i128 }`                                                                 | When a recipient calls `withdraw_to` or `batch_withdraw_to` and `amount > 0`. Destination may differ from recipient.                          |
| StreamPaused     | `["paused", stream_id: u64]`    | `StreamPaused { stream_id: u64, reason: String }`                                                                                                         | When a stream is paused by the sender (`pause_stream`) or admin (`pause_stream_as_admin`). The `reason` field carries the operational context code.         |
| StreamResumed    | `["resumed", stream_id: u64]`   | `StreamEvent::Resumed(stream_id: u64)`                                                                                                                    | When a paused stream is resumed by the sender (`resume_stream`) or admin (`resume_stream_as_admin`).                    |
| StreamCancelled  | `["cancelled", stream_id: u64]` | `StreamEvent::StreamCancelled(stream_id: u64)`                                                                                                            | When a stream is cancelled by the sender (`cancel_stream`) or admin (`cancel_stream_as_admin`). `status` is persisted as `Cancelled` and `cancelled_at` is set before this event is emitted. |
| StreamCloned     | `["cloned", stream_id: u64]`    | `StreamCloned { new_stream_id: u64, source_stream_id: u64, sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64, withdraw_dust_threshold: i128 }` | When `clone_stream` creates a new stream from an existing source stream. |
| KeeperCancelled  | `["kp_cncl", stream_id: u64]`   | `KeeperCancelled { stream_id: u64, keeper: Address, keeper_fee: i128, recipient_amount: i128, sender_refund: i128 }` | When `keeper_cancel` cancels an eligible expired stream after the keeper grace period. |
| StreamCompleted  | `["completed", stream_id: u64]` | `StreamEvent::StreamCompleted(stream_id: u64)`                                                                                                            | When `withdrawn_amount` reaches `deposit_amount` during a `withdraw` or `batch_withdraw` call. Emitted after Withdrawal. |
| StreamClosed     | `["closed", stream_id: u64]`    | `StreamEvent::StreamClosed(stream_id: u64)`                                                                                                               | When a completed stream's storage is removed via `close_completed_stream`. Emitted before the storage entry is deleted.  |
| RateUpdated      | `["rate_upd", stream_id: u64]`  | `RateUpdated { stream_id: u64, old_rate_per_second: i128, new_rate_per_second: i128, effective_time: u64 }`                                               | When `update_rate_per_second` successfully changes a stream's rate.                                                     |
| RateCapEnforced  | `["rate_cap", stream_id: u64]`  | `RateCapEnforced { stream_id: u64, attempted_rate: i128, max_rate_per_second: i128 }`                                                                     | When a rate update is rejected due to exceeding the governance-controlled maximum rate per second cap.                 |
| StreamEndShortened | `["end_shrt", stream_id: u64]` | `StreamEndShortened { stream_id: u64, old_end_time: u64, new_end_time: u64, refund_amount: i128 }`                                                       | When `shorten_stream_end_time` successfully shortens a stream.                                                           |
| StreamEndExtended | `["end_ext", stream_id: u64]`  | `StreamEndExtended { stream_id: u64, old_end_time: u64, new_end_time: u64 }`                                                                              | When `extend_stream_end_time` successfully extends a stream.                                                             |
| StreamToppedUp   | `["top_up", stream_id: u64]`    | `StreamToppedUp { stream_id: u64, top_up_amount: i128, new_deposit_amount: i128, new_end_time: u64 }`                                                     | When `top_up_stream` successfully increases a stream's deposit.                                                          |
| StreamRenewed    | `["renewed", old_stream_id: u64, new_stream_id: u64]` | `StreamRenewed { old_stream_id: u64, new_stream_id: u64 }` | When `renew_stream` successfully creates the next stream from a completed auto-renew-enabled stream. |
| RecipientUpdated | `["recp_upd", stream_id: u64]` | `RecipientUpdated { stream_id: u64, old_recipient: Address, new_recipient: Address }`                                                                     | When `update_recipient` successfully rotates a stream's receiving address.                                             |
| AdminUpdated     | `["AdminUpd"]`              | `(old_admin: Address, new_admin: Address)`                                                                                                                | When the contract admin is rotated via `set_admin`.                                                                     |
| ContractPauseChanged | `["ct_pause"]`                | `ContractPauseChanged { paused: bool }`                                                                                                                    | When the global contract pause state is toggled via `set_contract_paused`.                                              |
| ProtocolPaused   | `["pr_pause", admin: Address]`  | `ProtocolPaused { reason: String, paused_at: u64 }`                                                                                                       | When `pause_protocol` successfully pauses the protocol. Not emitted on idempotent calls.                               |
| ProtocolResumed  | `["pr_resume", admin: Address]` | `ProtocolResumed { resumed_at: u64 }`                                                                                                                     | When `resume_protocol` successfully resumes the protocol. Not emitted on idempotent calls.                             |
| SenderTransferred | `["sndr_xfr", stream_id: u64]` | `SenderTransferred { stream_id: u64, old_sender: Address, new_sender: Address }`                                                                          | When `transfer_sender` successfully rotates the stream sender. Emitted after state is persisted. Not emitted on failure. |
| RateDecreased | `["rate_dec", stream_id: u64]` | `RateDecreased { stream_id: u64, old_rate_per_second: i128, new_rate_per_second: i128, effective_time: u64, checkpointed_amount: i128, refund_amount: i128 }` | When `decrease_rate_per_second` safely decreases the streaming rate with checkpointing. |
| GlobalEmergencyPauseChanged | `["gl_pause"]` | `GlobalEmergencyPauseChanged { paused: bool }` | When the contract admin toggles the global emergency pause flag. |
| GlobalResumed | `["gl_resume"]` | `GlobalResumed { resumed_at: u64 }` | When the global emergency pause is lifted. |
| StreamDecommissioned | `["decomm", stream_id: u64]` | `StreamDecommissioned { stream_id: u64, decommissioned: bool }` | When a stream's decommissioned state is set. |
| StreamHealthChanged | `["health", stream_id: u64]` | `StreamHealthChanged { stream_id: u64, is_underfunded: bool, remaining_balance: i128, seconds_remaining: u64 }` | When a stream transitions between adequately funded and underfunded. Emitted by `decrease_rate_per_second`, `shorten_stream_end_time`, `top_up_stream`, and `cancel_stream`. Only emitted on actual health transitions, not on every mutation. |
| ExcessSwept | `["ex_swept", recipient: Address]` | `ExcessSwept { to: Address, amount: i128 }` | When the admin recovers tokens that exceed total stream liabilities via `sweep_excess`. |
| AutoClaimSet | `["ac_set", stream_id: u64]` | `AutoClaimSet { stream_id: u64, destination: Address }` | When a recipient configures or changes a permissionless final-claim destination via `set_auto_claim`. |
| AutoClaimRevoked | `["ac_revoke", stream_id: u64]` | `AutoClaimRevoked { stream_id: u64 }` | When a recipient revokes auto-claim configuration via `revoke_auto_claim`. |
| AutoClaimTriggered | `["ac_trig", stream_id: u64]` | `AutoClaimTriggered { stream_id: u64, destination: Address, amount: i128 }` | When a third party successfully executes a configured final claim via `trigger_auto_claim`. |
| MigrationCheckpoint | `["migrated"]` | `(from_version: u32, to_version: u32, timestamp: u64)` | No function currently emits this event. Reserved for future migration checkpoints. |
| ReservationReleased | `["res_rel", holder: Address]` | `(start_id: u64, count: u64, consumed: u64, reclaimed: u64)` | When a stream ID reservation is voluntarily released or reclaimed after expiry. |
| ClaimOwnershipTransferred | `["claim_own", stream_id: u64]` | `ClaimOwnershipTransferred { stream_id: u64, old_owner: Option<Address>, new_owner: Address }` | When claim ownership of a stream is transferred. |
| RecipientShareDelegated | `["del_share", stream_id: u64]` | `RecipientShareDelegated { parent_stream_id: u64, child_stream_id: u64, delegator: Address, delegatee: Address, share_bps: u32, new_parent_rate: i128, child_rate: i128 }` | When a recipient delegates a percentage yield share. |
| StreamOfferCreated | `["offr_crt", offer_id: u64]` | `StreamOfferCreated { offer_id: u64, sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64, expiry_time: Option<u64>, created_at: u64 }` | When a stream creation offer is created. |
| StreamOfferAccepted | `["offr_acc", offer_id: u64]` | `StreamOfferAccepted { offer_id: u64, effective_start_time: u64, recipient: Address }` | When a stream creation offer is accepted. |
| StreamOfferCancelled | `["offr_cxl", offer_id: u64]` | `StreamOfferCancelled { offer_id: u64, by: Address, refund_amount: i128 }` | When a stream creation offer is cancelled or rejected. |
| ContractUpgraded | `["upgraded"]` | `ContractUpgraded { new_wasm_hash: BytesN<32>, new_version: u32, upgraded_at: u64, upgraded_by: Address }` | After a successful admin `upgrade`. The host also emits `executable_update`, and Fluxora emits legacy `["upgrade"]` data `(new_wasm_hash, old_version, new_version, admin)`. Compatibility note: application-event version slots report the WASM executing the upgrade call; query `version()` after finality for the replacement version. |

**Additional topics (validator):** `cloned`, `kp_cncl`, `gl_pause`, `gl_resume`, `rate_dec`, `tmpl_def`, `health`, `ex_swept`, `ac_set`, `ac_revoke`, `ac_trig`, `renewed`, `migrated`, `res_rel`.

## Exact Soroban event structure

Soroban events are represented as JSON in test snapshots; the general shape is:

- **topics**: array of topic items (symbols or values)
- **data**: a value (single item) which can be a primitive, a struct, or a tuple

### 1) StreamCreated

Emitted by `persist_new_stream` after a successful `create_stream`, `create_streams`, or `create_streams_partial` call.

```
topics: ["created", <stream_id: u64>]
data:   StreamCreated {
          stream_id:              u64,
          sender:                 Address,
          recipient:              Address,
          deposit_amount:         i128,
          rate_per_second:        i128,
          start_time:             u64,
          cliff_time:             u64,
          end_time:               u64,
          withdraw_dust_threshold: i128,
          memo:                   Option<Bytes>,   // None when not supplied; max 64 bytes
          metadata:               Option<Map<Bytes,Bytes>>,  // None when not supplied
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
    pub reason: soroban_sdk::String,
}

#[contracttype]
pub enum PauseReason {
    Operational   = 0,  // Routine sender-initiated pause
    Administrative = 1, // Admin-initiated pause
    Emergency     = 2,  // Security-related pause
    Compliance    = 3,  // Regulatory/compliance hold
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
> to `StreamPaused { stream_id, reason }`. The `reason` field is a `String` (serialised reason code).
> Indexers must update their pause event parsers.
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
          new_end_time:       u64,  // end_time after the top-up (unchanged by top-up itself)
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

### 13) KeeperCancelled

Emitted by `keeper_cancel` after all token transfers succeed (CEI-compliant). The
event carries the full fee breakdown so off-chain indexers can reconstruct keeper
economics without inspecting individual token transfers.

**Fee accounting identity (always holds):**
```
keeper_fee + recipient_amount + sender_refund == deposit_amount - prior_withdrawn_amount
```

```
topics: ["kp_cncl", <stream_id: u64>]
data:   KeeperCancelled {
          stream_id:        u64,    // stream that was cancelled
          keeper:           Address, // keeper who triggered cancellation
          keeper_fee:       i128,   // KEEPER_FEE_BPS (50 bps) of gross sender refund
          recipient_amount: i128,   // accrued - prior withdrawals (may be 0)
          sender_refund:    i128,   // unstreamed deposit minus keeper fee
        }
```

Example (partial accrual, deposit=10000, accrued=5000):

```json
{
  "topics": ["kp_cncl", 7],
  "data": {
    "stream_id": 7,
    "keeper": "G...KEEPER...",
    "keeper_fee": 25,
    "recipient_amount": 5000,
    "sender_refund": 4975
  }
}
```

Example (fully accrued, zero keeper fee):

```json
{
  "topics": ["kp_cncl", 12],
  "data": {
    "stream_id": 12,
    "keeper": "G...KEEPER...",
    "keeper_fee": 0,
    "recipient_amount": 1000,
    "sender_refund": 0
  }
}
```

**Indexer notes:**
- `keeper_fee` is always `floor(unstreamed * 50 / 10000)`.
- A fully-accrued stream has `sender_refund = 0` and `keeper_fee = 0`.
- The event is emitted in the same transaction as the state write to `Cancelled`;
  no `StreamCancelled` event is emitted for keeper-initiated cancellations.

### 14) StreamHealthChanged

Emitted by `decrease_rate_per_second`, `shorten_stream_end_time`, `top_up_stream`,
and `cancel_stream` when the stream's funding health status transitions between
adequately funded and underfunded.

A stream is **underfunded** when `remaining_balance < rate_per_second × seconds_remaining`.
Terminal streams (`Completed`, `Cancelled`) have `seconds_remaining = 0` and are never underfunded.

This event is only emitted when the `is_underfunded` flag actually changes, not on every mutation.

```
topics: ["health", <stream_id: u64>]
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
  "topics": ["health", 0],
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
  "topics": ["health", 0],
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

### 15) Additional stream event schemas

These events are listed in the event table above; their exact payload shapes follow.

**StreamCloned:**

```
topics: ["cloned", <stream_id: u64>]
data:   StreamCloned {
          new_stream_id:           u64,
          source_stream_id:        u64,
          sender:                  Address,
          recipient:               Address,
          deposit_amount:          i128,
          rate_per_second:         i128,
          start_time:              u64,
          cliff_time:              u64,
          end_time:                u64,
          withdraw_dust_threshold: i128,
        }
```

**StreamRenewed:**

```
topics: ["renewed", <old_stream_id: u64>, <new_stream_id: u64>]
data:   StreamRenewed {
          old_stream_id: u64,
          new_stream_id: u64,
        }
```

**RateCapEnforced:**

```
topics: ["rate_cap", <stream_id: u64>]
data:   RateCapEnforced {
          stream_id:          u64,
          attempted_rate:     i128,
          max_rate_per_second: i128,
        }
```

**RateDecreased:**

```
topics: ["rate_dec", <stream_id: u64>]
data:   RateDecreased {
          stream_id:            u64,
          old_rate_per_second:  i128,
          new_rate_per_second:  i128,
          effective_time:       u64,
          checkpointed_amount:  i128,
          refund_amount:        i128,
        }
```

**RecipientUpdated:**

```
topics: ["recp_upd", <stream_id: u64>]
data:   RecipientUpdated {
          stream_id:      u64,
          old_recipient:  Address,
          new_recipient:  Address,
        }
```

**GlobalEmergencyPauseChanged:**

```
topics: ["gl_pause"]
data:   GlobalEmergencyPauseChanged {
          paused: bool,
        }
```

**GlobalResumed:**

```
topics: ["gl_resume"]
data:   GlobalResumed {
          resumed_at: u64,
        }
```

**ContractPauseChanged:**

```
topics: ["paused_ctl"]
data:   ContractPauseChanged {
          paused: bool,
        }
```

**ExcessSwept:**

```
topics: ["ex_swept", <recipient: Address>]
data:   ExcessSwept {
          to:     Address,
          amount: i128,
        }
```

**AutoClaimSet:**

```
topics: ["ac_set", <stream_id: u64>]
data:   AutoClaimSet {
          stream_id:    u64,
          destination:  Address,
        }
```

**AutoClaimRevoked:**

```
topics: ["ac_revoke", <stream_id: u64>]
data:   AutoClaimRevoked {
          stream_id: u64,
        }
```

**AutoClaimTriggered:**

```
topics: ["ac_trig", <stream_id: u64>]
data:   AutoClaimTriggered {
          stream_id:    u64,
          destination:  Address,
          amount:       i128,
        }
```

**StreamDecommissioned:**

```
topics: ["decomm", <stream_id: u64>]
data:   StreamDecommissioned {
          stream_id:      u64,
          decommissioned: bool,
        }
```

**RecipientShareDelegated:**

```
topics: ["del_share", <stream_id: u64>]
data:   RecipientShareDelegated {
          parent_stream_id:  u64,
          child_stream_id:   u64,
          delegator:         Address,
          delegatee:         Address,
          share_bps:         u32,
          new_parent_rate:   i128,
          child_rate:        i128,
        }
```

**StreamOfferCreated:**

```
topics: ["offr_crt", <offer_id: u64>]
data:   StreamOfferCreated {
          offer_id:        u64,
          sender:          Address,
          recipient:       Address,
          deposit_amount:  i128,
          rate_per_second: i128,
          start_time:      u64,
          cliff_time:      u64,
          end_time:        u64,
          expiry_time:     Option<u64>,
          created_at:      u64,
        }
```

**StreamOfferAccepted:**

```
topics: ["offr_acc", <offer_id: u64>]
data:   StreamOfferAccepted {
          offer_id:            u64,
          effective_start_time: u64,
          recipient:           Address,
        }
```

**StreamOfferCancelled:**

```
topics: ["offr_cxl", <offer_id: u64>]
data:   StreamOfferCancelled {
          offer_id:      u64,
          by:            Address,
          refund_amount: i128,
        }
```

**ClaimOwnershipTransferred:**

```
topics: ["claim_own", <stream_id: u64>]
data:   ClaimOwnershipTransferred {
          stream_id:  u64,
          old_owner:  Option<Address>,
          new_owner:  Address,
        }
```

**ContractUpgraded:**

```
topics: ["upgraded"]
data:   ContractUpgraded {
          new_wasm_hash: BytesN<32>,
          new_version:   u32,
          upgraded_at:   u64,
          upgraded_by:   Address,
        }
```

The historical `new_version` name does not mean the replacement WASM was
introspected. The current invocation continues executing the pre-update code,
so `ContractUpgraded.new_version` and both legacy tuple version slots equal the
executing `CONTRACT_VERSION`. Treat the host `executable_update` event and
`new_wasm_hash` as the executable-change record, then call `version()` after
transaction finality. Failed upgrades leave no events.

---

## Parsing recommendations for indexers


- Use `topics[0]` to filter by event type; use `topics[1]` to get the `stream_id`
  for all stream-level events.
- **v9 breaking event-payload change**: Starting from `CONTRACT_VERSION = 9`, the `amount` field on a `Withdrawal` event emitted by `delegated_withdraw` reports the **recipient's net amount** (`gross_withdrawable − relayer_fee`), **not** the gross withdrawable total. Indexers, dashboards, and accounting pipelines built against pre-v9 fixtures need to recompute against this new semantics, otherwise they will under-report by `relayer_fee`. The `Withdrawal.amount` from
  plain `withdraw` / `withdraw_to` / `batch_withdraw` paths is **unchanged** (still equals the amount transferred). Only the `delegated_withdraw` path now reports the net.
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

---

## Factory contract events (`fluxora_factory`)

All state-changing factory entrypoints emit structured events. Topics are ≤ 9
characters (`symbol_short!` constraint). Naming mirrors `FluxoraStream` where
applicable (e.g. `AdminUpd`).

| Event name | Topic(s) | Data (shape & types) | When emitted |
|---|---|---|---|
| FactoryInited | `["fct_init"]` | `FactoryInited { admin: Address, stream_contract: Address, max_deposit: i128, min_duration: u64 }` | Once, when `init` completes successfully. |
| FactoryAdminUpdated | `["AdminUpd"]` | `FactoryAdminUpdated { old_admin: Address, new_admin: Address }` | When `set_admin` rotates the factory admin. |
| StreamContractUpdated | `["stm_upd"]` | `StreamContractUpdated { old_contract: Address, new_contract: Address }` | When `set_stream_contract` changes the stream-contract pointer. |
| AllowlistUpdated | `["allow_upd"]` | `AllowlistUpdated { recipient: Address, allowed: bool }` | When `set_allowlist` adds (`allowed: true`) or removes (`allowed: false`) a recipient. |
| CapUpdated | `["cap_upd"]` | `CapUpdated { old_cap: i128, new_cap: i128 }` | When `set_cap` updates the factory deposit cap. |
| MinDurationUpdated | `["dur_upd"]` | `MinDurationUpdated { old_min_duration: u64, new_min_duration: u64 }` | When `set_min_duration` updates the minimum stream duration policy. |
| RateBoundsUpdated | `["rate_bnd"]` | `RateBoundsUpdated { min_rate: Option<i128>, max_rate: Option<i128> }` | When `set_rate_bounds` updates rate-per-second bounds. `None` = argument not supplied by caller. |
| FactoryPaused/Resumed | `["factory", "paused"]` / `["factory", "resumed"]` | `bool` | When `set_factory_paused` toggles the pause flag (pre-existing). |
| FactoryStreamCreated | `["fct_strm"]` | `FactoryStreamCreated { stream_id: u64, sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128 }` | After a policy-gated `create_stream` or batch `create_streams` succeeds (emits one event per created stream). Not emitted on any validation or downstream failure. |

### Example JSON (FactoryStreamCreated)

```json
{
  "topics": ["fct_strm"],
  "data": {
    "stream_id": 42,
    "sender": "G...SENDER...",
    "recipient": "G...RECIPIENT...",
    "deposit_amount": 50000,
    "rate_per_second": 10
  }
}
```

### Example JSON (AllowlistUpdated)

```json
{
  "topics": ["allow_upd"],
  "data": { "recipient": "G...RECIPIENT...", "allowed": true }
}
```

---

## Governance contract events (`fluxora_governance`)

All state-changing governance entrypoints emit structured events. Topics are ≤ 9
characters (`symbol_short!` constraint). 

| Event name | Topic(s) | Data (shape & types) | When emitted |
|---|---|---|---|
| ProposalCreated | `["proposed", proposal_id: u32]` | `ProposalCreated { proposal_id: u32, proposer: Address, target: Address }` | When `propose` is called successfully. |
| ProposalApproved | `["approved", proposal_id: u32]` | `ProposalApproved { proposal_id: u32, approver: Address, approval_count: u32 }` | When a co-signer successfully approves a proposal. |
| QuorumReached | `["quorum", proposal_id: u32]` | `QuorumReached { proposal_id: u32, quorum_reached_at: u64, executable_after: u64 }` | When a proposal reaches the approval threshold. |
| ProposalCancelled | `["cancelled", proposal_id: u32]` | `ProposalCancelled { proposal_id: u32, canceller: Address }` | When a proposal is cancelled. |
| ProposalExecuted | `["executed", proposal_id: u32]` | `ProposalExecuted { proposal_id: u32, executor: Address, target: Address, calldata: Bytes }` | When a proposal is executed successfully. |
| SignerAdded | `["sgnr_add"]` | `SignerAdded { signer: Address }` | When `add_signer` adds a new co-signer. |
| SignerRemoved | `["sgnr_rm"]` | `SignerRemoved { signer: Address }` | When `remove_signer` successfully removes a co-signer. |
| AdminChanged | `["adm_chg"]` | `AdminChanged { old: Address, new: Address }` | When the contract admin is rotated. |
| QuorumConfig | `["quor_cfg"]` | `QuorumConfig { threshold: u32, signer_count: u32 }` | Emitted after `SignerAdded` and `SignerRemoved` to allow indexers to track quorum health and threshold satisfiability. |

---

## Keeping this doc in sync

This file is derived from `contracts/stream/src/lib.rs`, `contracts/stream/src/events.rs`, and `contracts/stream/src/types.rs`. Each event is emitted by its corresponding helper in `events.rs` or directly from `lib.rs`. The canonical source of truth for event payload structs is `lib.rs` and `types.rs`.

Event emit helpers are in `contracts/stream/src/events.rs`. The test in `tests/event_schema_consistency.rs` cross-checks every event struct against the documented shapes below. Both the test and this document must be updated when event payloads change.

## Event schema evolution policy

The contract follows an **additive-only backward-compatibility policy** for all
event payloads. See [`docs/event-schema-evolution.md`](./event-schema-evolution.md)
for the complete rules:

- New optional fields may only be appended at the end of existing structs
- Existing fields must never change type, name, or position
- Topic symbols are permanent once released
- Deprecation follows a three-phase lifecycle (announce → signal → remove)

All changes to event payloads must comply with this policy.

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
| `set_admin`                                                  | `"AdminUpd"`    |
| `set_contract_paused`                                        | `"ct_pause"`    |
| `pause_protocol`                                             | `"pr_pause"`    |
| `resume_protocol`                                            | `"pr_resume"`   |
| `update_recipient`                                           | `"recp_upd"`    |
| `renew_stream`                                               | `"renewed"`     |
| `clone_stream`                                               | `"cloned"`      |
| `decrease_rate_per_second`                                   | `"rate_dec"`    |
| `set_global_emergency_paused`                                | `"gl_pause"`    |
| `resume_global_pause`                                        | `"gl_resume"`   |
| `revoke_auto_claim`                                          | `"ac_revoke"`   |
| `set_auto_claim`                                             | `"ac_set"`      |
| `trigger_auto_claim`                                         | `"ac_trig"`     |
| `sweep_excess`                                               | `"ex_swept"`    |
| `keeper_cancel`                                              | `"kp_cncl"`     |
| `decrease_rate_per_second`, `shorten_stream_end_time`, `top_up_stream`, `cancel_stream` | `"health"` |
| `transfer_claim_ownership`                                   | `"claim_own"`   |
| `delegate_recipient_share`                                   | `"del_share"`   |
| `create_stream_offer`                                        | `"offr_crt"`    |
| `accept_stream_offer`                                        | `"offr_acc"`    |
| `cancel_stream_offer`, `reject_stream_offer`                 | `"offr_cxl"`    |
| `set_decommissioned`                                         | `"decomm"`      |
| `upgrade` (primary)                                          | `"upgraded"`    |
| `upgrade` (backward compat)                                  | `"upgrade"`     |

If you change event topics or payloads in the contract, update this document and
include updated example snapshots in the PR.


## Additional event topics

- `claim_own`: Emitted when claim ownership is transferred via `transfer_claim_ownership`.
- `del_share`: Emitted when a recipient delegates a share of their yield via `delegate_recipient_share`.
- `offr_acc`: Emitted when a `StreamOffer` is accepted by its recipient.
- `offr_crt`: Emitted when a `StreamOffer` is created by a sender.
- `offr_cxl`: Emitted when a `StreamOffer` is cancelled by the sender or rejected by the recipient.
