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
| `BelowMinimumAmount` | 16 | Withdrawable amount is below the `expected_minimum_amount` committed in the signature | `delegated_withdraw` |
| `ReservationCountZero` | 17 | ID reservation count is zero | `reserve_stream_ids` |
| `ReservationLimitExceeded` | 18 | ID reservation count exceeds `MAX_ID_RESERVATION` | `reserve_stream_ids` |
| `SignatureDeadlineExpired` | 19 | Delegated withdrawal signature deadline has passed | `delegated_withdraw` |
| `TemplateNotFound` | 20 | Requested stream template does not exist | `get_stream_template`, `create_stream_from_template`, `delete_stream_template` |
| `TemplateLimitExceeded` | 21 | Per-owner or global template limit would be exceeded | `register_stream_template` |
| `TemplateUnauthorized` | 22 | Caller is not authorized to delete a template | `delete_stream_template` |
| `TokenVerificationFailed` | 23 | Token contract does not expose the expected SEP-41 interface during init | `init` |
| `ReservationNotFound` | 24 | No ID reservation exists for the specified holder | `release_id_reservation`, `reclaim_expired_id_reservation` |
| `ReservationStillActive` | 25 | Reservation has not yet expired and cannot be reclaimed | `reclaim_expired_id_reservation` |
| `ReservationNotExpirable` | 26 | Reservation has no expiry and cannot be reclaimed | `reclaim_expired_id_reservation` |
| `ReservationAlreadyActive` | 34 | A reservation is already active for this caller | `reserve_stream_ids` |
| `PauseReasonTooLong` | 27 | Pause reason string exceeds `MAX_PAUSE_REASON_BYTES` | `pause_protocol` |
| `ClockRegression` | 28 | Ledger-backed accrual observed a timestamp lower than the previous accrual timestamp | `calculate_accrued`, `get_withdrawable`, `withdraw`, `withdraw_to`, `batch_withdraw`, `batch_withdraw_to`, rate changes, `cancel_stream`, auto-claim paths |
| `MetadataTooLarge` | 29 | Stream metadata exceeds size limits | `create_stream`, `create_streams`, `create_streams_partial` |
| `RateCapExceeded` | 30 | Rate per second exceeds the configured maximum | `create_stream`, `update_rate_per_second` |
| `PauseCooldownActive` | 32 | Stream pause cooldown period is still active | `pause_stream` |
| `WithdrawalTooFrequent` | 33 | Withdrawal attempted before minimum interval elapsed | `withdraw`, `delegated_withdraw`, `batch_withdraw` |
| `KeeperGracePeriodNotElapsed` | 34 | Keeper cancellation grace period has not elapsed | `keeper_cancel` |
| `InvalidDustThreshold` | 35 | Withdraw dust threshold is negative or exceeds deposit amount | `create_stream`, `create_streams`, `create_streams_partial`, `create_stream_relative`, `create_stream_from_template` |

Non-error enum values used by stream creation and accrual:

| Enum | Value | Meaning |
|------|-------|---------|
| `Linear` | 0 | A `StreamKind` that accrues continuously over time after the start time. |
| `CliffOnly` | 1 | A `StreamKind` that unlocks the full deposit at the cliff time in one step. |

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

### ClockRegression (28)

**Definition**: Ledger-backed accrual observed a ledger timestamp lower than the previous accrual timestamp stored for the contract instance.

**Trigger Conditions**:
- Test harness sets `ledger().timestamp()` backwards after a prior accrual calculation
- Migration or environment change provides retrograde timestamps to accrual paths

**Client Action**: Treat as an infrastructure or test-environment failure. Do not retry at the lower timestamp; restore monotonic ledger time and rerun the transaction.

**Success Semantics**: No stream state is changed when the guard returns this error before withdrawable math.

---

### ReservationCountZero (17)

**Definition**: `reserve_stream_ids` was called with `count = 0`.

**Client Action**: Request at least one ID before reserving, or skip the reservation call when there are no streams to pre-allocate.

---

### ReservationLimitExceeded (18)

**Definition**: `reserve_stream_ids` was called with `count > MAX_ID_RESERVATION`.

**Client Action**: Split large batches into reservations of at most `MAX_ID_RESERVATION` IDs.

---

### SignatureDeadlineExpired (19)

**Definition**: A delegated withdrawal signature is structurally valid but its signed deadline has passed.

**Client Action**: Ask the recipient to sign a fresh delegated withdrawal payload with a later deadline.

---

### TemplateNotFound (20)

**Definition**: The requested stream template is not present in storage.

**Client Action**: Refresh the template list before retrying, or register the template before creating streams from it.

---

### TemplateLimitExceeded (21)

**Definition**: Registering a template would exceed either the per-owner or global template limit.

**Client Action**: Delete unused templates or reuse an existing template instead of registering another one.

---

### TemplateUnauthorized (22)

**Definition**: A caller attempted to delete or manage a template they do not own.

**Client Action**: Switch to the template owner account or leave the template unchanged.

---

### TokenVerificationFailed (23)

**Definition**: During initialization, the configured token contract did not expose the expected SEP-41 interface.

**Client Action**: Verify the token address and deploy/init against a compatible token contract before retrying.

---

### PauseReasonTooLong (27)

**Definition**: `pause_protocol` received a reason string longer than `MAX_PAUSE_REASON_BYTES`.

**Client Action**: Shorten the operator-facing pause reason and retry the pause transaction.

---

### InvalidDustThreshold (35)

**Definition**: Withdraw dust threshold is negative or exceeds deposit amount.

**Trigger Conditions**:
| Parameter | Invalid When |
|-----------|--------------|
| `withdraw_dust_threshold < 0` | Threshold must be non-negative |
| `withdraw_dust_threshold > deposit_amount` | Threshold cannot exceed total deposit |

**Affected Roles**:
| Role | Can Trigger | Notes |
|------|------------|-------|
| Sender | Yes | `create_stream`, `create_streams`, `create_streams_partial`, `create_stream_relative`, `create_stream_from_template` |

**Client Action**:
```rust
match client.try_create_stream(..., &withdraw_dust_threshold, ...) {
    Ok(stream_id) => { /* success */ }
    Err(ContractError::InvalidDustThreshold) => {
        // Ensure withdraw_dust_threshold >= 0
        // Ensure withdraw_dust_threshold <= deposit_amount
        // withdraw_dust_threshold == deposit_amount is allowed (boundary case)
    }
    Err(e) => { /* handle other errors */ }
}
```

**Success Semantics**: Returns `u64` stream_id with valid dust threshold.

**Integrator Note**: The dust threshold enforces a minimum withdrawable amount to prevent dust accumulation. The threshold must be in the range `[0, deposit_amount]`. When `withdraw_dust_threshold == deposit_amount`, withdrawals are only allowed when the full deposit is withdrawable (e.g., at stream end or after final drain).

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
| `delegated_withdraw` | - | - | - | InvalidSignature, BelowMinimumAmount, StreamNotFound, InvalidState |
| `top_up_stream` | - | StreamNotFound, Unauthorized, InvalidParams, InvalidState, ArithmeticOverflow, `[UnsupportedStreamKind](#unsupportedstreamkind-17)` | StreamNotFound | - |
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
| Retrograde ledger timestamp | ClockRegression | `ledger().timestamp()` < previous accrual timestamp in test/debug builds |

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
| ClockRegression | `clock_monotonicity.rs` seeds non-monotonic ledger timestamps |

Discriminant stability is verified by `test_contract_error_discriminants_are_stable` in `contracts/stream/src/test.rs`, which asserts the exact `u32` value of every `ContractError` variant and will fail at compile time if any value is changed.

---

## FactoryError Reference (Factory Contract)

The factory contract (`contracts/factory/src/lib.rs`) uses a dedicated `FactoryError`
enum that is independent of `FluxoraStream::ContractError`. Wallets, indexers, and
treasury tooling that interact with factory-routed stream creation MUST map these
discriminants (not the stream contract's) when decoding factory invocation failures.

> **Source of truth:** `contracts/factory/src/lib.rs` (`pub enum FactoryError`).
> The exact `u32` discriminants below are assertions-verified at compile time by
> `test_factory_error_discriminants_are_stable` in
> `contracts/factory/tests/factory_error_discriminants.rs`. If a discriminant
> changes without a coordinated docs PR, that test fails CI.

### Discriminant Table

Discriminants are stable, append-only, and must never be reordered. New variants
must be appended at the end so existing on-chain error mappings stay byte-identical.

| Discriminant | Variant | Triggering Condition | Functions Returning It |
|---:|---|---|---|
| 1 | `AlreadyInitialized` | `init` called when instance already has an `Admin` key | `init` |
| 2 | `NotInitialized` | A required instance config key (`Admin`, `StreamContract`, `MaxDepositCap`, `MinDuration`, `BatchCapEnforced`) is missing | `get_factory_config`, `set_*` setters, `create_stream`, `create_streams` |
| 3 | `Unauthorized` | **Reserved / forward-only.** No factory entry point currently constructs `FactoryError::Unauthorized`; every admin-only setter routes auth through `require_admin → admin.require_auth()`, producing either a Soroban auth revert (panic) or `NotInitialized` (2). Code 3 is retained in the enum so that future typed-auth paths and out-of-band error mirrors keep a stable discriminant. Clients should treat a non-admin auth failure on setters as a Soroban auth revert, not a typed enum value. | (reserved — see left column) |
| 4 | `RecipientNotAllowlisted` | `recipient` has no persistent allowlist entry | `create_stream`, `create_streams` |
| 5 | `DepositExceedsCap` | `deposit_amount > max_deposit` (per-entry) OR running batch-deposit sum would exceed `max_deposit` while `BatchCapEnforced = true` | `create_stream`, `create_streams` |
| 6 | `DurationTooShort` | `end_time - start_time < min_duration` | `create_stream`, `create_streams` |
| 7 | `InvalidTimeRange` | `start_time >= end_time` | `create_stream`, `create_streams` |
| 8 | `InvalidCliff` | `cliff_time < start_time` OR `cliff_time > end_time` (cliff must be inside the inclusive start/end window) | `create_stream`, `create_streams` |
| 9 | `CreationPaused` | `DataKey::CreationPaused == true` (factory-level pause); checked first, before any policy/allowlist read | `create_stream`, `create_streams` |
| 10 | `StreamContractPaused` | Downstream `FluxoraStream` returned `ContractError::ContractPaused` (creation pause active on the stream contract) | `create_stream` |
| 11 | `StreamContractError` | **Cross-contract failure wrapper**: downstream `FluxoraStream` rejected creation for any other reason (typed error OR transport-level panic). Also reused by `set_rate_bounds` when `min`/`max` are negative or `min > max`. See [Wrapper Semantics](#streamcontracterror-11-wrapper-semantics) below. | `create_stream` (catch-all), `set_rate_bounds` |
| 12 | `RateBelowMin` | `rate_per_second < MinRatePerSecond` and a min bound is configured (bounds are inclusive) | `create_stream`, `create_streams` |
| 13 | `RateAboveMax` | `rate_per_second > MaxRatePerSecond` and a max bound is configured (bounds are inclusive) | `create_stream`, `create_streams` |
| 14 | `InvalidCap` | `max_deposit <= 0`; accepted range is `1..=i128::MAX` | `init`, `set_cap` |
| 15 | `InvalidMinDuration` | `min_duration > MAX_MIN_DURATION_SECONDS` (≈ 3_153_600_000, i.e. 100 years × 365 days); accepted range is `0..=MAX_MIN_DURATION_SECONDS` | `init`, `set_min_duration` |
| 16 | `InvalidMemo` | `memo.len() > fluxora_stream::MAX_MEMO_BYTES` | `create_stream`, `create_streams` |

**Range constants referenced above:**

- `MAX_MIN_DURATION_SECONDS = 100 * 365 * 24 * 60 * 60 = 3_153_600_000` (~100 years, defined in `contracts/factory/src/lib.rs`).
- `MAX_MEMO_BYTES` is shared with the stream contract and trimmed to fit in the `soroban_sdk::Bytes` budget.

### StreamContractError (11) Wrapper Semantics

`StreamContractError` is **not** a typed fanned-out error from the downstream contract — it
is a single factory-side variant that fires whenever `FluxoraStream::try_create_stream`
returns any non-`ContractPaused` failure (including transport-level panics). Clients should
treat code 11 as **"factory routed to stream contract and got back something other than
ContractPaused"** and re-check the stream contract's `get_pause_info()` / `get_config()`
to find the actual underlying reason.

Concretely, the four `try_create_stream` result arms collapse to factory-side codes:

| `try_create_stream` result | Factory-side code |
|---|---|
| `Ok(Ok(stream_id))` | success |
| `Err(Ok(ContractError::ContractPaused))` | `StreamContractPaused` (10) |
| `Err(Ok(stream_contract_err))` for any other typed stream error | `StreamContractError` (11) |
| `Err(Err(_))` transport/host error | `StreamContractError` (11) |
| `Ok(Err(_))` (defensive arm; not expected in practice) | `StreamContractError` (11) |

Because the factory cannot forward the exact stream-side discriminant, clients that need
the precise stream-contract reason must also query the stream contract directly via the
stream contract's `docs/error.md` table. `set_rate_bounds` reuses variant 11 as a catch-all
for negative `min_rate`/`max_rate` arguments and `min > max` invariants; treat those
administrative uses as "configuration rejected by factory guard" rather than as a
cross-contract passthrough.

### Discriminant Stability Test (CI-friendly)

The companion test
`contracts/factory/tests/factory_error_discriminants.rs::test_factory_error_discriminants_are_stable`
asserts every `FactoryError as u32` value listed above. It is intentionally
`soroban_sdk::testutils`-free, runs in CI without external state, and fails fast
if a discriminant is unintentionally reordered or reassigned. Update both the test
and this table together when adding new variants.

### FactoryError Role-Based Error Matrix

| Operation | Recipient | Sender | Admin | Anyone |
|-----------|-----------|--------|-------|--------|
| `init` | - | - | AlreadyInitialized, InvalidCap, InvalidMinDuration | - |
| `create_stream` | - | RecipientNotAllowlisted, DepositExceedsCap, InvalidTimeRange, InvalidCliff, DurationTooShort, RateBelowMin, RateAboveMax, InvalidMemo, StreamContractPaused, StreamContractError | - | - |
| `create_streams` | - | RecipientNotAllowlisted, DepositExceedsCap, InvalidTimeRange, InvalidCliff, DurationTooShort, RateBelowMin, RateAboveMax, InvalidMemo, CreationPaused | - | - |
| `set_admin` / setters | - | - | Unauthorized, InvalidCap, InvalidMinDuration, StreamContractError | `NotInitialized` for views |
| `set_factory_paused` | - | - | Unauthorized, NotInitialized | - |
| `get_factory_config` / views | - | - | - | NotInitialized |

Any setter called before `init` returns `NotInitialized` (2). Non-admin auth attempts on
admin-only setters hit a Soroban host-level auth revert (panic), not a typed `Unauthorized`
result — see code 3 above.

### Edge Cases

| Edge Case | Error | Condition |
|-----------|-------|-----------|
| `create_stream` while factory paused | `CreationPaused` | `DataKey::CreationPaused == true`; checked before any policy read |
| `create_stream` while stream contract paused (different pause flag) | `StreamContractPaused` | downstream `ContractPaused` propagated from fluxora_stream |
| Deposit exactly at cap | success | boundary inclusive (`> max_deposit` is the rejection condition) |
| Duration exactly at `min_duration` | success | boundary inclusive (`< min_duration` is the rejection condition) |
| Cliff equal to `start_time` | success | `start <= cliff <= end` (inclusive on both sides) |
| Cliff equal to `end_time` | success | same |
| Zero `min_duration` in `init`/`set_min_duration` | success | `0` is accepted; disables factory-level minimum |
| `min_duration` at exactly `MAX_MIN_DURATION_SECONDS` | success | boundary inclusive |
| Downstream contract error other than `ContractPaused` | `StreamContractError` (11) | catch-all wrapper — see above |
| Memo at exactly `MAX_MEMO_BYTES` | success | `>` comparison used in source |
| Rate exactly at `MinRatePerSecond` / `MaxRatePerSecond` | success | bounds are inclusive |

---

---

## Cross-File Discriminant-Collision Audit

### Shared-Decoder Finding

`ContractError` (stream contract), `FactoryError` (factory contract), and
`GovernanceError` (governance contract) are **three independent
`#[contracterror] #[repr(u32)]` enums**, each compiled into a separate Soroban
contract binary. Soroban returns numeric error codes scoped to the invoking
contract's XDR context, so **no shared runtime decoder exists today**: a wallet
or indexer receives the raw `u32` together with the contract address that
produced it, and must route decoding through the correct enum based on that
address.

**Verified absence of a shared decoder (as of 2026-07-22):**

| Component | How errors are decoded |
|-----------|----------------------|
| Stream contract clients | Map `u32` → `ContractError` variant using stream contract address context |
| Factory contract clients | Map `u32` → `FactoryError` variant using factory contract address context |
| Governance contract clients | Map `u32` → `GovernanceError` variant using governance contract address context |
| Off-chain indexer / SDK | Must branch on `invoking_contract_address` **before** looking up the numeric code; a single flat code-to-message table shared across all three contracts would cause silent misclassification |

**Residual risk**: If a future SDK or indexer merges all three error namespaces
into one flat `code → message` lookup (without first routing by contract
address), numeric overlaps between sections become silent misclassifications.
The automated cross-check script `script/check-discriminant-collisions.py`
reports such overlaps on every CI run so they are never silently introduced.

### Automated Audit Script

`script/check-discriminant-collisions.py` parses all three discriminant tables
in this file on every CI run and reports:

| Finding type | Behaviour |
|-------------|-----------|
| **Intra-section collision** — two different variant names share the same code within one enum | Script exits **1** (hard failure). This is always a documentation error; Rust/Soroban forbids duplicate discriminants at the source level. |
| **Cross-section overlap** — the same numeric code appears in more than one enum section | Script exits **0** (warning only). Overlap is harmless when a shared decoder routes by contract address first. |
| **Out-of-order entries** — discriminants not monotonically increasing within a section | Script exits **0** (maintenance warning). Indicates a likely cut-paste error in the table. |

Run locally:

```bash
python3 script/check-discriminant-collisions.py
# or point at a different docs path:
python3 script/check-discriminant-collisions.py --docs path/to/error.md
```

The companion test suite is at `tests/test_check_discriminant_collisions.py`
(≥95% coverage, runs in the `docs-alignment-check` CI job).

### Companion Discriminant-Stability Tests

Both Rust-level discriminant-stability tests are wired into CI:

| Test | File | CI job |
|------|------|--------|
| `test_contract_error_discriminants_are_stable` | `contracts/stream/src/test.rs` | `test` job — `cargo test --workspace` (hard gate) |
| `test_factory_error_discriminants_are_stable` | `contracts/factory/tests/factory_error_discriminants.rs` | `test` job — `cargo test -p fluxora_factory` (explicit, hard gate) |

The factory test is intentionally `soroban_sdk::testutils`-free so it runs in
CI without any ledger or token deployment.

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

## GovernanceError Reference

The governance contract (`contracts/governance/src/lib.rs`) uses a separate `GovernanceError`
enum (annotated `#[contracterror] #[repr(u32)]`). Multisig operators and UI clients decode
raw numeric error codes returned by governance entrypoints using the table below.

### Quick-reference table

> **Recoverability key**
> - ✅ **Recoverable** — Retry later or with corrected inputs; no permanent state damage.
> - ❌ **Terminal** — The proposal (or contract) is permanently in an unrecoverable state for that action; do not retry the same call.
> - ⚠️ **Config fix needed** — Requires an admin or governance action before retrying.

| Code | Variant | Description | Raising entrypoint(s) | Recoverable? |
|-----:|---------|-------------|----------------------|:------------:|
| 1 | `NotInitialized` | Contract has not been initialised; required storage is absent. | `get_admin`, `get_threshold`, `get_signers`, `get_signer_index` (called internally by every mutating entrypoint) | ⚠️ Config fix needed — call `init` |
| 2 | `AlreadyInitialized` | Contract is already initialised; `init` may only be called once. | `init` | ❌ Terminal for this `init` call |
| 3 | `Unauthorized` | Caller is not the contract admin. | `set_admin`, `add_signer`, `remove_signer` | ⚠️ Config fix needed — retry from admin wallet |
| 4 | `NotASigner` | Caller is not a registered co-signer. | `propose`, `approve` | ⚠️ Config fix needed — admin must call `add_signer` |
| 5 | `ProposalNotFound` | No proposal exists with the supplied ID. | `get_proposal`, `approve`, `execute`, `cancel_proposal`, `is_executable`, `get_quorum_info` | ✅ Recoverable — verify ID from `ProposalCreated` event |
| 6 | `AlreadyExecuted` | Proposal has already been executed. | `approve`, `execute`, `cancel_proposal` | ❌ Terminal — proposal is complete |
| 7 | `QuorumNotReached` | Approval count is below the required threshold, or `QuorumInfo` entry is absent. | `execute` | ✅ Recoverable — collect more signer approvals |
| 8 | `TimelockNotElapsed` | Quorum was reached but `GOVERNANCE_TIMELOCK_SECONDS` (48 h) have not yet passed. | `execute` | ✅ Recoverable — retry after `executable_after` timestamp in `QuorumReached` event |
| 9 | `AlreadyApproved` | This signer has already approved this proposal. | `approve` | ❌ Terminal for this signer — the approval is already counted |
| 10 | `CalldataTooLarge` | `calldata.len()` exceeds `MAX_CALLDATA_BYTES` (4,096). | `propose` | ✅ Recoverable — compress or split the operation |
| 11 | `TooManySigners` | Signer list would exceed `MAX_SIGNERS` (20). | `init`, `add_signer` | ✅ Recoverable — remove an old signer first |
| 12 | `ProposalExpired` | Proposal age exceeds `MAX_PROPOSAL_AGE_SECONDS` (30 d). | `approve`, `execute` | ❌ Terminal — create a new proposal |
| 13 | `ProposalCancelled` | Proposal has been cancelled; no further approvals or execution are allowed. | `approve`, `execute`, `cancel_proposal` (repeated cancellation) | ❌ Terminal — create a new proposal if action still needed |
| 14 | `NotProposerOrAdmin` | Caller is neither the original proposer nor the contract admin. | `cancel_proposal` | ⚠️ Config fix needed — switch to proposer or admin wallet |
| 15 | `InvalidThreshold` | Threshold is zero or exceeds the signer count; invariant `1 ≤ threshold ≤ signers.len()` violated. | `init` | ✅ Recoverable — choose a valid threshold and retry `init` (or use governance migration) |
| 16 | `QuorumWouldBreak` | Removing the signer would leave fewer signers than the configured threshold. | `remove_signer` | ⚠️ Config fix needed — lower threshold first via a governed migration or add another signer |
| 17 | `DuplicateSigner` | Address is already registered in the co-signer set. | `init`, `add_signer` | ✅ Recoverable — deduplicate the signer list |
| 18 | `ArithmeticOverflow` | A proposal ID counter or timelock deadline calculation would overflow `u32`/`u64`. | `propose` (ID counter), `approve` (timelock deadline), `execute` (age deadline), `checked_deadline` (internal) | ⚠️ Should not occur under normal conditions; report as a bug |
| 19 | `InvalidCalldata` | `calldata` bytes deserialised but do not match any known `CallData` variant. | `execute` | ✅ Recoverable — re-encode calldata as a supported `CallData` variant and submit a new proposal |

### Detailed semantics

#### NotInitialized (1)

**Trigger**: Any entrypoint that reads `Admin`, `Signers`, `Threshold`, or `SignerIndex` from
instance storage before `init` has been called.

**Client action**: Block all governance UI actions until deployment confirms `init` has been
called with a valid `(admin, signers, threshold)` triple.

---

#### AlreadyInitialized (2)

**Trigger**: A second call to `init` when `DataKey::Admin` already exists in instance storage.

**Client action**: This is an operator configuration mistake. Read current state with
`get_admin()` / `get_signers()` / `get_threshold()` instead of retrying.

---

#### Unauthorized (3)

**Trigger**: `set_admin`, `add_signer`, or `remove_signer` called without the current admin's
`require_auth` passing.

**Client action**: Ensure the transaction is signed by the current admin key. Use `get_admin()`
to confirm which address holds the role.

---

#### NotASigner (4)

**Trigger**: `propose` or `approve` from an address absent from the `Signers` vector / `SignerIndex` map.

**Client action**: An admin must call `add_signer(new_address)` before retrying. Switching to a
registered co-signer wallet is the fastest workaround.

---

#### ProposalNotFound (5)

**Trigger**: Any entrypoint that calls `load_proposal` with an ID not in persistent storage,
or an ID beyond `NextProposalId - 1`.

**Client action**: Refresh the proposal list; the ID must originate from a `ProposalCreated`
event. IDs are monotonically increasing starting from 0.

---

#### AlreadyExecuted (6)

**Trigger**: `approve`, `execute`, or `cancel_proposal` when `proposal.executed == true`.

**Client action**: Stop collecting approvals. Show the executed state to the user and surface
the `ProposalExecuted` event details (executor, target, calldata).

---

#### QuorumNotReached (7)

**Trigger**: `execute` when either:
- `approval_count < threshold`, or
- `DataKey::QuorumReachedAt(id)` entry is absent (quorum has not been reached yet).

**Client action**: Continue collecting signer approvals until `approval_count >= threshold`.
Use `get_proposal(id).approvals.len()` and `get_threshold()` to compute remaining required
approvals.

---

#### TimelockNotElapsed (8)

**Trigger**: `execute` called before `quorum_info.reached_at + GOVERNANCE_TIMELOCK_SECONDS`.

**Client action**: This is the most common *expected* transient error for multisig UIs.
Display the `executable_after` timestamp from the `QuorumReached` event and schedule a
retry after that timestamp. **Do not treat this as a fatal error.**

```text
executable_after = quorum_reached_at + 172_800  (48 hours)
```

---

#### AlreadyApproved (9)

**Trigger**: The same co-signer calls `approve` a second time for the same proposal.

**Client action**: The signer's approval is already counted. Show the current
`approval_count` from `ProposalApproved` events. Do not request another approval from
that address.

---

#### CalldataTooLarge (10)

**Trigger**: `propose` when `calldata.len() > MAX_CALLDATA_BYTES` (4,096 bytes).

**Client action**: Compress or simplify the XDR-encoded `CallData`. If the operation
genuinely requires more data, split it across multiple proposals.

---

#### TooManySigners (11)

**Trigger**: `init` or `add_signer` would push `signers.len()` above `MAX_SIGNERS` (20).

**Client action**: Remove a stale or decommissioned signer first, or deploy a new
governance instance with a pruned signer set.

---

#### ProposalExpired (12)

**Trigger**: `approve` or `execute` when
`env.ledger().timestamp() > proposal.created_at + MAX_PROPOSAL_AGE_SECONDS` (30 days).

**Client action**: This proposal is permanently unexecutable. If the governed action is
still required, submit a new proposal. Consider increasing approval cadence to avoid
future expirations.

---

#### ProposalCancelled (13)

**Trigger**:
- `approve` or `execute` when `proposal.cancelled == true`.
- `cancel_proposal` called a second time on an already-cancelled proposal.

**Client action**: The proposal is permanently dead. Stop collecting approvals and surface
the `ProposalCancelled` event (canceller, timestamp) to operators. Create a new proposal
if the action is still needed.

> ⚠️ **Security note**: Misclassifying this as *recoverable* would cause operators to
> retry approvals on a cancelled proposal indefinitely, wasting gas. This is always a
> terminal state.

---

#### NotProposerOrAdmin (14)

**Trigger**: `cancel_proposal` from an address that is neither `proposal.proposer` nor the
current `Admin`.

**Client action**: Switch to the proposer or admin wallet. Use `get_proposal(id).proposer`
and `get_admin()` to identify the authorised cancellers.

---

#### InvalidThreshold (15)

**Trigger**: `init` when `threshold == 0` or `threshold > signers.len()`.

**Client action**: Choose a threshold satisfying `1 ≤ threshold ≤ signers.len()`. This only
arises during deployment; a governance migration is required for post-init changes.

---

#### QuorumWouldBreak (16)

**Trigger**: `remove_signer` when `signers.len() - 1 < threshold`.

**Client action**: Either add another signer first (`add_signer`) or lower the threshold
through a governed parameter change before removing the signer.

> ⚠️ **Security note**: This guard is critical — if bypassed, the governance contract could
> reach a state where quorum is mathematically unreachable, permanently bricking execution.

---

#### DuplicateSigner (17)

**Trigger**: `init` or `add_signer` when the candidate address already appears in the
`SignerIndex` map.

**Client action**: Deduplicate the signer list before submitting. Each co-signer address
may occupy exactly one slot.

---

#### ArithmeticOverflow (18)

**Trigger**:
- `propose`: `NextProposalId + 1` would overflow `u32::MAX`.
- `approve` / `execute`: `proposal.created_at + MAX_PROPOSAL_AGE_SECONDS` or
  `quorum_reached_at + GOVERNANCE_TIMELOCK_SECONDS` would overflow `u64::MAX`.

**Client action**: This should never occur under normal Soroban network conditions (ledger
timestamps are in the year ~2100 range for u64 overflow). If seen, report as a contract bug.

---

#### InvalidCalldata (19)

**Trigger**: `execute` deserialises the `calldata` bytes via `CallData::from_xdr` but the
resulting `ScVal` does not match any known `CallData` enum variant (e.g., the proposer
encoded a plain `u32` instead of a `CallData` XDR value).

**Client action**: Re-encode the desired operation as a supported `CallData` variant (see
the [Calldata encoding contract](governance.md#calldata-encoding-contract) in `governance.md`)
and submit a new proposal with the corrected bytes. The failed proposal remains
un-executed and can be retried if the calldata was encoded incorrectly.

**Note**: Completely non-XDR bytes cause a host abort (transaction reverted, not this error).
`InvalidCalldata` is only returned when deserialization succeeds but the decoded value has no
matching `CallData` arm in `dispatch_call`.

---

### Discriminant-sync verification

To verify that the documented discriminants match the live enum, run:

```bash
cargo test -p fluxora_governance governance_error_discriminants
```

This test (in `contracts/governance/src/lib.rs` or a companion test file) asserts the exact
`u32` value of every `GovernanceError` variant and will fail if any value is changed without
updating this table.

---

## Residual Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Error code changes | Low | High | Versioning in client SDKs |
| Missing error cases | Low | Medium | Comprehensive test coverage |
| Client mishandling | Medium | Medium | This documentation |
| Dust-attack bypass | Very Low | High | MIN_RATE_PER_SECOND enforced at validation layer |
