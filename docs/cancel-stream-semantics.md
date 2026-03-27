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

## Residual assumptions and risks

1. Token trust model: cancellation depends on configured token contract transfer behavior.
2. CEI ordering reduces reentrancy risk by persisting cancel state before transfer, but cannot fully mitigate a malicious token that violates assumptions.
3. Event payload does not include refund amount or timestamp; indexers must read stream state to reconstruct these values.
