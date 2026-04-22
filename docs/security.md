# Security

Notes for auditors and maintainers on security-relevant patterns used in the Fluxora stream contract.

## Checks–Effects–Interactions (CEI)

The contract follows the **Checks-Effects-Interactions** pattern to reduce reentrancy risk.
State updates are performed **before** any external token transfers in all functions that move funds.

- **`create_streams`**  
  The contract requires sender auth once, validates every batch entry first, and computes the total deposit with checked arithmetic before any token transfer. It then performs one pull transfer for the total and persists streams. If any validation/overflow/transfer step fails, Soroban reverts the transaction: no streams are stored and no creation events remain on-chain.

- **`withdraw`**  
  After all checks (auth, status, withdrawable amount), the contract updates `withdrawn_amount` and, when applicable, sets status to `Completed`, then persists the stream with `save_stream`. Only after that does it call the token contract to transfer tokens to the recipient.
  Completion is only allowed from `Active` status; cancelled streams remain `Cancelled` even when their accrued portion is fully withdrawn.

After all checks (auth, status, withdrawable amount), the contract:

1. Updates `withdrawn_amount` in the stream struct.
2. Conditionally sets `status` to `Completed` if the stream is now fully drained.
3. Calls `save_stream` to persist the new state.
4. **Only then** calls the token contract to transfer tokens to the recipient.

### `cancel_stream` and `cancel_stream_as_admin`

After checks and computing the refund amount, the contract:

1. Sets `stream.status = Cancelled` and records `cancelled_at`.
2. Calls `save_stream` to persist the updated state.
3. **Only then** transfers the unstreamed refund to the sender.

Both sender/admin cancellation entrypoints route through the same internal logic.
This guarantees identical externally visible semantics (state fields, refund math,
and emitted event shape) regardless of which authorized role executed the cancel.

Refund invariant for reviewers:

`refund_amount = deposit_amount - accrued_at(cancelled_at)`

where `accrued_at(cancelled_at)` is frozen for all future reads after cancellation.

### `top_up_stream`

After authorization and amount validation, the contract:

1. Increases `stream.deposit_amount` with overflow protection.
2. Calls `save_stream` to persist the new deposit amount.
3. **Only then** calls the token contract to pull the top-up amount from the funder (`pull_token`).

Observable contract guarantees for this entrypoint:

- Auth boundary: only `funder.require_auth()` is enforced. The contract does not restrict `funder` to the stream sender or admin.
- State boundary: only `Active` and `Paused` streams may be topped up. `Completed` and `Cancelled` return `ContractError::InvalidState`.
- Success surface: `deposit_amount` increases exactly by `amount`; schedule fields, `withdrawn_amount`, and `status` remain unchanged.
- Failure surface: invalid amount (`InvalidParams`), arithmetic overflow (`ArithmeticOverflow`), auth failure, or failed token pull leave storage and balances unchanged and emit no `top_up` event.

> **Audit note (resolved):** Prior to the fix in this change, `top_up_stream` pulled
> tokens from the funder _before_ persisting the updated `deposit_amount`. This violated
> CEI ordering: if the token contract had re-entered the stream contract between the
> external transfer and the `save_stream` call, it could have observed a stale
> `deposit_amount`. The call order has been corrected so state is always persisted first.


### `batch_withdraw` and `batch_withdraw_to`

These batch functions process multiple internal transfers. CEI is maintained per-iteration:
1. Stream state (`withdrawn_amount` and `status`) is updated and saved.
2. The running `contract_balance` is decremented in memory.
3. **Only then** is the `push_token` external call made to transfer funds to the recipient (or specified destination).
This ensures that any reentrancy from the token contract observes the completely updated stream state and bounded remaining contract balance.

### `shorten_stream_end_time`

Authorization and state gate:
- Caller must be the stream `sender`.
- Stream must be `Active` or `Paused` (terminal states return `InvalidState`).

Parameter/time gate (`InvalidParams` on failure):
- `new_end_time > now` (strictly future; equality is rejected).
- `new_end_time > start_time`.
- `new_end_time >= cliff_time`.
- `new_end_time < old_end_time` (strictly shorter; equal/later values are rejected).

Success path (CEI order):
1. Updates `stream.end_time` and `stream.deposit_amount`.
2. Calls `save_stream`.
3. **Only then** transfers the refund to the sender.
4. Emits `end_shrt(stream_id)` with `StreamEndShortened { old_end_time, new_end_time, refund_amount }`.

Failure path:
- No state changes.
- No token transfer.
- No `end_shrt` event.

Refund invariant:
- `refund_amount = old_deposit_amount - rate_per_second × (new_end_time - start_time)`
- On success, sender balance increases by `refund_amount` and contract token balance decreases by `refund_amount`.

### `withdraw_to`

Same ordering as `withdraw`; state is updated and saved before tokens are transferred
to the `destination` address.

---

## Token trust model

The contract interacts with exactly one token, fixed at `init` time and stored in
`Config.token`. This token is assumed to be a well-behaved SEP-41 / SAC token that:

- Does not re-enter the stream contract on `transfer`.
- Does not silently fail (panics or returns an error on insufficient balance).

If a malicious token is used, the CEI ordering above reduces (but does not eliminate)
reentrancy impact — state will already reflect the current operation when the re-entry occurs.

**Comprehensive documentation**: See [`token-assumptions.md`](token-assumptions.md) for the complete token trust model, explicit non-goals, and residual risks.

---

## Authorization paths

| Operation                 | Authorized callers                                      |
| ------------------------- | ------------------------------------------------------- |
| `create_stream`           | Sender (the address supplied as `sender`)               |
| `create_streams`          | Sender (once for the whole batch)                       |
| `pause_stream`            | Stream's `sender`                                       |
| `pause_stream_as_admin`   | Contract admin                                          |
| `resume_stream`           | Stream's `sender`                                       |
| `resume_stream_as_admin`  | Contract admin                                          |
| `cancel_stream`           | Stream's `sender`                                       |
| `cancel_stream_as_admin`  | Contract admin                                          |
| `withdraw`                | Stream's `recipient`                                    |
| `withdraw_to`             | Stream's `recipient`                                    |
| `batch_withdraw`          | Caller supplied as `recipient` (once for batch)         |
| `batch_withdraw_to`       | Caller supplied as `recipient` (once for batch)         |
| `update_rate_per_second`  | Stream's `sender`                                       |
| `shorten_stream_end_time` | Stream's `sender`                                       |
| `extend_stream_end_time`  | Stream's `sender`                                       |
| `top_up_stream`           | `funder` (any address; no sender relationship required) |
| `close_completed_stream`  | Permissionless (any caller)                             |
| `set_admin`               | Current contract admin                                  |
| `set_contract_paused`     | Contract admin                                          |

Cancellation-specific boundary checks:

1. Sender path (`cancel_stream`) cannot be executed by recipient or third party.
2. Admin path (`cancel_stream_as_admin`) cannot be executed by non-admin callers.
3. Streams in terminal states (`Completed`, `Cancelled`) are rejected with `InvalidState`.

---

## Overflow protection

All arithmetic that could overflow `i128` uses Rust's `checked_*` methods:

- `validate_stream_params`: `rate_per_second.checked_mul(duration)` — panics with a
  descriptive message if the product overflows. This is a deliberate fail-fast: supplying
  a rate and duration whose product cannot be represented as `i128` is always a caller error.
- `create_streams`: `total_deposit.checked_add(params.deposit_amount)` for batch totals.
- `top_up_stream`: `stream.deposit_amount.checked_add(amount)`.
- `update_rate_per_second` and `shorten/extend_stream_end_time`: each use `checked_mul`
  when re-validating the total streamable amount.
- `accrual::calculate_accrued_amount`: uses saturating/checked arithmetic and clamps the
  result at `deposit_amount`, ensuring `calculate_accrued` never returns a value greater
  than the deposited amount regardless of elapsed time or rate.

---

## Global pause

`set_contract_paused(true)` causes `create_stream` and `create_streams` to fail with
`ContractError::ContractPaused`. Existing streams are unaffected — withdrawals,
cancellations, and other operations continue normally. The pause flag is stored in
instance storage under `DataKey::CreationPaused`.

---

## Re-initialization prevention

`init` is bootstrap-authenticated and one-shot:

- It requires `admin.require_auth()` from the declared bootstrap admin.
- It checks `DataKey::Config` and panics with `"already initialised"` on any second call.

This ordering ensures that if a downstream token contract or hook re-enters the stream contract, the on-chain state (e.g. `withdrawn_amount`, `status`) already reflects the current operation, limiting reentrancy impact. For broader reentrancy mitigation, see [Issue #55](https://github.com/Fluxora-Org/Fluxora-Contracts/issues/55).

## Arithmetic Safety

The contract employs exhaustive arithmetic safety checks across all fund-related operations.

- **Checked Math**: All additions and multiplications involving `deposit_amount`, `rate_per_second`, or stream durations use `checked_*` methods to prevent overflows.
- **Structured Error Signals**: Arithmetic failures (such as a batch deposit exceeding `i128::MAX`) no longer trigger generic string-based panics. Instead, they emit a formal `ContractError::ArithmeticOverflow` (code 6). This provides crisp, programmable failure semantics for indexers, wallets, and treasury tooling.
- **Defensive Ordering**: In `top_up_stream`, the overflow check is performed **before** the token transfer. This prevents unnecessary token movement (and associated gas costs) for transactions destined to fail.
- **Accrual Capping**: Per-second accrual math implicitly caps at the `deposit_amount` on multiplication overflow, ensuring that technical overflows cannot be exploited to drain the contract beyond its funded limits.
This prevents unauthorized bootstrap and prevents later repointing to a different token
address or replacing the admin through `init`.

---

## Malicious Token Assumptions and Non-Goals

The streaming contract makes explicit assumptions about token behavior and defines clear non-goals for malicious token scenarios. These are documented in detail in [`token-assumptions.md`](token-assumptions.md).

### Key Assumptions

1. **No reentrancy**: The token contract does not call back into the streaming contract during transfers.
2. **Explicit failures**: The token contract panics or returns errors on insufficient balance/allowance, rather than silently failing.
3. **Standard SEP-41 interface**: The token implements the standard Soroban token interface.
4. **Deterministic behavior**: Token operations produce consistent, predictable results.

### Explicit Non-Goals

The following are **intentionally not mitigated** by the streaming contract:

1. **Malicious token contracts**: The contract does not protect against tokens that violate SEP-41 guarantees.
2. **Token supply manipulation**: The contract does not monitor or restrict token supply changes.
3. **Token upgradeability**: The contract does not protect against token contract upgrades that change behavior.
4. **Token balance verification**: The contract does not verify that actual token balances match internal accounting.
5. **Token allowance management**: The contract does not manage token allowances on behalf of users.
6. **Token decimals and precision**: The contract does not enforce or verify token decimal precision.

### Rationale

These non-goals are intentional design choices that:
- Reduce gas overhead and complexity
- Allow permissionless composability with any SEP-41 token
- Simplify the contract logic
- Place responsibility on token deployers and operators

### Residual Risks

1. **Non-standard tokens**: If a token violates SEP-41 guarantees, behavior may become unpredictable.
2. **Direct transfers**: Tokens sent directly to the contract address are permanently locked.
3. **Token upgrades**: If a token contract is upgraded to violate SEP-41 guarantees, behavior may change.

**Mitigation**: Use only well-audited, standard SEP-41 tokens. See [`token-assumptions.md`](token-assumptions.md) for detailed integration guidelines.

---

## Ledger Timestamp Assumptions (#313)

All time comparisons in the contract use `env.ledger().timestamp()`, which returns the
UNIX timestamp of the **current ledger close time** as a `u64`. The following invariants
are enforced and verified by boundary tests in `integration_suite.rs`.

### Cliff boundary

| Ledger time | `calculate_accrued` result | `withdraw` result |
|---|---|---|
| `< cliff_time` | `0` | `0` (no transfer, no state change) |
| `== cliff_time` | `(cliff_time − start_time) × rate_per_second` | full accrued amount |
| `> cliff_time` | linear accrual from `start_time` | withdrawable amount |

The cliff check is a **strict less-than** (`current_time < cliff_time`). At exactly
`T = cliff_time` the cliff is considered passed and accrual is computed from `start_time`.

### end_time boundary

| Ledger time | `calculate_accrued` result |
|---|---|
| `< end_time` | `(current_time − start_time) × rate_per_second` |
| `== end_time` | `deposit_amount` (capped) |
| `> end_time` | `deposit_amount` (capped; no extra accrual) |

Accrual uses `min(current_time, end_time)` before computing elapsed seconds, so the
result is deterministically capped at `deposit_amount` for all `T ≥ end_time`.

### Cancellation freeze

When `cancel_stream` or `cancel_stream_as_admin` executes, `cancelled_at` is set to
`env.ledger().timestamp()` at that instant. All subsequent calls to `calculate_accrued`
on a cancelled stream use `cancelled_at` as the effective `current_time`, freezing
accrual permanently. Advancing the ledger after cancellation does not increase the
withdrawable amount.

### start_time validation

`create_stream` and `create_streams` reject any `start_time < env.ledger().timestamp()`
with `ContractError::StartTimeInPast`. A `start_time` equal to the current ledger
timestamp is accepted (not considered "in the past").

### shorten_stream_end_time boundary

`new_end_time` must satisfy `new_end_time > env.ledger().timestamp()` (strictly future).
Equality with the current timestamp is rejected with `ContractError::InvalidParams`.

### Test coverage

All boundaries above are exercised by the `#[test]` functions in
`contracts/stream/tests/integration_suite.rs` under the `// Time-assumption boundary
tests (#313)` section. Each test uses `env.ledger().with_mut(|l| l.timestamp = ...)` to
set the ledger time precisely and asserts both the T−1 and T+1 cases around each gate.

---

## Reproducible WASM builds

The CI pipeline verifies that the WASM artifact produced by `cargo build --release --target wasm32-unknown-unknown` matches a committed reference checksum in `wasm/checksums.sha256`. This ensures that:

1. **Byte-identical output**: Any developer or CI runner with the pinned toolchain produces the same WASM binary.
2. **Supply chain integrity**: Changes to dependencies or toolchain that alter the WASM output are detected before merge.
3. **Auditability**: Auditors can independently rebuild and verify the deployed WASM matches the source.

### Determinism contract

| Factor                     | How it is pinned                                                |
|---------------------------|-----------------------------------------------------------------|
| Rust toolchain            | `rust-toolchain.toml` — `channel = "stable"`, targets pinned    |
| soroban-sdk version       | `contracts/stream/Cargo.toml` — `21.7.7` exact version          |
| Build profile             | `--release` with `wasm32-unknown-unknown` target                |
| Feature flags             | Only default features during WASM build (`testutils` is test-only) |
| `Cargo.lock`              | Committed; transitive dependencies locked                       |

### CI verification flow

1. Build WASM with pinned toolchain
2. Compute `sha256sum` of the artifact
3. Compare against `wasm/checksums.sha256`
4. Fail with actionable error if mismatch detected

### Updating checksums

When the contract source changes intentionally:

```bash
bash script/update-wasm-checksums.sh
git add wasm/checksums.sha256
git commit -m "chore: update wasm checksums"
```

### Residual risks

- **Optimized WASM**: The Stellar CLI `optimize` step may produce non-deterministic output. The reference checksum covers only the raw (unoptimized) WASM.
- **Cross-host builds**: The pinned `wasm32-unknown-unknown` target is deterministic across hosts, but minor differences in host libc or linker could theoretically affect non-WASM builds.
- **Dependency supply chain**: A compromised transitive dependency could alter WASM output. The `Cargo.lock` pin and checksum verification detect this at CI time.


## Accrual Fuzz Harness (#292)

Property-based tests for `calculate_accrued_amount` live in the `accrual_fuzz` module
inside `contracts/stream/src/accrual.rs`. They use the `proptest` crate to generate
arbitrary inputs and verify six mathematical invariants on every run.

### Fuzzing strategy

The harness generates random `(start_time, cliff_time, end_time, rate_per_second,
deposit_amount, current_time)` tuples via `proptest` strategies and asserts:

| # | Property | Assertion |
|---|---|---|
| 1 | **Boundedness** | `0 <= accrued <= deposit_amount` for all inputs |
| 2 | **Zero before cliff** | `accrued == 0` when `current_time < cliff_time` |
| 3 | **Monotonicity** | `accrued(t) <= accrued(t+1)` for all `t` |
| 4 | **Saturation** | `accrued == deposit` for all `t >= end_time` when `rate*(end-start) >= deposit` |
| 5 | **Determinism** | Same inputs always produce the same output |
| 6 | **Overflow safety** | No panic on any `i128`/`u64` combination, including `i128::MAX` rate and `u64::MAX` time |

### Edge cases targeted

- `rate_per_second = i128::MAX` with `elapsed_seconds = 2` → `checked_mul` overflows → returns `deposit_amount` (safe fallback)
- `current_time = u64::MAX` with any schedule → capped at `end_time` via `min(current_time, end_time)`
- `cliff_time > end_time` (degenerate schedule) → `current_time < cliff_time` always true → returns 0
- `deposit_amount = 0` → result is always 0 (bounded by deposit)
- `rate_per_second < 0` → returns 0 (negative rate guard)

### Running the harness

```bash
cargo test -p fluxora_stream accrual_fuzz
```

`proptest` runs 256 cases per property by default. To increase coverage:

```bash
PROPTEST_CASES=10000 cargo test -p fluxora_stream accrual_fuzz
```

### Found/fixed edge cases

No new bugs were found during initial harness development. The existing overflow
fallback (`None => deposit_amount` in `checked_mul`) was confirmed correct by
`prop_no_panic_on_extreme_inputs` and `prop_bounded_by_deposit`.
