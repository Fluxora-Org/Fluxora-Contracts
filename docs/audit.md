# Audit preparation

This document lists all public entrypoints and core invariants of the Fluxora stream contract to help external auditors scope the review. It is accurate as of the current codebase; no code changes are implied.

---

## Public entrypoints

The table below covers every `pub fn` in the `#[contractimpl]` block of
`contracts/stream/src/lib.rs`.  It is kept in sync by the CI check
`script/validate-doc-alignment.py` (audit-entrypoint drift check), which fails
with exit code 1 if any entrypoint is absent.  Two names are intentionally
excluded from this table because they are not user-facing ABI entrypoints:
`upgrade` (Soroban host upgrade helper, see `docs/upgrade.md`) and
`compute_keeper_fee_split` (pure arithmetic utility exposed for test crates).

<!-- ci:audit-entrypoint-table-begin -->
| Entrypoint | Parameters | Return type | Authorization | Description |
| --- | --- | --- | --- | --- |
| `init` | `env: Env`, `token: Address`, `admin: Address` | — | `admin.require_auth()` | One-time setup: store token and admin. Panics if already initialised. |
| `create_stream` | `env: Env`, `sender: Address`, `recipient: Address`, `deposit_amount: i128`, `rate_per_second: i128`, `start_time: u64`, `cliff_time: u64`, `end_time: u64` | `u64` | `sender.require_auth()` | Create stream, transfer deposit to contract, return new stream ID. |
| `create_stream_relative` | `env: Env`, `sender: Address`, `recipient: Address`, `deposit_amount: i128`, `rate_per_second: i128`, `start_delay: u64`, `cliff_delay: u64`, `duration: u64` | `u64` | `sender.require_auth()` (via `create_stream`) | Create a stream with relative delays instead of absolute timestamps. |
| `create_streams` | `env: Env`, `sender: Address`, `params: Vec<CreateStreamParams>` | `Vec<u64>` | `sender.require_auth()` | Atomic batch: create multiple streams, all-or-nothing. |
| `create_streams_relative` | `env: Env`, `sender: Address`, `params: Vec<CreateStreamRelativeParams>` | `Vec<u64>` | `sender.require_auth()` | Atomic batch using relative timing. |
| `create_streams_partial` | `env: Env`, `sender: Address`, `params: Vec<CreateStreamParams>` | `Vec<Result<u64, ContractError>>` | `sender.require_auth()` | Batch with per-entry failure isolation; successful entries are not rolled back on others' errors. |
| `pause_stream` | `env: Env`, `stream_id: u64` | — | `stream.sender.require_auth()` | Set stream status to Paused. Only Active streams. |
| `resume_stream` | `env: Env`, `stream_id: u64` | — | `stream.sender.require_auth()` | Set stream status to Active. Only Paused streams. |
| `cancel_stream` | `env: Env`, `stream_id: u64` | — | `stream.sender.require_auth()` | Refund unstreamed tokens to sender, set status to Cancelled. Active or Paused only. |
| `withdraw` | `env: Env`, `stream_id: u64` | `i128` | `stream.recipient.require_auth()` | Transfer accrued-but-not-withdrawn tokens to recipient; update `withdrawn_amount`; set Completed if full. |
| `withdraw_to` | `env: Env`, `stream_id: u64`, `destination: Address` | `i128` | `stream.recipient.require_auth()` | Withdraw accrued tokens to a specified destination address. |
| `update_recipient` | `env: Env`, `stream_id: u64`, `new_recipient: Address` | — | `stream.recipient.require_auth()` | Initiate recipient rotation; stores a pending update request. |
| `get_pending_recipient_update` | `env: Env`, `stream_id: u64` | `Option<Address>` | None (view) | Read the pending recipient update request for a stream. |
| `accept_recipient_update` | `env: Env`, `stream_id: u64` | — | new recipient's `require_auth()` | Finalise a pending recipient rotation; caller becomes the new recipient. |
| `cancel_recipient_update` | `env: Env`, `stream_id: u64` | — | `stream.sender.require_auth()` | Cancel a pending recipient update; sender can veto mid-rotation. |
| `batch_withdraw` | `env: Env`, `recipient: Address`, `stream_ids: Vec<u64>` | `Vec<i128>` | `recipient.require_auth()` | Withdraw accrued tokens from many streams in a single call. |
| `batch_withdraw_to` | `env: Env`, `recipient: Address`, `requests: Vec<WithdrawToRequest>` | `Vec<i128>` | `recipient.require_auth()` | Withdraw accrued tokens from many streams to per-stream destinations. |
| `delegated_withdraw` | `env: Env`, `stream_id: u64`, `relayer: Address`, `recipient_public_key: Bytes`, `nonce: u64`, `deadline: u64`, `expected_minimum_amount: i128`, `signature: Bytes` | `i128` | `relayer.require_auth()` + ed25519 sig from recipient | Relayer-executed withdrawal using recipient off-chain signature. Signature commits to `(stream_id, nonce, deadline, expected_minimum_amount)`. |
| `get_delegated_nonce` | `env: Env`, `recipient: Address` | `u64` | None (view) | Return current replay-protection nonce for a recipient. |
| `calculate_accrued` | `env: Env`, `stream_id: u64` | `i128` | None (view) | Total accrued so far (time-based). Withdrawable = accrued − `withdrawn_amount`. |
| `get_withdrawable` | `env: Env`, `stream_id: u64` | `i128` | None (view) | Compute current withdrawable balance (`accrued − withdrawn_amount`). |
| `get_claimable_at` | `env: Env`, `stream_id: u64`, `timestamp: u64` | `i128` | None (view) | Query the claimable amount at an arbitrary future or past timestamp. |
| `get_config` | `env: Env` | `Config` | None (view) | Return token and admin addresses. |
| `get_global_emergency_paused` | `env: Env` | `bool` | None (view) | Read the global emergency pause state. |
| `set_admin` | `env: Env`, `new_admin: Address` | — | `old_admin.require_auth()` | Rotate admin key; old admin must authorise. |
| `set_max_rate_per_second` | `env: Env`, `max_rate: i128` | — | `admin.require_auth()` | Set the global cap on `rate_per_second` for new streams. |
| `get_stream_state` | `env: Env`, `stream_id: u64` | `Stream` | None (view) | Return full stream struct. |
| `get_stream_health` | `env: Env`, `stream_id: u64` | `StreamHealth` | None (view) | Return health metrics: solvency, completion percentage, time remaining. |
| `get_stream_memo` | `env: Env`, `stream_id: u64` | `Option<Bytes>` | None (view) | Read the memo attached to a stream (up to `MAX_MEMO_BYTES`). |
| `get_stream_metadata` | `env: Env`, `stream_id: u64` | `Map<String, String>` | None (view) | Read the metadata map attached to a stream. |
| `get_stream_count` | `env: Env` | `u64` | None (view) | Read the global stream ID counter (total streams ever created). |
| `get_protocol_fees_accrued` | `env: Env` | `i128` | None (view) | Read total protocol fees accrued in instance storage. |
| `get_total_liabilities` | `env: Env` | `i128` | None (view) | Read total outstanding deposit liabilities tracked in instance storage. |
| `get_paused_stream_count` | `env: Env` | `u64` | None (view) | Read current count of paused streams (O(1), maintained in storage). |
| `update_rate_per_second` | `env: Env`, `stream_id: u64`, `new_rate_per_second: i128` | — | `stream.sender.require_auth()` | Increase rate (forward-only). Deposit must still cover `new_rate × duration`. Active or Paused only. |
| `decrease_rate_per_second` | `env: Env`, `stream_id: u64`, `new_rate_per_second: i128` | — | `stream.sender.require_auth()` | Decrease rate safely: checkpoints accrued-to-now, then updates rate. Active or Paused only. |
| `shorten_stream_end_time` | `env: Env`, `stream_id: u64`, `new_end_time: u64` | — | `stream.sender.require_auth()` | Reduce `end_time`; refund unstreamed tokens to sender. Active or Paused only. |
| `extend_stream_end_time` | `env: Env`, `stream_id: u64`, `new_end_time: u64` | — | `stream.sender.require_auth()` | Increase `end_time`. Existing deposit must cover `rate × new_duration`. No token transfer. |
| `top_up_stream` | `env: Env`, `stream_id: u64`, `funder: Address`, `amount: i128` | — | `funder.require_auth()` | Pull additional tokens into the stream deposit from an authorised funder. Active or Paused only. |
| `close_completed_stream` | `env: Env`, `stream_id: u64` | — | None (permissionless) | Remove storage for a Completed stream. |
| `close_cancelled_stream` | `env: Env`, `stream_id: u64` | — | None (permissionless) | Remove storage for a Cancelled stream after all accrued funds are withdrawn. |
| `register_stream_template` | `env: Env`, `owner: Address`, `template: StreamTemplate` | `u64` | `owner.require_auth()` | Register a reusable schedule template; return its template ID. |
| `delete_stream_template` | `env: Env`, `owner: Address`, `template_id: u64` | — | `owner.require_auth()` | Delete a schedule template owned by the caller. |
| `create_stream_from_template` | `env: Env`, `sender: Address`, `template_id: u64`, `recipient: Address`, `deposit_amount: i128` | `u64` | `sender.require_auth()` | Instantiate a stream using a registered template's timing parameters. |
| `get_stream_template` | `env: Env`, `template_id: u64` | `StreamTemplate` | None (view) | Read a saved schedule template by ID. |
| `version` | `env: Env` | `u32` | None (view) | Return compile-time `CONTRACT_VERSION` constant. |
| `get_recipient_streams` | `env: Env`, `recipient: Address` | `Vec<u64>` | None (view) | List stream IDs for a recipient, hard-capped at `RECIPIENT_STREAMS_PAGE_LIMIT`. |
| `get_recipient_streams_paginated` | `env: Env`, `recipient: Address`, `page: u32` | `Vec<u64>` | None (view) | Paginate recipient stream IDs by page number. |
| `get_recipient_stream_count` | `env: Env`, `recipient: Address` | `u64` | None (view) | Count streams for a recipient. |
| `get_streams_by_id_range` | `env: Env`, `start_id: u64`, `end_id: u64` | `Vec<Stream>` | None (view) | Read streams in an ID range for export; bounded by `MAX_PAGE_SIZE`. |
| `update_rate` | `env: Env`, `stream_id: u64`, `caller: Address`, `new_rate: i128` | — | sender or `admin.require_auth()` | Unified rate-update entry-point callable by either sender or admin. |
| `cancel_stream_as_admin` | `env: Env`, `stream_id: u64` | — | `admin.require_auth()` | Cancel any stream as contract admin; identical state/event semantics to `cancel_stream`. |
| `keeper_cancel` | `env: Env`, `stream_id: u64`, `keeper: Address` | — | `keeper.require_auth()` | Keeper-cancel an eligible stream after grace period; pays keeper fee from refund. |
| `get_keeper_fee_split` | `env: Env`, `stream_id: u64` | `(i128, i128)` | None (view) | Preview the `(keeper_fee, sender_refund)` split that `keeper_cancel` would pay. |
| `pause_stream_as_admin` | `env: Env`, `stream_id: u64`, `reason: Option<String>` | — | `admin.require_auth()` | Pause any stream as admin; same state semantics as `pause_stream`. |
| `resume_stream_as_admin` | `env: Env`, `stream_id: u64` | — | `admin.require_auth()` | Resume any paused stream as admin; same state semantics as `resume_stream`. |
| `bulk_resume_streams_as_admin` | `env: Env`, `stream_ids: Vec<u64>` | — | `admin.require_auth()` | Resume multiple paused streams in a single admin call. |
| `set_global_emergency_paused` | `env: Env`, `paused: bool` | — | `admin.require_auth()` | Toggle the global emergency pause flag that blocks all withdrawals. |
| `global_resume` | `env: Env` | — | `admin.require_auth()` | Clear the global emergency pause flag; alias for `set_global_emergency_paused(false)`. |
| `set_contract_paused` | `env: Env`, `paused: bool` | — | `admin.require_auth()` | Pause or unpause stream creation globally. |
| `pause_protocol` | `env: Env`, `admin: Address`, `reason: Option<String>` | — | `admin.require_auth()` | Globally pause new-stream creation with an optional reason string. |
| `resume_protocol` | `env: Env`, `admin: Address` | — | `admin.require_auth()` | Resume new-stream creation after a protocol pause. |
| `is_paused` | `env: Env` | `bool` | None (view) | Read whether protocol stream creation is currently paused. |
| `get_pause_info` | `env: Env` | `PauseInfo` | None (view) | Read current pause metadata (reason, timestamp, admin). |
| `sweep_excess` | `env: Env`, `recipient: Address` | `i128` | `admin.require_auth()` + `recipient.require_auth()` | Sweep excess contract balance (above tracked liabilities) to a recipient. |
| `set_auto_claim` | `env: Env`, `stream_id: u64`, `destination: Address` | — | `stream.recipient.require_auth()` | Set auto-claim destination; any caller may then trigger a claim permissionlessly. |
| `revoke_auto_claim` | `env: Env`, `stream_id: u64` | — | `stream.recipient.require_auth()` | Revoke the auto-claim destination for a stream. |
| `trigger_auto_claim` | `env: Env`, `stream_id: u64` | `i128` | None (permissionless) | Permissionlessly execute auto-claim withdrawal to the stored destination. |
| `get_auto_claim_status` | `env: Env`, `stream_id: u64` | `bool` | None (view) | Read whether auto-claim is currently enabled for a stream. |
| `get_auto_claim_destination` | `env: Env`, `stream_id: u64` | `Option<Address>` | None (view) | Read the auto-claim destination address if set. |
| `clone_stream` | `env: Env`, `source_stream_id: u64`, `deposit_amount: i128` | `u64` | `source.sender.require_auth()` | Clone a source stream's parameters into a new stream; caller supplies a new deposit. |
| `reserve_stream_ids` | `env: Env`, `caller: Address`, `count: u32` | `IdReservation` | `caller.require_auth()` | Reserve a contiguous range of stream IDs for later use; bounded by `MAX_ID_RESERVATION`. |
| `release_id_reservation` | `env: Env`, `caller: Address` | — | `caller.require_auth()` | Release an active stream ID reservation before expiry. |
| `reclaim_expired_id_reservation` | `env: Env`, `holder: Address` | — | None (permissionless) | Reclaim and clear an expired stream ID reservation held by any address. |
| `get_id_reservation` | `env: Env`, `caller: Address` | `Option<IdReservation>` | None (view) | View the active stream ID reservation for a caller, if any. |
| `bulk_cancel_streams` | `env: Env`, `stream_ids: Vec<u64>` | `Vec<Result<(), ContractError>>` | per-stream sender or admin | Cancel multiple streams; each entry independently authorised by the stream's sender or by admin. |
<!-- ci:audit-entrypoint-table-end -->

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
   IDs are assigned from a monotonically increasing `NextStreamId` counter; no reuse or gap-fill. For complete stream ID semantics including monotonicity guarantees, uniqueness proofs, counter management, batch operations, economic conservation, payout ordering, and verification commands, see [stream-id-monotonicity-uniqueness.md](./stream-id-monotonicity-uniqueness.md).

5. **Sender ≠ recipient**  
   Enforced in `create_stream`; self-streaming is disallowed.

6. **Deposit covers total streamable amount**  
   `deposit_amount >= rate_per_second × (end_time − start_time)` is enforced in `create_stream`.

7. **Deposit sufficiency preserved on extension**  
   `extend_stream_end_time` re-validates `deposit_amount >= rate_per_second × (new_end_time − start_time)` before updating `end_time`. If the check fails, the call panics and no state changes occur. No token transfer happens on extension — the deposit already held in the contract must cover the longer duration. Use `top_up_stream` first if the current deposit is insufficient.

8. **Time bounds**  
   `start_time < end_time` and `cliff_time ∈ [start_time, end_time]` are enforced in `create_stream`.

9. **Init once (authenticated bootstrap)**  
   `init` requires admin authorization and panics if config already exists; token is immutable after init and admin changes only via `set_admin`.

10. **Pause / resume / cancel authorization**  
    `pause_stream`, `resume_stream`, and `cancel_stream` require sender auth. The `_as_admin` variants require admin auth and provide the same behaviour. Only the recipient can call `withdraw`.

11. **Status transitions**
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

13. **Reentrancy Guard**

All token-transfer paths (`withdraw`, `withdraw_to`, `batch_withdraw`, `cancel_stream`) are protected by an explicit `DataKey::ReentrancyLock` guard. If a cross-contract callback (e.g., via a custom token hook) attempts to re-enter any of these functions while a transfer is in progress, the call will revert with `ContractError::InvalidState`.

14. **Contract balance consistency**  
    Deposit is pulled in `create_stream`; refunds and withdrawals only move amounts derived from that deposit (unstreamed to sender, accrued to recipient). No minting or arbitrary transfers.

---

For security patterns (e.g. CEI, reentrancy) see [docs/security.md](security.md).

---

## Delegation helper (`contracts/stream/src/delegation.rs`)

`delegated_withdraw` delegates its deadline and nonce validation to
`validate_delegation_params(env, stream_id, nonce, deadline)` in
`src/delegation.rs`.  Auditors reviewing the delegated-withdraw auth path
should start there.

### What the helper checks

| Check | Error on failure |
|---|---|
| `env.ledger().timestamp() > deadline` | `SignatureDeadlineExpired` |
| `nonce != stored_nonce(stream.recipient)` | `InvalidParams` |
| `stream_id` does not exist | `StreamNotFound` |

### What the helper does NOT check

The following checks remain in `delegated_withdraw` itself (after the helper returns `Ok`):

- `destination == env.current_contract_address()` → `InvalidParams`
- `stream.status == Completed` → `InvalidState`
- `stream.status == Paused` → `InvalidState`
- Ed25519 signature verification → `InvalidSignature` (or host trap)

### Nonce storage

Nonces are stored under `DataKey::WithdrawNonce(recipient_address)` in persistent
storage.  The nonce is incremented by `increment_withdraw_nonce` only after all
checks pass and a non-zero amount is transferred.  A zero-amount call does not
consume the nonce.
