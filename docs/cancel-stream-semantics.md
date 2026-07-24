# cancel_stream: refund and cancelled_at semantics

This note scopes and verifies one protocol slice: cancellation refund behavior and `cancelled_at` semantics.

## Scope

In scope:

1. `cancel_stream` and `cancel_stream_as_admin` success/failure behavior.
2. Authorization boundaries for sender/admin/unauthorized actors.
3. On-chain observables: stream storage fields, token balances, errors, events.
4. Time and status edge cases that affect refund and accrued freeze logic.

Out of scope:

1. Token contract implementation safety beyond SEP-41 assumptions.
2. Off-chain indexer uptime and ingestion correctness.
3. Broader stream lifecycle behavior unrelated to cancellation.

## Protocol semantics

On success:

1. Cancellation is allowed only for `Active` or `Paused` streams.
2. `cancelled_at` is set to current ledger timestamp.
3. Stream status becomes terminal `Cancelled`.
4. Refund transferred to sender is:

   `deposit_amount - accrued_at(cancelled_at)`

5. Accrued value is frozen at `cancelled_at` for all future `calculate_accrued` calls.
6. Event emitted: topic `("cancelled", stream_id)` with payload `StreamEvent::StreamCancelled(stream_id)`.

On failure:

1. Missing stream: `StreamNotFound`.
2. Invalid status (`Completed` or already `Cancelled`): `InvalidState`.
3. Sender path requires sender auth; admin path requires admin auth.
4. Failures are atomic: no transfer, no state update, no cancel event. This includes bulk cancellations (`bulk_cancel_streams`), which strictly follow an atomic-reject model: if any stream in a batch fails validation (e.g., unauthorized access or invalid state), the entire batch reverts and no streams are mutated.

## Authorization matrix

1. Sender may call `cancel_stream` for their stream.
2. Admin may call `cancel_stream_as_admin` for any stream.
3. Recipient and third parties cannot cancel without the required auth proof.

## Evidence in tests

Unit tests (`contracts/stream/src/test.rs`):

1. `test_cancel_stream_full_refund`
2. `test_cancel_stream_partial_refund`
3. `test_cancel_stream_as_admin`
4. `test_cancel_refund_plus_frozen_accrued_equals_deposit`
5. `test_cancel_event`
6. Strict auth tests for unauthorized recipient/third-party cancel attempts.

Integration tests (`contracts/stream/tests/integration_suite.rs`):

1. `cancel_stream_updates_state_before_transfer`
2. `cancel_stream_as_admin_updates_state_before_transfer`
3. `integration_cancel_partial_accrual_partial_refund`
4. `integration_cancel_refund_plus_frozen_accrued_equals_deposit`

## Optional Cancellation Fee

All streams may specify an optional cancellation fee (in basis points, where 1 bps = 0.01% and 10000 bps = 100%).

### Fee Semantics

The cancellation fee is applied **only** to the unstreamed refund portion:

1. When a stream is cancelled, the protocol calculates:

   ```
   accrued_at_cancel = calculate_accrued_at(cancelled_at)
   refund_gross = deposit_amount - accrued_at_cancel
   ```

2. If `cancellation_fee_bps > 0`, the fee is calculated as:

   ```
   fee = (refund_gross × cancellation_fee_bps) / 10000  (rounded down)
   refund_net = refund_gross - fee
   ```

3. The sender receives `refund_net` tokens.

4. **CRITICAL INVARIANT**: The recipient's frozen accrued amount is **never** reduced by the fee.
   - Recipient can always withdraw the full `accrued_at_cancel` via `withdraw()` or `withdraw_to()`
   - The fee is taken **only** from the sender's refund

### Edge Cases & Rounding

1. **Zero fee**: If `cancellation_fee_bps = 0`, no fee is applied; sender receives full refund.

2. **100% fee**: If `cancellation_fee_bps = 10000` (100%), the entire refund is deducted as fee; sender receives 0 tokens.

3. **No refund**: If stream is fully accrued (`accrued_at_cancel == deposit_amount`), then `refund_gross = 0`, so fee = 0, and sender gets nothing (as expected).

4. **Rounding**: Fee is calculated as integer division `(refund_gross × fee_bps) / 10000`, which truncates down. This ensures the sender never receives more tokens than the protocol allows and prevents dust accumulation.

5. **Zero refund**, any fee: If `refund_gross = 0`, then fee = 0 (regardless of `fee_bps`).

### Recipient Safety

The recipient's ability to withdraw accrued funds is **completely independent** of the cancellation fee:

- `calculate_accrued()` returns the full accrued amount, unaffected by the fee.
- The fee is deducted from the sender's refund, **not** from the recipient's accrued balance.
- After cancellation, the recipient calls `withdraw()` to claim the full accrued amount.

### Examples

**Example 1: 50% cancellation fee, cancel at 30% accrual**

- Deposit: 1000 tokens, Rate: 1 token/sec, End: 1000 sec
- Cancel at: 300 sec
- Accrued: 300 tokens
- Refund gross: 700 tokens
- Fee (50%): (700 × 5000) / 10000 = 350 tokens
- Refund net: 350 tokens
- Sender receives: 350 tokens
- Recipient can withdraw: 300 tokens (full accrued)
- Unaccounted (fee): 350 tokens (remains in contract)

**Example 2: 10% cancellation fee, fully accrued stream**

- Deposit: 1000, Rate: 1/sec, End: 1000 sec, Cancel at: 1000 sec
- Accrued: 1000 tokens
- Refund gross: 0 tokens
- Fee: 0 tokens
- Refund net: 0 tokens
- Sender receives: 0 tokens
- Recipient can withdraw: 1000 tokens

## Keeper-initiated cancellation (`keeper_cancel`)

### Purpose

Streams that have passed their `end_time` but whose sender never calls `cancel_stream` leave
unclaimed deposits locked in contract storage indefinitely, contributing to state bloat.
`keeper_cancel` allows any caller (a permissionless keeper) to cancel such a stream once a
configurable grace period has elapsed, returning funds to their rightful owners and paying a
small incentive to the keeper.

### Eligibility

A stream is eligible for keeper cancellation when:

1. Its status is `Active` or `Paused` (not already `Completed` or `Cancelled`).
2. `current_timestamp >= end_time + KEEPER_GRACE_PERIOD_SECONDS` (default: 7 days = 604 800 s).

### Token distribution

```
accrued         = calculate_accrued_at(end_time)          -- capped at deposit_amount
recipient_amount = accrued - withdrawn_amount              -- outstanding claimable balance
sender_refund_gross = deposit_amount - accrued             -- unstreamed portion
keeper_fee       = sender_refund_gross × KEEPER_FEE_BPS / 10 000   -- default: 0.5 %
sender_refund    = sender_refund_gross - keeper_fee
```

All three parties receive their tokens in a single transaction.

### Security invariants

1. **Recipient is never penalised**: `keeper_fee` is taken from `sender_refund_gross`, never from
   `recipient_amount`. The recipient always receives the full outstanding accrued balance.
2. **CEI ordering**: the stream is marked `Cancelled` in persistent storage before any token
   transfer. A re-entrant token cannot observe an inconsistent state.
3. **Keeper must sign (`keeper.require_auth()`)**: prevents a third party from redirecting the fee
   to an arbitrary address by supplying a different keeper address in the call.
4. **Terminal streams are rejected early**: if the stream is already `Completed` or `Cancelled`,
   the call fails with `ContractError::InvalidState` before any state change.

### Event

Topic: `("kp_cncl", stream_id)`

Payload: `KeeperCancelled { stream_id, keeper, keeper_fee, recipient_amount, sender_refund }`

### Constants

| Constant                      | Value            | Meaning                                               |
| ----------------------------- | ---------------- | ----------------------------------------------------- |
| `KEEPER_GRACE_PERIOD_SECONDS` | 604 800 (7 days) | Minimum seconds past `end_time` for eligibility       |
| `KEEPER_FEE_BPS`              | 50 (0.5 %)       | Keeper fee as basis points of the sender gross refund |

## Residual assumptions and risks

1. Token trust model: cancellation depends on configured token contract transfer behavior.
2. CEI ordering reduces reentrancy risk by persisting cancel state before transfer, but cannot fully mitigate a malicious token that violates assumptions.
3. Event payload does not include refund amount, fee, or timestamp; indexers must read stream state to reconstruct these values.
4. Cancellation fee is optional (defaults to 0); protocol behavior is identical to pre-fee version when `cancellation_fee_bps = 0`.

## Permissionless cleanup: `close_cancelled_stream`

When a stream has been `Cancelled` and the recipient has withdrawn the frozen accrued
amount, the contract exposes a permissionless cleanup entrypoint `close_cancelled_stream`.

- Purpose: reclaim persistent storage and remove the stream ID from the recipient index
  after the recipient is fully settled.
- Preconditions: stream must be `Cancelled` and the recipient must have no remaining
  claimable balance at `cancelled_at` (the call rejects with `InvalidState` otherwise).
- Event: emits `("closed", stream_id)` with `StreamEvent::StreamClosed(stream_id)`
  before deleting storage.

Keepers and off-chain indexers may call this entrypoint to free storage and reduce
recipient-index bloat once the recipient's claims are fully settled.

## CliffOnly Cancellation Semantics

When `StreamKind::CliffOnly` streams are cancelled (via `cancel_stream`, `cancel_stream_as_admin`, `bulk_cancel_streams`, or `keeper_cancel`), they exhibit distinct mathematical edge cases compared to `Linear` streams because partial accrual is impossible:

1. **Before the cliff time:** The recipient's accrued amount is strictly 0. 
   - A cancellation refunds the **full** `deposit_amount` back to the sender (`sender_refund_gross == deposit_amount`).
   - For a `bulk_cancel_streams` (admin only) or sender cancellation, the sender receives the entire deposit minus any configured cancellation fee. The recipient receives nothing.

2. **At or after the cliff time:** The recipient's accrued amount is the **full** `deposit_amount`.
   - A cancellation (e.g. via `keeper_cancel` after `end_time + GRACE`) calculates `sender_refund_gross = 0`.
   - Because `sender_refund_gross` is 0, the `keeper_fee` evaluates to 0, and the sender receives a net refund of 0.
   - The recipient is fully entitled to the `deposit_amount` and can withdraw it at their leisure.

In all paths, `TotalLiabilities`, `KeeperCancelled`, and `StreamCancelled` events operate correctly, preserving the structural invariants proven in adversarial test suites (`contracts/stream/tests/cliff_only_variant.rs`).
