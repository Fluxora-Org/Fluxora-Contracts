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

#### Optional Cancellation Fee (Security Properties)

If `cancellation_fee_bps > 0`, the protocol applies a fee only to the unstreamed refund:

1. Fee calculation: `fee = (refund_amount × cancellation_fee_bps) / 10000` (truncated down)
2. Sender refund: `refund_amount - fee`
3. **CRITICAL INVARIANT**: The recipient's accrued amount is **never** affected by the fee.
   - The recipient always receives `calculate_accrued(cancelled_at)` tokens, regardless of the fee.
   - Accrued tokens remain in the contract until the recipient calls `withdraw()`.

Security properties:
- **Fee only from unstreamed**: Fee is deducted from `sender_refund`, not from `recipient_accrued`.
- **Recipient safety**: `fee_bps` parameter cannot reduce the recipient's withdrawable balance.
- **No accrued truncation**: Accrual is independent of fee; `calculate_accrued` always returns the same value before and after cancellation.
- **Atomicity**: Fee is applied before any token transfer (CEI ordering).
- **Rounding safety**: Fee truncates down to prevent dust accumulation and ensure sender never receives more than allowed.
- **Validation**: `fee_bps` is validated to be in range `[0, 10000]`; values outside this range are rejected with `InvalidParams`.

Auditor checklist:
- [ ] Confirm `fee = (refund × fee_bps) / 10000` truncates down (no rounding up).
- [ ] Verify accrued calculation is independent of `cancellation_fee_bps`.
- [ ] Confirm recipient's `withdraw` receives full accrued, not reduced by fee.
- [ ] Check that fee is never applied to accrued amount.
- [ ] Verify state is persisted before token transfers (CEI).

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
| `sweep_excess`            | Contract admin **only** (recipient does not co-sign)    |
| `set_admin`               | Current contract admin                                  |
| `set_contract_paused`     | Contract admin                                          |
| `transfer_sender`         | Current stream sender                                   |

## Sweep excess — authorization model and liabilities invariant

### Authorization

`sweep_excess` requires **only** `admin.require_auth()`. The sweep destination
(`recipient` parameter) does **not** need to authorize. This is an intentional
design choice: the admin selects the destination address, so requiring the
recipient to co-sign would prevent sweeping to cold/offline treasury wallets
that cannot sign Soroban transactions.

> **Before V6**: `sweep_excess` incorrectly required `recipient.require_auth()`,
> forcing the destination wallet to co-sign every sweep. This blocked the
> common operational pattern of sweeping protocol-owned excess to a cold
> treasury.

### Liabilities invariant

The core safety property of `sweep_excess` is:

```
post_sweep_contract_balance >= read_total_liabilities
```

The function computes:

```
excess = contract_balance.saturating_sub(read_total_liabilities)
```

and transfers only `excess` to the recipient. If `excess <= 0`, no transfer
occurs and the function returns `0`. This ensures:

- Tokens that back active stream liabilities are never swept.
- A recipient's withdrawable entitlement is never affected by a sweep.
- Accrued protocol (keeper) fees are protected (as they are paid out immediately and thus naturally excluded from the contract balance).
- The contract always remains solvent after a sweep.

### Security properties

1. **Only admin can sweep**: A non-admin caller is rejected with
   `ContractError::Unauthorized`.
2. **Excess-only transfer**: `saturating_sub` ensures the transfer amount
   is bounded by the actual excess; if liabilities exceed the balance, zero
   is transferred.
3. **CEI ordering**: The excess is computed and the event is emitted **before**
   the token transfer, consistent with the rest of the contract.
4. **Reentrancy lock**: `acquire_reentrancy_lock` / `release_reentrancy_lock`
   protects the token transfer.
5. **Zero-excess no-op**: Calling when there is no excess returns `0` without
   any token transfer or state change.
6. **No recipient auth escalation**: The recipient address is specified by the
   admin; removing the recipient auth requirement does not give the admin any
   ability to drain funds that back active liabilities, because the transfer
   is bounded by `saturating_sub(total_liabilities)`.

## Admin powers

The stream contract admin can rotate the admin key, set governance-controlled
limits, pause or resume protocol operations, and use the explicit
`*_as_admin` stream controls listed in the authorization table above. These
powers do not replace sender or recipient authorization: stream creation still
requires the supplied `sender`, withdrawals still require the recipient, and
top-ups still require the supplied `funder`.

The optional factory wrapper has its own admin for allowlists, deposit caps,
minimum duration, and the configured stream-contract address. Factory policy
changes can decide whether a routed creation is accepted, but they do not give
the factory admin custody of sender funds. See the
[`fluxora_factory` authorization model](factory.md#cross-contract-authorization-model)
for the exact factory-to-stream signing tree.

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

## Global Emergency Pause

The contract supports two levels of pausing to manage risk:

1. **Creation Pause** (`set_creation_paused(true)`): Causes `create_stream` and `create_streams` to fail with `ContractError::ContractPaused`. Existing streams are unaffected — withdrawals, cancellations, and other operations continue normally. This is stored under `DataKey::CreationPaused`.
2. **Global Emergency Pause** (`set_contract_paused(true)`): A "circuit breaker" that blocks **all** mutation operations across the entire protocol. This includes creation, withdrawals, cancellations, rate updates, and time adjustments. This is stored under `DataKey::GlobalEmergencyPaused`.

During a Global Emergency Pause:
- New streams cannot be created.
- Recipients cannot withdraw accrued funds.
- Senders cannot cancel streams or recover refunds.
- All fund-moving entrypoints gated by `require_not_globally_paused` return `ContractError::ContractPaused`.

Read-only operations (`calculate_accrued`, `get_stream_state`) and admin-override functions remain operational so the protocol state can be audited and the pause can be lifted by the admin.

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

## Delegated withdraw (relayer support)

`delegated_withdraw` allows a relayer to execute a withdrawal on behalf of a recipient
using an off-chain Ed25519 signature. The design preserves all existing security
properties of `withdraw` while adding replay and expiry protection.

### Signature scheme

The recipient signs the following **40 raw bytes** (no prefix, no hashing):

```
stream_id              (8 bytes, u64 big-endian)
| nonce                (8 bytes, u64 big-endian)
| deadline             (8 bytes, u64 big-endian)
| expected_minimum_amount (16 bytes, i128 big-endian)
```

The 64-byte Ed25519 signature over these raw bytes is verified on-chain via
`env.crypto().ed25519_verify`. The caller supplies `recipient_public_key` (the
raw 32-byte Ed25519 key); the contract derives the corresponding Stellar account
address from that key and asserts it equals `stream.recipient` before calling
`ed25519_verify`. A key that does not match the stored recipient returns
`ContractError::InvalidSignature` immediately, without reaching the host-side
signature check.

**What the message binds:**
- `stream_id` — prevents using the same signature on a different stream.
- `nonce` — prevents replay; the recipient's nonce must match exactly.
- `deadline` — limits the window in which the signature is valid.
- `expected_minimum_amount` — closes the relayer front-running griefing vector:
  a relayer cannot delay until accrued < minimum, because the call reverts with
  `BelowMinimumAmount`.

> **Note on host trapping:** `env.crypto().ed25519_verify` is a Soroban host
> function that **traps** (panics) on an invalid signature rather than returning
> a typed error. This is by design in the Soroban SDK. The pre-condition checks
> (deadline, nonce, public-key binding) that precede it return typed
> `ContractError` variants; a relayer using `try_delegated_withdraw` will see
> `Err(Err(HostError))` only for a signature with wrong bytes after all
> pre-conditions pass. The distinction is observable on the client side.

### Trust assumption: recipient_public_key

The caller supplies `recipient_public_key` as a raw 32-byte Ed25519 public key.
The contract derives the Stellar account `Address` from that key and compares it
to `stream.recipient`. This check ensures that:

1. Only the actual stream recipient's key can authorize a delegated withdrawal.
2. A relayer cannot use an arbitrary self-generated key to burn the recipient's
   nonce or trigger an unintended withdrawal.
3. Contract-account recipients (not ed25519 accounts) cannot use `delegated_withdraw`;
   they must use the direct `withdraw` path.

### Replay protection (nonce)

- Each recipient has a per-address nonce stored under `DataKey::WithdrawNonce(recipient)`
  in persistent storage.
- The supplied `nonce` must equal the current stored nonce exactly — no skipping allowed.
- On a successful withdrawal that moves tokens, the nonce is incremented atomically
  **after** all checks and **before** the token transfer (CEI-compliant).
- If `withdrawable == 0` the nonce is **not** consumed, preserving the signature for
  a future call when tokens have accrued.

### Expiry (deadline)

- `deadline` is a ledger timestamp. The call is rejected with `SignatureDeadlineExpired`
  if `env.ledger().timestamp() > deadline`.
- A deadline equal to the current timestamp is accepted (not yet expired).

### Error codes for `delegated_withdraw`

| Condition | Error code |
|---|---|
| `env.ledger().timestamp() > deadline` | `SignatureDeadlineExpired` (19) |
| `nonce != stored_nonce` | `InvalidSignature` (15) |
| Public key does not derive `stream.recipient` | `InvalidSignature` (15) |
| Malformed/wrong ed25519 signature bytes | Host trap (`HostError`, not a typed `ContractError`) |
| `withdrawable < expected_minimum_amount` | `BelowMinimumAmount` (16) |
| Stream is Paused (non-terminal) or Completed | `InvalidState` |

### CEI ordering for `delegated_withdraw`

1. **Checks**: deadline → stream load → withdrawal frequency → nonce → public-key binding → signature.
2. **Effects**: update `withdrawn_amount`, optionally set `Completed`, call `save_stream`, increment nonce.
3. **Interactions**: `push_token` to `stream.recipient`, emit `withdrew` event (and optionally `completed` event).

### Authorization table addition

| Operation             | Authorized callers                                        |
|-----------------------|-----------------------------------------------------------|
| `delegated_withdraw`  | `relayer` (any address; recipient intent via Ed25519 signature) |
| `get_delegated_nonce` | Permissionless (view function)                            |

### Security invariants

- A used signature cannot be replayed (nonce incremented on success).
- An expired signature is rejected before any state change with `SignatureDeadlineExpired`.
- A signature from the wrong key is rejected by the public-key binding check with `InvalidSignature`.
- A malformed signature causes an `ed25519_verify` host trap (by Soroban SDK design).
- The `stream_id` and `expected_minimum_amount` are bound in the signed message.
- Tokens always transfer to `stream.recipient`; no relayer can redirect funds.
- Direct `withdraw` / `withdraw_to` / `batch_withdraw` are unaffected; their auth paths
  remain unchanged.
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
| Rust toolchain            | `rust-toolchain.toml` — channel and targets pinned; `rust-version = "1.94.1"` in each crate's `Cargo.toml` gives `cargo` an independent MSRV floor, cross-checked against the toolchain pin by `tests/test_rust_toolchain_pin.py` |
| soroban-sdk version       | `contracts/stream/Cargo.toml` — `21.7.7` exact version          |
| Build profile             | `--release` with `wasm32-unknown-unknown` target                |
| Feature flags             | Only default features during WASM build (`testutils` is test-only) |
| `Cargo.lock`              | Committed; transitive dependencies locked. CI-enforced: the `build` job runs `cargo update --locked --workspace` before any build step and fails if resolution would change it |

### CI verification flow

1. Build WASM with pinned toolchain (`cargo build --release --target wasm32-unknown-unknown`)
2. Run `bash script/verify-wasm-checksum.sh --no-build` — compares each artifact against `wasm/checksums.sha256`
3. Fail with actionable error message if any checksum mismatches
4. Upload raw and optimized WASM + hash files as CI artifacts (30-day retention)

### Local verification

To verify a build locally before deployment:

```bash
# Rebuild and verify in one step
bash script/verify-wasm-checksum.sh

# Verify existing artifacts without rebuilding
bash script/verify-wasm-checksum.sh --no-build
```

### Updating checksums

When the contract source changes intentionally:

```bash
bash script/update-wasm-checksums.sh
git add wasm/checksums.sha256
git commit -m "chore: update wasm checksums after <describe change>"
```

The script also accepts `--dry-run` to preview the new hashes without writing:

```bash
bash script/update-wasm-checksums.sh --dry-run
```

### Auditor verification steps

1. Clone the repository at the commit tagged for audit.
2. Confirm `rust-toolchain.toml` channel matches the CI build.
3. Run `bash script/verify-wasm-checksum.sh` — all entries must print `OK`.
4. Compare the passing hash against the on-chain contract hash via `stellar contract inspect`.

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

---

## Auto-claim Opt-in: Security Model

The auto-claim feature (`set_auto_claim` / `revoke_auto_claim` / `trigger_auto_claim`) introduces a permissionless trigger path. The following invariants ensure funds cannot be redirected or stolen.

### Destination immutability

The destination address is written to persistent storage by the recipient via `set_auto_claim`, which requires `recipient.require_auth()`. The caller of `trigger_auto_claim` supplies no destination parameter — the contract reads it from storage. There is no code path through which a third-party caller can influence where tokens are sent.

### CEI ordering in `trigger_auto_claim`

The function follows the same CEI pattern as `withdraw`:

1. All checks (stream exists, not Completed/Cancelled, time-terminal, destination set, not globally paused).
2. Compute withdrawable amount.
3. Update `stream.withdrawn_amount` and optionally set `status = Completed`.
4. Call `save_stream` to persist state.
5. **Only then** call `push_token` to transfer to destination.

### Global pause coverage

`trigger_auto_claim` calls `require_not_globally_paused` at entry, consistent with all other fund-moving entry points. During a global emergency pause, auto-claim triggers are blocked.

### Cancellation safety

If a stream is cancelled after opt-in, `trigger_auto_claim` returns `ContractError::InvalidState`. The stored destination entry is inert and does not affect the cancelled stream's accounting. Recipients may call `revoke_auto_claim` to reclaim the storage slot.

### No auth escalation

`trigger_auto_claim` does not call `require_auth` on any address. It is purely permissionless. The only privileged operation in the auto-claim flow is `set_auto_claim` (recipient auth) and `revoke_auto_claim` (recipient auth).

### Storage key isolation

Auto-claim destinations are stored under `DataKey::AutoClaimDestination(stream_id)` (discriminant 6), a separate persistent key from `DataKey::Stream(stream_id)` (discriminant 2). There is no cross-stream interference.

---

## Comprehensive Security Checklist

This checklist cross-references every security-relevant property of the stream
contract against its verification in `contracts/stream/tests/security_invariants.rs`,
the maintainer checklist (`docs/maintainer-security-checklist.md`), and the audit
register (`docs/audit.md`).

Use this as the entry point for any security review. Each item is either
covered by an automated test, a documented invariant, or an explicit risk
acceptance.

### Legend

| Marking | Meaning |
|---------|---------|
| ✅ Test | Validated by an automated test in `tests/security_invariants.rs` |
| ✅ Doc  | Documented in a referenced doc file (manual review required) |
| ✅ Both | Both automated test and documentation exist |
| ⚠️ Manual | Requires manual reviewer verification (no automated test) |
| 🔒 Accepted | Risk accepted by design (documented rationale) |

### 1. CEI Pattern (Checks-Effects-Interactions)

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 1.1 | Withdraw: state saved before token transfer | ✅ Test | `cei_withdraw_state_before_transfer` |
| 1.2 | Cancel: stream marked Cancelled before refund | ✅ Test | `cei_cancel_state_before_refund` |
| 1.3 | Top-up: deposit increased before token pull | ✅ Test | `cei_top_up_state_before_pull` |
| 1.4 | Shorten: deposit reduced before refund | ✅ Test | `cei_shorten_refund_before_transfer` |
| 1.5 | No `pull_token`/`push_token` before `save_stream` | ⚠️ Manual | Review each entrypoint in `src/lib.rs` |
| 1.6 | No state mutation after any external token call | ⚠️ Manual | Review each entrypoint in `src/lib.rs` |
| 1.7 | `cancel_stream` and `cancel_stream_as_admin` share code path | ✅ Doc | `docs/security.md` §cancel_stream |
| 1.8 | Batch withdraw saves each stream before its `push_token` | ⚠️ Manual | Review `batch_withdraw` in `src/lib.rs` |

### 2. Authorization Boundaries

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 2.1 | Sender auth required for `create_stream` | ✅ Both | `docs/security.md`, `tests/adversarial_auth.rs` |
| 2.2 | Recipient auth required for `withdraw` | ✅ Both | `docs/maintainer-security-checklist.md §2` |
| 2.3 | Admin auth required for `*_as_admin` operations | ✅ Both | `tests/adversarial_auth.rs` |
| 2.4 | Non-admin cannot pause/cancel as admin | ✅ Test | `test_admin_pause_rejects_non_admin` |
| 2.5 | Sender ≠ recipient enforced | ✅ Doc | `validate_stream_params` in `src/lib.rs` |
| 2.6 | Nonce-based replay protection for delegated withdraw | ✅ Both | `docs/security.md` §Delegated withdraw |
| 2.7 | `set_admin` requires current admin auth | ⚠️ Manual | Review `set_admin` in `src/lib.rs` |
| 2.8 | Permissionless entrypoints have no `require_auth()` | ⚠️ Manual | Review `close_completed_stream`, `trigger_auto_claim` |

### 3. Terminal State Gating

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 3.1 | Pause rejects Completed and Cancelled | ✅ Test | `terminal_pause_completed_fails` |
| 3.2 | Cancel rejects Completed | ✅ Test | `terminal_cancel_completed_fails` |
| 3.3 | Withdraw from Cancelled still works | ✅ Test | `terminal_withdraw_from_cancelled_succeeds` |
| 3.4 | Rate update rejects Cancelled | ✅ Test | `terminal_rate_update_cancelled_fails` |
| 3.5 | Top-up rejects Completed | ✅ Test | `terminal_top_up_completed_fails` |
| 3.6 | All terminal transitions follow state machine | ✅ Both | `docs/security.md`, `tests/security_invariants.rs` |
| 3.7 | Cancelled streams never transition to Completed | ✅ Doc | `docs/maintainer-security-checklist.md §3.2` |

### 4. Arithmetic Safety

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 4.1 | Batch deposit overflow caught | ✅ Test | `arithmetic_batch_deposit_overflow_caught` |
| 4.2 | Rate × duration overflow caught | ✅ Test | `arithmetic_rate_duration_overflow_caught` |
| 4.3 | Negative deposit rejected | ✅ Test | `arithmetic_negative_deposit_rejected` |
| 4.4 | All `checked_mul`/`checked_add` used | ⚠️ Manual | Search for unchecked `*`/`+` on user values |
| 4.5 | Accrual capped at `deposit_amount` | ✅ Both | `docs/security.md` overflow protection |
| 4.6 | Fuzz harness passes with `PROPTEST_CASES=10000` | ✅ Doc | `docs/security.md` §Accrual Fuzz Harness |

### 5. Init-Once Semantics

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 5.1 | Second init rejected with `AlreadyInitialised` | ✅ Test | `init_twice_rejected` |
| 5.2 | Failed re-init leaves config unchanged | ✅ Test | `init_config_unchanged_after_failed_reinit` |
| 5.3 | Init requires admin auth | ✅ Both | `test_init_rejects_wrong_signer_and_has_no_side_effects` |
| 5.4 | Token verification during init (SEP-41 smoke test) | ✅ Doc | `verify_token_behavior` in `token_check.rs` |

### 6. Duplicate ID Prevention

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 6.1 | `batch_withdraw` rejects duplicate stream IDs | ✅ Test | `duplicate_id_batch_withdraw_rejected` |
| 6.2 | `batch_withdraw_to` rejects duplicate stream IDs | ✅ Test | `duplicate_id_batch_withdraw_to_rejected` |
| 6.3 | `bulk_cancel_streams` rejects duplicates | ✅ Doc | `docs/gas.md` §O(n²) duplicate-ID scan |
| 6.4 | `bulk_resume_streams_as_admin` rejects duplicates | ✅ Doc | `docs/gas.md` §O(n²) duplicate-ID scan |

### 7. Pause State Enforcement

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 7.1 | Global emergency pause blocks withdrawals | ✅ Test | `pause_global_blocks_withdraw` |
| 7.2 | Global emergency pause blocks cancellations | ✅ Test | `pause_global_blocks_cancel` |
| 7.3 | Global emergency pause blocks top-ups | ✅ Test | `pause_global_blocks_top_up` |
| 7.4 | Creation pause blocks `create_stream` only | ✅ Test | `creation_pause_blocks_create_only` |
| 7.5 | Admin operations not blocked by pause | ⚠️ Manual | Review `require_not_globally_paused` call sites |
| 7.6 | `close_completed_stream` not blocked by pause | ✅ Doc | `docs/maintainer-security-checklist.md §8` |

### 8. Accrual Invariants

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 8.1 | Accrued never exceeds deposit (boundedness) | ✅ Test | `accrual_bounded_by_deposit` |
| 8.2 | Withdrawn never exceeds deposit | ✅ Test | `withdrawn_bounded_by_deposit` |
| 8.3 | Zero accrual before cliff time | ✅ Test | `accrual_zero_before_cliff` |
| 8.4 | Accrual monotonicity (non-decreasing) | ✅ Both | `tests/balance_conservation.rs`, `docs/security.md` |
| 8.5 | Cancelled stream accrual frozen at `cancelled_at` | ✅ Both | `docs/security.md` §Cancellation freeze |
| 8.6 | Saturation: `accrued == deposit` for `t ≥ end_time` when fully funded | ✅ Both | `tests/balance_conservation.rs` |

### 9. Event Compatibility

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 9.1 | `create_stream` emits `created` topic | ✅ Test | `event_create_stream_emits_created` |
| 9.2 | `withdraw` emits `withdrew` topic | ✅ Test | `event_withdraw_emits_withdrew` |
| 9.3 | `cancel_stream` emits `cancelled` topic | ✅ Test | `event_cancel_emits_cancelled` |
| 9.4 | No event on no-op paths | ⚠️ Manual | Review each entrypoint for early-return paths |
| 9.5 | `cancel_stream` and `cancel_stream_as_admin` emit identical events | ✅ Doc | `docs/maintainer-security-checklist.md §5.3` |
| 9.6 | `pause_stream` and `pause_stream_as_admin` emit identical events | ✅ Doc | `docs/maintainer-security-checklist.md §5.3` |

### 10. Reentrancy Guard (Invariant #13)

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 10.1 | Explicit lock acquired before state mutation for locked entrypoints | ✅ Doc | `docs/audit.md` §13.2 |
| 10.2 | Explicit lock released after token call for locked entrypoints | ✅ Doc | `docs/audit.md` §13.2 |
| 10.3 | Double-lock detection (revert on re-entry) | ✅ Doc | `acquire_reentrancy_lock` in `storage.rs` |
| 10.4 | CEI ordering for all other entrypoints | ✅ Test | §1 CEI Pattern tests |
| 10.5 | CEI-only is accepted posture (risk accepted) | ✅ Doc | `docs/audit.md` §13.4 |

### 11. Release Hardening

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 11.1 | WASM size within budget | ✅ Doc | `docs/gas.md` §WASM Size Budgets |
| 11.2 | Checksums match committed references | ✅ Doc | `docs/security.md` §Reproducible WASM builds |
| 11.3 | Full test suite passes (`cargo test --workspace`) | ✅ Doc | `docs/maintainer-security-checklist.md §10` |
| 11.4 | Fuzz harness run before release | ✅ Doc | `docs/security.md` §Accrual Fuzz Harness |
| 11.5 | Storage key discriminants unchanged | ✅ Test | `datakey_discriminant_count_stable` |
| 11.6 | Changelog updated with migration notes | ⚠️ Manual | `CHANGELOG.md` |
| 11.7 | Deployment checklist reviewed | ✅ Doc | `docs/mainnet-deployment-checklist-alignment.md` |

### 12. Irrevocable / Witness Mode

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 12.1 | Irrevocable stream rejects cancellation | ✅ Test | `irrevocable_cancel_rejected` |
| 12.2 | Irrevocable stream rejects shortening | ✅ Test | `irrevocable_shorten_rejected` |
| 12.3 | Witness-signed cancel requires valid ed25519 | ✅ Doc | `docs/security.md` §Witnessed cancel |

### 13. Liability Solvency

| # | Property | Coverage | Verification |
|---|----------|----------|-------------|
| 13.1 | Total liabilities increase by deposit on creation | ✅ Test | `liabilities_created_with_stream` |
| 13.2 | Sweep only transfers excess over liabilities | ✅ Both | `tests/adversarial_auth.rs`, `docs/security.md` |
| 13.3 | Post-sweep contract balance ≥ total liabilities | ✅ Both | `test_sweep_excess_preserves_solvency_invariant` |
