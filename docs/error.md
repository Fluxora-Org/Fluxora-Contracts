# ContractError: User-Facing Mapping for Clients

## Summary

This document provides a comprehensive mapping of `ContractError` variants to their semantic meaning,
trigger conditions, affected roles, and recommended client actions. Integrators (wallets, indexers,
treasury tooling) can use this reference to handle protocol exceptions correctly.

---

## Error Code Reference Table

| Error Code | Value | Description | Functions Returning It |
|------------|-------|-------------|------------------------|
| `StreamNotFound` | 1 | The specified stream does not exist | `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `calculate_accrued`, `get_stream_state`, admin overrides |
| `InvalidState` | 2 | Operation attempted in an invalid state | `cancel_stream`, `withdraw`, `withdraw_to`, `batch_withdraw`, `get_claimable_at`, admin overrides |
| `InvalidParams` | 3 | Function input parameters are invalid | `create_stream`, `withdraw_to`, `update_rate_per_second`, `top_up_stream`, `extend_stream_end_time`, `shorten_stream_end_time`, `batch_create_streams` |
| `ContractPaused` | 4 | Global emergency pause is active; mutations are blocked | `create_stream`, `create_streams`, `withdraw`, `withdraw_to`, `batch_withdraw`, `cancel_stream`, `top_up_stream`, `update_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time` |
| `StartTimeInPast` | 5 | `start_time` is before the current ledger timestamp | `create_stream`, `create_streams` |
| `ArithmeticOverflow` | 6 | Arithmetic overflow in stream calculations | `create_stream`, `create_streams`, `update_rate_per_second`, `top_up_stream`, `shorten_stream_end_time`, `extend_stream_end_time` |
| `Unauthorized` | 7 | Caller is not authorized to perform this operation | `init`, `set_admin`, `cancel_stream`, `top_up_stream`, `withdraw` (recipient check) |
| `AlreadyInitialised` | 8 | Contract has already been initialized | `init` |
| `InsufficientBalance` | 9 | Token transfer failed due to insufficient balance or allowance | `create_stream`, `cancel_stream`, `withdraw`, `top_up_stream` |
| `InsufficientDeposit` | 10 | Deposit amount does not cover the planned duration at the specified rate | `create_stream`, `create_streams`, `update_rate_per_second`, `extend_stream_end_time` |
| `StreamAlreadyPaused` | 11 | Stream is already in `Paused` state | `pause_stream`, `pause_stream_as_admin` |
| `StreamNotPaused` | 12 | Stream is not `Paused`; cannot resume an `Active` stream | `resume_stream`, `resume_stream_as_admin` |
| `StreamTerminalState` | 13 | Stream is `Completed` or `Cancelled`; modification blocked | `pause_stream`, `resume_stream`, admin overrides |
| `DuplicateStreamId` | 14 | Duplicate stream IDs supplied to a batch operation | `batch_withdraw` |

---

## Detailed Error Semantics

### StreamNotFound (1)

**Definition**: The requested stream ID does not exist in contract storage.

**Trigger Conditions**:
- `stream_id` is 0 or exceeds the current stream counter
- Stream was never created
- Stream ID was invalidated (rare, for admin interventions)

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Anyone | Yes | Permissionless read functions return this error |
| Recipient | Yes | `withdraw`, `get_stream_state` |
| Sender | Yes | `pause_stream`, `cancel_stream`, `top_up_stream` |
| Admin | Yes | `pause_stream_as_admin`, `cancel_stream_as_admin` |

**Client Action**:
```rust
match client.try_get_stream_state(&stream_id) {
    Ok(state) => { /* stream exists, use state */ }
    Err(ContractError::StreamNotFound) => {
        // Stream doesn't exist - check stream_id validity
        // Notify user or refresh stream list
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `StreamState` with valid fields.

---

### InvalidState (2)

**Definition**: Operation attempted in a state where it is not allowed.

**Trigger Conditions**:
| Scenario | Description |
|----------|-------------|
| Withdraw from Completed stream | All funds already withdrawn |
| Withdraw from non-terminal Paused stream | Must resume first |
| Cancel Completed stream | Already terminal |
| Top-up Completed/Cancelled stream | Cannot modify terminal streams |
| Admin resume when not globally paused | Emergency pause not active |

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Recipient | Yes | `withdraw` on wrong status |
| Sender | Yes | `cancel` on terminal stream |
| Admin | Yes | `resume_global_emergency_pause` when not paused |
| Anyone | No | Permissionless reads don't trigger |

**Client Action**:
```rust
match client.try_withdraw(&stream_id) {
    Ok(amount) => { /* success, update UI */ }
    Err(ContractError::InvalidState) => {
        let state = client.get_stream_state(&stream_id)?;
        match state.status {
            StreamStatus::Completed => "All funds withdrawn",
            StreamStatus::Paused => "Resume stream first",
            _ => "Contact support"
        }
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns positive `i128` amount (withdrawable balance).

---

### InvalidParams (3)

**Definition**: One or more input parameters are invalid.

**Trigger Conditions**:
| Parameter | Invalid When |
|-----------|--------------|
| `sender == recipient` | Sender and recipient addresses are identical |
| `deposit_amount <= 0` | Deposit must be positive |
| `rate_per_second <= 0` | Rate must be positive |
| `start_time >= end_time` | Start must be before end |
| `cliff_time < start_time` | Cliff cannot precede start |
| `cliff_time > end_time` | Cliff cannot follow end |
| `destination == contract_address` | Cannot withdraw to contract |
| `new_rate_per_second <= old_rate` | Rate can only increase |
| `new_rate_per_second <= 0` | Rate must be positive |
| `top_up_amount <= 0` | Top-up must be positive |
| `extend_end_time <= current_end_time` | New end must be later |
| `shorten_end_time >= current_end_time` | New end must be earlier |
| `shorten_end_time < current_ledger_timestamp` | Cannot shorten to past |

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | `create_stream`, `update_rate_per_second`, `top_up_stream` |
| Admin | Yes | `set_admin`, `init` (wrong config) |
| Anyone | Yes | Invalid addresses |

**Client Action**:
```rust
match client.try_create_stream(&sender, &recipient, &deposit, &rate, &start, &cliff, &end) {
    Ok(stream_id) => { /* success */ }
    Err(ContractError::InvalidParams) => {
        // Validate inputs locally before retrying
        // Check: sender != recipient, deposit > 0, rate > 0, start < end
        // cliff >= start, cliff <= end
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `u64` stream_id for create operations, `()` for updates.

---

### ContractPaused (4)

**Definition**: The protocol is globally paused. No new streams may be created.

**Trigger Conditions**:
- Admin called `set_global_emergency_paused(true)` or `set_contract_paused(true)`
- Contract is in global emergency pause or creation pause mode

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | Create streams blocked when paused |
| Recipient | No | Existing streams unaffected; can still withdraw |
| Admin | No | Admin operations exempt; admin can always pause/resume |

**Client Action**:
```rust
match client.try_create_stream(...) {
    Ok(stream_id) => { /* success */ }
    Err(ContractError::ContractPaused) => {
        // Notify user: "Protocol temporarily paused"
        // Check `is_paused()` for current status
        // Check `get_pause_info()` for reason and timestamp
        // Retry later or contact admin
        let info = client.get_pause_info();
        if let Some(ref reason) = info.reason {
            println!("Pause reason: {}", reason);
        }
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `u64` stream_id (when unpaused).

**Integrator Note**: During pause, `calculate_accrued` and `get_stream_state` remain functional.
Recipients can check their balance and withdraw from existing streams. Only NEW stream
creation is blocked. Use `is_paused()` for a quick check or `get_pause_info()` for full
audit trail (reason, timestamp, admin who paused).

---

### StartTimeInPast (5)

**Definition**: `start_time` is before the current ledger timestamp.

**Trigger Conditions**:
- `start_time < env.ledger().timestamp()` at creation time
- Stream cannot retroactively start

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | `create_stream`, `create_streams` |

**Client Action**:
```rust
let current_time = env.ledger().timestamp();
let start_time = calculate_future_start(current_time, delay_seconds);
match client.try_create_stream(..., &start_time, ...) {
    Ok(stream_id) => { /* success */ }
    Err(ContractError::StartTimeInPast) => {
        // Use current_time + 1 as start_time
        // Or schedule for future
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `u64` stream_id with future start_time.

---

### ArithmeticOverflow (6)

**Definition**: Arithmetic overflow in stream calculations.

**Trigger Conditions**:
| Calculation | Overflow Condition |
|-------------|-------------------|
| `rate * duration` | Result exceeds `i128::MAX` |
| `deposit + amount` (top-up) | Result exceeds `i128::MAX` |
| `duration` calculation | Overflow in u64 arithmetic |

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | Large deposit/rate combinations |
| Admin | Yes | Parameter adjustments |

**Client Action**:
```rust
match client.try_create_stream(..., &deposit, &rate, ...) {
    Ok(stream_id) => { /* success */ }
    Err(ContractError::ArithmeticOverflow) => {
        // Reduce deposit or rate
        // Break into multiple streams
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `u64` stream_id.

**Integrator Note**: The contract caps at `i128::MAX` which is ~1.7×10³⁸ for 18-decimal tokens.
This is effectively unlimited for any realistic token amount.

---

### Unauthorized (7)

**Definition**: Caller is not authorized to perform this operation.

**Trigger Conditions**:
| Operation | Authorization Requirement |
|-----------|---------------------------|
| `cancel_stream` | Caller is sender or admin |
| `top_up_stream` | Caller is sender or admin |
| `withdraw` | Caller is recipient |
| `init` | First caller only |
| `set_admin` | Current admin only |

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Recipient | Yes | `withdraw` when not recipient |
| Sender | Yes | `cancel` when not sender/admin |
| Third Party | Yes | Any unauthorized call |
| Admin | Yes (by others) | Wrong admin calling |

**Client Action**:
```rust
match client.try_withdraw(&stream_id) {
    Ok(amount) => { /* success */ }
    Err(ContractError::Unauthorized) => {
        // User is not the recipient
        // Check `get_stream_state` to verify recipient address
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns positive `i128` amount.

---

### AlreadyInitialised (8)

**Definition**: Contract has already been initialized.

**Trigger Conditions**:
- `init` called when `Config` already exists in storage
- Second initialization attempt

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Anyone | Yes | Only first `init` succeeds |

**Client Action**:
```rust
match client.try_init(&token, &admin) {
    Ok(()) => { /* success */ }
    Err(ContractError::AlreadyInitialised) => {
        // Contract already initialized - this is expected if already set up
        // Call `get_config` to verify configuration
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `()` on first initialization.

---

### InsufficientBalance (9)

**Definition**: Token transfer failed due to insufficient balance or allowance.

**Trigger Conditions**:
- Sender's token balance < deposit_amount
- Sender's token allowance < deposit_amount (if not unlimited)
- Insufficient balance during `cancel_stream` refund
- Insufficient balance during `top_up_stream`

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | Primary case |
| Admin | Yes | If admin funds streams |

**Client Action**:
```rust
match client.try_create_stream(...) {
    Ok(stream_id) => { /* success */ }
    Err(ContractError::InsufficientBalance) => {
        // Check token balance and allowance
        // Fund account or increase allowance
        let balance = token_client.balance(&sender);
        let allowance = token_client.allowance(&sender, &contract_address);
        // Notify user to fund account
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `u64` stream_id.

---

### InsufficientDeposit (10)

**Definition**: Deposit amount does not cover the planned duration at the specified rate.

**Trigger Conditions**:
| Condition | Formula |
|-----------|---------|
| New stream | `deposit < rate * (end - start)` |
| Rate update | `deposit < new_rate * remaining_duration` |
| Extend end time | `deposit < rate * new_total_duration` |

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | `create_stream`, `update_rate_per_second`, `extend_stream_end_time` |

**Client Action**:
```rust
let duration = end_time - start_time;
let minimum_deposit = rate_per_second * duration as i128;
match client.try_create_stream(..., &(minimum_deposit + 1), ...) {
    Ok(stream_id) => { /* success */ }
    Err(ContractError::InsufficientDeposit) => {
        // Increase deposit to minimum_deposit or higher
        // Or reduce rate or duration
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `u64` stream_id.

---

### StreamAlreadyPaused (11)

**Definition**: Stream is already in `Paused` state.

**Trigger Conditions**:
- `pause_stream` called on already-paused stream
- `pause_stream_as_admin` called on already-paused stream

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | `pause_stream` |
| Admin | Yes | `pause_stream_as_admin` |

**Client Action**:
```rust
match client.try_pause_stream(&stream_id) {
    Ok(()) => { /* success */ }
    Err(ContractError::StreamAlreadyPaused) => {
        // Stream already paused - this is idempotent
        // Check `get_stream_state` to confirm status
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `()`.

---

### StreamNotPaused (12)

**Definition**: Stream is not in `Paused` state.

**Trigger Conditions**:
- `resume_stream` called on `Active` stream (not paused)
- `resume_stream_as_admin` called on non-paused stream

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | `resume_stream` on active stream |
| Admin | Yes | `resume_stream_as_admin` on active stream |

**Client Action**:
```rust
match client.try_resume_stream(&stream_id) {
    Ok(()) => { /* success */ }
    Err(ContractError::StreamNotPaused) => {
        // Stream not paused - check status
        let state = client.get_stream_state(&stream_id)?;
        if state.status == StreamStatus::Active {
            // Already active, no action needed
        }
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `()`.

---

### StreamTerminalState (13)

**Definition**: Stream is in a terminal state (`Completed` or `Cancelled`).

**Trigger Conditions**:
| Status | Blocked Operations |
|--------|-------------------|
| Completed | `pause_stream`, `cancel_stream`, `top_up_stream`, `update_rate_per_second` |
| Cancelled | `pause_stream`, `resume_stream`, `cancel_stream`, `top_up_stream`, `update_rate_per_second` |

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | Attempting to modify terminal stream |
| Recipient | No | Read operations still work |
| Admin | Yes | Admin overrides also blocked |

**Client Action**:
```rust
match client.try_pause_stream(&stream_id) {
    Ok(()) => { /* success */ }
    Err(ContractError::StreamTerminalState) => {
        let state = client.get_stream_state(&stream_id)?;
        match state.status {
            StreamStatus::Completed => "Stream fully vested",
            StreamStatus::Cancelled => "Stream cancelled",
            _ => "Unexpected state"
        }
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `()`.

---

### DuplicateStreamId (14)

**Definition**: Duplicate stream IDs were supplied to a batch operation.

**Trigger Conditions**:
- `batch_withdraw` called with a `stream_ids` vector containing the same ID more than once

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Recipient | Yes | `batch_withdraw` with repeated IDs |

**Client Action**:
```rust
match client.try_batch_withdraw(&recipient, &stream_ids) {
    Ok(results) => { /* success */ }
    Err(ContractError::DuplicateStreamId) => {
        // Deduplicate stream_ids before retrying
        // Use a set to ensure uniqueness
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `Vec<BatchWithdrawResult>` with unique entries.

---

## Previously Panicking Paths (Now Structured Errors)

The following input-error paths previously caused a host-level panic. They now return
structured `ContractError` variants so clients can handle them programmatically:

| Former Panic | Now Returns | Functions |
|---|---|---|
| `panic_with_error!(ContractPaused)` in `require_not_globally_paused` | `ContractError::ContractPaused` | `withdraw`, `withdraw_to`, `batch_withdraw`, `cancel_stream`, `update_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time` |
| `panic_with_error!(ArithmeticOverflow)` in batch deposit sum | `ContractError::ArithmeticOverflow` | `create_streams` |
| `panic_with_error!(ArithmeticOverflow)` in rate × duration | `ContractError::ArithmeticOverflow` | `update_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time` |
| `assert!("batch_withdraw stream_ids must be unique")` | `ContractError::DuplicateStreamId` | `batch_withdraw` |

---

## Panic Messages (Non-Error Results)

These are runtime panics that should not occur in normal operation and represent
infrastructure-level failures (not user input errors):

| Panic Message | Cause | Client Action |
|---------------|-------|---------------|
| `contract not initialised: missing config` | Storage access before `init` | Call `init` first |

---

## Role-Based Error Matrix

| Operation | Recipient | Sender | Admin | Anyone |
|-----------|-----------|--------|-------|--------|
| `create_stream` | - | InvalidParams, InsufficientBalance, InsufficientDeposit | - | - |
| `pause_stream` | - | StreamNotFound, Unauthorized, StreamAlreadyPaused, StreamTerminalState | Same + StreamNotFound | StreamNotFound |
| `resume_stream` | - | StreamNotFound, Unauthorized, StreamNotPaused, StreamTerminalState | Same + StreamNotFound | StreamNotFound |
| `cancel_stream` | - | StreamNotFound, Unauthorized, InvalidState | StreamNotFound, Unauthorized | - |
| `withdraw` | StreamNotFound, Unauthorized, InvalidState | - | - | - |
| `top_up_stream` | - | StreamNotFound, Unauthorized, InvalidParams, InvalidState, ArithmeticOverflow | StreamNotFound | - |
| `calculate_accrued` | StreamNotFound | StreamNotFound | StreamNotFound | StreamNotFound |
| `get_stream_state` | StreamNotFound | StreamNotFound | StreamNotFound | StreamNotFound |

---

## Edge Cases: Time-Driven Errors

| Edge Case | Error | Condition |
|-----------|-------|-----------|
| Stream past end_time | InvalidState | `withdraw` on completed stream |
| Stream at exact end_time | Success | Full withdrawal allowed |
| Stream before cliff | InvalidState | `withdraw` returns 0 |
| Stream at exact cliff | Success | Accrual begins (from start_time) |
| Future start_time | Success | Stream created but no accrual yet |
| Cancel before cliff | Success | Full refund (accrued = 0) |
| Cancel after end_time | InvalidState | No refund (accrued = deposit) |

---

## Testing Coverage

Error handling is verified by tests in `contracts/stream/src/test.rs`:

| Error | Test Pattern |
|-------|--------------|
| StreamNotFound | `try_get_stream_state` with invalid ID |
| InvalidParams | `try_create_stream` with `sender == recipient`, `deposit <= 0`, etc. |
| ContractPaused | Global pause then create |
| Unauthorized | Wrong recipient `try_withdraw` |
| InsufficientBalance | Sender with no tokens |
| InsufficientDeposit | `deposit < rate * duration` |
| StreamTerminalState | Pause/complete then modify |
| AutoClaimNotSet | `try_trigger_auto_claim` without prior `set_auto_claim` |

Discriminant stability is verified by `test_contract_error_discriminants_are_stable` in `contracts/stream/src/test.rs`, which asserts the exact `u32` value of every `ContractError` variant and will fail at compile time if any value is changed.

---

## Scope

### Included

- All 14 `ContractError` variants
- Role-based error mapping
- Success/failure semantics for each operation
- Time-driven edge cases
- Client action recommendations

### Excluded

| Exclusion | Rationale | Residual Risk |
|-----------|-----------|---------------|
| Token-specific errors | Delegated to token contract | Low - caught by `InsufficientBalance` |
| Gas budget errors | Soroban runtime errors | Low - indicates contract size issues |
| Storage serialization errors | Runtime infrastructure | Very Low |

---

## Residual Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Error code changes | Low | High | Versioning in client SDKs |
| Missing error cases | Low | Medium | Comprehensive test coverage |
| Client mishandling | Medium | Medium | This documentation |
