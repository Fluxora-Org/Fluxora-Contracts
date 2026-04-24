# Security Documentation for Fluxora Streaming Contract

## Threat Model and Admin Powers

### Overview

This document describes the security posture of the Fluxora streaming contract on Soroban (Stellar), including threat assumptions, trust boundaries, role-based access control, and the extent of administrative powers.

---

## 1. Trust Assumptions

### Core Assumptions

| Assumption | Description | Risk Level |
|------------|-------------|------------|
| **Token Contract Integrity** | The token contract (address provided in `init()`) behaves according to the Soroban token interface standard (`transfer`, `balance`, `allowance`). | High |
| **Ledger Timestamp Reliability** | `env.ledger().timestamp()` returns monotonically increasing, non-manipulable timestamps. | High |
| **Authentication Correctness** | `Address::require_auth()` correctly verifies signatures and cannot be bypassed. | High |
| **Arithmetic Safety** | `i128` operations with `checked_mul`/`checked_add` prevent overflow. | Medium |
| **TTL Extension Sufficiency** | Storage TTL thresholds (17,280 ledgers ≈ 1 day, 120,960 ≈ 7 days at 5s/ledger) are sufficient for active streams. | Medium |

### Out of Scope

- Token contract vulnerabilities (reentrancy, malicious implementations)
- Stellar network consensus failures or ledger reorganizations
- Private key compromises of users or admin
- Front-running mitigation (time-based operations are deterministic)

---

## 2. Role Definitions

### 2.1 Contract Admin

**Establishment**: Set during `init()` — cannot be changed except via `set_admin()`.

**Authorization Requirement**: All admin operations require `admin.require_auth()`.

**Powers**:

| Operation | Description | Emergency Use |
|-----------|-------------|---------------|
| `set_admin(new_admin)` | Rotate admin key | Key compromise recovery |
| `set_global_emergency_paused(bool)` | Global pause for all user mutations | Circuit breaker |
| `cancel_stream_as_admin(id)` | Cancel any stream | Dispute resolution |
| `pause_stream_as_admin(id)` | Pause any stream | Freeze suspicious activity |
| `resume_stream_as_admin(id)` | Resume any paused stream | Restore after resolution |
| `set_contract_paused(bool)` | Legacy pause (deprecated in favor of global emergency pause) | - |

**Cannot Do**:
- Withdraw funds from streams (only recipients can)
- Modify stream parameters (rate, end_time) without sender authorization
- Create streams on behalf of others
- Change token address after initialization

### 2.2 Stream Sender

**Definition**: Address that creates and funds a stream.

**Authorization Requirement**: `sender.require_auth()` for:

| Operation | Description |
|-----------|-------------|
| `create_stream()` | Create and fund new stream |
| `pause_stream()` | Pause own stream |
| `resume_stream()` | Resume own stream |
| `cancel_stream()` | Cancel own stream (refund unstreamed) |
| `update_rate_per_second()` | Increase streaming rate |
| `shorten_stream_end_time()` | Reduce duration (refund excess) |
| `extend_stream_end_time()` | Extend duration (if deposit covers) |

**Cannot Do**:
- Withdraw from stream (recipient-only)
- Modify recipient address
- Cancel after stream is Completed

### 2.3 Stream Recipient

**Definition**: Address that receives streamed tokens.

**Authorization Requirement**: `recipient.require_auth()` for:

| Operation | Description |
|-----------|-------------|
| `withdraw()` | Claim accrued tokens to own address |
| `withdraw_to(destination)` | Claim accrued tokens to any address |
| `batch_withdraw()` | Claim from multiple streams atomically |

**Cannot Do**:
- Cancel or pause streams
- Modify stream parameters
- Transfer recipient status to another address

### 2.4 Permissionless Actors

| Operation | Authorization | Description |
|-----------|---------------|-------------|
| `close_completed_stream()` | None | Archive completed streams (gas refund incentive) |
| `get_stream_state()` | None | Read stream data |
| `calculate_accrued()` | None | View accrued amount |
| `get_withdrawable()` | None | View claimable amount |
| `get_recipient_streams()` | None | Enumerate recipient streams |
| `get_config()` | None | View contract configuration |
| `version()` | None | View contract version |

---

## 3. Administrative Powers Analysis

### 3.1 Global Emergency Pause

**Mechanism**:
```rust
fn require_not_globally_paused(env: &Env) {
    assert!(!is_global_emergency_paused(env), "contract is globally paused");
}
```

**Blocked Operations** (when `paused == true`):
- `create_stream` / `create_streams`
- `withdraw` / `withdraw_to` / `batch_withdraw`
- `pause_stream` / `resume_stream`
- `cancel_stream`
- `update_rate_per_second`
- `shorten_stream_end_time`
- `extend_stream_end_time`
- `top_up_stream`
- `set_admin`

**Unblocked Operations**:
- All `*_as_admin` operations
- `close_completed_stream`
- All read/view functions
- `set_global_emergency_paused` (to disable)

**Risk**: Admin can freeze all user withdrawals. Mitigation:
- Multi-sig admin recommended
- Emergency pause should be time-limited (on-chain governance for extension)
- Pause state is public (`get_global_emergency_paused`)

### 3.2 Admin Cancel

**Behavior** (`cancel_stream_as_admin`):
- Same refund calculation as sender-initiated cancel
- Emits identical `cancelled` event
- Does NOT require sender authorization

**Risk**: Admin can cancel any stream, potentially:
- Refunding unstreamed tokens to sender (expected)
- Recipient still gets accrued amount

**Mitigation**: Admin must be a trusted entity (DAO, multi-sig). All cancels are emitted on-chain.

### 3.3 Admin Pause/Resume

**Behavior**:
- Bypasses sender authorization
- Same state transition as sender operations
- Emits identical events

**Use Case**: Freeze suspicious streams during investigation.

### 3.4 Admin Key Rotation

**Mechanism**: `set_admin(new_admin)` — only current admin can execute.

**Security Considerations**:
- Old admin loses all powers immediately after transaction
- No timelock — rotation is instantaneous
- Recommended: Multi-sig with rotation requiring multiple signatures

---

## 4. Threat Scenarios

### 4.1 Admin Misbehavior

| Threat | Impact | Mitigation |
|--------|--------|-------------|
| Malicious pause | Freeze all withdrawals | Multi-sig admin, governance oversight |
| Unauthorized cancel | Premature stream termination | On-chain events for auditing |
| Key compromise | Attacker gains admin powers | Multi-sig, key rotation, timelocks |
| Rogue admin drains funds | **Impossible** — admin cannot withdraw | Token transfers only to recipients/senders |

### 4.2 Sender Malice

| Threat | Mitigation |
|--------|-------------|
| Create stream with insufficient deposit | `validate_stream_params` checks `deposit >= rate × duration` |
| Cancel after recipient accrued | Recipient keeps accrued amount (refund only unstreamed) |
| Reduce rate (disallowed) | `update_rate_per_second` requires `new_rate > old_rate` |
| Extend stream without deposit | `extend_stream_end_time` validates deposit covers extended duration |

### 4.3 Recipient Malice

| Threat | Mitigation |
|--------|-------------|
| Withdraw more than accrued | `withdrawable = accrued - withdrawn_amount` prevents over-withdrawal |
| Double-withdraw | State update before token transfer (CEI pattern) |
| Front-run cancellation | Cancellation uses `ledger.timestamp()` at call time |

### 4.4 Time-Based Attacks

| Threat | Mitigation |
|--------|-------------|
| Start time in past | `start_time < current_timestamp` → `ContractError::StartTimeInPast` |
| Cliff after end | `cliff_time <= end_time` enforced |
| End before start | `start_time < end_time` enforced |
| Cancellation timestamp freeze | `cancelled_at` stored; accrual frozen at that time |

### 4.5 Arithmetic Attacks

| Threat | Mitigation |
|--------|-------------|
| Overflow in `rate × duration` | `checked_mul()` → `ArithmeticOverflow` error |
| Overflow in deposit sum | `checked_add()` for batch total |
| Negative amounts | `deposit_amount > 0`, `rate_per_second > 0` enforced |

### 4.6 Storage Expiration

| Threat | Mitigation |
|--------|-------------|
| Stream data expires | TTL extended on every read/write: 17,280 ledger threshold, 120,960 bump |
| Recipient index expires | Extended on access |
| Instance config expires | Extended on every instance access |

**TTL Constants**:
- Threshold: 17,280 ledgers (~1 day at 5s/ledger)
- Bump amount: 120,960 ledgers (~7 days)

---

## 5. Invariants

### 5.1 Balance Invariants

```
∀ stream: stream.deposit_amount >= stream.withdrawn_amount
∀ stream (Cancelled): stream.deposit_amount - stream.withdrawn_amount = refund_amount_sent + (accrued_at_cancel - withdrawn_amount)
Contract token balance = Σ(stream.deposit_amount) - Σ(stream.withdrawn_amount)
```

### 5.2 State Transition Invariants

```
Active → Paused   (only via pause/pause_as_admin)
Active → Completed (only when withdrawn_amount == deposit_amount)
Active → Cancelled (only via cancel/cancel_as_admin)
Paused  → Active   (only via resume/resume_as_admin)
Paused  → Cancelled (only via cancel/cancel_as_admin)
Completed → (terminal, can only be closed)
Cancelled → (terminal, can only be read)
```

### 5.3 Accrual Invariants

```
If now < cliff_time:   accrued = 0
If cliff_time ≤ now ≤ end_time: accrued = min(rate × (now - start_time), deposit_amount)
If now > end_time:     accrued = deposit_amount
If status == Cancelled: accrued frozen at cancellation_time
```

### 5.4 Authorization Invariants

```
Operation requires sender auth → Non-admin cannot bypass
Operation requires recipient auth → Admin cannot bypass (except cancel/pause/resume)
Admin operations → Always require admin.require_auth()
Permissionless reads → No auth required
```

---

## 6. Event Emission Guarantees

Every state-changing operation emits exactly one primary event:

| Operation | Event Topic | Payload |
|-----------|-------------|---------|
| `create_stream` | `created` | `StreamCreated` |
| `pause_stream` | `paused` | `StreamEvent::Paused` |
| `resume_stream` | `resumed` | `StreamEvent::Resumed` |
| `cancel_stream` | `cancelled` | `StreamEvent::Cancelled` |
| `withdraw` | `withdrew` | `Withdrawal` |
| `withdraw_to` | `wdraw_to` | `WithdrawalTo` |
| `update_rate_per_second` | `rate_upd` | `RateUpdated` |
| `shorten_stream_end_time` | `end_shrt` | `StreamEndShortened` |
| `extend_stream_end_time` | `end_ext` | `StreamEndExtended` |
| `top_up_stream` | `top_up` | `StreamToppedUp` |
| `close_completed_stream` | `closed` | `StreamEvent::Closed` |
| `set_global_emergency_paused` | `gl_pause` | `GlobalEmergencyPauseChanged` |
| `set_admin` | `AdminUpd` | `(old_admin, new_admin)` |

**Event Ordering** (when multiple events in same tx):
- `withdraw`/`withdraw_to` → `completed` (if stream fully drained)
- `cancel_stream` → (no `completed` event)

---

## 7. Security Checklist

### 7.1 CEI Pattern Compliance

**State before external calls**:
- `cancel_stream`: updates status → then `push_token`
- `withdraw`: updates `withdrawn_amount` → then `push_token`
- `top_up_stream`: updates `deposit_amount` → then `pull_token`

**Reentrancy**: Soroban token contract assumed non-reentrant; CEI provides defense-in-depth.

### 7.2 Input Validation

| Validation | Enforced |
|------------|----------|
| Positive amounts | `deposit_amount > 0`, `rate > 0` |
| Sender != Recipient |  |
| `start_time < end_time` |  |
| `start_time >= current_time` | |
| `cliff_time` in `[start, end]` | |
| Deposit ≥ rate × duration |  |

### 7.3 Access Control

| Pattern | Implementation |
|---------|----------------|
| Sender-only | `sender.require_auth()` |
| Recipient-only | `recipient.require_auth()` |
| Admin-only | `get_admin().require_auth()` |
| No auth (reads) | No `require_auth()` call |

---

## 8. Residual Risks & Exceptions

| Risk | Description | Mitigation |
|------|-------------|------------|
| **Admin single point of failure** | Compromised admin can pause/cancel | Recommend multi-sig; rotation capability exists |
| **Timestamp manipulation** | Validators influence `ledger.timestamp()` | Time bounds are approximate; no financial reliance on precise timing |
| **Token transfer failures** | Malicious token reverts | Contract panics atomically; no state change |
| **Storage exhaustion** | Unlimited streams per recipient | No hard limit; gas costs prevent DoS |
| **Front-running cliff claims** | Withdraw exactly at cliff time | Accrual is continuous; no advantage |
| **Batch withdrawal duplicates** | Same ID twice | Explicit duplicate check with `assert` |

### Explicitly Out of Scope

- **MEV protection**: Time-based streams are inherently MEV-vulnerable
- **Cross-contract reentrancy**: Assumes token contracts are non-reentrant
- **Rate decrease support**: Forward-only rate changes by design
- **Recipient transfer**: Streams are non-transferable by design
- **Partial cancellation**: Only full stream cancellation supported

---

## 9. Emergency Procedures

### 9.1 Admin Actions Under Attack

| Scenario | Action |
|----------|--------|
| Suspicious stream activity | `pause_stream_as_admin(stream_id)` |
| Widespread exploit | `set_global_emergency_paused(true)` |
| Admin key compromise | `set_admin(new_admin)` (if old admin still controls) |
| Token contract issues | Cannot change token; requires redeployment |

### 9.2 Recovery Flow

1. **Pause all user operations**: `set_global_emergency_paused(true)`
2. **Investigate**: Review events, identify affected streams
3. **Remediate**: 
   - Cancel malicious streams (`cancel_stream_as_admin`)
   - Resume legitimate streams (`resume_stream_as_admin`)
4. **Unpause**: `set_global_emergency_paused(false)`

---

## 10. Audit Recommendations

### Critical Review Points

1. **Admin Powers Documentation** — Ensure DAO/multi-sig understands scope
2. **TTL Strategy** — Verify thresholds match expected stream lifetimes
3. **Arithmetic Safety** — Fuzz test `checked_mul` paths with max `i128`
4. **Event Completeness** — Indexers must handle all event types
5. **Cancellation Accounting** — Verify refunds don't underflow

### Test Coverage Requirements

- **Target**: ≥95% on `lib.rs`
- **Critical paths**:
  - All admin operations with unauthorized callers
  - Overflow scenarios in rate × duration
  - TTL extension on all storage access paths
  - State transitions for all status combinations
  - Batch withdrawal duplicate detection

---

## 11. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-01-29 | Initial security documentation |


*This document should be reviewed after any contract upgrade or change to admin powers.*