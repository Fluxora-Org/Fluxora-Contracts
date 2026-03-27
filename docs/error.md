# ContractError: User-Facing Mapping for Clients

Complete reference for all `ContractError` variants returned by the Fluxora stream contract. Integrators, wallets, indexers, and treasury tooling should use this table to map on-chain error codes to user-facing messages and recovery actions.

**Source of truth:** `contracts/stream/src/lib.rs` (`ContractError` enum)

---

## Discriminant Stability Policy

`ContractError` is a `#[soroban_sdk::contracterror]` enum with `#[repr(u32)]`. Each variant's numeric discriminant is **stable and immutable** for the lifetime of any deployed contract instance. Clients must use the numeric discriminant (not the variant name string) when decoding errors from RPC responses.

| Discriminant | Variant | Summary |
|---|---|---|
| 1 | `StreamNotFound` | Stream ID does not exist |
| 2 | `InvalidState` | Operation not valid for current stream status or uninitialised contract |
| 3 | `InvalidParams` | One or more input parameters are invalid |
| 4 | `ContractPaused` | Global pause active; stream creation blocked |
| 5 | `StartTimeInPast` | `start_time` is before current ledger timestamp |
| 6 | `Unauthorized` | Caller failed an explicit role check |
| 7 | `AlreadyInitialised` | Contract already initialised; `init` is one-shot |
| 8 | `InsufficientBalance` | Token balance or allowance insufficient |
| 9 | `InsufficientDeposit` | Deposit does not cover total streamable amount |

---

## Per-Variant Reference

### 1 — `StreamNotFound`

**When returned:** Any function that loads a stream by ID when the ID has never been created, or when the stream has been removed via `close_completed_stream`.

**User-facing message:** "Stream not found. The stream ID may be invalid or the stream may have been closed."

**Recovery:** Verify the stream ID. If the stream was recently closed, it is no longer queryable on-chain.

**Affected functions:**
`get_stream_state`, `calculate_accrued`, `get_withdrawable`, `get_claimable_at`,
`pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `withdraw_to`,
`batch_withdraw`, `top_up_stream`, `update_rate_per_second`,
`shorten_stream_end_time`, `extend_stream_end_time`, `close_completed_stream`,
`pause_stream_as_admin`, `resume_stream_as_admin`, `cancel_stream_as_admin`.

---

### 2 — `InvalidState`

**When returned:** Operation is not valid for the stream's current status, or the contract has not been initialised.

**Specific triggers:**

| Trigger | Function | Stream status required |
|---|---|---|
| Pause a non-Active stream | `pause_stream`, `pause_stream_as_admin` | Must be `Active` |
| Resume a non-Paused stream | `resume_stream`, `resume_stream_as_admin` | Must be `Paused` |
| Cancel a terminal stream | `cancel_stream`, `cancel_stream_as_admin` | Must be `Active` or `Paused` |
| Withdraw from `Completed` stream | `withdraw`, `withdraw_to`, `batch_withdraw` | Must be `Active` or `Cancelled` |
| Withdraw from `Paused` stream | `withdraw`, `withdraw_to` | Must be `Active` or `Cancelled` |
| Close a non-Completed stream | `close_completed_stream` | Must be `Completed` |
| Call config-dependent function before `init` | `get_config`, `get_token`, `get_admin` | Contract must be initialised |

**User-facing message:** Depends on context — see trigger table above. Examples:
- "This stream is already completed."
- "This stream is paused. Withdrawals are not available."
- "This stream cannot be cancelled in its current state."

**Recovery:** Check `get_stream_state` to confirm the current status before retrying.

**Affected functions:**
`pause_stream`, `pause_stream_as_admin`, `resume_stream`, `resume_stream_as_admin`,
`cancel_stream`, `cancel_stream_as_admin`, `withdraw`, `withdraw_to`, `batch_withdraw`,
`close_completed_stream`, `get_config` (uninitialised), `get_token` (uninitialised),
`get_admin` (uninitialised).

---

### 3 — `InvalidParams`

**When returned:** One or more input parameters fail validation.

**Specific triggers:**

| Trigger | Function |
|---|---|
| `deposit_amount <= 0` | `create_stream`, `create_streams` |
| `rate_per_second <= 0` | `create_stream`, `create_streams` |
| `sender == recipient` | `create_stream`, `create_streams` |
| `start_time >= end_time` | `create_stream`, `create_streams` |
| `cliff_time < start_time` or `cliff_time > end_time` | `create_stream`, `create_streams` |
| `rate * duration` overflows `i128` | `create_stream`, `create_streams` |
| Batch total deposit overflows `i128` | `create_streams` |
| `top_up_stream` amount <= 0 | `top_up_stream` |
| `top_up_stream` deposit overflow | `top_up_stream` |
| `update_rate_per_second` new rate <= 0 | `update_rate_per_second` |
| `update_rate_per_second` new rate * duration overflows | `update_rate_per_second` |
| `shorten_stream_end_time` new end >= current end | `shorten_stream_end_time` |
| `shorten_stream_end_time` new end <= start | `shorten_stream_end_time` |
| `extend_stream_end_time` new end <= current end | `extend_stream_end_time` |
| `extend_stream_end_time` new rate * duration overflows | `extend_stream_end_time` |

**User-facing message:** Depends on trigger. Examples:
- "Deposit amount must be greater than zero."
- "Sender and recipient must be different addresses."
- "Stream end time must be after start time."
- "Cliff time must be within the stream duration."

**Recovery:** Correct the invalid parameter and retry.

**Affected functions:**
`create_stream`, `create_streams`, `top_up_stream`, `update_rate_per_second`,
`shorten_stream_end_time`, `extend_stream_end_time`.

---

### 4 — `ContractPaused`

**When returned:** The global emergency pause is active. Only `create_stream` and `create_streams` are blocked; all other operations continue normally.

**User-facing message:** "Stream creation is temporarily paused. Please try again later."

**Recovery:** Wait for the admin to call `set_contract_paused(false)`. Existing streams are unaffected — withdrawals, cancellations, and other operations continue normally.

**Affected functions:** `create_stream`, `create_streams`.

---

### 5 — `StartTimeInPast`

**When returned:** `start_time` is strictly before the current ledger timestamp at the time of the `create_stream` call. `start_time == ledger.timestamp()` ("start now") is valid.

**User-facing message:** "Stream start time cannot be in the past. Please use the current time or a future time."

**Recovery:** Set `start_time` to the current ledger timestamp or later and retry. Failure is atomic: no stream is persisted, no tokens move, and no `created` event is emitted.

**Affected functions:** `create_stream`, `create_streams`.

---

### 6 — `Unauthorized`

**When returned:** An explicit role check fails after `require_auth`. Currently used in `withdraw_to` when the caller is not the stream's recipient.

**Note:** Most authorization failures in Soroban surface as host traps (panics), not as this error code. This variant is reserved for cases where the contract performs an explicit role check after the auth host call.

**User-facing message:** "You are not authorized to perform this operation."

**Recovery:** Ensure the transaction is signed by the correct authorized address for the operation.

**Affected functions:** `withdraw_to` (non-recipient destination check).

---

### 7 — `AlreadyInitialised`

**When returned:** `init` has already been called successfully. The contract is one-shot initialised; any second call returns this error regardless of the supplied parameters. The existing `Config` and `NextStreamId` are unchanged.

**User-facing message:** "Contract is already initialised."

**Recovery:** No action needed — the contract is already ready to use. Call `get_config` to verify the current token and admin addresses.

**Affected functions:** `init`.

---

### 8 — `InsufficientBalance`

**When returned:** Reserved for cases where the contract can detect an insufficient token balance before calling the token contract. In practice, most token transfer failures surface as host traps from the token client rather than this error code.

**User-facing message:** "Insufficient token balance or allowance."

**Recovery:** Ensure the sender has sufficient token balance and has approved the contract for the required amount.

**Affected functions:** `create_stream`, `create_streams`, `cancel_stream`, `withdraw` (token client failures).

---

### 9 — `InsufficientDeposit`

**When returned:** `deposit_amount < rate_per_second * (end_time - start_time)`. The deposit must cover the full streamable amount for the entire duration.

Also returned by `extend_stream_end_time` if the existing deposit does not cover the extended duration.

**User-facing message:** "Deposit amount is insufficient to cover the full stream duration. Required: rate × duration."

**Recovery:**
- For `create_stream`: increase `deposit_amount` to at least `rate_per_second * (end_time - start_time)`.
- For `extend_stream_end_time`: call `top_up_stream` first to increase the deposit, then retry the extension.

**Affected functions:** `create_stream`, `create_streams`, `extend_stream_end_time`.

---

## Scope Boundary and Audit Notes

### In scope
- All `ContractError` variants and their discriminants.
- The specific conditions that trigger each variant.
- Recovery guidance for integrators.
- Discriminant stability guarantee.

### Out of scope
- Host trap panics (e.g., `require_auth` failures, arithmetic panics). These are not `ContractError` variants and cannot be caught by `try_*` client methods. They surface as transaction failures with no structured error code.
- Token contract errors. The stream contract does not wrap or re-classify errors from the token client.
- Off-chain indexer behavior when errors occur.

### Residual risks
1. **`InvalidState` is overloaded.** It covers both "wrong stream status" and "contract not initialised." Clients that need to distinguish these cases must check `get_config` first to determine if the contract is initialised.
2. **`InsufficientBalance` (discriminant 8) is rarely returned.** Most token transfer failures surface as host traps. Clients should not rely on catching this error code for balance checks — use the token contract's `balance` view instead.
3. **Discriminant stability.** The discriminant table above is immutable for any deployed instance. A new contract version may add variants (appended at the end) but must never change existing discriminants. See `docs/upgrade.md` for the versioning policy.
