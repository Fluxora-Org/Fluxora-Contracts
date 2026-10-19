# Resource Limits Protocol Assertion Bugfix Design

## Overview

Budget guardrail tests in `contracts/stream/src/test.rs` assert CPU and memory
ceilings using raw numeric literals (`1_000_000`, `500_000`, etc.) that have no
derivation path from the pinned Soroban protocol version. The codebase pins a
Rust toolchain (`rust-toolchain.toml`: `1.94.1`) and an SDK version
(`contracts/stream/Cargo.toml`: `soroban-sdk 21.7.7`), but neither file
records the target Soroban **protocol number**, and no code enforces that the
test limits correspond to what that protocol mandates on-chain.

The fix introduces a single authoritative file,
`contracts/stream/src/protocol_limits.rs`, that:

1. Declares `SOROBAN_PROTOCOL_VERSION: u32` as the sole canonical pin.
2. Derives all CPU/memory ceiling constants from that version via a `const fn`
   that is exhaustive over every known version — making an unknown version a
   compile error.
3. Is imported by the budget guardrail tests so that all assertions reference
   the derived constants, not raw literals.

Any future protocol bump that is not accompanied by an updated limits table
will break the build, making the coupling between protocol pin and test limits
automatic and visible in CI.

---

## Glossary

- **Bug_Condition (C)**: A test limit constant that is a raw numeric literal
  with no derivation path from `SOROBAN_PROTOCOL_VERSION`.
- **Property (P)**: After the fix, every CPU/memory ceiling used in a budget
  guardrail assertion is computed from `SOROBAN_PROTOCOL_VERSION` via
  `protocol_budget_limits()`.
- **Preservation**: All existing budget guardrail tests continue to pass; no
  other tests are affected; both compilation targets (`native` and
  `wasm32-unknown-unknown`) remain error-free.
- **`protocol_budget_limits()`**: The `const fn` in `protocol_limits.rs` that
  maps a protocol version number to a `ProtocolBudgetLimits` struct holding all
  CPU/memory ceilings.
- **`SOROBAN_PROTOCOL_VERSION`**: The `const u32` in `protocol_limits.rs` that
  is the single authoritative record of the target Soroban protocol.
- **`ProtocolBudgetLimits`**: A `const`-constructable struct holding all
  derived ceiling values for a specific protocol version.
- **soroban-sdk 21.7.7**: The SDK version pinned in
  `contracts/stream/Cargo.toml`; maps to Soroban protocol 21 on Stellar
  networks.

---

## Bug Details

### Bug Condition

The bug manifests when a budget guardrail test asserts a CPU or memory ceiling
using a raw integer literal rather than a constant derived from
`SOROBAN_PROTOCOL_VERSION`. The test function compiles and runs regardless of
what protocol version is pinned elsewhere, so the assertion can be silently
stale.

**Formal Specification:**

```
FUNCTION isBugCondition(input)
  INPUT: input of type (pinnedProtocolVersion: u32, testLimitConstant: u64)
  OUTPUT: boolean

  RETURN testLimitConstant IS hardcoded_literal
         AND NOT derivedFrom(testLimitConstant, pinnedProtocolVersion)
END FUNCTION
```

### Examples

- **Single withdraw ceiling** — `cpu <= 1_000_000` and `mem <= 500_000` appear
  as raw literals in `test_budget_single_withdraw_hot_path`. Expected: these
  values should be `LIMITS.single_withdraw_cpu_max` and
  `LIMITS.single_withdraw_mem_max` from `protocol_limits.rs`.

- **Batch withdraw (10) ceiling** — `cpu <= 5_000_000` and `mem <= 2_000_000`
  appear as raw literals in `test_budget_batch_withdraw_10_streams`. Expected:
  these values should be `LIMITS.batch_withdraw_10_cpu_max` and
  `LIMITS.batch_withdraw_10_mem_max`.

- **Batch create (5) ceiling** — `cpu <= 3_000_000` and `mem <= 1_500_000`
  appear as raw literals in `test_budget_create_streams_batch_5`. Expected:
  these values should be `LIMITS.create_streams_5_cpu_max` and
  `LIMITS.create_streams_5_mem_max`.

- **Protocol bump edge case** — if `SOROBAN_PROTOCOL_VERSION` is changed to an
  unknown version (e.g. `99`), `protocol_budget_limits()` should produce a
  compile error (via `panic!` in a const context or an explicit `unreachable!()`
  that exhausts all known arms), forcing the developer to add the new limits
  table before CI can pass.

---

## Expected Behavior

### Preservation Requirements

**Unchanged Behaviors:**

- `test_budget_single_withdraw_hot_path`, `test_budget_batch_withdraw_10_streams`,
  and `test_budget_create_streams_batch_5` must continue to pass with the same
  pass/fail semantics when the protocol pin is correct.
- `test_budget_withdraw_zero_short_circuit` and
  `test_budget_batch_withdraw_cheaper_than_n_singles` (relative-cost tests) must
  continue to pass unchanged; they do not use absolute ceilings so they are
  unaffected by this fix.
- All other test files under `contracts/stream/src/` and
  `contracts/stream/tests/` must produce the same pass/fail outcome as before.
- `cargo build` and `cargo build --target wasm32-unknown-unknown` must succeed
  without warnings introduced by this change (the new module is `#[cfg(test)]`
  only and does not affect the WASM artifact).

**Scope:**

All inputs that do NOT involve the budget guardrail ceiling constants are
completely unaffected. This includes:
- Contract logic (`lib.rs`, `accrual.rs`, `delegation.rs`, etc.)
- Non-budget tests (auth, state, event, integration tests)
- The WASM build artifact (the new module is test-only)

---

## Hypothesized Root Cause

Based on the bug description and code inspection, the root causes are:

1. **No dedicated version record**: The codebase pins the Rust toolchain and
   the SDK version but has no `const` that names the Soroban **protocol
   number**. Because the protocol number is implicit, nothing can reference it.

2. **Limits written at assertion time**: The ceiling values
   (`1_000_000`, `5_000_000`, etc.) were typed directly into `assert!` macro
   arguments. There is no intermediate named constant that could signal to a
   reviewer which protocol version they correspond to.

3. **No exhaustiveness enforcement**: Even if a developer added a named
   constant, a simple `const CPU_MAX: u64 = 1_000_000;` would not break if the
   protocol version was bumped — the constant would just be stale. Only a
   `match`/`const fn` that enumerates known versions can enforce coupling.

4. **`withdrawal_frequency.rs` uses `protocol_version: 20`**: A separate test
   file already hard-codes a different protocol version (`20`) in a `LedgerInfo`
   struct. This demonstrates that the mismatch problem already exists elsewhere
   and that `SOROBAN_PROTOCOL_VERSION` should become the single source referenced
   by all protocol-sensitive tests.

---

## Correctness Properties

Property 1: Bug Condition — Limit Constants Are Derived from Protocol Version

_For any_ CPU or memory ceiling constant used in a budget guardrail assertion
(`test_budget_single_withdraw_hot_path`, `test_budget_batch_withdraw_10_streams`,
`test_budget_create_streams_batch_5`), the fixed code SHALL compute that
constant from `protocol_budget_limits(SOROBAN_PROTOCOL_VERSION)` rather than
expressing it as a raw numeric literal, so that `isBugCondition` returns `false`
for every limit constant after the fix is applied.

**Validates: Requirements 2.1, 2.2, 2.3, 2.4**

Property 2: Preservation — Non-Limit Test Behavior Is Unchanged

_For any_ test that does NOT reference the absolute CPU/memory ceiling constants
(i.e., `isBugCondition` returns `false` for that test's inputs), the fixed code
SHALL produce exactly the same pass/fail outcome as the original code, preserving
all existing contract behavior, build targets, and test results.

**Validates: Requirements 3.1, 3.2, 3.3, 3.4**

---

## Fix Implementation

### Changes Required

Assuming our root cause analysis is correct, the fix consists of two file
operations: creating a new constants module and updating the three budget
guardrail tests to reference it.

---

**File 1 (new)**: `contracts/stream/src/protocol_limits.rs`

**Purpose**: Single authoritative source of truth for the Soroban protocol
version and all derived resource ceilings.

**Specific Changes**:

1. **Declare `SOROBAN_PROTOCOL_VERSION`**: A `pub const u32` set to `21`,
   matching `soroban-sdk 21.7.7`. This is the sole place in the codebase where
   the protocol number is written.

2. **Define `ProtocolBudgetLimits` struct**: A plain `pub struct` with `pub`
   fields for every ceiling used in the guardrail tests:
   - `single_withdraw_cpu_max: u64`
   - `single_withdraw_mem_max: u64`
   - `batch_withdraw_10_cpu_max: u64`
   - `batch_withdraw_10_mem_max: u64`
   - `create_streams_5_cpu_max: u64`
   - `create_streams_5_mem_max: u64`

3. **Implement `protocol_budget_limits(version: u32) -> ProtocolBudgetLimits`**
   as a `pub const fn`. The body is a `match` on `version` with one arm for
   `21` returning the current ceilings (values identical to the existing raw
   literals, so tests continue to pass). A wildcard arm calls `panic!("unknown
   Soroban protocol version: …")`. Because this is a `const fn` and `LIMITS`
   is declared as a `const`, an unknown version is a **compile error**.

4. **Export `LIMITS` as a `pub const`**: Computed as
   `protocol_budget_limits(SOROBAN_PROTOCOL_VERSION)`, so every import site
   gets a zero-cost, already-resolved struct.

---

**File 2 (modified)**: `contracts/stream/src/test.rs`

**Purpose**: Replace raw literals in the three absolute-ceiling guardrail tests.

**Specific Changes**:

5. **Add module import**: At the top of the `#[cfg(test)]` section (or inside
   each budget test), bring `crate::protocol_limits::LIMITS` into scope.

6. **`test_budget_single_withdraw_hot_path`**: Replace:
   ```rust
   assert!(cpu <= 1_000_000, …);
   assert!(mem <= 500_000, …);
   ```
   with:
   ```rust
   assert!(cpu <= LIMITS.single_withdraw_cpu_max, …);
   assert!(mem <= LIMITS.single_withdraw_mem_max, …);
   ```

7. **`test_budget_batch_withdraw_10_streams`**: Replace:
   ```rust
   assert!(cpu <= 5_000_000, …);
   assert!(mem <= 2_000_000, …);
   ```
   with:
   ```rust
   assert!(cpu <= LIMITS.batch_withdraw_10_cpu_max, …);
   assert!(mem <= LIMITS.batch_withdraw_10_mem_max, …);
   ```

8. **`test_budget_create_streams_batch_5`**: Replace:
   ```rust
   assert!(cpu <= 3_000_000, …);
   assert!(mem <= 1_500_000, …);
   ```
   with:
   ```rust
   assert!(cpu <= LIMITS.create_streams_5_cpu_max, …);
   assert!(mem <= LIMITS.create_streams_5_mem_max, …);
   ```

---

**File 3 (modified)**: `contracts/stream/src/lib.rs`

**Specific Changes**:

9. **Declare the new module**: Add `#[cfg(test)] mod protocol_limits;` alongside
   the existing `#[cfg(test)] mod checksum;` declaration so the module is only
   compiled in test mode and does not affect the WASM artifact.

---

### No Changes Required

- `rust-toolchain.toml` — already records the Rust channel; does not need
  modification for this fix.
- `contracts/stream/Cargo.toml` — already records `soroban-sdk 21.7.7`; the
  protocol number `21` is derivable from this and is now captured in
  `protocol_limits.rs`.
- `contracts/stream/tests/withdrawal_frequency.rs` — uses `protocol_version: 20`
  in a `LedgerInfo` struct for a different, non-budget purpose. Migrating it is
  out of scope for this fix (see Preservation Requirements).

---

## Testing Strategy

### Validation Approach

The testing strategy follows a two-phase approach: first, verify the bug
condition holds on the unfixed code (counterexamples exist), then verify the fix
satisfies Property 1 (all limits derived) and Property 2 (no regressions).

---

### Exploratory Bug Condition Checking

**Goal**: Surface counterexamples that demonstrate the bug BEFORE implementing
the fix. Confirm or refute the root cause analysis.

**Test Plan**: Inspect `test.rs` statically and confirm raw literals exist with
no reference to any protocol-version constant. Then introduce a deliberately
wrong ceiling (e.g., temporarily change the raw literal `1_000_000` to `1`) and
confirm the test fails, proving the assertion actually guards something. Finally,
verify that changing the `1_000_000` back and simultaneously bumping a hypothetical
`SOROBAN_PROTOCOL_VERSION` (before the fix) does NOT break the test — confirming
the decoupling bug.

**Test Cases**:

1. **Raw literal inspection** (static): Grep `test.rs` for `cpu <=` and
   `mem <=` and verify the right-hand side is a raw integer (will succeed on
   unfixed code — confirms bug condition).

2. **Ceiling too low** (dynamic): Temporarily set the raw literal ceiling to `1`
   in `test_budget_single_withdraw_hot_path` and run the test. It must fail,
   proving the assertion is reachable (will fail on unfixed code — expected
   counterexample).

3. **Protocol bump is silent** (structural): Before the fix, change no test
   literals but pretend the protocol is `22`. Run tests — they still pass,
   confirming the decoupling (will pass on unfixed code — expected
   counterexample demonstrating the bug).

**Expected Counterexamples**:

- Budget guardrail tests use magic numbers with no named constant linking them
  to a protocol version.
- Changing the SDK version or protocol number leaves all assertions untouched.

---

### Fix Checking

**Goal**: Verify that for all inputs where the bug condition holds, the fixed
code produces the expected behavior (limits are derived, not hardcoded).

**Pseudocode:**

```
FOR ALL limitConstant WHERE isBugCondition(limitConstant) DO
  result ← protocol_budget_limits(SOROBAN_PROTOCOL_VERSION).limitConstant
  ASSERT derivedFrom(result, SOROBAN_PROTOCOL_VERSION)
         AND result == previousRawLiteral   // values unchanged → tests still pass
END FOR
```

**Validation**: After the fix, run all three absolute-ceiling budget tests with
the correct protocol pin. They must pass with identical semantics to before.
Then change `SOROBAN_PROTOCOL_VERSION` to an unknown value (e.g., `99`) and
attempt to compile — the build must fail with the `panic!` in the wildcard arm
of `protocol_budget_limits`, proving CI enforcement.

---

### Preservation Checking

**Goal**: Verify that for all inputs where the bug condition does NOT hold, the
fixed code produces the same result as the original code.

**Pseudocode:**

```
FOR ALL test WHERE NOT isBugCondition(test) DO
  ASSERT run_test_original(test) == run_test_fixed(test)
END FOR
```

**Testing Approach**: Property-based testing is recommended for the numeric
derivation property because it can verify across many synthetic protocol version
inputs that the `const fn` is deterministic and returns the correct struct for
version `21`. For the broader preservation check, running the full test suite
is the primary method.

**Test Plan**: Run the full `cargo test` suite after the fix and compare results
against the pre-fix baseline.

**Test Cases**:

1. **Full test suite pass**: All tests in `contracts/stream/src/test.rs` and
   `contracts/stream/tests/` produce the same pass/fail outcome as before.

2. **WASM build unchanged**: `cargo build --target wasm32-unknown-unknown`
   produces a WASM artifact with the same exported symbols and no new size
   overhead (the new module is `#[cfg(test)]` only).

3. **Relative-cost tests unaffected**: `test_budget_withdraw_zero_short_circuit`
   and `test_budget_batch_withdraw_cheaper_than_n_singles` use relative
   comparisons only and must pass without modification.

---

### Unit Tests

- Compile `protocol_limits.rs` in isolation and assert
  `protocol_budget_limits(21).single_withdraw_cpu_max == 1_000_000`.
- Assert each field of `ProtocolBudgetLimits` for protocol `21` matches the
  original raw literals, so the semantic threshold is preserved.
- Assert `LIMITS` (the top-level `const`) resolves without a compile error when
  `SOROBAN_PROTOCOL_VERSION == 21`.

### Property-Based Tests

- Generate arbitrary `u32` values for the protocol version in range `[0, 20]`
  and `[22, u32::MAX]` and assert that `protocol_budget_limits(v)` panics at
  compile time (or at runtime in a non-const context) — confirming exhaustiveness.
- For version `21`, generate random multipliers and assert all derived fields
  scale proportionally if the `const fn` is ever refactored to use arithmetic
  rather than a lookup table.
- Assert that `LIMITS.single_withdraw_cpu_max <= LIMITS.batch_withdraw_10_cpu_max`
  (batch ceiling must not be lower than a single-op ceiling), as a sanity
  invariant across the struct fields.

### Integration Tests

- Run `cargo test --test '*'` targeting `contracts/stream` and confirm zero
  regressions.
- Run `cargo build --target wasm32-unknown-unknown -p fluxora_stream` and
  confirm it compiles cleanly.
- Run the CI workflow (`.github/workflows/ci.yml`) locally and confirm the
  budget guardrail step passes with `SOROBAN_PROTOCOL_VERSION = 21` and fails
  when the version is set to an unknown value, demonstrating automatic
  enforcement.
