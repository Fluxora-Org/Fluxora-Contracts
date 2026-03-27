# Audit preparation

This document lists all public entrypoints and core invariants of the Fluxora stream contract to help external auditors scope the review. It is accurate as of the current codebase; no code changes are implied.

---

## Public entrypoints

| Entrypoint | Parameters | Return type | Authorization | Description |
|------------|------------|-------------|---------------|-------------|
| `init` | `env: Env`, `token: Address`, `admin: Address` | — | Bootstrap admin only (`admin.require_auth()`) | One-time setup: store token and admin. Panics if already initialised. |
| `create_stream` | `env: Env`, `sender: Address`, `recipient: Address`, `deposit_amount: i128`, `rate_per_second: i128`, `start_time: u64`, `cliff_time: u64`, `end_time: u64` | `u64` | Sender | Create stream, transfer deposit to contract, return new stream ID. |
| `pause_stream` | `env: Env`, `stream_id: u64` | — | Sender | Set stream status to Paused. Only Active streams. |
| `resume_stream` | `env: Env`, `stream_id: u64` | — | Sender | Set stream status to Active. Only Paused streams. |
| `cancel_stream` | `env: Env`, `stream_id: u64` | — | Sender | Refund unstreamed tokens to sender, set status to Cancelled. Active or Paused only. |
| `withdraw` | `env: Env`, `stream_id: u64` | `i128` | Recipient only | Transfer accrued-but-not-withdrawn tokens to recipient; update withdrawn_amount; set Completed if full. |
| `calculate_accrued` | `env: Env`, `stream_id: u64` | `i128` | None (view) | Total accrued so far (time-based). Withdrawable = accrued − withdrawn_amount. |
| `get_config` | `env: Env` | `Config` | None (view) | Return token and admin addresses. |
| `get_stream_state` | `env: Env`, `stream_id: u64` | `Stream` | None (view) | Return full stream state. |
| `cancel_stream_as_admin` | `env: Env`, `stream_id: u64` | — | Admin only | Same behaviour as cancel_stream; admin auth instead of sender. |
| `pause_stream_as_admin` | `env: Env`, `stream_id: u64` | — | Admin only | Same behaviour as pause_stream; admin auth. |
| `resume_stream_as_admin` | `env: Env`, `stream_id: u64` | — | Admin only | Same behaviour as resume_stream; admin auth. |
| `update_rate_per_second` | `env: Env`, `stream_id: u64`, `new_rate_per_second: i128` | — | Sender | Increase rate (forward-only). Deposit must still cover `new_rate × duration`. Active or Paused only. |
| `shorten_stream_end_time` | `env: Env`, `stream_id: u64`, `new_end_time: u64` | — | Sender | Reduce `end_time`; refund unstreamed tokens to sender. Active or Paused only. |
| `extend_stream_end_time` | `env: Env`, `stream_id: u64`, `new_end_time: u64` | — | Sender | Increase `end_time`. Existing `deposit_amount` must cover `rate × new_duration`. No token transfer. Active or Paused only. |
| `top_up_stream` | `env: Env`, `stream_id: u64`, `funder: Address`, `amount: i128` | — | Funder | Pull additional tokens into the stream deposit. Active or Paused only. |
| `close_completed_stream` | `env: Env`, `stream_id: u64` | — | Anyone | Remove storage for a Completed stream. Permissionless cleanup. |
| `set_admin` | `env: Env`, `new_admin: Address` | — | Admin | Rotate admin key. |
| `version` | `env: Env` | `u32` | None (view) | Return compile-time contract version. |

There is no `version` entrypoint in the contract.

---

## Types (reference)

- **Config**: `{ token: Address, admin: Address }`
- **Stream**: `stream_id: u64`, `sender: Address`, `recipient: Address`, `deposit_amount: i128`, `rate_per_second: i128`, `start_time: u64`, `cliff_time: u64`, `end_time: u64`, `withdrawn_amount: i128`, `status: StreamStatus`, `cancelled_at: Option<u64>`
- **StreamStatus**: `Active` \| `Paused` \| `Completed` \| `Cancelled`

---

## Invariants

Auditors can use these as a checklist; the implementation is intended to preserve them across all operations.

1. **Accrued never exceeds deposit**  
   `calculate_accrued` (and thus accrued amount used in withdraw/cancel) is clamped to `[0, deposit_amount]`. Overflow in rate × time is capped to `deposit_amount`.

2. **Withdrawn amount never exceeds deposit**  
   `withdrawn_amount` is only increased by `withdraw` by the withdrawable amount (accrued − withdrawn_amount), and stream becomes Completed when `withdrawn_amount == deposit_amount`; no further withdrawals allowed.

3. **Only the recipient can withdraw**  
   `withdraw` requires `stream.recipient.require_auth()`; sender and admin cannot withdraw on behalf of the recipient.

4. **Stream IDs are unique**  
   IDs are assigned from a monotonically increasing `NextStreamId` counter; no reuse or gap-fill.

5. **Sender ≠ recipient**  
   Enforced in `create_stream`; self-streaming is disallowed.

6. **Deposit covers total streamable amount**  
   `deposit_amount >= rate_per_second × (end_time − start_time)` is enforced in `create_stream`.

7. **Deposit sufficiency preserved on extension**  
   `extend_stream_end_time` re-validates `deposit_amount >= rate_per_second × (new_end_time − start_time)` before updating `end_time`. If the check fails, the call panics and no state changes occur. No token transfer happens on extension — the deposit already held in the contract must cover the longer duration. Use `top_up_stream` first if the current deposit is insufficient.

7. **Time bounds**  
   `start_time < end_time` and `cliff_time ∈ [start_time, end_time]` are enforced in `create_stream`.

8. **Init once (authenticated bootstrap)**  
   `init` requires admin authorization and panics if config already exists; token is immutable after init and admin changes only via `set_admin`.

9. **Pause / resume / cancel authorization**  
   `pause_stream`, `resume_stream`, and `cancel_stream` require sender auth. The `_as_admin` variants require admin auth and provide the same behaviour. Only the recipient can call `withdraw`.

10. **Status transitions**  
    - Pause: only Active → Paused.  
    - Resume: only Paused → Active.  
    - Cancel: only Active or Paused → Cancelled.  
    - Withdraw: when `withdrawn_amount` reaches `deposit_amount`, status becomes Completed.  
    Completed and Cancelled are terminal.

12. **Cancellation timestamp and refund semantics**
   - On successful cancel, `cancelled_at` is set to current ledger timestamp.
   - Accrual for cancelled streams is frozen at `cancelled_at`.
   - Refund paid to sender is exactly `deposit_amount - accrued_at(cancelled_at)`.
   - `cancel_stream` and `cancel_stream_as_admin` must produce identical state/event semantics except for the required authorizer.

11. **Contract balance consistency**  
    Deposit is pulled in `create_stream`; refunds and withdrawals only move amounts derived from that deposit (unstreamed to sender, accrued to recipient). No minting or arbitrary transfers.

---

For security patterns (e.g. CEI, reentrancy) see [docs/security.md](security.md).
