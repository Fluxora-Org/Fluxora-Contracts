# Fluxora Streaming Contract — Security Documentation

**Issue #262 · CEI Ordering: External Token Calls After State Updates**
**Version:** 1 · **Contract:** `FluxoraStream` · **Platform:** Soroban (Stellar)

---

## 1. Scope

This document covers the security properties of the `FluxoraStream` contract with
particular focus on the Checks-Effects-Interactions (CEI) pattern, token trust
assumptions, authorization paths, and edge-case behavior across stream lifecycle states.

---

## 2. CEI Ordering — Guarantee Matrix

CEI requires that all state writes happen **before** any external token call in the
same entrypoint. Every entrypoint that moves tokens is audited below.

| Entrypoint | State saved before transfer? | Transfer direction | Notes |
|---|---|---|---|
| `create_stream` | ✅ Validation + pull only; `persist_new_stream` called **after** `pull_token` succeeds | IN (pull from sender) | If pull fails, no stream ID is allocated and no state is written. |
| `create_streams` | ✅ Full validation pass, then single bulk `pull_token`; ID allocation and persistence happen **after** | IN (bulk pull) | Two-pass design: validate → transfer → persist. Atomic for the whole batch. |
| `withdraw` | ✅ `withdrawn_amount` incremented and `save_stream` called **before** `push_token` | OUT (push to recipient) | CEI comment in source. `completed_now` flag set before push. |
| `withdraw_to` | ✅ Same as `withdraw`; state saved before `push_token` to `destination` | OUT (push to destination) | Destination validation (≠ contract address) happens before state write. |
| `batch_withdraw` | ✅ Per-stream state saved before each `push_token` | OUT (push per stream) | Each iteration follows CEI independently. |
| `cancel_stream` | ✅ `status = Cancelled`, `cancelled_at = Some(now)` persisted before `push_token` refund | OUT (refund to sender) | Shared via `cancel_stream_internal`. |
| `cancel_stream_as_admin` | ✅ Same — delegates to `cancel_stream_internal` | OUT (refund to sender) | Identical externally visible behavior to sender path. |
| `shorten_stream_end_time` | ✅ `end_time` and `deposit_amount` updated, `save_stream` called before refund push | OUT (refund to sender) | Refund skipped if `refund_amount == 0`. |
| `top_up_stream` | ✅ `deposit_amount` incremented and `save_stream` called **before** `pull_token` | IN (pull from funder) | Intentional reversal: state update precedes pull so reentrancy cannot double-credit. |

**Residual risk:** Soroban's execution model does not support mid-transaction reentrancy
in the same way EVM does, but CEI is maintained throughout as a defense-in-depth
invariant and to remain safe if the token contract ever makes a cross-contract call.

---

## 3. Token Trust Model

### 3.1 Single trusted token

The token address is set once in `init()` and stored in `Config.token`. It cannot be
changed after initialization. All token interactions go through `pull_token` and
`push_token`, which construct a `token::Client` against this single address.

### 3.2 Assumptions about the token contract

The contract assumes the token:

- Conforms to the Soroban SEP-41 (CAP-0046) token interface.
- Does **not** reenter `FluxoraStream` during a `transfer` call.
- Reverts atomically on insufficient balance or allowance, causing the entire Soroban
  transaction to roll back.
- Does not perform fee-on-transfer or rebasing that would cause the received amount to
  differ from the requested amount.

**If any of these assumptions are violated**, CEI ordering limits the blast radius but
does not eliminate all risk. Operators should only configure well-audited token contracts.

### 3.3 No token balance tracking

The contract does not maintain an internal ledger of its token balance. It relies on the
token contract's own accounting. The invariant `sum(deposit_amount - withdrawn_amount)
for all active streams ≤ contract token balance` is maintained by construction but is
not asserted on-chain.

---

## 4. Authorization Matrix

| Operation | Authorized caller(s) | Auth check location |
|---|---|---|
| `init` | `admin` param (bootstrap) | `admin.require_auth()` before any state write |
| `create_stream` | `sender` | `sender.require_auth()` at entry |
| `create_streams` | `sender` | `sender.require_auth()` at entry |
| `pause_stream` | stream `sender` | `require_stream_sender(&stream.sender)` |
| `resume_stream` | stream `sender` | `require_stream_sender(&stream.sender)` |
| `cancel_stream` | stream `sender` | `require_stream_sender(&stream.sender)` |
| `withdraw` | stream `recipient` | `stream.recipient.require_auth()` |
| `withdraw_to` | stream `recipient` | `stream.recipient.require_auth()` |
| `batch_withdraw` | `recipient` param (must match all streams) | `recipient.require_auth()` at entry; per-stream ownership check |
| `update_rate_per_second` | stream `sender` | `require_stream_sender` |
| `shorten_stream_end_time` | stream `sender` | `require_stream_sender` |
| `extend_stream_end_time` | stream `sender` | `require_stream_sender` |
| `top_up_stream` | `funder` param (any address) | `funder.require_auth()` |
| `set_admin` | current `admin` | `old_admin.require_auth()` |
| `set_contract_paused` | `admin` | `get_admin(&env)?.require_auth()` |
| `set_global_emergency_paused` | `admin` | `get_admin(&env).unwrap().require_auth()` |
| `pause_stream_as_admin` | `admin` | `get_admin(&env)?.require_auth()` |
| `resume_stream_as_admin` | `admin` | `get_admin(&env)?.require_auth()` |
| `cancel_stream_as_admin` | `admin` | explicit `admin.require_auth()` |
| `close_completed_stream` | anyone (permissionless) | — |
| All `get_*` / `calculate_*` / `version` | anyone | — |

### 4.1 Auth gap: `top_up_stream`

`top_up_stream` accepts any `funder` address and only requires that address to authorize.
It does **not** restrict `funder` to the stream sender or admin. This is intentional
(treasury workflows) but operators should note that any party can increase a stream's
deposit — they cannot reduce it or redirect funds.

---

## 5. Global Pause Flags

Two independent pause mechanisms exist:

| Flag | Key | Blocks | Does not block |
|---|---|---|---|
| `GlobalPaused` | `DataKey::GlobalPaused` | `create_stream`, `create_streams` | All other operations |
| `GlobalEmergencyPaused` | `DataKey::GlobalEmergencyPaused` | `cancel_stream`, `withdraw`, `withdraw_to`, `batch_withdraw`, `update_rate_per_second`, `shorten/extend_stream_end_time`, `top_up_stream` | Admin overrides (`*_as_admin`), views, `close_completed_stream` |

`GlobalPaused` is a soft creation pause. `GlobalEmergencyPaused` is a full operational
freeze for user-facing mutations. Admin entrypoints are intentionally exempt so
operators can intervene during an emergency.

**Note:** `set_admin` is not blocked by `GlobalEmergencyPaused` in the current
implementation. This allows admin rotation even under a full freeze.

---

## 6. Arithmetic Safety

All multiplications that could overflow `i128` use `checked_mul`. Division is not used
in accrual math; only multiplication and comparison. Specific cases:

- `validate_stream_params`: `rate_per_second.checked_mul(duration)` — returns
  `ArithmeticOverflow` on overflow.
- `create_streams` total deposit accumulation: `checked_add` with `panic_with_error!`
  on overflow.
- `update_rate_per_second`: `checked_mul` for new total streamable.
- `shorten_stream_end_time` / `extend_stream_end_time`: `checked_mul` for new streamable.
- `top_up_stream`: `checked_add` on deposit amount.
- `cancel_stream_internal` refund: `checked_sub` — returns `InvalidState` on underflow
  (should be unreachable given deposit ≥ accrued invariant).

---

## 7. Known Limitations and Residual Risks

| Item | Risk level | Rationale |
|---|---|---|
| No internal balance ledger | Low | Relies on token contract correctness; no on-chain verification that contract balance equals sum of stream deposits. |
| `top_up_stream` open to any funder | Informational | Intentional; cannot drain funds. |
| `set_global_emergency_paused` uses `.unwrap()` | Low | Contract is not initialized before admin exists; `unwrap` panic is equivalent to `InvalidState` error in practice. Recommend changing to `?` with `Result` return for consistency. |
| Fee-on-transfer tokens | Medium | Not supported. Deposit accounting would diverge from actual balance. |
| No stream recipient update | Informational | If a recipient loses key access, their accrued value is permanently locked. |

# Fluxora Streaming Contract — Events Reference

**Contract:** `FluxoraStream` · **Version:** 1 · **Platform:** Soroban (Stellar)

All events are emitted via `env.events().publish((topic, stream_id), payload)`.
Topics are `symbol_short!` values (max 9 ASCII chars). Indexers should subscribe
to the contract address and filter by topic symbol.

---

## Event Catalogue

### `created` — Stream Created

Emitted when a new stream is successfully funded and persisted.

**Topic:** `("created", stream_id: u64)`

**Payload:** `StreamCreated`

| Field | Type | Description |
|---|---|---|
| `stream_id` | `u64` | Unique ID of the new stream |
| `sender` | `Address` | Address that funded the stream |
| `recipient` | `Address` | Address entitled to withdraw |
| `deposit_amount` | `i128` | Total tokens locked in contract |
| `rate_per_second` | `i128` | Accrual speed (tokens/sec) |
| `start_time` | `u64` | Ledger timestamp when accrual begins |
| `cliff_time` | `u64` | Ledger timestamp before which withdrawals return 0 |
| `end_time` | `u64` | Ledger timestamp when accrual stops |

**Emitting entrypoints:** `create_stream`, `create_streams` (one event per stream entry)

---

### `withdrew` — Withdrawal

Emitted when the recipient withdraws accrued tokens to their own address.
Not emitted when `withdrawable == 0`.

**Topic:** `("withdrew", stream_id: u64)`

**Payload:** `Withdrawal`

| Field | Type | Description |
|---|---|---|
| `stream_id` | `u64` | Stream the withdrawal was taken from |
| `recipient` | `Address` | Address that received the tokens |
| `amount` | `i128` | Tokens transferred in this withdrawal |

**Emitting entrypoints:** `withdraw`, `batch_withdraw`

---

### `wdraw_to` — Withdrawal to Destination

Emitted when the recipient redirects a withdrawal to a different address.
Not emitted when `withdrawable == 0`.

**Topic:** `("wdraw_to", stream_id: u64)`

**Payload:** `WithdrawalTo`

| Field | Type | Description |
|---|---|---|
| `stream_id` | `u64` | Stream the withdrawal was taken from |
| `recipient` | `Address` | Address that authorized the call |
| `destination` | `Address` | Address that received the tokens |
| `amount` | `i128` | Tokens transferred |

**Emitting entrypoints:** `withdraw_to`

**Note:** Both `recipient` (authorizer) and `destination` (receiver) are recorded for
full audit trail. These may be equal (self-redirect is permitted).

---

### `paused` — Stream Paused

Emitted when a stream transitions from `Active` → `Paused`.

**Topic:** `("paused", stream_id: u64)`

**Payload:** `StreamEvent::Paused(stream_id: u64)`

**Emitting entrypoints:** `pause_stream`, `pause_stream_as_admin`

---

### `resumed` — Stream Resumed

Emitted when a stream transitions from `Paused` → `Active`.

**Topic:** `("resumed", stream_id: u64)`

**Payload:** `StreamEvent::Resumed(stream_id: u64)`

**Emitting entrypoints:** `resume_stream`, `resume_stream_as_admin`

---

### `cancelled` — Stream Cancelled

Emitted when a stream transitions to the terminal `Cancelled` state.
At the time of emission, the sender's refund has already been transferred.

**Topic:** `("cancelled", stream_id: u64)`

**Payload:** `StreamEvent::StreamCancelled(stream_id: u64)`

**Emitting entrypoints:** `cancel_stream`, `cancel_stream_as_admin`

---

### `completed` — Stream Completed

Emitted when all deposited tokens have been withdrawn, transitioning the stream
to the terminal `Completed` state. Always emitted **after** the corresponding
`withdrew` or `wdraw_to` event in the same transaction.

**Topic:** `("completed", stream_id: u64)`

**Payload:** `StreamEvent::StreamCompleted(stream_id: u64)`

**Emitting entrypoints:** `withdraw`, `withdraw_to`, `batch_withdraw`

**Indexer note:** A single transaction may emit both `withdrew`/`wdraw_to` and
`completed` for the same `stream_id`. Process both.

---

### `closed` — Stream Closed

Emitted immediately before the stream's persistent storage entry is deleted.
After this event, `get_stream_state(stream_id)` returns `StreamNotFound`.

**Topic:** `("closed", stream_id: u64)`

**Payload:** `StreamEvent::StreamClosed(stream_id: u64)`

**Emitting entrypoints:** `close_completed_stream`

---

### `rate_upd` — Rate Updated

Emitted when the sender increases a stream's `rate_per_second`.

**Topic:** `("rate_upd", stream_id: u64)`

**Payload:** `RateUpdated`

| Field | Type | Description |
|---|---|---|
| `stream_id` | `u64` | Affected stream |
| `old_rate_per_second` | `i128` | Previous accrual rate |
| `new_rate_per_second` | `i128` | New accrual rate (strictly greater) |
| `effective_time` | `u64` | Ledger timestamp of the change |

**Emitting entrypoints:** `update_rate_per_second`

---

### `end_shrt` — Stream End Shortened

Emitted when a stream's end time is moved earlier and excess deposit refunded.

**Topic:** `("end_shrt", stream_id: u64)`

**Payload:** `StreamEndShortened`

| Field | Type | Description |
|---|---|---|
| `stream_id` | `u64` | Affected stream |
| `old_end_time` | `u64` | Previous end timestamp |
| `new_end_time` | `u64` | New (earlier) end timestamp |
| `refund_amount` | `i128` | Tokens returned to sender |

**Emitting entrypoints:** `shorten_stream_end_time`

---

### `end_ext` — Stream End Extended

Emitted when a stream's end time is moved later without changing deposit.

**Topic:** `("end_ext", stream_id: u64)`

**Payload:** `StreamEndExtended`

| Field | Type | Description |
|---|---|---|
| `stream_id` | `u64` | Affected stream |
| `old_end_time` | `u64` | Previous end timestamp |
| `new_end_time` | `u64` | New (later) end timestamp |

**Emitting entrypoints:** `extend_stream_end_time`

---

### `top_up` — Stream Topped Up

Emitted when additional tokens are deposited into an existing stream.

**Topic:** `("top_up", stream_id: u64)`

**Payload:** `StreamToppedUp`

| Field | Type | Description |
|---|---|---|
| `stream_id` | `u64` | Affected stream |
| `top_up_amount` | `i128` | Additional tokens deposited |
| `new_deposit_amount` | `i128` | Updated total deposit after top-up |

**Emitting entrypoints:** `top_up_stream`

---

### `AdminUpd` — Admin Updated

Emitted when the admin address is rotated.

**Topic:** `("AdminUpd",)`

**Payload:** `(old_admin: Address, new_admin: Address)`

**Emitting entrypoints:** `set_admin`

---

### `gl_pause` — Global Emergency Pause Changed

Emitted when the global emergency pause flag is toggled.

**Topic:** `("gl_pause",)`

**Payload:** `GlobalEmergencyPauseChanged`

| Field | Type | Description |
|---|---|---|
| `paused` | `bool` | New pause state (`true` = paused, `false` = resumed) |

**Emitting entrypoints:** `set_global_emergency_paused`

---

## Event Ordering Guarantees

Within a single transaction, events are emitted in this order where multiple may apply:

1. `withdrew` or `wdraw_to` (transfer confirmation)
2. `completed` (if stream reaches terminal state via withdrawal)
3. `cancelled` (if cancellation triggered refund)
4. `closed` (always last; storage is deleted after this)

The `created` event is always the only event in a stream-creation transaction.
For `create_streams`, one `created` event is emitted per stream entry, in input order.
# Fluxora Streaming Contract — Protocol Semantics

**Contract:** `FluxoraStream` · **Version:** 1 · **Platform:** Soroban (Stellar)

---

## 1. Overview

Fluxora is a token streaming protocol on Stellar/Soroban. A *stream* locks a deposit
of tokens in the contract and releases entitlement to a recipient linearly over time
according to a schedule. Senders fund streams; recipients claim accrued value; the
admin may intervene in emergencies.

---

## 2. Stream Lifecycle

```
          create_stream
               │
               ▼
           [ Active ] ──── pause_stream ────► [ Paused ]
               │                                  │
               │◄─────── resume_stream ───────────┘
               │
               ├──── cancel_stream ──────────────► [ Cancelled ]
               │                                       │
               │                                  withdraw() still works
               │                                  (accrued at cancel time)
               │
               └──── withdraw() drains all ──────► [ Completed ]
                                                       │
                                               close_completed_stream()
                                                       │
                                                  (storage deleted)
```

**Terminal states:** `Cancelled` and `Completed`. Neither can transition to any other state.

---

## 3. Accrual Formula

Accrual is computed as a pure function of on-chain parameters — no mutable accrual
counter is stored. The formula applied in `accrual::calculate_accrued_amount`:

```
if now < cliff_time:
    accrued = 0

elif now >= cliff_time:
    elapsed = min(now, end_time) - start_time
    accrued = min(elapsed × rate_per_second, deposit_amount)
```

Key properties:
- Accrual is **time-based**, not state-based. Pausing a stream does not stop the clock.
- Accrual is **monotonically non-decreasing** (rate-increase and top-up only).
- Accrual is **capped at `deposit_amount`** — it cannot exceed the locked balance.
- Before `cliff_time`: zero accrual, regardless of elapsed time since `start_time`.
- At `cliff_time`: accrual jumps to `(cliff_time - start_time) × rate` in one step.

### 3.1 Accrual by stream status

| Status | `calculate_accrued` returns |
|---|---|
| `Active` | `formula(now)` — grows with ledger time |
| `Paused` | `formula(now)` — clock still runs, same formula |
| `Cancelled` | `formula(cancelled_at)` — frozen at cancellation timestamp |
| `Completed` | `deposit_amount` — deterministic, timestamp-independent |

### 3.2 Withdrawable amount

```
withdrawable = calculate_accrued(stream_id) - stream.withdrawn_amount
```

A call to `withdraw()` transfers exactly `withdrawable` if > 0, or returns 0
with no side effects (idempotent).

---

## 4. Stream Creation

### Required invariants at creation time

| Invariant | Error if violated |
|---|---|
| `deposit_amount > 0` | `InvalidParams` |
| `rate_per_second > 0` | `InvalidParams` |
| `sender ≠ recipient` | `InvalidParams` |
| `start_time < end_time` | `InvalidParams` |
| `start_time >= ledger.timestamp()` | `StartTimeInPast` |
| `cliff_time ∈ [start_time, end_time]` | `InvalidParams` |
| `deposit_amount >= rate × (end_time - start_time)` | `InsufficientDeposit` |

Setting `cliff_time = start_time` means no cliff — withdrawal is available immediately
after `start_time`. Deposits may exceed the minimum required; the excess remains locked
and is effectively donated to the recipient's balance.

### Atomicity

The token pull from sender (`pull_token`) happens **before** stream state is persisted.
If the pull fails (insufficient balance or allowance), no stream is created, no ID is
consumed, and no event is emitted.

For `create_streams`, a single bulk pull is done after validating all entries. If pull
fails, none of the streams in the batch are created.

---

## 5. Cancellation Semantics

When a stream is cancelled (by sender or admin):

1. `accrued_at_cancel = formula(ledger.timestamp())` is computed.
2. `refund = deposit_amount - accrued_at_cancel` is transferred to sender.
3. Stream state is persisted as `Cancelled` with `cancelled_at = now` **before** the
   token push (CEI compliance).
4. The accrued portion is **not** auto-transferred to recipient. It remains locked and
   must be claimed via `withdraw()`.

**Edge cases:**

| Scenario | Sender refund | Recipient can withdraw |
|---|---|---|
| Cancel before cliff | 100% of deposit | 0 (no accrual before cliff) |
| Cancel at 30% elapsed | 70% | 30% |
| Cancel at 100% elapsed (after end_time) | 0% | 100% |
| Cancel on paused stream | same formula | same formula |

After cancellation, `calculate_accrued` returns the frozen value at `cancelled_at`.
Subsequent calls to `withdraw()` drain that amount; the stream status stays `Cancelled`
(it does **not** transition to `Completed`).

---

## 6. Schedule Modifications

### Rate increase (`update_rate_per_second`)

- New rate must be **strictly greater** than current rate (forward-only).
- Existing `deposit_amount` must cover `new_rate × (end_time - start_time)`.
- Historical accrual up to `effective_time` is unchanged.
- From `effective_time` onward, accrual uses the new rate.

### Shorten end time (`shorten_stream_end_time`)

- `new_end_time` must be in the future and less than the current `end_time`.
- New `deposit_amount` is set to `rate × (new_end_time - start_time)`.
- Excess deposit `(old_deposit - new_deposit)` is immediately refunded to sender.
- Already-accrued amount is unaffected.

### Extend end time (`extend_stream_end_time`)

- `new_end_time` must be greater than current `end_time`.
- Existing `deposit_amount` must cover `rate × (new_end_time - start_time)`.
- No token movement — existing deposit absorbs the extension.

### Top-up (`top_up_stream`)

- Any address may fund a stream top-up (not restricted to sender).
- Increases `deposit_amount` only; rate and schedule are unchanged.
- `deposit_amount` update and `save_stream` happen **before** the token pull (CEI).

---

## 7. Withdrawal Modes

### `withdraw(stream_id)`

Transfers all currently withdrawable tokens to `stream.recipient`. Blocked if status
is `Paused` or `Completed`. Returns 0 silently before cliff or when nothing new has
accrued.

### `withdraw_to(stream_id, destination)`

Same accounting as `withdraw`, but sends tokens to `destination`. Authorization is
still by `stream.recipient`. `destination` must not equal the contract address.

### `batch_withdraw(recipient, stream_ids)`

Processes multiple streams in one call. Requires `recipient` authorization once.
All `stream_ids` must be unique (O(n²) duplicate check). `Completed` streams in the
batch are silently skipped (amount 0). `Paused` streams return `InvalidState` and
abort the whole batch.

---

## 8. Storage and TTL

| Data | Storage type | TTL bump trigger |
|---|---|---|
| `Config`, `NextStreamId`, pause flags | Instance | Every entrypoint via `bump_instance_ttl` |
| `Stream(id)` | Persistent | On every `load_stream` (read) and `save_stream` (write) |
| `RecipientStreams(address)` | Persistent | On every read/write of the index |

TTL thresholds: minimum 17,280 ledgers (~1 day), extended to 120,960 ledgers (~7 days).
Streams that go unqueried for more than 7 days may expire from persistent storage.
Frontends and keepers should query active streams periodically to keep TTLs alive.

---

## 9. Error Reference

| Code | Name | Typical cause |
|---|---|---|
| 1 | `StreamNotFound` | `stream_id` does not exist or has been closed |
| 2 | `InvalidState` | Operation not valid in current stream status |
| 3 | `InvalidParams` | Parameter validation failed |
| 4 | `ContractPaused` | `GlobalPaused` flag is true; creation blocked |
| 5 | `StartTimeInPast` | `start_time < ledger.timestamp()` at creation |
| 6 | `ArithmeticOverflow` | `checked_mul`/`checked_add` overflow in math |
| 7 | `Unauthorized` | Caller does not match required role |
| 8 | `AlreadyInitialised` | `init` called on already-initialized contract |
| 9 | `InsufficientBalance` | Reserved; token client panics on underfund |
| 10 | `InsufficientDeposit` | Deposit too small to cover `rate × duration` |