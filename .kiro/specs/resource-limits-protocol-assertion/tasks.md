# Implementation Tasks

## Task List

- [ ] 1. Create `contracts/stream/src/protocol_limits.rs`
  - Declare `pub const SOROBAN_PROTOCOL_VERSION: u32 = 21;` — this is the single authoritative protocol pin for the entire workspace
  - Define `pub struct ProtocolBudgetLimits` with six `pub` fields (all `u64`):
    - `single_withdraw_cpu_max`
    - `single_withdraw_mem_max`
    - `batch_withdraw_10_cpu_max`
    - `batch_withdraw_10_mem_max`
    - `create_streams_5_cpu_max`
    - `create_streams_5_mem_max`
  - Implement `pub const fn protocol_budget_limits(version: u32) -> ProtocolBudgetLimits` with an exhaustive `match`:
    - Arm `21` returns `ProtocolBudgetLimits { single_withdraw_cpu_max: 1_000_000, single_withdraw_mem_max: 500_000, batch_withdraw_10_cpu_max: 5_000_000, batch_withdraw_10_mem_max: 2_000_000, create_streams_5_cpu_max: 3_000_000, create_streams_5_mem_max: 1_500_000 }`
    - Wildcard arm: `_ => panic!("unknown Soroban protocol version — update protocol_limits.rs")` — this makes an unknown version a **compile error** when `LIMITS` is evaluated as a `const`
  - Export `pub const LIMITS: ProtocolBudgetLimits = protocol_budget_limits(SOROBAN_PROTOCOL_VERSION);`
  - **Acceptance**: `LIMITS.single_withdraw_cpu_max == 1_000_000`, `LIMITS.single_withdraw_mem_max == 500_000`, `LIMITS.batch_withdraw_10_cpu_max == 5_000_000`, `LIMITS.batch_withdraw_10_mem_max == 2_000_000`, `LIMITS.create_streams_5_cpu_max == 3_000_000`, `LIMITS.create_streams_5_mem_max == 1_500_000`

- [ ] 2. Register the new module in `contracts/stream/src/lib.rs`
  - Add `#[cfg(test)] mod protocol_limits;` directly after `#[cfg(test)] mod checksum;` on line 6 (between `mod checksum;` and `mod token_check;`)
  - The `#[cfg(test)]` gate ensures the module is compiled in test mode only and adds zero bytes to the WASM artifact
  - **Acceptance**: `cargo build --target wasm32-unknown-unknown -p fluxora_stream` compiles without errors or size change; `cargo test -p fluxora_stream` compiles without "unresolved module" errors

- [ ] 3. Replace raw literals in `test_budget_single_withdraw_hot_path` (`contracts/stream/src/test.rs` ~line 17089)
  - Add `use crate::protocol_limits::LIMITS;` at the top of the budget test section (or at module level inside `#[cfg(test)]`)
  - Replace `cpu <= 1_000_000` with `cpu <= LIMITS.single_withdraw_cpu_max`
  - Replace `mem <= 500_000` with `mem <= LIMITS.single_withdraw_mem_max`
  - Update the assertion messages to read `"single withdraw cpu={cpu} exceeds protocol-{} guardrail {}"` using `crate::protocol_limits::SOROBAN_PROTOCOL_VERSION` and the limit value
  - **Acceptance**: Test passes; no raw integer literals remain in this function's assert arms

- [ ] 4. Replace raw literals in `test_budget_batch_withdraw_10_streams` (`contracts/stream/src/test.rs` ~line 17145)
  - Replace `cpu <= 5_000_000` with `cpu <= LIMITS.batch_withdraw_10_cpu_max`
  - Replace `mem <= 2_000_000` with `mem <= LIMITS.batch_withdraw_10_mem_max`
  - Update assertion messages to include protocol version and derived limit value
  - **Acceptance**: Test passes; no raw integer literals remain in this function's assert arms

- [ ] 5. Replace raw literals in `test_budget_create_streams_batch_5` (`contracts/stream/src/test.rs` ~line 17323)
  - Replace `cpu <= 3_000_000` with `cpu <= LIMITS.create_streams_5_cpu_max`
  - Replace `mem <= 1_500_000` with `mem <= LIMITS.create_streams_5_mem_max`
  - Update assertion messages to include protocol version and derived limit value
  - **Acceptance**: Test passes; no raw integer literals remain in this function's assert arms

- [ ] 6. Verify the full test suite passes with the correct protocol pin
  - Run `cargo test -p fluxora_stream` and confirm all tests pass with zero regressions
  - Confirm `test_budget_withdraw_zero_short_circuit` and `test_budget_batch_withdraw_cheaper_than_n_singles` pass unchanged (they use relative comparisons, not absolute ceilings)
  - Run `cargo build --target wasm32-unknown-unknown -p fluxora_stream` and confirm it succeeds
  - **Acceptance**: All pre-fix tests pass; WASM build is clean

- [ ] 7. Validate CI enforcement — mismatch breaks the build
  - Temporarily change `SOROBAN_PROTOCOL_VERSION` in `protocol_limits.rs` from `21` to `99`
  - Run `cargo test -p fluxora_stream` and confirm the build **fails** with a compile error referencing the wildcard panic arm (not a test assertion failure — a compile-time error)
  - Restore `SOROBAN_PROTOCOL_VERSION` to `21` and confirm the build passes again
  - **Acceptance**: A protocol version not covered by the `match` arms causes a compile-time failure, satisfying the "mismatch fails CI" acceptance criterion
