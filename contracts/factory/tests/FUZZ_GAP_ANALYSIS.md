# Fuzz Coverage Gap Analysis for FluxoraFactory

## Issue #891: Add fuzz coverage for contracts/factory/tests/factory_fuzz.rs

### Current Coverage (Original factory_fuzz.rs)

The original `factory_fuzz.rs` contains a single proptest that fuzzes:
- `create_stream` with randomized:
  - `cap` (i128: 100..1_000_000)
  - `min_duration` (u64: 10..3600)
  - `deposit_amount` (i128: 1..2_000_000)
  - `duration` (u64: 1..7200)
  - `is_allowlisted` (bool)

Properties verified:
1. RecipientNotAllowlisted iff !is_allowlisted
2. DepositExceedsCap iff deposit_amount > cap
3. InvalidTimeRange iff start_time >= end_time
4. DurationTooShort iff duration < min_duration
5. No valid input is wrongly rejected

### Gap Analysis: Uncovered Public Entry Points

The following public functions in `factory/src/lib.rs` were **not** fuzzed:

#### Admin Setters (Numeric/Address Inputs)

1. **`init`**
   - Inputs: `max_deposit` (i128), `min_duration` (u64)
   - Validation: `max_deposit > 0`, `min_duration <= MAX_MIN_DURATION_SECONDS`
   - Risk: integer overflow/underflow, boundary conditions

2. **`set_cap`**
   - Input: `max_deposit` (i128)
   - Validation: `max_deposit > 0`
   - Risk: negative values, zero, overflow

3. **`set_min_duration`**
   - Input: `min_duration` (u64)
   - Validation: `min_duration <= MAX_MIN_DURATION_SECONDS`
   - Risk: exceeding max constant, boundary at MAX_MIN_DURATION_SECONDS

4. **`set_rate_bounds`**
   - Inputs: `min_rate` (Option<i128>), `max_rate` (Option<i128>)
   - Validation: non-negative values, min <= max when both set
   - Risk: negative rates, inverted bounds, None handling

5. **`set_factory_paused`**
   - Input: `paused` (bool)
   - Validation: none (simple toggle)
   - Risk: low, but state persistence should be verified

#### View Functions (Numeric Inputs)

6. **`get_factory_streams_paginated`**
   - Inputs: `start_index` (u32), `limit` (u32)
   - Validation: none (should handle extreme values gracefully)
   - Risk: out-of-bounds access, integer overflow in pagination logic

### Extended Fuzz Harness (factory_fuzz_extended.rs)

Added 6 new proptest properties covering all gaps:

1. **`prop_init_cap_validation`**
   - Fuzzes `max_deposit` across full i128 range
   - Fuzzes `min_duration` beyond MAX_MIN_DURATION_SECONDS
   - Verifies: InvalidCap iff max_deposit <= 0
   - Verifies: InvalidMinDuration iff min_duration > MAX_MIN_DURATION_SECONDS

2. **`prop_set_cap_validation`**
   - Fuzzes `new_cap` across full i128 range
   - Verifies: InvalidCap iff new_cap <= 0

3. **`prop_set_min_duration_validation`**
   - Fuzzes `new_min_duration` beyond MAX_MIN_DURATION_SECONDS
   - Verifies: InvalidMinDuration iff new_min_duration > MAX_MIN_DURATION_SECONDS

4. **`prop_set_rate_bounds_validation`**
   - Fuzzes `min_rate` and `max_rate` as Option<i128>
   - Verifies: InvalidRateBounds iff min < 0 OR max < 0 OR (both set and min > max)

5. **`prop_set_factory_paused_toggle`**
   - Fuzzes `paused` as bool
   - Verifies: state persistence matches input

6. **`prop_paginated_boundary_conditions`**
   - Fuzzes `start_index` (0..1000) and `limit` (0..200)
   - Verifies: no panic on extreme values

### Prioritization Rationale

Prioritized functions with:
- **Numeric inputs** (i128, u64, u32) — higher risk of overflow/underflow
- **Validation logic** — complex error paths
- **Boundary conditions** — edge cases at min/max values

De-prioritized:
- Address-shaped inputs (already validated by Soroban SDK)
- State-machine flows (covered by existing unit tests)
- Allowlist operations (simple boolean logic)

### Execution Plan

1. Run each property test with default proptest config (256 cases per property)
2. Monitor for panics, assertion failures, or unexpected errors
3. Report any crashes with minimal reproduction case
4. Do NOT silently patch — report clearly with stack trace
