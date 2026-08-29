# Cliff Boundary Test Scenarios - Quick Reference

## Test Configuration Pattern

All boundary tests use this setup:
```rust
start = now
cliff = start + 1000
end = start + 10000
deposit = 10000 * ONE  // 100,000,000 stroops
duration = 10000 seconds
rate = 1000 stroops/second
```

**Expected vested at cliff:** `1000/10000 * 10000 * ONE = 1000 * ONE`

## Scenario Matrix

### 1. Read Operations at Boundaries

**Test:** `cliff_boundary_reads_are_exact`

| Timestamp | vested_of | withdrawable_of | refundable_of |
|-----------|-----------|-----------------|---------------|
| cliff - 1 | 0 | 0 | 10000 * ONE |
| cliff | 1000 * ONE | 1000 * ONE | 9000 * ONE |
| cliff + 1 | 1001 * ONE | 1001 * ONE | 8999 * ONE |

**Invariants verified:**
- `vested + refundable == deposited` (exact)
- Step function at cliff (not gradual)
- Linear accrual after cliff

---

### 2. Withdrawal at Cliff Instant

**Test:** `withdrawal_succeeds_exactly_at_cliff`

```
Time: cliff
Before: recipient balance = X
Action: withdraw(None)  // Full withdrawal
After:  
  - withdrawn = 1000 * ONE
  - recipient balance = X + 1000 * ONE
  - withdrawable = 0
```

**Verifies:** Full cliff amount withdrawable at exact instant

---

### 3. Partial Withdrawal at Cliff

**Test:** `partial_withdrawal_at_cliff_leaves_remainder_available`

```
Time: cliff
Action: withdraw(Some(600 * ONE))
Result:
  - withdrawn = 600 * ONE
  - withdrawable = 400 * ONE  // Remainder available
```

**Verifies:** Partial withdrawal arithmetic is exact

---

### 4. Cancellation at cliff-1

**Test:** `cancellation_before_cliff_refunds_everything`

```
Time: cliff - 1
Before: sender balance = Y
Action: cancel()
After:
  - sender balance = Y + 10000 * ONE  // Full refund
  - withdrawable = 0
  - pool = 0
  - status = Cancelled
```

**Verifies:** Pre-cliff cancellation refunds entire deposit

---

### 5. Cancellation at Cliff Instant

**Test:** `cancellation_exactly_at_cliff_vests_cliff_amount`

```
Time: cliff
Before: sender balance = Y
Action: cancel()
After:
  - sender balance = Y + 9000 * ONE  // Unvested refund
  - withdrawable = 1000 * ONE        // Vested stays
  - deposited = 1000 * ONE           // Reduced
  - status = Cancelled
```

**Verifies:** Cliff amount vests to recipient, remainder refunded

---

### 6. Cancellation at cliff+1

**Test:** `cancellation_after_cliff_includes_additional_accrual`

```
Time: cliff + 1
Expected vested: 1001 * ONE  // One more second accrued
Before: sender balance = Y
Action: cancel()
After:
  - sender balance = Y + 8999 * ONE  // Unvested refund
  - withdrawable = 1001 * ONE        // Vested (cliff + 1 second)
```

**Verifies:** Cancellation includes all accrued seconds

---

### 7. Cliff Equals Start Time

**Test:** `cliff_equals_start_has_immediate_accrual`

```
cliff = start
Time: start + 1
Result:
  - vested = ONE  // One second accrued immediately
  - No cliff gate exists
```

**Verifies:** cliff==start means no gate, immediate accrual

---

### 8. Cliff Equals End Time (Lump Sum)

**Test:** `cliff_equals_end_is_lump_sum_at_final_instant`

```
cliff = end
Time: end - 1
Result: vested = 0  // Still gated

Time: end
Result: vested = deposited  // Full amount at maturity
```

**Verifies:** Lump sum vesting at final instant

---

### 9. Withdrawal Before Cliff - Error Handling

**Test:** `withdrawal_before_cliff_fails_with_correct_error`

```
Time: cliff - 1
Action: withdraw(None)
Result: Error::NothingToWithdraw

Action: withdraw(Some(100 * ONE))
Result: Error::NothingToWithdraw
```

**Verifies:** Correct error type for pre-cliff withdrawal attempts

---

### 10. Batch Operations at Cliff

**Test:** `batch_reads_handle_cliff_boundaries_correctly`

```
Create 3 streams:
  - id1: cliff = base_cliff + 10
  - id2: cliff = base_cliff
  - id3: cliff = base_cliff - 10

Time: base_cliff

vested_of_batch([id1, id2, id3]) = [0, 1000*ONE, 1000*ONE]
withdrawable_of_batch([id1, id2, id3]) = [0, 1000*ONE, 1000*ONE]
```

**Verifies:** Batch operations handle mixed cliff states correctly

---

### 11. Pause Across Cliff Boundary

**Test:** `pause_across_cliff_preserves_cliff_gate`

```
Time: cliff - 100
Action: pause()

Time: cliff + 500 (wall-clock, while paused)
Result: vested = 0  // Stream clock frozen before cliff

Action: resume()
advance(100)  // Stream clock now at cliff
Result: vested = 1000 * ONE
```

**Verifies:**
- Cliff evaluated on stream clock, not wall clock
- Pause freezes accrual clock
- Resume continues from freeze point

---

### 12. Multiple Partial Withdrawals at Cliff

**Test:** `multiple_partial_withdrawals_at_cliff_are_exact`

```
Time: cliff
Expected: 1000 * ONE available

Action: withdraw(Some(300 * ONE))  →  300 * ONE
Action: withdraw(Some(400 * ONE))  →  400 * ONE  
Action: withdraw(Some(300 * ONE))  →  300 * ONE

Total withdrawn: 1000 * ONE (exact)
Remaining withdrawable: 0
```

**Verifies:** Multiple withdrawals tracked exactly, no double-counting

---

## Invariants Checked by All Tests

Every test calls `h.assert_pool_exact()` which verifies:

1. **I1 - Bounds**: `0 <= withdrawn <= vested <= deposited`
2. **I4 - Conservation**: `vested + refundable == deposited` (exact, no dust)
3. **I5 - Pause coherence**: `paused_at.is_some() ⟺ status == Paused`
4. **Pool invariant**: `contract_balance == Σ(stream.deposited - stream.withdrawn)`

## Error Cases Tested

| Scenario | Error | Test |
|----------|-------|------|
| Withdraw before cliff | `NothingToWithdraw` | #9 |
| Withdraw with zero balance | `NothingToWithdraw` | #1 |

## Edge Cases Covered

- ✓ cliff == start (no gate)
- ✓ cliff == end (lump sum)
- ✓ Pause across cliff boundary
- ✓ Multiple partial withdrawals
- ✓ Batch operations with mixed states
- ✓ Cancellation at all three boundaries

## Time Precision

All tests use **exact second-level precision**:
- `cliff - 1`: One second before cliff
- `cliff`: Exactly at cliff instant
- `cliff + 1`: One second after cliff

This ensures no tolerance or rounding masks off-by-one errors.

## Running Individual Tests

```bash
# All cliff tests
cargo test -p fluxora-stream cliff -- --nocapture

# Specific boundary test
cargo test -p fluxora-stream cliff_boundary_reads_are_exact -- --nocapture

# All boundary tests (grep for "boundary" or "cliff_equals")
cargo test -p fluxora-stream test::cliff:: -- --nocapture
```

## Expected Output

```
running 19 tests
test cliff::nothing_is_withdrawable_one_second_before_the_cliff ... ok
test cliff::cliff_releases_all_accrual_since_start_not_since_the_cliff ... ok
test cliff::the_cliff_step_lands_on_the_exact_second ... ok
test cliff::accrual_is_linear_after_the_cliff ... ok
test cliff::cliff_at_end_time_is_a_lump_sum ... ok
test cliff::cliff_at_start_time_means_no_cliff ... ok
test cliff::cancelling_before_the_cliff_refunds_the_whole_deposit ... ok
test cliff::cliff_boundary_reads_are_exact ... ok
test cliff::withdrawal_succeeds_exactly_at_cliff ... ok
test cliff::partial_withdrawal_at_cliff_leaves_remainder_available ... ok
test cliff::cancellation_exactly_at_cliff_vests_cliff_amount ... ok
test cliff::cancellation_before_cliff_refunds_everything ... ok
test cliff::cancellation_after_cliff_includes_additional_accrual ... ok
test cliff::cliff_equals_start_has_immediate_accrual ... ok
test cliff::cliff_equals_end_is_lump_sum_at_final_instant ... ok
test cliff::withdrawal_before_cliff_fails_with_correct_error ... ok
test cliff::batch_reads_handle_cliff_boundaries_correctly ... ok
test cliff::pause_across_cliff_preserves_cliff_gate ... ok
test cliff::multiple_partial_withdrawals_at_cliff_are_exact ... ok

test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 127 filtered out
```

---

**Quick verification checklist:**
- [ ] 19 tests pass (7 existing + 12 new)
- [ ] No panics or overflows
- [ ] Pool invariant holds after every operation
- [ ] Conservation law exact (no dust)
- [ ] All error types correct
