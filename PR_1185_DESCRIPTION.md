# Pull Request: #1185 - cancel_stream: refund and cancelled_at semantics

## Description

This PR resolves issue #1185 by establishing, validating, and documenting the on-chain protocol assurances for `cancel_stream` refund mechanics and `cancelled_at` timestamp semantics across all stream cancellation entrypoints (`cancel_stream`, `cancel_stream_as_admin`, `witnessed_cancel_stream`, `delegated_cancel`, `bulk_cancel_streams`, `keeper_cancel`).

It verifies that all cancellation paths strictly enforce:
1. **State Machine Transition**: Status updates to terminal `StreamStatus::Cancelled` and `cancelled_at` stores the exact ledger timestamp `Some(now)` at execution time.
2. **Refund Conservation Invariant**: Unstreamed deposit (`deposit_amount - accrued_at(cancelled_at)`) is refunded to the sender, while accrued value remains frozen for recipient claim (`sender_refund + frozen_recipient_accrued == deposit_amount`).
3. **Authorization Matrix**: `sender.require_auth()` for `cancel_stream`, `admin.require_auth()` for `cancel_stream_as_admin`, and cryptographic ed25519 signature verification for delegated/witnessed variants.
4. **Irrevocable Protection**: Streams created with `irrevocable = true` strictly reject all cancellation calls with `ContractError::Unauthorized`.
5. **Checks-Effects-Interactions (CEI)**: State updates and liability counters are persisted before external token transfer.
6. **Documentation Synchronization**: Backfilled `delegated_cancel` and `get_delegated_cancel_nonce` into `docs/streaming.md` validator indices and added root `issue1185.md` specification.

## Type of Change

- [x] Bug fix / Invariant verification (non-breaking change ensuring protocol assurances)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [x] Documentation update
- [x] Test coverage & validator index alignment
- [ ] Refactoring (no functional changes)

## Related Issues

Closes #1185

## Changes Made

- **`docs/streaming.md`**:
  - Added missing `delegated_cancel` and `get_delegated_cancel_nonce` entrypoints to all validator index lists, bringing missing entrypoint count to 0.
- **`issue1185.md`**:
  - Created specification and verification summary for issue 1185 following repo issue conventions (`issue896.md`, `issue913.md`, `issue914.md`, `issue925.md`).
- **`docs/cancel-stream-semantics.md`**:
  - Audited and verified full protocol specification for refund mathematics, fee deductions, keeper cancellations, witnessed cancellations, and cliff-only variants.

## Testing

### Test Coverage

- [x] Entrypoint documentation validator passed (`0` missing entrypoints).
- [x] Error discriminant collision check passed (`0` duplicate discriminants).
- [x] Cancellation test suite verified across `contracts/stream/src/test.rs` and `contracts/stream/tests/` (`balance_conservation.rs`, `bulk_cancel.rs`, `delegated_cancel.rs`, `witnessed_cancel.rs`, `adversarial_auth.rs`).

### Security Considerations

- **CEI Ordering**: `status` and `cancelled_at` are persisted before `push_token` refund transfer to eliminate re-entrancy risks.
- **Accrual Freeze**: Post-cancellation entitlement checks freeze recipient accrual at `cancelled_at`, ensuring no double-counting or unearned claims.
- **Irrevocable Invariant**: `irrevocable` flag is unalterable post-creation and blocks all cancellation entrypoints.

## PR Summary Table

| Category | Details |
| --- | --- |
| **Issue Resolved** | #1185 (`cancel_stream: refund and cancelled_at semantics`) |
| **Documentation** | [docs/cancel-stream-semantics.md](file:///c:/Users/HP/Desktop/Prosper's workspace/Fluxora-Contracts/docs/cancel-stream-semantics.md), [docs/streaming.md](file:///c:/Users/HP/Desktop/Prosper's workspace/Fluxora-Contracts/docs/streaming.md), [issue1185.md](file:///c:/Users/HP/Desktop/Prosper's workspace/Fluxora-Contracts/issue1185.md) |
| **Core Contract** | [contracts/stream/src/lib.rs](file:///c:/Users/HP/Desktop/Prosper's workspace/Fluxora-Contracts/contracts/stream/src/lib.rs#L6305-L6351) |
