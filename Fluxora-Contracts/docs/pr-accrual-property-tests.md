# PR: test(accrual): accrual property tests: bounds and monotonicity

## Summary

This PR adds comprehensive property-based tests for the `calculate_accrued_amount`
function in `contracts/stream/src/accrual.rs`. The tests systematically verify:

1. **Bounds**: `0 <= accrued(t) <= deposit_amount` for all stream configurations and times
2. **Monotonicity**: `accrued(t1) <= accrued(t2)` for all `t1 <= t2` after cliff
3. **Edge cases**: cliff == start, cliff == end, zero values, overflow boundaries
4. **Determinism**: same inputs always produce same output

The PR also fixes a duplicate `sort_array` function definition in `property_monotonicity`
module.

---

## Scope

### Included

- Pure function unit tests for `calculate_accrued_amount` in `accrual.rs`
- Bounds verification across all valid stream configurations
- Monotonicity verification across time progression
- Edge case coverage for degenerate schedules
- Integer overflow boundary testing
- Contract-level integration tests in `test.rs` for stream status combinations

### Excluded

| Exclusion | Rationale | Residual Risk |
|-----------|-----------|---------------|
| Token transfer mechanics | Covered in `test_token_edge_cases.rs` | Low - covered by integration tests |
| Storage persistence | Covered by `integration_suite.rs` | Low - covered by integration tests |
| Soroban environment mocking | Not applicable to pure function | N/A |
| Gas budget optimization | Pure function tests don't consume gas | N/A |
| Admin/authorization edge cases | Covered in `adversarial_auth.rs` | Low - covered by auth tests |

---

## Test Modules Added/Modified

### `accrual::property_monotonicity` (fixed)
- Fixed duplicate `sort_array` function definition

### `accrual::accrual_bounds_and_monotonicity` (new)

| Test | Description |
|------|-------------|
| `zero_deposit_always_returns_zero` | Edge case: deposit = 0 |
| `zero_deposit_with_cliff_always_returns_zero` | Edge case: deposit = 0 with cliff |
| `zero_rate_always_returns_zero` | Edge case: rate = 0 |
| `zero_rate_with_cliff_always_returns_zero` | Edge case: rate = 0 with cliff |
| `cliff_equals_end_returns_zero_always` | Edge case: cliff == end |
| `cliff_equals_end_large_deposit_still_zero` | Edge case: cliff == end with large deposit |
| `cliff_greater_than_end_returns_zero` | Edge case: cliff > end |
| `cliff_greater_than_end_after_cliff_still_zero` | Edge case: cliff > end behavior |
| `zero_duration_stream_returns_zero` | Edge case: start == end |
| `exact_overflow_boundary_rate_times_one` | Boundary: rate=1, elapsed=i128::MAX |
| `exact_overflow_boundary_large_rate` | Boundary: large rate overflow |
| `exact_boundary_rate_times_duration` | Boundary: rate * duration = deposit |
| `monotonicity_standard_stream_every_second` | Monotonicity: 1100 iterations |
| `monotonicity_cliff_at_start` | Monotonicity: cliff == start |
| `monotonicity_high_rate_capped_by_deposit` | Monotonicity: high rate streams |
| `monotonicity_across_cliff_boundary` | Monotonicity: cliff transition |
| `monotonicity_long_duration_stream` | Monotonicity: u32::MAX duration |
| `boundedness_all_results_non_negative` | Bounds: non-negative for all configs |
| `boundedness_all_results_capped_by_deposit` | Bounds: capped for all configs |
| `boundedness_equals_deposit_at_end_when_saturating` | Bounds: exact saturation |
| `boundedness_less_than_deposit_at_end_when_undersaturating` | Bounds: undersaturation |
| `determinism_hundred_iterations` | Determinism: 100 iterations |
| `zero_time_before_cliff_returns_zero` | Edge: t=0 before cliff |
| `zero_time_at_start_returns_zero` | Edge: t=0 at start |
| `max_time_caps_at_deposit` | Edge: u64::MAX time |
| `max_time_all_maxima` | Edge: all u64::MAX parameters |
| `negative_rate_returns_zero` | Edge: negative rate protection |
| `frozen_accrual_at_cancel_time` | Integration: cancelled stream simulation |

---

## Verification Performed

### Automated Tests

```bash
cargo test -p fluxora_stream accrual --lib
```

### Manual Review Checklist

- [x] Reviewed `calculate_accrued_amount` logic
- [x] Verified bounds clamping: `accrued.min(deposit_amount).max(0)`
- [x] Verified monotonicity guard: `current_time.min(end_time)`
- [x] Verified overflow protection: `checked_mul` with deposit fallback
- [x] Verified underflow guard: `checked_sub` with 0 fallback
- [x] Reviewed integration with `calculate_accrued` wrapper in `lib.rs`
- [x] Verified status handling (Completed, Cancelled, Paused, Active)

---

## Security Analysis

### What these tests guard against

1. **Accrual inflation**: Bounds tests ensure `accrued <= deposit` regardless of rate,
   elapsed time, or overflow conditions. Prevents recipients from draining more
   tokens than deposited.

2. **Accrual reversal / double-spend**: Monotonicity tests ensure accrued amounts
   never decrease. Prevents withdrawal, reset, withdrawal attacks.

3. **Integer overflow**: Overflow boundary tests verify `checked_mul` guard works
   at exact overflow points.

4. **Integer underflow**: Elapsed underflow tests verify `checked_sub` guard works
   for degenerate schedules (cliff < start).

5. **Non-determinism**: Determinism tests verify pure function behavior. Prevents
   block-dependent or environment-dependent accrual.

### Relationship to Existing Controls

- `checked_mul` overflow guard → exercised by overflow boundary tests
- `deposit_amount` clamp → exercised by boundedness tests
- `validate_stream_params` validation → complement, not replacement
- CEI ordering → not affected (pure function)

---

## Role Participation

| Role | May Call | Must Prove | Cannot Do |
|------|----------|------------|-----------|
| Anyone | `calculate_accrued` | Nothing | N/A (permissionless read) |
| Recipient | `withdraw` | Authorization | Withdraw before cliff |
| Sender | `cancel` | Authorization | Cancel completed stream |
| Admin | `cancel_as_admin` | Authorization | Resume completed |

---

## Error Semantics

The pure function `calculate_accrued_amount` returns 0 for invalid inputs:

| Input Condition | Return | Rationale |
|-----------------|--------|-----------|
| `current_time < cliff_time` | 0 | Before cliff, nothing vests |
| `rate_per_second < 0` | 0 | Protected (validation rejects) |
| `elapsed_now < start_time` | 0 | Elapsed underflow guard |
| `elapsed * rate > deposit` | `deposit` | Overflow cap |
| `start_time >= end_time` | 0 | Invalid schedule (validation rejects) |

The contract's `calculate_accrued` wrapper adds status-based behavior:

| Status | Behavior |
|--------|----------|
| Active | Uses `env.ledger().timestamp()` |
| Paused | Uses `env.ledger().timestamp()` (accrual continues) |
| Completed | Returns `deposit_amount` (deterministic) |
| Cancelled | Uses `cancelled_at` (frozen accrual) |

---

## Documentation Consistency

### For Integrators (Wallets, Indexers, Treasury Tooling)

```rust
// Expected accrual at time T for Active/Paused streams:
fn expected_accrual(stream: &Stream, current_time: u64) -> i128 {
    if current_time < stream.cliff_time {
        return 0;
    }
    let elapsed = current_time.min(stream.end_time) - stream.start_time;
    (elapsed * stream.rate_per_second).min(stream.deposit_amount).max(0)
}

// For Cancelled streams: use cancelled_at instead of current_time
// For Completed streams: return deposit_amount directly
```

### For Auditors

The pure function tests verify observable behavior only. No hidden assumptions.

---

## Residual Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Environment time manipulation | Low | Medium | Contract uses ledger timestamp, not wall clock |
| Floating-point imprecision | None | N/A | All math is integer-based |
| Compiler optimization bugs | Very Low | High | Rust safe math with checked operations |
| Test gap for unvisited paths | Low | Medium | Coverage tools verify line/branch coverage |
