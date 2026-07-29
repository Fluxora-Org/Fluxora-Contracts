Description
Within the Fluxora streaming contract, this work tightens externally visible assurances for `cancel_stream` refund calculations and `cancelled_at` timestamp semantics across all cancellation entrypoints (`cancel_stream`, `cancel_stream_as_admin`, `witnessed_cancel_stream`, `delegated_cancel`, `bulk_cancel_streams`, `keeper_cancel`). Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured.

Requirements
1. Characterize the intended protocol semantics for `cancel_stream: refund and cancelled_at semantics` in both success and failure cases.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

Suggested execution
1. Verify `contracts/stream/src/lib.rs` shared `cancel_stream_internal` implementation:
   - Status transition to `StreamStatus::Cancelled`
   - `cancelled_at` set to current ledger timestamp
   - Refund calculation: `deposit_amount - accrued_at(cancelled_at)`
   - Accrual freeze for subsequent entitlement checks
   - CEI state persistence prior to external token transfers
   - Liability reduction by refunded amount
   - Emission of `StreamCancelled(stream_id)` event
2. Verify test coverage in `contracts/stream/src/test.rs` and `contracts/stream/tests/` (including `delegated_cancel.rs`, `witnessed_cancel.rs`, `bulk_cancel.rs`, `balance_conservation.rs`, `adversarial_auth.rs`).
3. Maintain documentation in `docs/cancel-stream-semantics.md` and `docs/streaming.md`.

Acceptance criteria
1. Full authorization matrix enforced (Sender for `cancel_stream`, Admin for `cancel_stream_as_admin`, Witness/Relayer for signed variants; unauthorized callers strictly rejected).
2. Balance conservation invariant holds: `sender_refund + frozen_recipient_accrued == deposit_amount`.
3. Irrevocable streams (`irrevocable = true`) reject cancellation with `ContractError::Unauthorized`.
4. Documentation in `docs/cancel-stream-semantics.md` and `docs/streaming.md` accurately describes on-chain behavior.

Security notes
1. Checks-Effects-Interactions (CEI): `status` and `cancelled_at` are persisted before `push_token` refund transfer to prevent re-entrancy anomalies.
2. Accrual entitlement freeze guarantees recipient cannot be deprived of accrued value while ensuring sender receives unstreamed excess.

Guidelines
Minimum 95% test coverage
Timeframe: 96 hours
