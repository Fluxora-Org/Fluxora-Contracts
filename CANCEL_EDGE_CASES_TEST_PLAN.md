# Cancel Edge Cases (Fully Accrued / At Cliff) — Test Implementation Plan

## Issue
**Integration suite: cancel edge cases (fully accrued / at cliff)**

Objective: Tighten externally visible assurances for cancel operations, ensuring:
- Treasury operators and recipient-facing applications can reason about behavior using only on-chain observables
- Time boundaries (cliff, end) and numeric ranges are crisp with no ambiguity
- Refund semantics are crystal clear and match published protocol documentation

## Scope: Protocol-Level Invariants

### Success Semantics
1. Cancel valid only in `Active` or `Paused` states
2. `cancelled_at` set to current ledger timestamp
3. Refund = `deposit_amount - accrued_at(cancelled_at)`
4. Stream transitions to terminal `Cancelled` state
5. `StreamCancelled` event emitted
6. Accrued amount frozen at `cancelled_at` for all future queries

### Failure Semantics
1. `Completed` or already `Cancelled` state → `InvalidState`
2. Sender cannot cancel other's streams (auth failure)
3. Unauthorized caller → auth failure
4. All failures are atomic (no state change, no transfer, no event)

## Time Boundary Edge Cases to Cover

### 1. **Cancel Exactly at Cliff Time**
- **Scenario**: cliff_time == t_cancel
- **Expected Behavior**:
  - If cliff_time == start_time: accrual = (t_cancel - start_time) × rate
  - If cliff_time > start_time: accrual = 0 (cliff not yet reached)
  - Refund = deposit - accrued
  - Status → Cancelled
  - `cancelled_at = cliff_time`

**Test**: `test_integration_cancel_exactly_at_cliff`

### 2. **Cancel Exactly at End Time (100% Accrued)**
- **Scenario**: t_cancel == end_time
- **Expected Behavior**:
  - Accrued = min(rate × (end_time - start_time), deposit), capped at deposit
  - Refund = deposit - accrued = 0 (fully accrued)
  - Status → Cancelled
  - `cancelled_at = end_time`
  - Sender receives nothing
  - All tokens held in contract for recipient

**Test**: `test_integration_cancel_exactly_at_end_time`

### 3. **Cancel One Ledger Tick Before End Time**
- **Scenario**: t_cancel = end_time - 1
- **Expected Behavior**:
  - Accrued slightly less than deposit (by 1 second × rate)
  - Refund = 1 × rate
  - Status → Cancelled
  - Precision verification at boundary

**Test**: `test_integration_cancel_one_second_before_end_time`

### 4. **Cancel Well Beyond End Time**
- **Scenario**: t_cancel >> end_time
- **Expected Behavior**:
  - Accrued capped at deposit_amount (no post-end growth)
  - Refund = 0
  - Status → Cancelled
  - Frozen at full deposit amount

**Test**: `test_integration_cancel_beyond_end_time`

## Frozen Accrual Edge Cases

### 5. **Frozen Accrual Verification — Multiple Calls Post-Cancel**
- **Scenario**: Cancel at time T, then query accrued at T, T+1s, T+1000s
- **Expected Behavior**:
  - All calls return same accrued value (frozen at T)
  - No growth despite time advancement
  - Idempotent guarantee for indexers

**Test**: `test_integration_cancel_frozen_accrual_multiple_queries`

### 6. **Frozen Accrual with Prior Withdrawal**
- **Scenario**: Withdraw X tokens, then cancel, then verify frozen accrual excludes withdrawn
- **Expected Behavior**:
  - `frozen_accrued = accrued_at_cancel_time` (independent of withdrawn_amount)
  - withdrawable_remaining = frozen_accrued - withdrawn_amount
  - Invariant: refund + frozen_accrued == deposit

**Test**: `test_integration_cancel_frozen_accrual_after_withdrawal`

## Cliff with Cancel Edge Cases

### 7. **Cancel Before Cliff — Zero Accrual**
- **Scenario**: start_time < t_cancel < cliff_time
- **Expected Behavior**:
  - Accrued = 0
  - Refund = deposit_amount
  - Status → Cancelled
  - Sender gets full refund

**Test**: `test_integration_cancel_before_cliff_zero_accrual`

### 8. **Cancel Exactly When Cliff Transitions from Zero to Non-Zero**
- **Scenario**: t_cancel == cliff_time (boundary where accruel switches on)
- **Expected Behavior**:
  - Accrued begins to accrue (if cliff == start, then accrued = (cliff - start) × rate)
  - Refund = deposit - (cliff - start) × rate
  - Status → Cancelled

**Test**: `test_integration_cancel_at_cliff_boundary`

### 9. **Cancel Just After Cliff**
- **Scenario**: t_cancel = cliff_time + 1
- **Expected Behavior**:
  - Accrued = (cliff_time + 1 - start_time) × rate
  - Confirms cliff boundary working correctly

**Test**: `test_integration_cancel_just_after_cliff`

## Refund Precision Edge Cases

### 10. **Refund Calculation Precision**
- **Scenario**: deposit = 1000, rate = 3, duration = 333, t_cancel = 100
- **Expected Behavior**:
  - Accrued = 100 × 3 = 300
  - Refund = 1000 - 300 = 70
  - No arithmetic overflow or rounding issues

**Test**: `test_integration_cancel_refund_precision`

## Authorization Edge Cases

### 11. **Recipient Cannot Cancel (Unauthorized)**
- **Scenario**: Recipient calls `cancel_stream` on own stream
- **Expected Behavior**:
  - Auth failure (recipient != sender)
  - No state change
  - No event

**Test**: `test_integration_cancel_recipient_unauthorized`

### 12. **Admin Can Cancel Any Stream**
- **Scenario**: Admin calls `cancel_stream_as_admin` on anyone's stream
- **Expected Behavior**:
  - Succeeds (admin auth satisfied)
  - Sender gets refund regardless
  - Status → Cancelled

**Test**: `test_integration_cancel_admin_can_cancel_any_stream`

## Event & CEI Ordering

### 13. **Cancel Event Emitted with Correct Topic and Payload**
- **Scenario**: Cancel stream and inspect events
- **Expected Behavior**:
  - Event topic: `("cancelled", stream_id)`
  - Event payload: `StreamCancelled(stream_id)`
  - Event emitted after state persisted (CEI ordering)

**Test**: `test_integration_cancel_event_topic_and_payload`

## Atomicity & Rollback

### 14. **Cancel Failure is Atomic**
- **Scenario**: Cancel already-cancelled stream
- **Expected Behavior**:
  - InvalidState error
  - No refund transfer
  - No state change
  - `cancelled_at` unchanged
  - No event emitted

**Test**: `test_integration_cancel_already_cancelled_atomic`

## Summary Table

| # | Test Name | Edge Case | Pass Criteria |
|----|-----------|-----------|---------------|
| 1 | `cancel_exactly_at_cliff` | Cliff boundary | Accrued correct, status Cancelled |
| 2 | `cancel_exactly_at_end_time` | 100% accrual | Zero refund, all locked for recipient |
| 3 | `cancel_one_second_before_end` | Precision boundary | Refund == rate, frozen at boundary |
| 4 | `cancel_beyond_end_time` | Post-end stability | Accrued capped, frozen at full |
| 5 | `cancel_frozen_accrual_multiple_queries` | Idempotency | All queries == accrued_at_cancel |
| 6 | `cancel_frozen_accrual_after_withdrawal` | Frozen != withdrawn | Invariant holds |
| 7 | `cancel_before_cliff_zero_accrual` | Pre-cliff state | Full refund, zero withdrawn |
| 8 | `cancel_at_cliff_boundary` | Cliff toggle | Accrual switches on correctly |
| 9 | `cancel_just_after_cliff` | Post-cliff stability | Accrual grows normally |
| 10 | `cancel_refund_precision` | Arithmetic | Precision maintained |
| 11 | `cancel_recipient_unauthorized` | Auth | Failure is atomic |
| 12 | `cancel_admin_override` | Admin path | Succeeds with admin auth |
| 13 | `cancel_event_correct` | Events | Topic/payload correct |
| 14 | `cancel_already_cancelled_atomic` | Atomicity | No side effects |

## Verification

Each test MUST verify:
1. **Pre-cancel state**: stream exists, status is Active (or Paused)
2. **Cancel call**: succeeds or fails atomically
3. **Post-cancel state**:
   - `status == Cancelled`
   - `cancelled_at` set correctly
   - No state mutation on failure
4. **Balances**:
   - Sender receives correct refund
   - Recipient balance unchanged (accrued locked)
   - Contract balance == accrued amount
5. **Event**: `StreamCancelled` emitted with correct topic
6. **Frozen accrual**: Multiple `calculate_accrued` calls return same value
7. **Invariant**: `refund + frozen_accrued == deposit`

## Documentation Updates

Update `docs/streaming.md`:
- Explicitly enumerate time boundaries
- Document cliff behavior around cliff_time
- Document fully accrued (end_time) behavior
- Document refund precision and invariants
- Add explicit failure modes table

---

**Status**: Implementation in progress
**Target Coverage**: ≥95% of touched cancel-related code
**Timeframe**: 96 hours per issue guidelines
