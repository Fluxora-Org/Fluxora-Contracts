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
| `ContractPaused` | 4 | Global emergency pause or creation pause is active | `create_stream`, `create_streams`, `create_streams_partial`, `withdraw`, `withdraw_to`, `batch_withdraw`, `cancel_stream`, `top_up_stream`, `update_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time`, `update_recipient`, `trigger_auto_claim` |
| `StartTimeInPast` | 5 | `start_time` is before the current ledger timestamp | `create_stream`, `create_streams`, `create_streams_partial` |
| `ArithmeticOverflow` | 6 | Arithmetic overflow in stream calculations | `create_stream`, `create_streams`, `create_streams_partial`, `update_rate_per_second`, `top_up_stream`, `shorten_stream_end_time`, `extend_stream_end_time` |
| `Unauthorized` | 7 | Caller is not authorized to perform this operation | `init`, `set_admin`, `cancel_stream`, `top_up_stream`, `withdraw` (recipient check) |
| `AlreadyInitialised` | 8 | Contract has already been initialized | `init` |
| `InsufficientBalance` | 9 | Token transfer failed due to insufficient balance or allowance | `create_stream`, `create_streams_partial`, `cancel_stream`, `withdraw`, `top_up_stream` |
| `InsufficientDeposit` | 10 | Deposit amount does not cover the planned duration at the specified rate | `create_stream`, `create_streams`, `update_rate_per_second`, `extend_stream_end_time` |
| `StreamAlreadyPaused` | 11 | Stream is already in `Paused` state | `pause_stream`, `pause_stream_as_admin` |
| `StreamNotPaused` | 12 | Stream is not `Paused`; cannot resume an `Active` stream | `resume_stream`, `resume_stream_as_admin` |
| `StreamTerminalState` | 13 | Stream is `Completed` or `Cancelled`; modification blocked | `pause_stream`, `resume_stream`, admin overrides |
| `DuplicateStreamId` | 14 | Duplicate stream IDs supplied to a batch operation | `batch_withdraw` |
| `InvalidSignature` | 15 | Delegated withdrawal signature is invalid, expired, or nonce mismatch | `delegated_withdraw` |
| `BelowMinimumAmount` | 16 | Withdrawable amount is below the `expected_minimum_amount` committed in the signature | `delegated_withdraw` | `RateTooLow` | 17 | rate_per_second < MIN_RATE_PER_SECOND (dust-attack prevention) | create_stream, create_streams, create_stream_relative, create_streams_relative, create_stream_from_template |

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
| Sender | Yes | `create_stream` blocked if EITHER pause mode is active. `cancel`/`update` blocked ONLY if Global Emergency Pause is active. |
| Recipient | Yes | `withdraw` blocked ONLY if Global Emergency Pause is active. |
| Admin | No | Admin operations (pause/resume/init) are never blocked by the pause flag. |

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

**Integrator Note**: During any pause, `calculate_accrued` and `get_stream_state` remain functional.
Recipients can always check their balance.
- If `is_creation_paused()` is true: Only NEW stream creation is blocked.
- If `is_global_emergency_paused()` is true: All mutations (creation, withdrawal, cancellation) are blocked.
Use `is_paused()` (checks both) or inspect `get_pause_info()` for full details.

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

### InvalidSignature (15)

**Definition**: Delegated withdrawal signature is invalid, expired, or nonce mismatch.

**Trigger Conditions**:
- `delegated_withdraw` called with an invalid ed25519 signature
- Signature has expired (timestamp check failed)
- Nonce mismatch (replay protection)
- Signature does not match the expected payload structure

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Relayer | Yes | Invalid signature from recipient |
| Recipient | Yes | Expired or replayed signature |

**Client Action**:
```rust
match client.try_delegated_withdraw(&relayer, &stream_id, &signature, &nonce, &expected_minimum) {
    Ok(amount) => { /* success */ }
    Err(ContractError::InvalidSignature) => {
        // Signature validation failed
        // Check: signature is valid ed25519, nonce is current, not expired
        // Request new signature from recipient
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns positive `i128` amount withdrawn.

---

### BelowMinimumAmount (16)

**Definition**: Withdrawable amount is below the `expected_minimum_amount` committed in the signature.

**Trigger Conditions**:
- `delegated_withdraw` called when accrued amount is less than the `expected_minimum_amount` specified in the signed payload
- Protects recipient from relayer front-running or timing issues

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Relayer | Yes | Attempting withdrawal before sufficient accrual |
| Recipient | No | Recipient sets the minimum in signature |

**Client Action**:
```rust
match client.try_delegated_withdraw(&relayer, &stream_id, &signature, &nonce, &expected_minimum) {
    Ok(amount) => { /* success */ }
    Err(ContractError::BelowMinimumAmount) => {
        // Accrued amount is below expected minimum
        // Wait for more accrual or request new signature with lower minimum
        let current_accrued = client.calculate_accrued(&stream_id)?;
        // Retry when current_accrued >= expected_minimum
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns positive `i128` amount withdrawn (>= expected_minimum).

---

### RateTooLow (17) — Dust-Attack Prevention
Introduced: Issue #576
Discriminant: 17
Affected entrypoints: All stream creation functions

Trigger
    `rate_per_second < MIN_RATE_PER_SECOND  // MIN_RATE_PER_SECOND = 100`

   ## Rationale
Streams with very low rate_per_second (e.g. 1 stroop/second) accrue imperceptibly slowly while occupying persistent ledger storage for years. This creates several problems:
1 Ledger bloat: Each stream consumes a persistent storage entry (~200+ bytes) that must be kept alive via TTL bumps.
2 Recipient index pollution: Low-value streams inflate RecipientStreams indices, increasing query costs for legitimate users.
3 Griefing vector: An attacker could create thousands of 1-stroop streams to a victim's address, making their recipient index unusable.
4 Indexer overhead: Indexers must track and process these streams indefinitely.

`MIN_RATE_PER_SECOND = 100`

At 100 stroops/second (~0.00001 USDC/sec at 7 decimals):
1 day accrual = 8,640,000 stroops = 0.864 USDC
1 year accrual = ~3,154,000,000 stroops = ~315 USDC
This is low enough for legitimate micro-streams (testnet faucets, small grants) while high enough to prevent state-bloat attacks.

## Fix
Increase rate_per_second to at least 100:

// Before (fails with RateTooLow)
rate_per_second: 1,

// After (succeeds)
rate_per_second: 100,  // At minimum threshold

## Integration Notes
- Frontends should validate rate_per_second >= 100 client-side before submitting transactions.
- Treasury tools should reject stream proposals with - rates below the minimum.
- The minimum is a compile-time constant; changing it requires a contract redeployment.
- Spikes in RateTooLow errors in logs may indicate a dust-attack attempt.

## Detailed Error Semantics
StreamNotFound (1)
Definition: The requested stream ID does not exist in contract storage.
Trigger Conditions:
stream_id is 0 or exceeds the current stream counter
Stream was never created
Stream was closed via close_completed_stream

# Client Action:
`match client.try_get_stream_state(&stream_id) {
    Ok(state) => { /* stream exists, use state */ }
    Err(ContractError::StreamNotFound) => {
        // Stream doesn't exist - check stream_id validity
        // Notify user or refresh stream list
    }
    Err(e) => { /* handle other errors */ }
}`

InvalidState (2)
Definition: Operation attempted in a state where it is not allowed.
Trigger Conditions:
| Scenario                                 | Description                    |
| ---------------------------------------- | ------------------------------ |
| Withdraw from Completed stream           | All funds already withdrawn    |
| Withdraw from non-terminal Paused stream | Must resume first              |
| Cancel Completed stream                  | Already terminal               |
| Top-up Completed/Cancelled stream        | Cannot modify terminal streams |

InvalidParams (3)
Definition: One or more input parameters are invalid.
Trigger Conditions:
| Parameter                         | Invalid When                                                |
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

# Client Action:
match client.try_create_stream(...) {
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
Trigger Conditions:
| Operation       | Authorization Requirement |
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
| `delegated_withdraw` | - | - | - | InvalidSignature, BelowMinimumAmount, StreamNotFound, InvalidState |
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
| DuplicateStreamId | `batch_withdraw` with repeated stream IDs |
| InvalidSignature | `delegated_withdraw` with invalid or expired signature |
| BelowMinimumAmount | `delegated_withdraw` when accrued < expected_minimum |

Discriminant stability is verified by `test_contract_error_discriminants_are_stable` in `contracts/stream/src/test.rs`, which asserts the exact `u32` value of every `ContractError` variant and will fail at compile time if any value is changed.

---

## FactoryError Reference

The factory contract (`contracts/factory`) uses a separate `FactoryError` enum.

| Error | Description |
|-------|-------------|
| `AlreadyInitialized` | Factory has already been initialized; `init` may only be called once |
| `NotInitialized` | Factory has not been initialized; call `init` first |
| `Unauthorized` | Caller is not the factory admin |
| `RecipientNotAllowlisted` | Recipient address is not on the factory allowlist |
| `DepositExceedsCap` | Requested deposit exceeds the per-stream cap configured in the factory |
| `DurationTooShort` | Stream duration is below the factory-enforced minimum |

---

## Scope

### Included

- All 14 `ContractError` variants
- Role-based error mapping
- Success/failure semantics for each operation
- Time-driven edge cases
- Client action recommendations
- Dust-attack prevention guidance

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
| Client mishandling | Medium | Medium | This documentation | Dust-attack bypass | Very Low |	High | MIN_RATE_PER_SECOND enforced at validation layer
