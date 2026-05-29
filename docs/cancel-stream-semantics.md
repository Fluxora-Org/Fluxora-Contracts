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
4. Failures are atomic: no transfer, no state update, no cancel event.

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

## Residual assumptions and risks

1. Token trust model: cancellation depends on configured token contract transfer behavior.
2. CEI ordering reduces reentrancy risk by persisting cancel state before transfer, but cannot fully mitigate a malicious token that violates assumptions.
3. Event payload does not include refund amount, fee, or timestamp; indexers must read stream state to reconstruct these values.
4. Cancellation fee is optional (defaults to 0); protocol behavior is identical to pre-fee version when `cancellation_fee_bps = 0`.
