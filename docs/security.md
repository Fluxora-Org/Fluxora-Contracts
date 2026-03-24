# Security

Notes for auditors and maintainers on security-relevant patterns used in the Fluxora stream contract.

## Checks–Effects–Interactions (CEI)

The contract follows the **Checks-Effects-Interactions** pattern to reduce reentrancy risk.
State updates are performed **before** any external token transfers in all functions that move funds.

- **`create_streams`**  
  The contract requires sender auth once, validates every batch entry first, and computes the total deposit with checked arithmetic before any token transfer. It then performs one pull transfer for the total and persists streams. If any validation/overflow/transfer step fails, Soroban reverts the transaction: no streams are stored and no creation events remain on-chain.

- **`withdraw`**  
  After all checks (auth, status, withdrawable amount), the contract updates `withdrawn_amount` and, when applicable, sets status to `Completed`, then persists the stream with `save_stream`. Only after that does it call the token contract to transfer tokens to the recipient.

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

### `top_up_stream`

After authorization and amount validation, the contract:
1. Increases `stream.deposit_amount` with overflow protection.
2. Calls `save_stream` to persist the new deposit amount.
3. **Only then** calls the token contract to pull the top-up amount from the funder (`pull_token`).

> **Audit note (resolved):** Prior to the fix in this change, `top_up_stream` pulled
> tokens from the funder *before* persisting the updated `deposit_amount`. This violated
> CEI ordering: if the token contract had re-entered the stream contract between the
> external transfer and the `save_stream` call, it could have observed a stale
> `deposit_amount`. The call order has been corrected so state is always persisted first.

### `shorten_stream_end_time`

1. Updates `stream.end_time` and `stream.deposit_amount`.
2. Calls `save_stream`.
3. **Only then** transfers the refund to the sender.

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

---

## Authorization paths

| Operation              | Authorized callers                          |
|------------------------|---------------------------------------------|
| `create_stream`        | Sender (the address supplied as `sender`)   |
| `create_streams`       | Sender (once for the whole batch)           |
| `pause_stream`         | Stream's `sender`                           |
| `pause_stream_as_admin`| Contract admin                              |
| `resume_stream`        | Stream's `sender`                           |
| `resume_stream_as_admin`| Contract admin                             |
| `cancel_stream`        | Stream's `sender`                           |
| `cancel_stream_as_admin`| Contract admin                             |
| `withdraw`             | Stream's `recipient`                        |
| `withdraw_to`          | Stream's `recipient`                        |
| `batch_withdraw`       | Caller supplied as `recipient` (once for batch) |
| `update_rate_per_second`| Stream's `sender`                          |
| `shorten_stream_end_time`| Stream's `sender`                         |
| `extend_stream_end_time`| Stream's `sender`                          |
| `top_up_stream`        | `funder` (any address; no sender relationship required) |
| `close_completed_stream`| Permissionless (any caller)               |
| `set_admin`            | Current contract admin                      |
| `set_contract_paused`  | Contract admin                              |

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
instance storage under `DataKey::GlobalPaused`.

---

## Re-initialization prevention

`init` is bootstrap-authenticated and one-shot:

- It requires `admin.require_auth()` from the declared bootstrap admin.
- It checks `DataKey::Config` and panics with `"already initialised"` on any second call.

This prevents unauthorized bootstrap and prevents later repointing to a different token
address or replacing the admin through `init`.
