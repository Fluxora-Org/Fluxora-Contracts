# Fluxora Stream Contract Documentation

Onboarding and integration reference for developers and auditors. Describes stream lifecycle, accrual formula, cliff/end_time behavior, access control, events, and error codes.

**Source of truth:** `contracts/stream/src/lib.rs`, `contracts/stream/src/accrual.rs`

**Alignment verification:** See [protocol-narrative-code-alignment.md](./protocol-narrative-code-alignment.md) for complete mapping between this documentation and implementation.

## Sync Checklist

When changing the contract:

- Update this doc if you change lifecycle, access control, events, or error semantics
- Update `protocol-narrative-code-alignment.md` to reflect changes
- Run `cargo test -p fluxora_stream` before committing
- Update snapshot tests if externally visible behavior changes
- No behavior change required for doc-only updates

**Entrypoint index (validator):** `accept_recipient_update`, `batch_withdraw_to`, `bulk_cancel_streams`, `cancel_recipient_update`, `close_cancelled_stream`, `close_completed_stream`, `compute_keeper_fee_split`, `delete_stream_template`, `get_global_emergency_paused`, `get_paused_stream_count`, `get_pending_recipient_update`, `get_protocol_fees_accrued`, `get_recipient_stream_count`, `get_stream_health`, `get_stream_memo`, `get_stream_template`, `global_resume`, `keeper_cancel`, `set_contract_paused`, `set_global_emergency_paused`, `version`, `migration_v5_to_v6`, `set_max_rate_per_second`.
**Entrypoint index (validator):** `accept_recipient_update`, `batch_withdraw_to`, `bulk_cancel_streams`, `cancel_recipient_update`, `delete_stream_template`, `get_global_emergency_paused`, `get_keeper_fee_split`, `get_pending_recipient_update`, `get_recipient_stream_count`, `get_stream_health`, `get_stream_memo`, `get_stream_template`, `global_resume`, `keeper_cancel`, `set_contract_paused`, `set_global_emergency_paused`, `version`, `migration_v5_to_v6`, `set_max_rate_per_second`.

## Externally Visible Assurances

This document provides crisp success and failure semantics for all protocol operations. Treasury operators, recipient applications, and auditors can reason about contract behavior using only:

1. **On-chain observables**: Persistent storage fields, emitted events, token transfers
2. **Published documentation**: This file and referenced specifications
3. **Error classifications**: Structured `ContractError` variants

No hidden rules or implementation details are required to understand protocol behavior.

### Per-stream metadata (TLV extension, CONTRACT_VERSION 4)

From **CONTRACT_VERSION 4**, every stream may carry an optional bounded key-value map
(`metadata: Option<Map<Bytes, Bytes>>`) for rich integration data such as invoice IDs,
project codes, and external reference URIs.

#### API

| Entrypoint | Description |
|---|---|
| `create_stream(…, metadata)` | Pass `Some(map)` to attach metadata at creation, or `None` to omit. |
| `get_stream_metadata(stream_id)` | Returns `Option<Map<Bytes, Bytes>>`. Permissionless read. |

Metadata is also propagated through `create_streams`, `create_streams_relative`,
`create_streams_partial`, and `create_stream_from_template` via the `metadata` field on
`CreateStreamParams` / `CreateStreamRelativeParams`.

#### Bounds (enforced at `create_stream` time — `ContractError::MetadataTooLarge` on violation)

| Bound | Constant | Value |
|---|---|---|
| Maximum key-value pair count | `MAX_METADATA_KEYS` | 8 |
| Maximum aggregate (all keys + values) bytes | `MAX_METADATA_BYTES` | 512 |
| Maximum single key length | `MAX_METADATA_KEY_BYTES` | 32 |
| Maximum single value length | `MAX_METADATA_VALUE_BYTES` | 128 |

#### Invariants

- Validated entirely at creation time; **immutable post-creation**.
- Stream ID is **not allocated** if metadata validation fails (no partial state written).
- No token movement occurs if metadata validation fails.
- `StreamCreated` event includes the `metadata` field for indexer consumption.

#### Example (Rust client)

```rust
let mut meta = Map::new(&env);
meta.set(Bytes::from_slice(&env, b"invoice_id"), Bytes::from_slice(&env, b"INV-2026-001"));
meta.set(Bytes::from_slice(&env, b"project"),    Bytes::from_slice(&env, b"PROJ-42"));

let stream_id = client.create_stream(
    &sender, &recipient,
    &deposit, &rate,
    &start, &cliff, &end,
    &0_i128, // dust threshold
    &None,   // memo
    &Some(meta),
);

// Later — permissionless read
let stored_meta = client.get_stream_metadata(&stream_id);
```

### Schedule templates (presets)

From **CONTRACT_VERSION 3**, integrators can register **relative** schedule skeletons (`register_stream_template`) and create streams from them (`create_stream_from_template`). This standardizes recurring payroll windows and trims repeated calldata versus always passing `start_delay` / `cliff_delay` / `duration` through the client for identical shapes.

- **Auth**: registering and deleting templates requires the template `owner` signer. Creating a stream from a template requires the **funding `sender`** to authorize (same as `create_stream_relative`).
- **Caps**: per-owner and global template counts are bounded; see `MAX_TEMPLATES_PER_OWNER` and `MAX_GLOBAL_TEMPLATES` in `contracts/stream/src/lib.rs`.
- **Errors**: `TemplateNotFound`, `TemplateLimitExceeded`, `TemplateUnauthorized`.

### Stream Kinds (Linear vs. Cliff-Only)

From **CONTRACT_VERSION 4**, the contract supports distinct streaming styles, governed by the `StreamKind` field on the stream configuration:

- **Linear** (Default/Legacy): Accrues tokens continuously and linearly over time at `rate_per_second` once the stream has started, subject to a standard cliff window (during which nothing can be withdrawn).
- **[CliffOnly](#cliff-only-streams)**: A one-shot, instant unlock stream variant. Tokens do not accrue continuously over time. Instead:
  - Before the `cliff_time`, `0` tokens are accrued/withdrawable (all funds are locked).
  - At or after the `cliff_time`, the total `deposit_amount` is immediately and fully unlocked and made claimable by the recipient.
  - To enforce the single-unlock model, `rate_per_second` is forced to `0` during creation and all subsequent mutation/adjustment requests are rejected.

### ID pre-allocation (`reserve_stream_ids`) — issue #584

Off-chain orchestrators and indexers that build payment batches often need to know stream IDs **before** submitting `create_stream` transactions, to pre-populate database records or cross-reference external invoice systems.

`reserve_stream_ids(caller, count)` atomically advances the global ID counter by `count` and returns the reserved range as a `Vec<u64>`.  Subsequent `create_stream` calls by the same `caller` consume IDs from the reservation in order; once exhausted (or if no reservation exists) the live counter is used.

| Constant | Value | Purpose |
|---|---|---|
| `MAX_ID_RESERVATION` | 100 | Cap per call — prevents counter-inflation attacks |
| `RESERVATION_TTL_LEDGERS` | 17 280 (~1 day) | Reservation expiry — abandoned ranges do not block the counter forever |

**Security notes:**
- `count = 0` -> `ReservationCountZero` (17).
- `count > 100` -> `ReservationLimitExceeded` (18).
- A new reservation for the same caller **overwrites** the previous one; the old IDs remain as a gap in the counter (same as any abandoned reservation).
- The TTL ensures persistent storage entries are cleaned up automatically.

**Usage pattern:**
```
1. Call reserve_stream_ids(caller, N)  → get [id_0, id_1, …, id_{N-1}]
2. Pre-populate off-chain DB with those IDs
3. Submit N create_stream transactions — each consumes the next reserved ID in order
```

---

## 1. Stream Lifecycle

### Phases

| Phase            | Action                                        | Notes                                                                 |
| ---------------- | --------------------------------------------- | --------------------------------------------------------------------- |
| **Creation**     | `create_stream` / `create_streams_partial` | Sender deposits tokens; stream starts as `Active`                     |
| **Clone**        | `clone_stream`                                | Copies rate, cliff offset, threshold, and memo from a source stream; accepts new recipient and timing |
| **Top-up**       | `top_up_stream`                               | Extra deposit locked (sender or admin only); schedule unchanged       |
| **Pause**        | `pause_stream` / `pause_stream_as_admin`      | Stops withdrawals; accrual continues by time                          |
| **Resume**       | `resume_stream` / `resume_stream_as_admin`    | Restores withdrawals; blocked if past `end_time` (Terminal)           |
| **Cancellation** | `cancel_stream` / `cancel_stream_as_admin` / `bulk_cancel_streams` | Refunds unstreamed amount; frozen accrued stays for recipient         |
| **Withdrawal**   | `withdraw` / `withdraw_to` / `batch_withdraw` | Recipient pulls accrued tokens; allowed on Paused if past `end_time`  |
| **Completion**   | Automatic                                     | When `withdrawn_amount == deposit_amount`, status becomes `Completed` |
| **Rotation**     | `update_recipient` / `accept_recipient_update` / `cancel_recipient_update` | Sender proposes a new recipient; the current recipient must accept. Pending rotations are queryable via `get_pending_recipient_update`. Acceptance updates both the stream record and recipient indexes atomically. |
| **Auto-claim**   | `set_auto_claim` / `revoke_auto_claim` / `trigger_auto_claim` | Recipient opts in to permissionless final claim at `end_time` to a chosen destination |

### State Transitions

- **Active** ↔ **Paused** (via pause/resume)
- **Active** or **Paused** → **Cancelled** (terminal)
- **Active** or **Paused** → **Completed** (when recipient withdraws full deposit; terminal)

Terminal states: `Completed`, `Cancelled`. Both may be closed via `close_completed_stream` to reclaim storage and index space. A stream is also considered technically terminal if `ledger.timestamp() >= end_time`.
In this "time-terminal" state, pause/resume is blocked, but withdrawal is always allowed regardless of previous pause status.

**Cancelled stream closure rule**: A `Cancelled` stream may only be closed after the recipient has fully withdrawn the frozen accrued amount. Attempting to close a `Cancelled` stream with remaining claimable balance returns `ContractError::InvalidState`. This prevents storage cleanup from destroying recipient funds.

### Cancellation Semantics (Issue Scope)

This section is the protocol-level contract for `cancel_stream` and `cancel_stream_as_admin`.

Success semantics (observable):

1. Preconditions: stream status is `Active` or `Paused`.
2. `cancelled_at` is set to current ledger timestamp.
3. Accrued amount is frozen at `cancelled_at` (no post-cancel time growth).
4. Refund is `deposit_amount - accrued_at_cancelled_at`.
5. Stream transitions to terminal `Cancelled` state.
6. `StreamCancelled` event is emitted with topic `("cancelled", stream_id)`.

Failure semantics (observable):

1. Missing stream: `ContractError::StreamNotFound`.
2. Non-cancellable status (`Completed` or already `Cancelled`): `ContractError::InvalidState`.
3. Modification in terminal state (past `end_time` for pause/resume): `ContractError::StreamTerminalState`.
4. Unauthorized caller on sender path: `ContractError::Unauthorized`.
5. Unauthorized caller on admin path: `ContractError::Unauthorized`.
6. Redundant state change (pause already paused): `ContractError::StreamAlreadyPaused`.
7. Redundant state change (resume already active): `ContractError::StreamNotPaused`.
8. Any failure is atomic: no refund transfer, no state mutation, no cancel event.

Role boundaries:

1. `cancel_stream`: only the stream `sender` can authorize.
2. `cancel_stream_as_admin`: only contract `admin` can authorize.
3. Recipient and third parties cannot cancel through either path unless they hold required credentials.

Invariants after successful cancellation:

1. `status == Cancelled` and `cancelled_at.is_some()`.
2. `calculate_accrued(stream_id)` always returns accrued at `cancelled_at`.
3. `refund + frozen_accrued == deposit_amount`.
4. Recipient may withdraw only frozen accrued remainder (`frozen_accrued - withdrawn_amount`).

Scope boundary and exclusions:

1. In scope: refund math, `cancelled_at` persistence/freeze semantics, cancel auth paths, cancel event consistency.
2. Out of scope: token-level trust assumptions beyond documented model, off-chain indexer liveness, and economic policy choices (for example who should bear operational costs).
3. Residual risk: if a non-standard token violates SEP-41 expectations, transfer behavior may diverge; CEI ordering reduces but cannot fully eliminate external token risk.

### Keeper Cancellation & Fee Accounting

The `keeper_cancel` entrypoint allows any third-party keeper to cancel an expired, unwithdrawn stream after the grace period has elapsed.

**Fee Accounting Note**:
- The keeper fee (50 BPS) is deducted solely from the *unstreamed* refund bound for the sender.
- The contract does **not** retain a protocol split of this fee. The entire fee is transferred directly to the keeper.
- The view function `get_protocol_fees_accrued` (added in #623) tracks the cumulative total of keeper fees *paid out* of the contract, rather than an internal sweepable balance.
- **Accounting Invariant**: The contract's token balance must securely cover all remaining liabilities. Since the keeper fee is transferred entirely to the keeper and leaves the contract, the tracked total in `get_protocol_fees_accrued` is strictly monotone and safely independent of the contract's real-time asset/liability ratio.

### Clone Semantics

This section defines the success and failure behavior of `clone_stream`.

Success semantics (observable):

1. Preconditions: Source stream must be in `Active` or `Paused` status.
2. The contract creates a new stream inheriting the rate, cliff offset, dust threshold, and memo from the source stream.
3. The new stream is initialized in the `Active` status.
4. Tokens are pulled from the source stream's sender for the new deposit.

Failure semantics (observable):

1. Terminal source state: If the source stream status is `Completed` or `Cancelled`, the operation is rejected with `ContractError::StreamTerminalState`.
2. Unauthorized: If the caller is not the sender of the source stream, the operation is rejected.

### Global Pause Semantics (Issue Scope)

This section is the protocol-level contract for the global pause state managed via `pause_protocol` and `resume_protocol`.

**Entrypoints:**

| Function | Description |
|----------|-------------|
| `pause_protocol(admin, reason)` | Globally pause new stream creation with audit trail (reason, timestamp, admin) |
| `resume_protocol(admin)` | Globally resume new stream creation, clearing audit trail |
| `is_paused()` | Query if protocol is currently paused (permissionless) |
| `get_pause_info()` | Query detailed pause info including audit trail (permissionless) |
| `set_max_rate_per_second(max_rate)` | Admin-only governance entrypoint that sets the maximum allowed stream rate for future rate updates |
| `migration_v5_to_v6(admin)` | Admin-only deployment checkpoint that emits a `migrated` audit event; no storage transformation is required |

**Pause reason length:** The `reason` string passed to `pause_protocol` is bounded by `MAX_PAUSE_REASON_BYTES = 256`. Strings longer than 256 bytes are rejected with `ContractError::InvalidParams`. This prevents unbounded ledger-entry growth (Issue #513).

Success semantics (observable):

1. Preconditions: Caller must be the authorized contract `admin`.
2. Storage: The `CreationPaused` data key is set to `true` or `false` in instance storage.
3. Event: `ContractPaused(bool)` is emitted with topic `("paused_ctl",)`.
4. Effect on creation: When paused, `create_stream` and `create_streams` return `ContractError::ContractPaused` and all new stream creation is blocked.
5. Effect on existing streams: Active streams are intentionally unaffected. Withdrawals, top-ups, pause/resume/cancel operations on individual streams continue to function normally.

Failure semantics (observable):

1. Unauthorized caller on admin path: `ContractError::Unauthorized`.
2. Any failure is atomic: no storage mutation, no event emitted.

Role boundaries:

1. `pause_protocol` / `resume_protocol`: only the contract `admin` can authorize.
2. Senders and recipients cannot pause the global contract. Senders manage individual streams via `pause_stream`.

Invariants when globally paused:

1. No new streams can be persisted (no `created` events, no deposit tokens pulled).
2. Existing streams do not change status due to a global pause.
3. Audit trail (reason, timestamp, admin) is queryable via `get_pause_info()`.

Scope boundary: The global pause is strictly an administrative circuit breaker for new liabilities. It does not freeze funds of existing users or prevent recipients from withdrawing their vested entitlement.

**Note on Stream Creation:**
Stream creation is blocked while the protocol is globally paused. The `create_stream` function returns `ContractError::ContractPaused` if `is_paused()` is true. This applies to both single-stream and batch (`create_streams`) creation.

```mermaid
stateDiagram-v2
    direction LR
    [*] --> Active : create_stream
    Active --> Paused : pause_stream
    Paused --> Active : resume_stream
    Active --> Cancelled : cancel_stream
    Paused --> Cancelled : cancel_stream
    Active --> Completed : withdraw full amount
    Cancelled --> [*]
    Completed --> [*]
```

### Sequence Diagram

The following diagram shows the full create → withdraw flow, including optional pause/resume and cancel paths.

```mermaid
sequenceDiagram
    participant Sender
    participant Contract as FluxoraStream
    participant Token as USDC Token
    participant Recipient

    Note over Sender, Recipient: 1. Stream Creation

    Sender ->> Contract: create_stream(sender, recipient, deposit_amount, rate_per_second, start_time, cliff_time, end_time)
    Contract ->> Contract: require_auth(sender)<br/>validate params
    Contract ->> Token: transfer(sender → contract, deposit_amount)
    Token -->> Contract: OK
    Contract -->> Sender: stream_id
    Note right of Contract: Event: ("created", stream_id) → StreamCreated

    Note over Sender, Recipient: 2. Cliff Period (no withdrawals)

    Recipient ->> Contract: withdraw(stream_id)
    Contract -->> Recipient: 0
    Note right of Contract: No state change, no transfer, no withdraw/completed events

    Note over Sender, Recipient: 3. After Cliff — Partial Withdrawal

    Recipient ->> Contract: withdraw(stream_id)
    Contract ->> Contract: require_auth(recipient)<br/>calculate_accrued() − withdrawn_amount
    Contract ->> Token: transfer(contract → recipient, withdrawable)
    Token -->> Contract: OK
    Contract -->> Recipient: withdrawable
    Note right of Contract: Event: ("withdrew", stream_id) → Withdrawal { stream_id, recipient, amount }

    Note over Sender, Recipient: 4. Optional — Pause / Resume

    Sender ->> Contract: pause_stream(stream_id)
    Contract ->> Contract: require_auth(sender)<br/>status = Paused
    Contract -->> Sender: OK
    Note right of Contract: Event: ("paused", stream_id)

    Recipient ->> Contract: withdraw(stream_id)
    Contract --x Recipient: Error: InvalidState (if before end_time)

    Note over Sender, Recipient: 4b. Terminal Liquidity (Paused past end_time)
    Note right of Contract: Time >= end_time
    Recipient ->> Contract: withdraw(stream_id)
    Contract ->> Contract: status = Completed
    Contract ->> Token: transfer(contract → recipient, total)
    Contract -->> Recipient: OK
    Note right of Contract: Event: ("completed", stream_id)

    Sender ->> Contract: resume_stream(stream_id)
    Contract ->> Contract: require_auth(sender)<br/>status = Active
    Contract -->> Sender: OK
    Note right of Contract: Event: ("resumed", stream_id)

    Note over Sender, Recipient: 5a. Happy Path — Complete Withdrawal

    Recipient ->> Contract: withdraw(stream_id)
    Contract ->> Contract: require_auth(recipient)<br/>withdrawable = deposit_amount − withdrawn_amount
    Contract ->> Token: transfer(contract → recipient, withdrawable)
    Token -->> Contract: OK
    Contract ->> Contract: status = Completed
    Contract -->> Recipient: withdrawable
    Note right of Contract: Event: ("withdrew", stream_id) → Withdrawal { stream_id, recipient, amount }
    Note right of Contract: Event: ("completed", stream_id)

    Note over Sender, Recipient: 5b. Alternative — Cancellation

    Sender ->> Contract: cancel_stream(stream_id)
    Contract ->> Contract: require_auth(sender)<br/>calculate unstreamed refund
    Contract ->> Contract: status = Cancelled
    Contract ->> Token: transfer(contract → sender, unstreamed)
    Token -->> Contract: OK
    Contract -->> Sender: OK
    Note right of Contract: Event: ("cancelled", stream_id)
    Note over Recipient: Recipient can still withdraw<br/>accrued amount before cancellation
```

---

## 2. Accrual Formula

**Location:** `contracts/stream/src/accrual.rs`

The mathematical accrual behavior branches on the stream's `StreamKind`:

### Linear Streams
```text
if current_time < cliff_time           → return 0
if checkpointed_at >= end_time or rate < 0 → return checkpointed_amount or 0

elapsed_now = min(current_time, end_time)
elapsed_seconds = elapsed_now - checkpointed_at   // 0 if underflow
added = elapsed_seconds * rate_per_second         // on overflow → deposit_amount
return min(checkpointed_amount + added, deposit_amount).max(0)
```

### Cliff-Only Streams
```text
if current_time < cliff_time  → return 0
else                         → return deposit_amount
```

### Rules

- **Before cliff:** Returns 0 (no withdrawals allowed)
- **After cliff:** Accrual computed from `start_time`, not from cliff
- **No cliff:** Set `cliff_time = start_time` for immediate vesting
- **After end_time:** Elapsed time is capped at `end_time` (no post-end accrual)
- **Overflow:** Multiplication overflow yields `deposit_amount` (safe upper bound)
- **Active streams:** Accrual computed using current ledger timestamp
- **Paused streams:** Accrual computed using current ledger timestamp (same as Active; pause only blocks withdrawals, not accrual)
- **Completed:** `calculate_accrued` returns `deposit_amount` (deterministic final value, timestamp-independent)
- **Cancelled:** `calculate_accrued` is frozen at `cancelled_at` (no post-cancel growth)

### Ledger-Time Monotonicity Guard

Ledger-backed accrual paths cache the last observed accrual timestamp in instance storage and compare each later ledger timestamp against it before evaluating withdrawable math. The guard is intentionally global and short-lived: it catches non-monotonic test harness setup, migration mistakes, or future environment changes without adding per-stream storage.

`accrual.rs` also contains a `debug_assert!(current_ts >= prev_ts, "retrograde ledger timestamp")`. In test/debug builds, the same condition returns `ContractError::ClockRegression` instead of allowing a retrograde timestamp to reduce computed accrual. Production Stellar ledgers are still assumed to be monotonically non-decreasing by protocol.

`get_claimable_at(stream_id, timestamp)` is exempt because the timestamp is caller-supplied simulation input rather than `ledger().timestamp()`.

### Status-Specific Behavior Matrix

| Status    | Time Source            | Expected Behavior                      |
| --------- | ---------------------- | -------------------------------------- |
| Active    | env.ledger().timestamp | Accrual grows with wall-clock time     |
| Paused    | env.ledger().timestamp | Same as Active (accrual continues)     |
| Completed | N/A (ignored)          | Returns deposit_amount (deterministic) |
| Cancelled | cancelled_at           | Frozen at cancellation time            |

### Withdrawable Amount

```text
withdrawable = accrued - withdrawn_amount
```

### Withdrawal Dust Threshold (#423)

From **CONTRACT_VERSION 5**, senders can optionally set a `withdraw_dust_threshold` per stream to reduce fee and event spam from tiny micro-withdrawals.

- **Enforcement**: If `withdrawable < withdraw_dust_threshold`, the withdrawal returns `0` (no transfer, no event).
- **Exceptions (Threshold Ignored)**:
    - **Terminal State**: Once the stream reaches `end_time` or is `Cancelled`, the threshold is ignored to ensure the recipient can pull all remaining funds.
    - **Final Drain**: If the withdrawal would result in `withdrawn_amount == deposit_amount` (completing the stream), it is allowed even if the amount is below the threshold.
- **Default**: The threshold defaults to `0` if not specified at creation.

> **See also:** [dust-threshold.md](./dust-threshold.md) — formula for choosing a safe threshold value, worked USDC examples, a validation table, and guidance for template authors.

### Withdrawal Frequency Limit (#574)

From **CONTRACT_VERSION 6**, all withdrawal operations enforce a minimum ledger interval between consecutive withdrawals on the same stream to prevent excessive ledger entry generation and I/O costs from high-frequency polling.

- **Constant**: `MIN_WITHDRAW_INTERVAL_LEDGERS = 17` (approximately 1 minute at ~5 seconds per ledger close, subject to network conditions)
- **Enforcement**: `withdraw`, `delegated_withdraw`, and `batch_withdraw` all enforce `current_ledger - last_withdraw_ledger >= MIN_WITHDRAW_INTERVAL_LEDGERS`
- **Error**: Returns `ContractError::WithdrawalTooFrequent` (error code 17) if the interval check fails
- **Atomicity**: For `batch_withdraw`, if any stream in the batch violates the rate limit, the entire batch reverts
- **Per-Stream**: Each stream tracks its own `last_withdraw_ledger` independently
- **First Withdrawal**: Always succeeds (`last_withdraw_ledger` is initialized to 0 at stream creation)
- **State Update**: `last_withdraw_ledger` is updated to `env.ledger().sequence()` only after a successful withdrawal (withdrawable > 0)
- **Zero Withdrawable**: If a withdrawal returns 0 (before cliff, dust threshold, etc.), `last_withdraw_ledger` is not updated

**Invariant**: `current_ledger >= last_withdraw_ledger` at all times (guaranteed by monotonic ledger progression).

**Example**: If a withdrawal succeeds at ledger 100, the next withdrawal can occur at ledger 117 or later (100 + 17 = 117).

### Frontend: get_claimable_at (simulation)

`get_claimable_at(stream_id, timestamp)` is a read-only view that returns the amount that would be claimable (withdrawable) at an arbitrary timestamp. Use it for:

- **Planning:** "How much will be claimable at time T?" without sending a transaction.
- **Simulation:** Pass a future timestamp to show projected claimable amount.
- **Consistency:** For the current ledger time, result matches `get_withdrawable(stream_id)`.

Behaviour: Active/Paused streams use the given `timestamp` (clamped to schedule); Cancelled streams use `min(timestamp, cancelled_at)` so accrual is frozen at cancellation. Completed streams return 0.

### Frontend: get_stream_health (view summary)

`get_stream_health(stream_id)` returns a structured health summary for a stream.

- **is_underfunded**: `true` if the current `deposit_amount` is insufficient to cover the total tokens that will accrue by `end_time` at the current `rate_per_second`.
- **is_expired**: `true` if `ledger.timestamp() >= end_time` and the stream is not yet `Completed` or `Cancelled`.
- **accrued_to_date**: Real-time total tokens accrued since `start_time`.
- **remaining_deposit**: `deposit_amount - withdrawn_amount`. For cancelled streams, this reflects the unwithdrawn portion of the original deposit, even though the unstreamed portion has been refunded.
- **seconds_until_depletion**: Estimated seconds until the stream's deposit is fully exhausted by accrual. Capped at `end_time`. For cancelled streams, this continues to reflect the hypothetical depletion time based on the original rate.

Use this to show real-time health indicators in UIs, alert senders of underfunding, or notify recipients of expired streams ready for final withdrawal.

---

## 3. Cliff and end_time Behavior

### Cliff

- Must be in `[start_time, end_time]` (enforced at creation)
- Before `cliff_time`: accrued = 0, no withdrawals
- At or after `cliff_time`: accrual uses elapsed time from `start_time`, not cliff

### end_time

- Must satisfy `start_time < end_time`
- Accrual uses `min(current_time, end_time)` as the upper bound
- After `end_time`, accrued stays at `min((end_time - start_time) * rate_per_second, deposit_amount)`
- No extra accrual beyond `end_time`

### Deposit Validation

At creation:

```text
deposit_amount >= rate_per_second * (end_time - start_time)
```

The same sufficiency check is enforced when extending a stream's `end_time`:

```text
deposit_amount >= rate_per_second * (new_end_time - start_time)
```

If the existing deposit does not cover the extended duration, `extend_stream_end_time` returns `ContractError::InsufficientDeposit` and no state changes occur. Use `top_up_stream` first to increase the deposit, then extend.

### Mutation Restrictions on Cliff-Only Streams

To preserve the absolute one-shot unlock nature of a `[CliffOnly](#cliff-only-streams)` stream variant and guarantee its immutability post-creation, **all mutating endpoints are strictly blocked**. Attempting to call any of the following functions on a `[CliffOnly](#cliff-only-streams)` stream will return `[ContractError::UnsupportedStreamKind](./error.md#unsupportedstreamkind-17)` and revert all state changes:

- `top_up_stream`
- `update_rate_per_second`
- `decrease_rate_per_second`
- `shorten_stream_end_time`
- `extend_stream_end_time`

Any such attempt is atomic: no balances are transferred, no state is updated, and no events are emitted.

### Shorten `end_time` Semantics

`shorten_stream_end_time(stream_id, new_end_time)` is sender-only and only valid for `Active`/`Paused` streams.

Validation boundaries:
- `new_end_time > now`
- `new_end_time > start_time`
- `new_end_time >= cliff_time`
- `new_end_time < old_end_time`

On success:
- `new_deposit_amount = rate_per_second * (new_end_time - start_time)`
- `refund_amount = old_deposit_amount - new_deposit_amount`
- Contract persists `end_time` and `deposit_amount`, then transfers `refund_amount` to sender, then emits `end_shrt`.

On failure (`InvalidParams` or `InvalidState`):
- No state change
- No token transfer
- No `end_shrt` event

### Start Time Boundary (Creation)

- `start_time` **must be >= current ledger timestamp** at creation time.
- `start_time == now` is valid ("start now").
- `start_time < now` is rejected with `ContractError::StartTimeInPast`.
- Failure is atomic: no stream is persisted, no tokens move, and no `created` event is emitted.

**Limits Policy (Defense in Depth):**

- No arbitrary hard-coded caps (e.g. "max 1M tokens").
- The technical upper bound is `i128::MAX` or the underlying token's total supply.
- Rationale: Accrual math (in `accrual.rs`) is already overflow-safe via `checked_mul` and clamping.
- Application-specific limits should be handled in the frontend or factory contracts.

### Batch Creation: Atomic vs Partial

The contract provides two entrypoints for creating multiple streams in a single transaction. Both accept a vector of `CreateStreamParams` and require a single authorization from the `sender`. For both functions, providing an empty vector safely returns an empty result (`Ok(Vec::new())`) with no side effects and no token transfers.

#### `create_streams` (Atomic)

```rust
pub fn create_streams(
    env: Env,
    sender: Address,
    streams: Vec<CreateStreamParams>,
) -> Result<Vec<u64>, ContractError>
```

**Semantics:** All-or-nothing.
- The contract first validates all entries.
- If any single entry fails validation (e.g., `StartTimeInPast`, `InvalidParams`), the entire transaction reverts.
- A single bulk token transfer is made for the sum of all `deposit_amount`s. If the sender lacks sufficient balance for the aggregate total, the transaction reverts.
- Returns a `Vec<u64>` containing the new stream IDs in the exact order of the input.

#### `create_streams_partial` (Non-Atomic)

```rust
pub fn create_streams_partial(
    env: Env,
    sender: Address,
    streams: Vec<CreateStreamParams>,
) -> Result<Vec<CreateStreamResult>, ContractError>

pub struct CreateStreamResult {
    pub success: bool,
    pub stream_id: Option<u64>,
    pub error: Option<u32>,
}
```

**Semantics:** Failure isolation per entry.
- The contract attempts to create each stream independently.
- **Token Transfer Handling:** Tokens are pulled from the sender *per entry*. If an entry fails validation, it is skipped entirely (no tokens are pulled). If the per-entry token transfer fails, it is recorded as `InsufficientBalance` (error code 9).
- Subsequent entries continue processing normally regardless of prior failures.
- **Return Value:** Callers receive a `Vec<CreateStreamResult>` matching the input order. To learn which elements succeeded, callers iterate the result vector and check `result.success`. Successful entries include `Some(stream_id)`, while failed entries include `Some(error_code)`.

**Example:**
```rust
let results = contract.create_streams_partial(&sender, &params)?;
for (i, res) in results.iter().enumerate() {
    if res.success {
        println!("Stream {} created with ID {}", i, res.stream_id.unwrap());
    } else {
        println!("Stream {} failed with error code {}", i, res.error.unwrap());
    }
}
```

### Relative-Time Helpers: `create_stream_relative` and `create_streams_relative`

The contract provides convenience entry points that compute stream times relative to the current ledger timestamp, eliminating off-chain calculation errors that lead to `StartTimeInPast` failures.

#### Motivation

Off-chain applications often construct stream parameters ahead of time, e.g., "start 1 day from now". If there is clock drift between the application server and the Soroban ledger, the calculated `start_time` may already be in the past when the transaction is executed, causing `StartTimeInPast` rejection.

Relative-time helpers avoid this by deferring timestamp computation to the contract itself, which always has the authoritative ledger timestamp.

#### `create_stream_relative`

**Signature:**
```rust
pub fn create_stream_relative(
    env: Env,
    sender: Address,
    recipient: Address,
    deposit_amount: i128,
    rate_per_second: i128,
    start_delay: u64,     // Seconds to add to current timestamp
    cliff_delay: u64,     // Seconds to add to current timestamp
    duration: u64,        // Total seconds from start_time to end_time
) -> Result<u64, ContractError>
```


**Computation:**
```
current_time = env.ledger().timestamp()
start_time   = current_time + start_delay
cliff_time   = current_time + cliff_delay
end_time     = start_time + duration
```

**Validation:**
- Checks for overflow/underflow in all additions
- Delegates to `create_stream` with computed absolute times
- Inherits all validation rules: deposit sufficiency, cliff bounds, etc.
- **Never produces `StartTimeInPast`** error (computed times are always >= current_time)

**Example:**
```
// Create a stream starting in 1 day, cliff in 3 days, running for 30 days
contract.create_stream_relative(
    &sender,
    &recipient,
    &100_000_000,           // 100M tokens
    &1_157_407,             // ~1% per day
    &86400,                 // start_delay: 1 day
    &259200,                // cliff_delay: 3 days
    &2_592_000,             // duration: 30 days
)?;
```

#### `create_streams_relative`

**Signature:**
```rust
pub fn create_streams_relative(
    env: Env,
    sender: Address,
    streams_relative: Vec<CreateStreamRelativeParams>,
) -> Result<Vec<u64>, ContractError>
```

**Parameters (per entry):**
```rust
pub struct CreateStreamRelativeParams {
    pub recipient: Address,
    pub deposit_amount: i128,
    pub rate_per_second: i128,
    pub start_delay: u64,
    pub cliff_delay: u64,
    pub duration: u64,
}
```

**Batch semantics:**
- Empty batch returns `Ok(Vec::new())` with no side effects
- All entries are converted to absolute times (overflow checks per entry)
- Delegates to `create_streams` with converted parameters
- Atomic: all or nothing (any validation failure aborts entire batch)
- Single authorization and token transfer for all streams (gas efficient)

**Example:**
```
let params = vec![
    CreateStreamRelativeParams {
        recipient: alice,
        deposit_amount: 1000,
        rate_per_second: 1,
        start_delay: 0,           // Immediate
        cliff_delay: 0,           // Immediate
        duration: 86400,          // 1 day
    },
    CreateStreamRelativeParams {
        recipient: bob,
        deposit_amount: 2000,
        rate_per_second: 2,
        start_delay: 86400,       // 1 day delay
        cliff_delay: 172800,      // 2 day cliff
        duration: 2592000,        // 30 days
    },
];
contract.create_streams_relative(&sender, &params)?;
```

**Error handling:**
- `InvalidParams`: If any time offset causes u64 overflow, or if other validation fails (rate, deposit, cliff bounds, etc.)
- `ContractPaused`: If creation is globally paused
- All other errors: Same as `create_stream` / `create_streams`

---

## 4. Access Control

| Function                  | Authorized Caller             | Auth Check                                  |
| ------------------------- | ----------------------------- | ------------------------------------------- |
| `init`                    | Bootstrap admin signer (once) | `admin.require_auth()`                      |
| `create_stream`           | Sender                        | `sender.require_auth()`                     |
| `clone_stream`            | Source stream's sender        | `source.sender.require_auth()`              |
| `create_streams`          | Sender                        | `sender.require_auth()` (once per batch)    |
| `create_stream_relative`  | Sender                        | `sender.require_auth()`                     |
| `create_streams_relative` | Sender                        | `sender.require_auth()` (once per batch)    |
| `pause_stream`            | Sender                        | `sender.require_auth()`                     |
| `resume_stream`           | Sender                        | `sender.require_auth()`                     |
| `cancel_stream`           | Sender                        | `sender.require_auth()`                     |
| `withdraw`                | Recipient                     | `recipient.require_auth()`                  |
| `withdraw_to`             | Recipient                     | `recipient.require_auth()`                  |
| `batch_withdraw`          | Recipient                     | `recipient.require_auth()` (once per batch) |
| `calculate_accrued`       | Anyone                        | None (view)                                 |
| `get_withdrawable`        | Anyone                        | None (view)                                 |
| `get_claimable_at`        | Anyone                        | None (view)                                 |
| `get_config`              | Anyone                        | None (view)                                 |
| `get_stream_state`        | Anyone                        | None (view)                                 |
| `get_stream_health`       | Anyone                        | None (view)                                 |
| `get_streams_by_id_range` | Anyone                        | None (view, paginated)                      |
| `get_recipient_streams_paginated` | Anyone                  | None (view, paginated)                      |
| `pause_stream_as_admin`   | Admin                         | `admin.require_auth()`                      |
| `resume_stream_as_admin`  | Admin                         | `admin.require_auth()`                      |
| `cancel_stream_as_admin`  | Admin                         | `admin.require_auth()`                      |
| `close_completed_stream`  | Anyone                        | None (permissionless terminal cleanup)     |
| `top_up_stream`           | Funder address                | `funder.require_auth()`                     |
| `update_rate_per_second`  | Sender                        | `sender.require_auth()`                     |
| `update_recipient`        | Recipient                     | `recipient.require_auth()`                  |
| `decrease_rate_per_second`| Sender                        | `sender.require_auth()`                     |
| `shorten_stream_end_time` | Sender                        | `sender.require_auth()`                     |
| `extend_stream_end_time`  | Sender                        | `sender.require_auth()`                     |
| `transfer_sender`         | Current stream sender         | `sender.require_auth()`                     |
| `set_auto_claim`          | Recipient                     | `recipient.require_auth()`                  |
| `revoke_auto_claim`       | Recipient                     | `recipient.require_auth()`                  |
| `trigger_auto_claim`      | Anyone                        | None (permissionless; destination fixed by recipient) |
| `get_auto_claim_destination` | Anyone                     | None (view)                                 |
| `delegated_withdraw`         | Relayer (ed25519 sig from recipient) | `relayer.require_auth()` + ed25519 sig |
| `get_delegated_nonce`        | Anyone                     | None (view)                                 |
| `release_id_reservation`     | Reservation holder         | `holder.require_auth()`                     |
| `reclaim_expired_id_reservation` | Anyone                 | None (permissionless cleanup)               |

**Note:** Sender-managed functions (`pause_stream`, `resume_stream`, `cancel_stream`) require sender auth. Admin uses separate `_as_admin` entry points.

### Paginated Export Views (Issue #429)

Two bounded view entrypoints support off-chain export and migration without unbounded loops:

#### `get_streams_by_id_range(start_id, end_id, limit) -> Vec<Stream>`

Returns streams within an ID range with a strict result limit (capped at `MAX_PAGE_SIZE = 100`).

**Parameters:**
- `start_id: u64` — First stream ID (inclusive)
- `end_id: u64` — Last stream ID (inclusive). Use `u64::MAX` for open-ended.
- `limit: u64` — Max results (enforced ≤ 100)

**Semantics:**
- Returns streams in ascending ID order
- Skips closed/archived streams silently
- Empty range (`start_id > end_id`) returns empty vector
- Zero limit returns empty vector

**DoS Protection:**
- Hard limit of 100 streams per call regardless of requested `limit`
- Gas cost is O(result_count), not O(range_size)

**Migration Pattern:**
```rust
let total = client.get_stream_count();
let mut start = 0u64;
while start < total {
    let page = client.get_streams_by_id_range(&start, &(start + 99), &100);
    // Export page...
    start += page.len() as u64;  // Handle closed streams
}
```

#### `get_recipient_streams_paginated(recipient, cursor, limit) -> Vec<u64>`

Cursor-based pagination for recipient stream export (capped at `MAX_PAGE_SIZE = 100`).

**Parameters:**
- `recipient: Address` — Address to query
- `cursor: u64` — 0-based starting index
- `limit: u64` — Max results (enforced ≤ 100)

**Semantics:**
- Cursor is index into sorted recipient stream list
- Returns stream IDs in ascending order
- Empty result indicates end of data or cursor beyond bounds

**Pagination Pattern:**
```rust
let mut cursor = 0u64;
loop {
    let page = client.get_recipient_streams_paginated(&recipient, &cursor, &50);
    if page.is_empty() { break; }
    // Export page...
    cursor += page.len() as u64;
}
```

**Comparison with Unbounded Views:**

| Function | Use Case | Limit | Risk |
|----------|----------|-------|------|
| `get_recipient_streams` | Small portfolios (<100) | None | Memory exhaustion |
| `get_recipient_streams_paginated` | Large portfolios | 100/page | Bounded, safe |
| `get_streams_by_id_range` | Full contract export | 100/page | Bounded, safe |

### top_up_stream: Observable Semantics

`top_up_stream(stream_id, funder, amount)` is a deposit-only mutation for existing streams.

- Auth boundary: only `funder` must authorize. The contract does not require `funder` to be the stream sender or the contract admin.
- Allowed states: `Active` and `Paused` only. `Completed` and `Cancelled` return `ContractError::InvalidState`.
- Amount validation: `amount <= 0` returns `ContractError::InvalidParams`.
- State transition on success: only `deposit_amount` changes, and it increases by exactly `amount`.
- Preserved fields on success: `sender`, `recipient`, `start_time`, `cliff_time`, `end_time`, `rate_per_second`, `withdrawn_amount`, and `status`.
- Atomic failure semantics: failed auth, failed token pull, or arithmetic overflow revert the whole transaction, leaving balances, stored deposit, and emitted contract events unchanged.
- Event semantics: a successful top-up emits exactly one contract event with topics `("top_up", stream_id)` and payload `StreamToppedUp { stream_id, top_up_amount: amount, new_deposit_amount }`.

Treasury policy note: if an application wants to restrict who may fund streams, that policy must be enforced off-chain or in a wrapper contract. The base stream contract intentionally accepts any self-authorizing funder.

### decrease_rate_per_second: Observable Semantics

`decrease_rate_per_second(stream_id, new_rate_per_second)` allows the stream sender to safely reduce the streaming rate.
A naive decrease would retroactively lower the recipient's accrued tokens. To prevent this, the contract **checkpoints** the stream: it locks in the mathematical accrual up to the current timestamp under the old rate, and applies the new rate only moving forward.

- **Check-Effects-Interactions (CEI)**: Computes accrual, reduces deposit amount, persists stream state, and finally refunds the difference to the sender.
- **Rate Validation**: `0 < new_rate_per_second < current rate_per_second`.
- **Refund**: The sender receives a refund of `old_deposit - new_deposit`, where `new_deposit = checkpointed_amount + new_rate * remaining_seconds`.
- **Refund-non-negativity invariant**: The refund maths uses `checked_sub` (`old_deposit - new_deposit`), not `saturating_sub`. Any rate change whose `new_deposit` would **exceed** `old_deposit` — i.e. the new schedule streamable amount is *larger* than what has already been deposited — is rejected with `ContractError::ArithmeticOverflow`. The contract never silently grows a stream's deposit ceiling via `decrease_rate_per_second`; only `top_up_stream` adds new deposit, and `update_rate_per_second` re-prices under the existing ceiling. See `contracts/stream/tests/rate_decrease_after_withdraw.rs` for the regression coverage of this invariant.

#### Failures
- **Unauthorized**: Caller is not the original sender.
- **InvalidState**: Stream is already expired (`now >= end_time`).
- **StreamTerminalState**: Stream is Cancelled or Completed.
- **InvalidParams**: `new_rate_per_second <= 0` or `new_rate_per_second >= old_rate`.
- **ArithmeticOverflow**: `old_deposit < new_deposit` per the refund-non-negativity invariant above. The contract refuses to *grow* the stream's deposit ceiling via a rate decrease.

### update_rate_per_second: Observable Semantics

`update_rate_per_second(stream_id, new_rate_per_second)` allows the stream sender to increase the streaming rate for an existing stream.

#### Success Semantics (Observable)

- **Authorization**: Only the stream `sender` can authorize the call.
- **State Requirements**: Stream must be in `Active` or `Paused` status (not `Completed` or `Cancelled`).
- **Rate Validation**: `new_rate_per_second > 0` and `new_rate_per_second > current rate_per_second` (forward-only increases).
- **Deposit Coverage**: `deposit_amount >= new_rate_per_second * (end_time - start_time)` must hold.
- **Accrual Impact**: The accrual calculation uses the new rate retroactively for the entire elapsed time since `start_time`, ensuring accrued amounts are monotonically non-decreasing.
- **Partial Withdrawal Interaction**: `withdrawn_amount` remains unchanged. Withdrawable amount becomes `accrued (with new rate) - withdrawn_amount`.
- **Event**: Emits `("rate_upd", stream_id)` with `RateUpdated` payload including old/new rates and `effective_time`.
- **No State Changes**: `status`, `deposit_amount`, `withdrawn_amount`, schedule fields (`start_time`, `cliff_time`, `end_time`) are preserved.

#### Failure Semantics (Observable)

- **StreamNotFound**: Invalid `stream_id`.
- **Unauthorized**: Caller is not the stream sender.
- **InvalidState**: Stream is `Completed` or `Cancelled`.
- **InvalidParams**: `new_rate_per_second <= 0` or `new_rate_per_second <= old_rate`.
- **InsufficientDeposit**: `deposit_amount < new_rate_per_second * (end_time - start_time)`.
- **Atomicity**: Any failure reverts the entire transaction with no state changes or events.

#### Invariants

- Accrued amounts never decrease due to rate updates.
- Recipient entitlement is preserved or increased.
- Deposit coverage ensures the stream remains fully fundable at the new rate.

### transfer_sender: Observable Semantics

`transfer_sender(stream_id, new_sender)` allows the current stream sender to rotate the treasury key for an existing stream.

#### Success Semantics (Observable)

- **Authorization**: Only the current stream `sender` can authorize the call.
- **State Requirements**: Stream must be in `Active` or `Paused` status (not `Completed` or `Cancelled`).
- **Parameter Validation**: `new_sender != current_sender` and `new_sender != recipient`.
- **State Change**: `stream.sender` is updated to `new_sender`. All other fields are unchanged.
- **Immediate Effect**: `new_sender` gains all sender-role privileges (pause, resume, cancel, rate updates, schedule changes) immediately. `old_sender` loses them immediately.
- **Recipient Entitlement**: Unchanged. Accrued amounts, `withdrawn_amount`, and schedule are unaffected.
- **Event**: Emits `("sndr_xfr", stream_id)` with `SenderTransferred { stream_id, old_sender, new_sender }`.

#### Failure Semantics (Observable)

- **StreamNotFound**: Invalid `stream_id`.
- **Unauthorized**: Caller is not the current stream sender.
- **InvalidState**: Stream is `Completed` or `Cancelled`.
- **InvalidParams**: `new_sender == current_sender` or `new_sender == recipient`.
- **Atomicity**: Any failure reverts the entire transaction with no state changes or events.

#### Use Case

Treasury key rotation: when a treasury wallet is being rotated, the operator calls `transfer_sender` to hand over stream management rights to the new key without disrupting the recipient's accrual or requiring stream recreation.

### batch_withdraw: completed stream behavior

`batch_withdraw` processes each stream ID in order. A stream with status `Completed` **does not error** — it contributes a zero-amount result (`BatchWithdrawResult { stream_id, amount: 0 }`) and is skipped silently. No token transfer and no event are emitted for that entry. This allows callers to pass a mixed list of active and already-completed streams without pre-filtering.

A `Paused` stream **does** return `ContractError::InvalidState` and reverts the entire batch.

### One-Shot Init and Immutable Bootstrap

`init(token, admin)` has explicit externally observable bootstrap semantics:

- One-shot: first successful call writes `Config { token, admin }` and `NextStreamId = 0`.
- Auth boundary: the supplied `admin` address must authorize the call.
- Re-init failure: any second call returns `ContractError::AlreadyInitialised`.
- Failure atomicity: failed auth or re-init leaves bootstrap storage unchanged.
- Immutability boundary: `token` is immutable after init; `admin` can rotate only via `set_admin` with current-admin auth.

Residual assumption: deployment flow must ensure the intended bootstrap admin signs the first init transaction.

### create_streams: Batch Atomicity, Single Auth, and Empty Vector Semantics

`create_streams(sender, streams)` is the batch creation entrypoint for treasury operators and indexers.

#### Non-Empty Batch Semantics

- Single auth: only `sender` must authorize, and it is checked once for the entire batch.
- Batch validation: every entry is validated before token transfer or persistence.
- Atomic transfer: the contract pulls exactly `sum(deposit_amount)` once.
- Atomic persistence: if any entry fails validation (or total-deposit sum overflows), no stream is created.
- Event behavior: on success, one `created` event is emitted per created stream; on failure, no `created` events are emitted.
- Ordering guarantee: returned stream IDs are contiguous and in the same order as input entries.

#### Empty Vector Semantics

When `streams` is an empty vector:

**Success Behavior (Observable):**
- Returns `Ok(Vec::new())` (empty result vector)
- No tokens are transferred (total_deposit = 0, no `pull_token` call)
- No streams are persisted (stream count unchanged)
- No `StreamCreated` events are emitted
- Stream ID counter is not advanced
- Contract state remains unchanged
- Authorization is still required: `sender.require_auth()` is called and must succeed
- No errors are raised (empty batch is valid and succeeds)

**Failure Behavior (Observable):**
- If `sender` is not authorized: authorization failure before any state changes
- If contract is globally paused: `ContractError::ContractPaused` returned, no state changes
- Any failure is atomic: no state mutation, no token transfer, no events

**Invariants After Empty Batch:**
- Returned vector has length 0
- Stream count unchanged
- Token balances unchanged
- No new events in event log
- Recipient stream indices unchanged
- Multiple empty batches have identical observable effects (idempotent)

**Rationale:**
- Empty batch is a valid no-op: allows callers to submit conditional batches without special-casing
- Authorization is still required: maintains consistent auth semantics across all entry points
- No state advance: ensures stream IDs remain contiguous and predictable
- Idempotent: enables safe retry logic in integrators

#### Scope Note

These guarantees are limited to `create_streams` creation semantics. They do not change withdrawal, pause/resume, cancellation, or cleanup rules.

### batch_withdraw: Recipient-Only Auth, Completed Stream Handling, and Empty Vector Semantics

`batch_withdraw(recipient, stream_ids)` enforces recipient-only authorization and deterministic completion semantics:

### batch_withdraw_to: Per-Entry Destination Routing with Atomic Reversion

`batch_withdraw_to(recipient, withdrawals)` extends the same recipient-only authorization model to per-stream destinations. The recipient must authorize the batch, every stream must belong to that recipient, and the batch reverts atomically if any entry is unauthorized, invalid, or otherwise rejected. Destination addresses may be any non-contract address; routing to the contract address is rejected with `ContractError::InvalidParams`.

#### Non-Empty Batch Semantics

- Auth boundary: only the stream `recipient` can authorize `batch_withdraw`.
- Non-recipient calls fail before transfer/state/event side effects.
- Uniqueness check: `stream_ids` must not contain duplicates; duplicates return `ContractError::DuplicateStreamId` and revert the entire batch.
- Completed streams: contribute a zero-amount result and are skipped silently (no error, no event).
- Active/Paused streams: processed normally; `Paused` streams return `ContractError::InvalidState` and revert the entire batch.
- Event ordering on active final drain: `withdrew` is emitted before `completed`.

#### Empty Vector Semantics

When `stream_ids` is an empty vector:

**Success Behavior (Observable):**
- Returns `Ok(Vec::new())` (empty result vector)
- No streams are processed
- No tokens are transferred
- No events are emitted
- Contract state remains unchanged
- Authorization is still required: `recipient.require_auth()` is called and must succeed
- No errors are raised (empty batch is valid and succeeds)

**Failure Behavior (Observable):**
- If `recipient` is not authorized: authorization failure before any state changes
- If contract is globally paused: `ContractError::ContractPaused` returned, no state changes
- Any failure is atomic: no state mutation, no token transfer, no events

**Invariants After Empty Batch:**
- Returned vector has length 0
- No stream state changed
- Token balances unchanged
- No new events in event log
- Multiple empty batches have identical observable effects (idempotent)

**Rationale:**
- Empty batch is a valid no-op: allows callers to submit conditional batches without special-casing
- Authorization is still required: maintains consistent auth semantics across all entry points
- Idempotent: enables safe retry logic in integrators

---

### Auto-claim Opt-in Semantics

`set_auto_claim`, `revoke_auto_claim`, and `trigger_auto_claim` implement a recipient-controlled permissionless claim mechanism.

#### Overview

Recipients may opt in to have their final withdrawal triggered by any third party (keeper, bot, or user) once the stream reaches `end_time`. The destination address is chosen and stored on-chain by the recipient — no caller can redirect funds.

#### `set_auto_claim(stream_id, destination)`

- **Auth**: `recipient.require_auth()` — only the stream recipient may set or change the destination.
- **Constraints**: stream must exist and not be `Completed` or `Cancelled`; `destination` must not be the contract address.
- **Validation**: destination is validated to ensure it's not the zero address and not the contract itself.
- **Idempotent**: calling again with a new address overwrites the previous destination.
- **Event**: `("ac_set", stream_id)` → `AutoClaimSet { stream_id, destination }`.
- **Storage**: destination is stored in persistent storage under `DataKey::AutoClaimDestination(stream_id)`.

#### `get_auto_claim_status(stream_id) -> AutoClaimStatus`

Pre-flight check query that returns the auto-claim configuration status and claimable amount. This allows callers to validate before executing `trigger_auto_claim`, reducing failed transactions and wasted gas on invalid destinations.

- **Auth**: None required (read-only view function).
- **Returns**: `AutoClaimStatus` enum with three variants:
  - `NotSet`: No auto-claim destination has been configured for this stream.
  - `ValidDestination { destination, claimable }`: Destination is set and valid, with the current claimable amount.
  - `InvalidDestination { destination }`: Destination is set but invalid (zero address or contract itself).
- **Claimable calculation**: Computed as `accrued_amount - withdrawn_amount` at current timestamp, capped at 0.
- **Validation checks**:
  - Destination is not the zero address
  - Destination is not the contract address itself
- **Usage pattern**:
  ```rust
  let status = client.get_auto_claim_status(&stream_id);
  match status {
      AutoClaimStatus::ValidDestination { destination, claimable } => {
          if claimable > 0 {
              client.trigger_auto_claim(&stream_id);
          }
      }
      AutoClaimStatus::NotSet => {
          // No auto-claim configured
      }
      AutoClaimStatus::InvalidDestination { destination } => {
          // Destination is invalid, cannot trigger
      }
  }
  ```
- **Benefits**:
  - Prevents wasted gas on invalid destinations
  - Allows off-chain systems to batch valid claims
  - Provides transparency for keepers and bots
  - No state changes or side effects

#### `get_auto_claim_destination(stream_id) -> Option<Address>`

Simple query that returns the stored auto-claim destination address, or `None` if not set.

- **Auth**: None required (read-only view function).
- **Returns**: `Option<Address>` — the destination address if set, otherwise `None`.
- **Note**: Does not validate the destination. Use `get_auto_claim_status` for validation.

#### `revoke_auto_claim(stream_id)`

- **Auth**: `recipient.require_auth()`.
- **Idempotent**: safe to call even if no destination is set (no error, no event side-effects beyond the revoke event).
- **Event**: `("ac_revoke", stream_id)` → `AutoClaimRevoked { stream_id }`.

#### `trigger_auto_claim(stream_id)`

- **Auth**: **none** — permissionless. Any account may call this.
- **Preconditions** (all must hold):
  1. Stream exists.
  2. Stream is not `Completed` or `Cancelled`.
  3. `ledger.timestamp() >= stream.end_time` (time-terminal).
  4. Auto-claim destination is set (`AutoClaimNotSet` otherwise).
  5. Contract is not globally paused.
- **Accounting**: identical to `withdraw_to` — computes `accrued - withdrawn_amount`, caps by contract balance, updates `withdrawn_amount`, may transition to `Completed`.
- **Destination immutability**: tokens are sent to the address stored by the recipient. The caller cannot influence the destination.
- **Events**:
  - `("withdrew", stream_id)` → `Withdrawal { stream_id, recipient, amount }` (indexer compatibility).
  - `("ac_trig", stream_id)` → `AutoClaimTriggered { stream_id, destination, amount }`.
  - `("completed", stream_id)` → `StreamEvent::StreamCompleted(stream_id)` if stream transitions to `Completed`.

#### Cancellation interaction
 
If a stream is cancelled after opt-in, `trigger_auto_claim` returns `InvalidState`. The auto-claim destination entry remains in storage but is inert. Recipients may call `revoke_auto_claim` to clean up storage.
 
#### Revocation Boundary and Timing Safety
 
1. **Post-Revoke Trigger Prevention**: Once a recipient calls `revoke_auto_claim`, the auto-claim configuration is immediately deleted. Any subsequent `trigger_auto_claim` calls fail with `ContractError::InvalidParams` without transferring any tokens.
2. **Early Trigger Restriction**: Permissionless triggering is strictly disallowed before the stream's `end_time` is reached. Early calls return `ContractError::InvalidState` and transfer zero funds.
3. **Destination Update Immendiate Effect**: If the destination is updated, the change is immediately effective. Triggering auto-claim afterwards sends funds ONLY to the recipient's currently selected destination.
 
#### Security invariants
 
1. Only the recipient can set or change the destination (`require_auth` enforced).
2. The caller of `trigger_auto_claim` has zero influence over where tokens go.
3. CEI ordering is preserved: stream state is saved before the token transfer.
4. Global emergency pause blocks `trigger_auto_claim` (same as `withdraw`).

---

### delegated_withdraw: Relayer-Submitted Withdrawal with Minimum Amount Guard

`delegated_withdraw` allows a relayer (keeper, bot, or any third party) to submit a withdrawal on behalf of a recipient without requiring the recipient to sign a Soroban transaction themselves. The recipient instead signs an off-chain ed25519 message committing to the exact parameters of the withdrawal.

#### Signed message format

```
message = stream_id            (u64,  8 bytes, big-endian)
        | nonce                (u64,  8 bytes, big-endian)
        | deadline             (u64,  8 bytes, big-endian)
        | expected_minimum_amount (i128, 16 bytes, big-endian)
```

Total: 40 bytes.

#### `expected_minimum_amount` — front-running protection

Without this field, a relayer could delay the transaction until the accrued amount is much smaller than the recipient expected (e.g. after a rate decrease or near stream end), constituting a griefing vector. By committing to a minimum, the call reverts with `BelowMinimumAmount` (16) if `withdrawable < expected_minimum_amount`. Pass `0` to accept any positive amount.

#### Nonce — replay protection

Each recipient has a per-address nonce stored in `DataKey::DelegatedWithdrawNonce(recipient)`. The nonce starts at 0 and is incremented on every successful `delegated_withdraw`. Replaying a used signature returns `InvalidSignature` (15). Query the current nonce via `get_delegated_nonce(recipient)`.

#### Failure semantics

| Condition | Error |
|-----------|-------|
| `ledger.timestamp() > deadline` | `InvalidSignature` (15) |
| `nonce != stored_nonce` | `InvalidSignature` (15) |
| ed25519 signature invalid | host trap (panic) |
| `withdrawable < expected_minimum_amount` | `BelowMinimumAmount` (16) |
| Stream paused (non-terminal) | `InvalidState` (2) |
| Stream completed | `InvalidState` (2) |

---

## 5. Events

### Event Schema

#### StreamCreated

Emitted when a new stream is created via `create_stream` or `create_streams`.

**Topic:** `("created", stream_id)`

**Payload:** `StreamCreated` struct containing:

- `stream_id` (u64): Unique identifier for the stream
- `sender` (Address): Address that created and funded the stream
- `recipient` (Address): Address that receives the streamed tokens
- `deposit_amount` (i128): Total tokens deposited
- `rate_per_second` (i128): Streaming rate in tokens per second
- `start_time` (u64): When streaming begins (ledger timestamp)
- `cliff_time` (u64): When tokens first become available (vesting cliff)
- `end_time` (u64): When streaming completes (ledger timestamp)

#### Withdrawal

Emitted when a recipient successfully withdraws tokens via `withdraw`.

**Topic:** `("withdrew", stream_id)`

**Payload:** `Withdrawal` struct containing:

- `stream_id` (u64): Unique identifier for the stream
- `recipient` (Address): Address that received the tokens
- `amount` (i128): Amount of tokens withdrawn

#### RateUpdated

Emitted when a sender successfully updates the streaming rate via `update_rate_per_second`.

**Topic:** `("rate_upd", stream_id)`

**Payload:** `RateUpdated` struct containing:

- `stream_id` (u64): Unique identifier of the stream
- `old_rate_per_second` (i128): The previous streaming rate
- `new_rate_per_second` (i128): The new streaming rate
- `effective_time` (u64): Ledger timestamp when the rate update became effective

#### Other Events

| Topic                      | Payload                                       | When Emitted                                       |
| -------------------------- | --------------------------------------------- | -------------------------------------------------- |
| `("created", stream_id)`   | `StreamCreated` (struct payload)              | `create_stream` / `create_streams`                 |
| `("cloned", stream_id)`    | `StreamCloned` (struct payload)               | `clone_stream` — carries `source_stream_id` for indexer correlation |
| `("paused", stream_id)`    | `StreamEvent::Paused(stream_id)`              | `pause_stream` / `pause_stream_as_admin`           |
| `("resumed", stream_id)`   | `StreamEvent::Resumed(stream_id)`             | `resume_stream` / `resume_stream_as_admin`         |
| `("cancelled", stream_id)` | `StreamEvent::StreamCancelled(stream_id)`     | `cancel_stream` / `cancel_stream_as_admin`         |
| `("withdrew", stream_id)`  | `Withdrawal { stream_id, recipient, amount }` | `withdraw`                                         |
| `("completed", stream_id)` | `StreamEvent::StreamCompleted(stream_id)`     | `withdraw` / `batch_withdraw` (active final drain) |
| `("rate_upd", stream_id)` | `RateUpdated` (struct payload)                | `update_rate_per_second`                          |
| `("closed", stream_id)`    | `StreamEvent::StreamClosed(stream_id)`        | `close_completed_stream`                           |
| `("top_up", stream_id)`    | `StreamToppedUp` (struct payload)             | `top_up_stream`                                    |

---

## `withdraw_to` Destination Rules

`withdraw_to(stream_id, destination)` lets the recipient redirect accrued tokens to any address **except the contract itself**.

| Destination                  | Allowed | Error on rejection      |
| ---------------------------- | ------- | ----------------------- |
| Contract address (`env.current_contract_address()`) | ❌ No | `ContractError::InvalidParams` |
| Recipient address (self-redirect) | ✅ Yes | — |
| Sender address               | ✅ Yes  | —                       |
| Any other third-party address | ✅ Yes | —                       |

**Atomicity guarantee:** If the destination check fails, the call returns `InvalidParams` with **no side effects** — `withdrawn_amount` is not incremented, no token transfer occurs, and no event is emitted. The stream state is identical to its state before the call.

**Auth:** `recipient.require_auth()` is always enforced before the destination check.

---

## 6. Error Behavior (ContractError + Panics)

Errors are surfaced either as `ContractError` variants or as panic/assert messages.
Integrators should treat `ContractError` as stable error codes, and panic strings
as best-effort diagnostics. The table below focuses on creation and lifecycle
errors relevant to stream creation and timing.

| Message                                                                 | Function                           | Trigger                                       |
| ----------------------------------------------------------------------- | ---------------------------------- | --------------------------------------------- |
| `"already initialised"`                                                 | `init`                             | Re-init attempt                               |
| authorization failure                                                   | `init`                             | caller did not satisfy `admin.require_auth()` |
| `"deposit_amount must be positive"`                                     | `create_stream` / `create_streams` | deposit_amount <= 0                           |
| `"rate_per_second must be positive"`                                    | `create_stream` / `create_streams` | rate_per_second <= 0                          |
| `"sender and recipient must be different"`                              | `create_stream` / `create_streams` | sender == recipient                           |
| `"start_time must be before end_time"`                                  | `create_stream` / `create_streams` | start_time >= end_time                        |
| `"cliff_time must be within [start_time, end_time]"`                    | `create_stream` / `create_streams` | cliff out of range                            |
| `"deposit_amount must cover total streamable amount (rate * duration)"` | `create_stream` / `create_streams` | underfunded                                   |
| `"overflow calculating total streamable amount"`                        | `create_stream` / `create_streams` | overflow in rate \* duration                  |
| `"overflow calculating total batch deposit"`                            | `create_streams`                   | overflow in sum of deposits                   |
| `ContractError::StartTimeInPast`                                        | `create_stream` / `create_streams` | start_time < ledger timestamp                 |
| `ContractError::ClockRegression` (17)                                   | Ledger-backed accrual paths        | ledger timestamp < previous accrual timestamp |
| `ContractError::StreamAlreadyPaused` (10)                               | `pause_stream`                     | Double pause                                  |
| `ContractError::StreamNotPaused` (11)                                   | `resume_stream`                    | Resume active stream                          |
| `ContractError::StreamTerminalState` (12)                               | `pause_stream` / `resume_stream`   | Modification past end_time                    |
| `ContractError::StreamNotFound` (1)                                     | Various                            | Invalid stream_id                             |
| `[ContractError::UnsupportedStreamKind](./error.md#unsupportedstreamkind-17)` (17) | Mutating functions                 | Attempting to mutate a [CliffOnly](#cliff-only-streams) stream       |
| `ContractError::Unauthorized` (6)                                       | Various                            | Auth check failed                             |
| `ContractError::InvalidState` (2)                                       | `withdraw`                         | Withdraw from non-terminal paused             |
| `ContractError::InvalidState` (2)                                       | `cancel_stream`                    | Cancel completed/cancelled                    |
| `"invalid state for stream closure"`                                    | `close_completed_stream`           | Close non-terminal (Active/Paused) stream    |
| `ContractError::InvalidState` (2)                                       | `close_completed_stream`           | Close Cancelled stream with remaining claimable balance |
| `"contract not initialised: missing config"`                            | Functions requiring config         | Config missing                                |

## Protocol-Level Pausing

The protocol supports two distinct pausing modes managed by the contract admin. These modes allow for graduated intervention depending on the situation (e.g., routine maintenance vs. emergency exploit investigation).

### Pause Modes Comparison

| Mode | Flag | Blocked Operations | Allowed Operations |
|---|---|---|---|
| **Creation Only** | `CreationPaused` | `create_stream`, `create_streams` | `withdraw`, `cancel_stream`, `top_up_stream`, `update_rate_per_second`, `extend_stream_end_time`, `shorten_stream_end_time` |
| **Global Emergency** | `GlobalEmergencyPaused` | **ALL** mutation operations (Create, Withdraw, Cancel, Update, etc.) | `get_stream_state`, `calculate_accrued`, `close_completed_stream` (read-only and cleanup) |

### Gating Semantics

1. **Creation Functions**: Blocked if *either* `GlobalEmergencyPaused` or `CreationPaused` is set.
2. **Mutation Functions**: Blocked ONLY if `GlobalEmergencyPaused` is set.
3. **Read-Only Functions**: Never blocked; users can always calculate their accrued balance even during a total emergency pause.
4. **Admin Functions**: Never blocked; admins can always pause/resume the protocol or rotate the admin address.

### Observable Behavior

When an operation is blocked by a protocol-level pause, it returns `ContractError::ContractPaused` (4). No state changes occur, and no tokens are transferred.

---

## Error Reference

For a full list of contract errors, see [error.md](./error.md).

---

## Cross-References

### Related Documentation

- **[Protocol Narrative vs Code Alignment](./protocol-narrative-code-alignment.md)** - Complete verification that this documentation matches implementation
- **[Audit Documentation](./audit.md)** - Entrypoints and invariants for auditors
- **[Error Reference](./error.md)** - Complete error code catalog
- **[Security Guidelines](./security.md)** - Security considerations and best practices
- **[Storage Layout](./storage.md)** - Contract storage architecture
- **[Deployment Guide](./DEPLOYMENT.md)** - Step-by-step deployment checklist

### For Integrators

- **Treasury Operators**: See §1 (Lifecycle), §4 (Access Control), §5 (Events)
- **Recipient Applications**: See §2 (Accrual Formula), §4 (Withdrawal), §5 (Events)
- **Indexers**: See §5 (Events), §6 (Error Behavior)
- **Auditors**: See [protocol-narrative-code-alignment.md](./protocol-narrative-code-alignment.md) for complete verification

### Verification

This documentation is verified against implementation in [protocol-narrative-code-alignment.md](./protocol-narrative-code-alignment.md):

- ✅ All 20 operations have explicit authorization rules
- ✅ All 6 valid state transitions documented
- ✅ All 6 invalid state transitions documented
- ✅ Accrual formula matches implementation line-by-line
- ✅ All 7 event types verified
- ✅ All 8 error codes mapped
- ✅ Zero contradictions found

Last verified: 2026-03-27


---

## Admin Recovery: sweep_excess

### Overview

The `sweep_excess` function allows the contract admin to recover trapped tokens that exceed the sum of all outstanding stream liabilities. This addresses scenarios where tokens become stranded in the contract due to:

1. **Stream cancellations** where refunds fail or sender addresses are lost
2. **Rate decreases** via `decrease_rate_per_second` where excess deposits are refunded but the refund fails
3. **Rounding errors** that accumulate over many stream operations
4. **Failed refund transfers** during any operation that returns tokens to senders

### Function Signature

```rust
pub fn sweep_excess(env: Env, recipient: Address) -> Result<i128, ContractError>
```

### Parameters

- `recipient`: Address to receive the excess tokens

### Authorization

- **Required**: Contract admin must authorize the call via `admin.require_auth()`
- **Unauthorized callers**: Returns `ContractError::Unauthorized`

### Calculation

```text
excess = contract_token_balance - total_liabilities
```

Where:
- `contract_token_balance`: Current token balance of the contract (queried from token contract)
- `total_liabilities`: Sum of all outstanding stream deposits tracked in `DataKey::TotalLiabilities`

### Success Semantics (Observable)

1. **Preconditions**: Caller must be the authorized contract admin
2. **Calculation**: Computes `excess = balance - liabilities`
3. **Early return**: If `excess <= 0`, returns `Ok(0)` with no transfer or event
4. **Event**: Emits `ExcessSwept { to, amount }` with topic `("ex_swept", recipient)`
5. **Transfer**: Transfers `excess` tokens from contract to `recipient`
6. **Return**: Returns the amount swept (`excess`)

### Failure Semantics (Observable)

1. **Unauthorized**: If caller is not admin → `ContractError::Unauthorized`
2. **Invalid state**: If contract not initialized → `ContractError::InvalidState`
3. **Transfer failure**: If token transfer fails → propagates token contract error
4. **Reentrancy**: If reentrancy lock is held → `ContractError::InvalidState`

### Safety Guarantees

1. **Recipient protection**: Never sweeps tokens that are owed to stream recipients
2. **Liability tracking**: Uses `TotalLiabilities` counter to ensure all active stream deposits are protected
3. **CEI pattern**: Emits event before token transfer to reduce reentrancy risk
4. **Reentrancy guard**: Acquires lock before transfer, releases after
5. **Idempotent**: Safe to call multiple times; returns 0 when no excess exists

### Invariants

1. After successful sweep: `contract_balance == total_liabilities`
2. Active stream deposits are never affected
3. Recipient entitlements remain unchanged
4. No state mutation if `excess <= 0`

### Usage Notes

- **Permissionless query**: Anyone can calculate potential excess by comparing `token.balance(contract)` with the sum of all active stream `deposit_amount - withdrawn_amount` values
- **Operational hygiene**: Should be called periodically by operators to maintain clean accounting
- **No impact on streams**: Does not affect any stream state, accrual, or withdrawal operations
- **Multiple calls**: Can be called multiple times as excess accumulates

### Example Scenarios

#### Scenario 1: Stream Cancellation with Lost Sender

```text
1. Stream created: 1000 tokens deposited
2. Stream cancelled at 50% completion
3. 500 tokens should be refunded to sender
4. Sender address is compromised/lost
5. 500 tokens remain trapped in contract
6. Admin calls sweep_excess to recover the 500 tokens
```

#### Scenario 2: Rate Decrease

```text
1. Stream created: 1000 tokens, 10 tokens/sec, 100 seconds
2. Rate decreased at t=50 to 5 tokens/sec
3. New total needed: 500 (accrued) + 250 (remaining) = 750
4. 250 tokens refunded to sender
5. If refund fails, 250 tokens trapped
6. Admin calls sweep_excess to recover the 250 tokens
```

#### Scenario 3: Multiple Streams with Accumulated Excess

```text
1. 100 streams created and cancelled over time
2. Small rounding errors accumulate (1-2 tokens per stream)
3. Total excess: ~150 tokens
4. Admin calls sweep_excess to recover accumulated excess
```

### Access Control Table Entry

| Function        | Authorized Caller | Auth Check              |
| --------------- | ----------------- | ----------------------- |
| `sweep_excess`  | Admin             | `admin.require_auth()`  |

### Event

```rust
#[contracttype]
#[derive(Clone, Debug)]
pub struct ExcessSwept {
    pub to: Address,
    pub amount: i128,
}
```

**Topic**: `("ex_swept", recipient)`

### Error Codes

- `ContractError::Unauthorized` (7): Caller is not the admin
- `ContractError::InvalidState` (2): Contract not initialized or reentrancy detected
- Token transfer errors: Propagated from token contract

### Security Considerations

1. **Admin trust**: This function requires trusting the admin not to abuse it. The admin could theoretically call it with their own address to extract excess funds.
2. **Liability tracking accuracy**: The safety of this function depends on accurate `TotalLiabilities` tracking. Any bug in liability accounting could allow sweeping of recipient funds.
3. **Audit trail**: All sweeps are logged via `ExcessSwept` events for transparency and auditing.
4. **No emergency pause bypass**: This function does not bypass global emergency pause (if implemented in future versions).

### Testing

Comprehensive test coverage includes:

- ✅ Returns 0 when no excess exists
- ✅ Sweeps correct amount after stream cancellation
- ✅ Sweeps correct amount after rate decrease
- ✅ Requires admin authorization
- ✅ Emits `ExcessSwept` event
- ✅ Protects recipient funds (never sweeps liabilities)
- ✅ Can be called multiple times
- ✅ Works correctly with multiple streams
- ✅ Handles edge cases (completed streams, paused streams, etc.)

See `contracts/stream/tests/integration_suite.rs` for full test suite.

## Keeper Cancellation

### Overview

After a stream's `end_time` passes and a configurable grace period elapses, any address may
call `keeper_cancel` to close the stream and collect a small incentive fee.  This prevents
unclaimed deposits from remaining locked in contract storage indefinitely.

| Constant | Value | Notes |
| -------- | ----- | ----- |
| `KEEPER_GRACE_PERIOD_SECONDS` | 604 800 s (7 days) | Seconds after `end_time` before eligibility |
| `KEEPER_FEE_BPS` | 50 bps (0.5 %) | Fee as a fraction of the unstreamed sender refund |

### Token distribution on `keeper_cancel`

1. `recipient_amount = accrued − withdrawn_amount` → transferred to recipient.
2. `sender_refund_gross = deposit_amount − accrued` (unstreamed portion).
3. `keeper_fee = sender_refund_gross × KEEPER_FEE_BPS / 10 000` → transferred to keeper.
4. `sender_refund = sender_refund_gross − keeper_fee` → transferred to sender.

When `sender_refund_gross == 0` (stream fully accrued), the keeper receives no fee.

### `keeper_cancel`

**Authorization:** `keeper.require_auth()` — prevents fee redirection by a third party.

**Errors:**

| Error | Condition |
| ----- | --------- |
| `StreamNotFound` | `stream_id` does not exist |
| `InvalidState` | Stream is already `Cancelled` or `Completed` |
| `KeeperGracePeriodNotElapsed` | `now < end_time + KEEPER_GRACE_PERIOD_SECONDS` |

### `get_keeper_fee_split` (view)

**Purpose:** Preview the `(keeper_fee, sender_refund)` split that `keeper_cancel` would pay,
without moving any funds or changing any state.  Keepers should call this before paying gas
to confirm the fee is worthwhile.

**Entry-point:**

```rust
pub fn get_keeper_fee_split(env: Env, stream_id: u64) -> Result<(i128, i128), ContractError>
```

**Authorization:** None (public view).

**Returns:**

| Condition | Return |
| --------- | ------ |
| Grace period not yet elapsed | `Ok((0, 0))` — not yet eligible, no error |
| Stream is eligible | `Ok((keeper_fee, sender_refund))` matching `keeper_cancel` payouts |
| Stream is `Cancelled` or `Completed` | `Err(InvalidState)` |
| Stream does not exist | `Err(StreamNotFound)` |

**Invariants:**

- `keeper_fee + sender_refund == deposit_amount − accrued` (gross unstreamed) when eligible.
- Output is identical to the amounts computed inside `keeper_cancel` for the same ledger timestamp.
- No state writes, no TTL changes, no token operations — cannot be abused for griefing.

**Example (Rust client):**

```rust
let (fee, refund) = client.get_keeper_fee_split(&stream_id)?;
if fee > gas_cost_estimate {
    client.keeper_cancel(&stream_id, &keeper_address);
}
```

**Test coverage:** See `contracts/stream/tests/keeper_cancel.rs`.

- ✅ View/cancel parity (preview matches actual keeper and sender payouts)
- ✅ Not-yet-eligible stream returns `(0, 0)`
- ✅ Active stream before `end_time` returns `(0, 0)`
- ✅ Fully-accrued stream returns `(0, 0)` (no gross, no fee)
- ✅ `Cancelled` stream returns `InvalidState`
- ✅ `Completed` stream returns `InvalidState`
- ✅ Non-existent stream returns `StreamNotFound`
- ✅ Paused eligible stream returns correct split
- ✅ `fee + refund == gross` invariant
- ✅ Idempotency (two calls at same timestamp return same result)

---

## ID Reservation (Off-Chain Orchestration)

### reserve_stream_ids

**Purpose:** Pre-allocate a contiguous range of stream IDs before creating streams, enabling off-chain orchestrators to pre-populate database records or reference external invoice systems with deterministic IDs.

**Entry-point:**

```rust
pub fn reserve_stream_ids(
    env: Env,
    caller: Address,
    count: u32,
) -> Result<Vec<u64>, ContractError>
```

**Authorization:** Requires `caller` signature.

**Parameters:**

- `caller`: Address making the reservation
- `count`: Number of IDs to reserve (1 – `MAX_ID_RESERVATION` = 100)

**Returns:** `Vec<u64>` containing the reserved IDs in ascending order.

**Behavior:**

1. Atomically advances the global `NextStreamId` counter by `count`
2. Stores an `IdReservation { start_id, count, consumed: 0 }` keyed by `caller`
3. Returns `[start_id, start_id+1, ..., start_id+count-1]`
4. Subsequent `create_stream` calls from `caller` consume IDs from the reservation in order
5. When fully consumed, the reservation is automatically deleted
6. A second `reserve_stream_ids` call before the first is exhausted **replaces** the old reservation (unconsumed IDs become permanent gaps; the counter is never rewound)

**Security:**

- `count` capped at `MAX_ID_RESERVATION = 100` to prevent counter-inflation attacks
- Authorization required to prevent third parties from consuming a victim's counter space
- Gaps from unconsumed reservations are permanent but bounded (max 100 per call)

**Errors:**

- `ReservationCountZero` (17): `count` is 0
- `ReservationLimitExceeded` (18): `count > MAX_ID_RESERVATION`

**Example:**

```rust
// Off-chain orchestrator reserves 5 IDs
let ids = client.reserve_stream_ids(&orchestrator, &5); // [0, 1, 2, 3, 4]

// Pre-populate database with these IDs
database.insert_pending_streams(ids);

// Later: create streams — they'll get the reserved IDs
let id0 = client.create_stream(&orchestrator, ...); // Uses ID 0
let id1 = client.create_stream(&orchestrator, ...); // Uses ID 1
```

### get_id_reservation

**Purpose:** View the active ID reservation for a caller (if any).

**Entry-point:**

```rust
pub fn get_id_reservation(env: Env, caller: Address) -> Option<IdReservation>
```

**Authorization:** None (view function).

**Returns:**

- `Some(IdReservation { start_id, count, consumed })` if caller has an active reservation
- `None` if no reservation exists

**Example:**

```rust
let res = client.get_id_reservation(&caller);
match res {
    Some(r) => println!("Reserved {}-{}, consumed {}", r.start_id, r.start_id + r.count - 1, r.consumed),
    None => println!("No active reservation"),
}
```

**Test coverage:** See `contracts/stream/tests/id_reservation.rs` for comprehensive tests covering:

- ✅ Basic reservation (single, max, sequential)
- ✅ Error cases (zero count, over-limit)
- ✅ `get_id_reservation` view (before/after reserve)
- ✅ `create_stream` consuming reservations
- ✅ Counter-gap semantics (overwrites, exhaustion)
- ✅ Multi-caller isolation

