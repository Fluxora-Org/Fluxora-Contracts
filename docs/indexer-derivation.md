# Indexer Derivation Specification

How indexers should interpret Fluxora stream events, when they must read
on-chain state, and how to derive fields like `cancelled_at`, refund amounts,
and completion status from the event stream alone or in combination with
`get_stream_state`.

**Consistency requirement:** This document is derived from and must remain
consistent with [`docs/events.md`](./events.md) and [`docs/streaming.md`](./streaming.md).
If the contract changes event shapes or lifecycle semantics, update all three files.

---

## 1. Overview

The Fluxora stream contract emits structured events for every state-mutating
operation. An indexer can reconstruct the full lifecycle of any stream by
processing events in ledger order. However, some fields (notably `cancelled_at`
and the exact refund amount) are **not embedded in events** and require a
`get_stream_state` RPC call to derive accurately.

This document specifies:

1. Which fields can be derived from events alone.
2. Which fields require a `get_stream_state` read.
3. The exact derivation rules for each terminal state.
4. Worked examples for each lifecycle path.

---

## 2. Event-to-State Mapping

### 2.1 Stream creation

**Event:** `("created", stream_id)` → `StreamCreated { ... }`

**Derivable from event alone:**

| Field | Source |
|---|---|
| `stream_id` | `topics[1]` or `data.stream_id` |
| `sender` | `data.sender` |
| `recipient` | `data.recipient` |
| `deposit_amount` | `data.deposit_amount` |
| `rate_per_second` | `data.rate_per_second` |
| `start_time` | `data.start_time` |
| `cliff_time` | `data.cliff_time` |
| `end_time` | `data.end_time` |
| `memo` | `data.memo` (may be `None`) |
| `status` | Infer `Active` (all newly created streams start Active) |
| `withdrawn_amount` | Infer `0` |
| `cancelled_at` | Infer `None` |

**No `get_stream_state` call required** for initial indexing of a new stream.

---

### 2.2 Withdrawal

**Event:** `("withdrew", stream_id)` → `Withdrawal { stream_id, recipient, amount }`

**Derivable from event alone:**

| Field | Derivation |
|---|---|
| `withdrawn_amount` | Accumulate: `withdrawn_amount += amount` |
| `status` | Remains `Active` unless a `completed` event follows in the same transaction |

**Note:** `Withdrawal` does not embed the new `withdrawn_amount` total. Indexers
must maintain a running sum.

---

### 2.3 Completion

**Event:** `("completed", stream_id)` → `StreamEvent::StreamCompleted(stream_id)`

**Always emitted on the same call as the final `Withdrawal` event** that drains
an `Active` stream. The `completed` event appears **after** the `withdrew` event
in the same transaction's event list.

**Derivable from event alone:**

| Field | Derivation |
|---|---|
| `status` | Set to `Completed` |
| `withdrawn_amount` | Equal to `deposit_amount` (all tokens withdrawn) |

**No `get_stream_state` call required** to detect completion.

> **Important:** `Completed` is only emitted when an `Active` (or `Paused`)
> stream's `withdrawn_amount` reaches `deposit_amount`. Cancelled streams do
> **not** emit `completed` even if the recipient withdraws the full accrued
> amount.

---

### 2.4 Cancellation

**Event:** `("cancelled", stream_id)` → `StreamEvent::StreamCancelled(stream_id)`

**The `cancelled` event does NOT embed:**
- `cancelled_at` timestamp
- Refund amount
- Accrued amount at cancellation

**Fields that require `get_stream_state`:**

| Field | Why event is insufficient |
|---|---|
| `cancelled_at` | Not in event payload; must read `stream.cancelled_at` |
| Refund amount | Not in event; derive as `deposit_amount - accrued_at_cancelled_at` |
| Accrued at cancel | Not in event; derive from `cancelled_at` using accrual formula |

**Derivation procedure:**

```
1. On observing ("cancelled", stream_id):
   a. Call get_stream_state(stream_id)
   b. Read stream.cancelled_at  → the freeze timestamp
   c. Compute accrued_at_cancel = accrual_formula(stream, cancelled_at)
   d. Compute refund = stream.deposit_amount - accrued_at_cancel
   e. Set stream.status = Cancelled
   f. Set stream.cancelled_at = cancelled_at
```

**Accrual formula** (from `docs/streaming.md`):

```
if cancelled_at < cliff_time:
    accrued_at_cancel = 0
else:
    elapsed = min(cancelled_at, end_time) - start_time
    accrued_at_cancel = min(elapsed × rate_per_second, deposit_amount)
```

For streams with rate changes (`checkpointed_amount`, `checkpointed_at`), use
the checkpoint-aware formula:

```
if cancelled_at < cliff_time:
    accrued_at_cancel = 0
else:
    elapsed_since_checkpoint = min(cancelled_at, end_time) - checkpointed_at
    accrued_at_cancel = min(
        checkpointed_amount + elapsed_since_checkpoint × rate_per_second,
        deposit_amount
    )
```

**Example:**

```
Stream: deposit=3000, rate=1, start=0, cliff=0, end=3000
Cancelled at t=1200

accrued_at_cancel = min(1200 × 1, 3000) = 1200
refund = 3000 - 1200 = 1800

Recipient can still withdraw: 1200 tokens
Sender received refund: 1800 tokens
```

---

### 2.5 Pause and Resume

**Events:**
- `("paused", stream_id)` → `StreamEvent::Paused(stream_id)`
- `("resumed", stream_id)` → `StreamEvent::Resumed(stream_id)`

**Derivable from event alone:**

| Event | Status transition |
|---|---|
| `paused` | `Active` → `Paused` |
| `resumed` | `Paused` → `Active` |

**No `get_stream_state` call required** for pause/resume tracking.

> **Accrual note:** Pausing does **not** stop accrual. The accrual formula is
> time-based and unaffected by pause status. Indexers must not freeze accrual
> calculations when a `paused` event is observed.

---

### 2.6 Stream closed

**Event:** `("closed", stream_id)` → `StreamEvent::StreamClosed(stream_id)`

**Derivable from event alone:**

| Field | Derivation |
|---|---|
| `status` | Stream is permanently removed from on-chain storage |

After observing a `closed` event, `get_stream_state(stream_id)` will return
`StreamNotFound`. Indexers should mark the stream as archived and stop
attempting to read its on-chain state.

**Emitted before storage deletion.** The event is published in the same
transaction that removes the stream entry. Indexers processing events in ledger
order will always see the `closed` event before the storage entry disappears.

---

### 2.7 Rate changes

**Events:**
- `("rate_upd", stream_id)` → `RateUpdated { stream_id, old_rate_per_second, new_rate_per_second, effective_time }`
- `("rate_dec", stream_id)` → `RateDecreased { stream_id, old_rate_per_second, new_rate_per_second, effective_time, checkpointed_amount, refund_amount }`

**Derivable from event alone:**

| Field | Source |
|---|---|
| `rate_per_second` | `data.new_rate_per_second` |
| `checkpointed_amount` | `data.checkpointed_amount` (only in `RateDecreased`) |
| `checkpointed_at` | `data.effective_time` |
| Refund to sender | `data.refund_amount` (only in `RateDecreased`) |

For `RateUpdated` (increase only), the `checkpointed_amount` and `checkpointed_at`
are not in the event. Call `get_stream_state` if you need the updated checkpoint
values for accrual calculations.

---

### 2.8 Schedule changes

**Events:**
- `("end_shrt", stream_id)` → `StreamEndShortened { stream_id, old_end_time, new_end_time, refund_amount }`
- `("end_ext", stream_id)` → `StreamEndExtended { stream_id, old_end_time, new_end_time }`

**Derivable from event alone:**

| Field | Source |
|---|---|
| `end_time` | `data.new_end_time` |
| `deposit_amount` (after shorten) | `old_deposit - refund_amount` |
| Refund to sender | `data.refund_amount` (only in `StreamEndShortened`) |

---

### 2.9 Top-up

**Event:** `("top_up", stream_id)` → `StreamToppedUp { stream_id, top_up_amount, new_deposit_amount, new_end_time }`

**Derivable from event alone:**

| Field | Source |
|---|---|
| `deposit_amount` | `data.new_deposit_amount` |

---

## 3. When to Call `get_stream_state`

| Scenario | Required? | Reason |
|---|---|---|
| Initial stream creation | No | All fields in `StreamCreated` event |
| Withdrawal tracking | No | Accumulate `amount` from `Withdrawal` events |
| Completion detection | No | `completed` event is definitive |
| Pause/resume tracking | No | Events are definitive |
| Cancellation: `cancelled_at` | **Yes** | Not in event payload |
| Cancellation: refund amount | **Yes** | Requires `cancelled_at` for accrual formula |
| Rate increase: checkpoint values | **Yes** | Not in `RateUpdated` event |
| Rate decrease: checkpoint values | No | In `RateDecreased` event |
| Schedule shorten: new deposit | No | Derivable from `refund_amount` |
| After `closed` event | No | Storage is gone; use cached state |
| Reconciliation / audit | Recommended | Verify indexer state matches on-chain |

---

## 4. State Reconciliation

Indexers should periodically reconcile their cached state against on-chain
state using `get_stream_state`. This is especially important after:

- Network outages or missed events
- Contract upgrades (new `CONTRACT_VERSION`)
- Any operation that requires a `get_stream_state` call (see table above)

**Reconciliation procedure:**

```
1. For each tracked stream_id:
   a. Call get_stream_state(stream_id)
   b. If StreamNotFound: stream was closed; mark as archived
   c. Otherwise: compare cached fields with on-chain fields
   d. On mismatch: update cache and log the discrepancy
```

---

## 5. Worked Examples

### Example A: Full lifecycle — Active → Completed

```
Ledger events (in order):

1. ("created", 0) → StreamCreated { deposit=1000, rate=1, start=0, end=1000, ... }
   Indexer: status=Active, withdrawn=0, deposit=1000

2. ("withdrew", 0) → Withdrawal { amount=300 }
   Indexer: withdrawn=300

3. ("withdrew", 0) → Withdrawal { amount=700 }
   ("completed", 0) → StreamCompleted(0)
   Indexer: withdrawn=1000, status=Completed

4. ("closed", 0) → StreamClosed(0)
   Indexer: archived (no further on-chain state)
```

No `get_stream_state` calls required.

---

### Example B: Cancellation mid-stream

```
Ledger events (in order):

1. ("created", 1) → StreamCreated { deposit=3000, rate=1, start=0, end=3000, ... }
   Indexer: status=Active, withdrawn=0

2. ("cancelled", 1) → StreamCancelled(1)
   Indexer action:
     → Call get_stream_state(1)
     → Read: cancelled_at=1200
     → Compute: accrued_at_cancel = min(1200×1, 3000) = 1200
     → Compute: refund = 3000 - 1200 = 1800
     → Set: status=Cancelled, cancelled_at=1200
     → Record: sender_refund=1800, recipient_claimable=1200

3. ("withdrew", 1) → Withdrawal { amount=1200 }
   Indexer: withdrawn=1200
   Note: status remains Cancelled (no "completed" event emitted)
```

`get_stream_state` required at step 2 to obtain `cancelled_at`.

---

### Example C: Pause → Resume → Completion

```
Ledger events (in order):

1. ("created", 2) → StreamCreated { deposit=2000, rate=2, start=0, end=1000, ... }
   Indexer: status=Active

2. ("paused", 2) → Paused(2)
   Indexer: status=Paused
   Note: accrual continues (time-based); do NOT freeze accrual calculation

3. ("resumed", 2) → Resumed(2)
   Indexer: status=Active

4. ("withdrew", 2) → Withdrawal { amount=2000 }
   ("completed", 2) → StreamCompleted(2)
   Indexer: withdrawn=2000, status=Completed
```

No `get_stream_state` calls required.

---

### Example D: Rate decrease mid-stream

```
Ledger events (in order):

1. ("created", 3) → StreamCreated { deposit=5000, rate=5, start=0, end=1000, ... }
   Indexer: rate=5, checkpointed_amount=0, checkpointed_at=0

2. ("rate_dec", 3) → RateDecreased {
     old_rate=5, new_rate=2,
     effective_time=400,
     checkpointed_amount=2000,  ← accrued under old rate at t=400
     refund_amount=1200
   }
   Indexer: rate=2, checkpointed_amount=2000, checkpointed_at=400
   Note: deposit_amount reduced by refund_amount (5000 - 1200 = 3800)

3. ("withdrew", 3) → Withdrawal { amount=3800 }
   ("completed", 3) → StreamCompleted(3)
   Indexer: withdrawn=3800, status=Completed
```

No `get_stream_state` calls required (all checkpoint data in `RateDecreased` event).

---

### Example E: Cancellation before cliff

```
Ledger events (in order):

1. ("created", 4) → StreamCreated { deposit=3000, rate=1, start=0, cliff=1500, end=3000, ... }
   Indexer: status=Active, cliff_time=1500

2. ("cancelled", 4) → StreamCancelled(4)
   Indexer action:
     → Call get_stream_state(4)
     → Read: cancelled_at=800  (before cliff=1500)
     → Compute: accrued_at_cancel = 0  (before cliff → no accrual)
     → Compute: refund = 3000 - 0 = 3000
     → Set: status=Cancelled, cancelled_at=800
     → Record: sender_refund=3000, recipient_claimable=0
```

`get_stream_state` required to obtain `cancelled_at`.

---

## 6. Indexer Implementation Checklist

- [ ] Parse `StreamCreated` event to initialise stream state
- [ ] Accumulate `Withdrawal.amount` into `withdrawn_amount`
- [ ] Detect completion via `StreamCompleted` event (not by comparing `withdrawn_amount == deposit_amount`)
- [ ] On `StreamCancelled`: call `get_stream_state` to obtain `cancelled_at`
- [ ] Derive refund and accrued-at-cancel using the checkpoint-aware accrual formula
- [ ] Do NOT freeze accrual on `Paused` events (accrual is time-based)
- [ ] On `StreamClosed`: mark stream as archived; stop reading on-chain state
- [ ] Handle `RateDecreased` event: update `rate_per_second`, `checkpointed_amount`, `checkpointed_at`, `deposit_amount`
- [ ] Handle `RateUpdated` event: update `rate_per_second`; call `get_stream_state` for checkpoint values if needed
- [ ] Handle `StreamEndShortened`: update `end_time` and `deposit_amount`
- [ ] Handle `StreamEndExtended`: update `end_time`
- [ ] Handle `StreamToppedUp`: update `deposit_amount`
- [ ] Periodically reconcile cached state against `get_stream_state` for audit integrity
- [ ] Handle `StreamNotFound` from `get_stream_state` (stream was closed between event and read)

---

## 7. Event Ordering Guarantees

Within a single transaction, events are emitted in this order:

| Operation | Event sequence |
|---|---|
| `withdraw` (final drain) | `withdrew` → `completed` |
| `withdraw_to` (final drain) | `wdraw_to` → `completed` |
| `batch_withdraw` (per stream) | `withdrew` [→ `completed`] for each stream in order |
| `close_completed_stream` | `closed` (then storage deleted) |
| `cancel_stream` | `cancelled` (state persisted before event) |

Across transactions, events appear in ledger sequence number order. Indexers
must process events in strict ledger order to maintain correct state.

---

## 8. Handling Missed Events

If an indexer misses events (e.g. due to network outage), it should:

1. Identify the last processed ledger sequence number.
2. Re-fetch events from that ledger onwards using the Horizon or RPC event API.
3. Re-apply missed events in order.
4. For any stream in an uncertain state, call `get_stream_state` to reconcile.

The contract does not provide a "replay" mechanism. All historical events are
available via the Stellar event archive for the contract's lifetime.
