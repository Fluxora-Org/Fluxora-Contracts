# Fluxora Stream Contract Documentation

Onboarding and integration reference for developers and auditors. Describes stream lifecycle, accrual formula, cliff/end_time behavior, access control, events, and error codes.

**Source of truth:** `contracts/stream/src/lib.rs`, `contracts/stream/src/accrual.rs`

## Sync Checklist

When changing the contract:

- Update this doc if you change lifecycle, access control, events, or panic messages
- Run `cargo test -p fluxora_stream` before committing
- No behavior change required for doc-only updates

---

## 1. Stream Lifecycle

### Phases

| Phase | Action | Notes |
|-------|--------|-------|
| **Creation** | `create_stream` | Sender deposits tokens; stream starts as `Active` |
| **Pause** | `pause_stream` / `pause_stream_as_admin` | Stops withdrawals; accrual continues by time |
| **Resume** | `resume_stream` / `resume_stream_as_admin` | Restores withdrawals |
| **Cancellation** | `cancel_stream` / `cancel_stream_as_admin` | Refunds unstreamed amount to sender; accrued amount stays for recipient |
| **Withdrawal** | `withdraw` | Recipient pulls accrued tokens |
| **Completion** | Automatic | When `withdrawn_amount == deposit_amount`, status becomes `Completed` |

### State Transitions

- **Active** ↔ **Paused** (via pause/resume)
- **Active** or **Paused** → **Cancelled** (terminal)
- **Active** → **Completed** (when recipient withdraws full deposit; terminal)

Terminal states: `Completed`, `Cancelled`. They cannot transition to any other state.

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
    Note right of Contract: Event: ("created", stream_id) → deposit_amount

    Note over Sender, Recipient: 2. Cliff Period (no withdrawals)

    Recipient ->> Contract: withdraw(stream_id)
    Contract --x Recipient: panic: "nothing to withdraw"

    Note over Sender, Recipient: 3. After Cliff — Partial Withdrawal

    Recipient ->> Contract: withdraw(stream_id)
    Contract ->> Contract: require_auth(recipient)<br/>calculate_accrued() − withdrawn_amount
    Contract ->> Token: transfer(contract → recipient, withdrawable)
    Token -->> Contract: OK
    Contract -->> Recipient: withdrawable
    Note right of Contract: Event: ("withdrew", stream_id) → withdrawable

    Note over Sender, Recipient: 4. Optional — Pause / Resume

    Sender ->> Contract: pause_stream(stream_id)
    Contract ->> Contract: require_auth(sender)<br/>status = Paused
    Contract -->> Sender: OK
    Note right of Contract: Event: ("paused", stream_id)

    Recipient ->> Contract: withdraw(stream_id)
    Contract --x Recipient: panic: "cannot withdraw from paused stream"

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
    Note right of Contract: Event: ("withdrew", stream_id) → withdrawable

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

```text
if current_time < cliff_time           → return 0
if start_time >= end_time or rate < 0  → return 0

elapsed_now = min(current_time, end_time)
elapsed_seconds = elapsed_now - start_time   // 0 if underflow
accrued = elapsed_seconds * rate_per_second  // on overflow → deposit_amount
return min(accrued, deposit_amount).max(0)
```

### Rules

- **Before cliff:** Returns 0 (no withdrawals allowed)
- **After cliff:** Accrual computed from `start_time`, not from cliff
- **No cliff:** Set `cliff_time = start_time` for immediate vesting
- **After end_time:** Capped at `deposit_amount`
- **Overflow:** Multiplication overflow yields `deposit_amount` (safe upper bound)
- **Completed:** `calculate_accrued` returns `deposit_amount` (deterministic final value)
- **Cancelled:** `calculate_accrued` is frozen at `cancelled_at` (no post-cancel growth)

### Withdrawable Amount

```text
withdrawable = accrued - withdrawn_amount
```

---

## 3. Cliff and end_time Behavior

### Cliff

- Must be in `[start_time, end_time]` (enforced at creation)
- Before `cliff_time`: accrued = 0, no withdrawals
- At or after `cliff_time`: accrual uses elapsed time from `start_time`, not cliff

### end_time

- Must satisfy `start_time < end_time`
- Accrual uses `min(current_time, end_time)` as the upper bound
- After `end_time`, accrued stays at `deposit_amount`
- No extra accrual beyond `end_time`

### Deposit Validation

At creation:

```text
deposit_amount >= rate_per_second * (end_time - start_time)
```

---

## 4. Access Control

| Function | Authorized Caller | Auth Check |
|----------|-------------------|------------|
| `init` | Deployer (once) | None |
| `create_stream` | Sender | `sender.require_auth()` |
| `pause_stream` | Sender | `sender.require_auth()` |
| `resume_stream` | Sender | `sender.require_auth()` |
| `cancel_stream` | Sender | `sender.require_auth()` |
| `withdraw` | Recipient | `recipient.require_auth()` |
| `calculate_accrued` | Anyone | None (view) |
| `get_config` | Anyone | None (view) |
| `get_stream_state` | Anyone | None (view) |
| `pause_stream_as_admin` | Admin | `admin.require_auth()` |
| `resume_stream_as_admin` | Admin | `admin.require_auth()` |
| `cancel_stream_as_admin` | Admin | `admin.require_auth()` |

**Note:** Sender-managed functions (`pause_stream`, `resume_stream`, `cancel_stream`) require sender auth. Admin uses separate `_as_admin` entry points.

---

## 5. Events

| Topic | Payload | When Emitted |
|-------|---------|--------------|
| `("created", stream_id)` | `deposit_amount` (i128) | `create_stream` |
| `("paused", stream_id)` | `StreamEvent::Paused(stream_id)` | `pause_stream` / `pause_stream_as_admin` |
| `("resumed", stream_id)` | `StreamEvent::Resumed(stream_id)` | `resume_stream` / `resume_stream_as_admin` |
| `("cancelled", stream_id)` | `StreamEvent::Cancelled(stream_id)` | `cancel_stream` / `cancel_stream_as_admin` |
| `("withdrew", stream_id)` | `withdrawable` (i128) | `withdraw` |

---

## 6. Error Codes (Panic Messages)

All failures use `panic!` / `assert!`. No custom error enum.

| Message | Function | Trigger |
|---------|----------|---------|
| `"already initialised"` | `init` | Re-init attempt |
| `"deposit_amount must be positive"` | `create_stream` | deposit_amount <= 0 |
| `"rate_per_second must be positive"` | `create_stream` | rate_per_second <= 0 |
| `"sender and recipient must be different"` | `create_stream` | sender == recipient |
| `"start_time must be before end_time"` | `create_stream` | start_time >= end_time |
| `"cliff_time must be within [start_time, end_time]"` | `create_stream` | cliff out of range |
| `"deposit_amount must cover total streamable amount (rate * duration)"` | `create_stream` | underfunded |
| `"overflow calculating total streamable amount"` | `create_stream` | overflow in rate * duration |
| `"stream not found"` | Various | Invalid stream_id |
| `"stream is already paused"` | `pause_stream` | Double pause |
| `"stream must be active to pause"` | `pause_stream` | Pause non-active stream |
| `"stream is active, not paused"` | `resume_stream` | Resume active stream |
| `"stream is completed"` | `resume_stream` | Resume completed |
| `"stream is cancelled"` | `resume_stream` | Resume cancelled |
| `"stream must be active or paused to cancel"` | `cancel_stream` / `cancel_stream_as_admin` | Cancel completed/cancelled |
| `"stream already completed"` | `withdraw` | Withdraw from completed |
| `"cannot withdraw from paused stream"` | `withdraw` | Withdraw while paused |
| `"nothing to withdraw"` | `withdraw` | accrued == withdrawn_amount |
| `"stream is not active"` | `pause_stream_as_admin` | Admin pause non-active |
| `"stream is not paused"` | `resume_stream_as_admin` | Admin resume non-paused |
| `"contract not initialised: missing config"` | Functions requiring config | Config missing |

## Error Reference
For a full list of contract errors, see [error.md](./error.md).

