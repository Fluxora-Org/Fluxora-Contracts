# Audit preparation

This document lists all public entrypoints and core invariants of the Fluxora stream contract to help external auditors scope the review. It is accurate as of the current codebase; no code changes are implied.

The public entrypoint table below is kept in sync with every `pub fn` on the `FluxoraStream` `#[contractimpl]` block in `contracts/stream/src/lib.rs`. CI runs `script/validate-doc-alignment.py` to fail on missing or stale rows.

---

## Public entrypoints

| Entrypoint | Parameters | Return type | Authorization | Description |
| --- | --- | --- | --- | --- |
| `accept_recipient_update` | `env: Env`, `stream_id: u64` | — | Current recipient | Finalize a pending recipient rotation proposed by the sender. |
| `batch_withdraw_to` | env: Env, recipient: Address, withdrawals: Vec<WithdrawToParam> | Vec<BatchWithdrawResult> | Recipient | Withdraw multiple accrued tokens in a single batched call, each to a specified destination, returning per-row net amounts. |
| `batch_withdraw` | `env: Env`, `recipient: Address`, `stream_ids: Vec<u64>` | `Vec<BatchWithdrawResult>` | Recipient | Withdraw accrued tokens from multiple streams atomically; duplicate IDs revert the batch. |
| `withdraw` | env: Env, stream_id: u64 | i128 | Recipient | Transfer accrued-but-not-withdrawn tokens to recipient; may set Completed. |
| `withdraw_from_pool` | env: Env, stream_id: u64, caller: Address | i128 | Pool participant | Withdraw the caller's pro-rata share from a pooled stream once accrued. |
| `withdraw_to` | env: Env, stream_id: u64, destination: Address | i128 | Recipient | Withdraw accrued tokens to a specified destination address. |
| `witnessed_cancel_stream` | env: Env, stream_id: u64, witness_public_key: BytesN<32>, deadline: u64, witness_signature: BytesN<64> | — | Sender + ed25519-witnessed attestor | Cancel a stream attested off-chain by a compliance watchtower via an ed25519 signature whose pubkey is recorded on the stream. |
| `bulk_cancel_streams` | `env: Env`, `sender: Address`, `stream_ids: Vec<u64>` | — | Sender | Atomically cancel multiple owned streams and refund aggregate unstreamed balance. |
| `bulk_resume_streams_as_admin` | `env: Env`, `stream_ids: Vec<u64>` | — | Admin | Atomically resume multiple paused streams; all-or-nothing validation. |
| `calculate_accrued` | `env: Env`, `stream_id: u64` | `i128` | None (view) | Total accrued amount at current ledger time; clamped to deposit. |
| `cancel_recipient_update` | `env: Env`, `stream_id: u64` | — | Sender | Cancel a pending recipient rotation before acceptance. |
| `cancel_stream` | `env: Env`, `stream_id: u64` | — | Sender | Refund unstreamed tokens to sender; freeze accrual at cancellation time. Active or Paused only. |
| `cancel_stream_as_admin` | `env: Env`, `stream_id: u64` | — | Admin | Same cancellation semantics as `cancel_stream` with admin authorization. |
| `cancel_stream_offer` | `env: Env`, `sender: Address`, `offer_id: u64` | — | Sender | Cancel a pending stream offer; refund escrowed deposit to the sender. |
| `clone_stream` | `env: Env`, `stream_id: u64`, `new_recipient: Address`, `start_time: u64`, `end_time: u64`, `deposit: i128`, `force: bool` | `u64` | Source stream sender | Create a new stream copying rate/cliff offset from an existing stream. |
| `close_cancelled_stream` | `env: Env`, `stream_id: u64` | — | Anyone | Permissionless storage cleanup for Cancelled streams with zero claimable balance. |
| `close_completed_stream` | `env: Env`, `stream_id: u64` | — | Anyone | Permissionless storage cleanup for Completed streams. |
| `create_pooled_stream` | `env: Env`, `sender: Address`, `recipients: Vec<(Address, u32)>`, `deposit_amount: i128`, `rate_per_second: i128`, `start_time: u64`, `cliff_time: u64`, `end_time: u64`, `withdraw_dust_threshold: i128`, `memo: Option<Bytes>`, `kind: StreamKind` | `u64` | Sender | Create a pro-rata multi-recipient stream; each recipient withdraws share × accrued. |
| `create_stream` | `env: Env`, `sender: Address`, `recipient: Address`, `deposit_amount: i128`, `rate_per_second: i128`, `start_time: u64`, `cliff_time: u64`, `end_time: u64`, `withdraw_dust_threshold: i128`, `memo: Option<Bytes>`, `kind: StreamKind`, `irrevocable: Option<bool>`, `witness: Option<Address>` | `u64` | Sender | Create a stream, pull deposit into the contract, return new stream ID. |
| `create_stream_from_template` | `env: Env`, `sender: Address`, `template_id: u64`, `recipient: Address`, `deposit_amount: i128`, `rate_per_second: i128`, `withdraw_dust_threshold: i128`, `memo: Option<Bytes>`, `metadata: Option<Map<Bytes, Bytes>>`, `kind: StreamKind`, `irrevocable: Option<bool>` | `u64` | Sender | Create a stream using a registered schedule template plus caller-funded amounts. |
| `create_stream_offer` | `env: Env`, `sender: Address`, `recipient: Address`, `deposit_amount: i128`, `rate_per_second: i128`, `start_time: u64`, `cliff_time: u64`, `end_time: u64`, `withdraw_dust_threshold: i128`, `memo: Option<Bytes>`, `kind: StreamKind`, `metadata: Option<Map<Bytes, Bytes>>`, `expiry_time: Option<u64>` | `u64` | Sender | Create a signed offer for a recipient to later accept; deposit escrowed at creation. |
| `create_stream_relative` | `env: Env`, `sender: Address`, `params: CreateStreamRelativeParams` | `u64` | Sender | Create a stream with timing expressed relative to the current ledger timestamp. |
| `create_stream_with_lookback` | `env: Env`, `sender: Address`, `recipient: Address`, `deposit_amount: i128`, `rate_per_second: i128`, `start_time: u64`, `cliff_time: u64`, `end_time: u64`, `withdraw_dust_threshold: i128`, `memo: Option<Bytes>`, `kind: StreamKind`, `max_lookback_ledgers: Option<u32>` | `u64` | Sender | Create a stream with an optional per-withdrawal lookback bound; `Some(0)` is rejected, `None` removes any bound. |
| `create_streams` | `env: Env`, `sender: Address`, `streams: Vec<CreateStreamParams>` | `Vec<u64>` | Sender | Atomically create multiple streams with a single sender authorization and deposit pull. |
| `create_streams_partial` | `env: Env`, `sender: Address`, `streams: Vec<CreateStreamParams>` | `Vec<CreateStreamResult>` | Sender | Batch create with per-entry success/failure results instead of all-or-nothing semantics. |
| `create_streams_relative` | `env: Env`, `sender: Address`, `streams_relative: Vec<CreateStreamRelativeParams>` | `Vec<u64>` | Sender | Batch create using relative timing parameters converted to absolute timestamps. |
| `decrease_rate_per_second` | `env: Env`, `stream_id: u64`, `new_rate_per_second: i128` | — | Sender | Decrease stream rate and refund excess deposit to sender; Active or Paused only. |
| `delegate_recipient_share` | `env: Env`, `stream_id: u64`, `recipient: Address`, `share_bps: u32`, `new_recipient: Address` | `u64` | Recipient | Split off a child stream at a fixed basis-point share of the parent rate; bounded depth. |
| `delegated_withdraw` | `env: Env`, `stream_id: u64`, `relayer: Address`, `recipient_public_key: BytesN<32>`, `nonce: u64`, `deadline: u64`, `expected_minimum_amount: i128`, `signature: BytesN<64>` | `i128` | Relayer + ed25519 sig from recipient | Withdraw on behalf of recipient; signature commits to stream, nonce, deadline, and minimum amount. |
| `delete_stream_template` | `env: Env`, `owner: Address`, `template_id: u64` | — | Template owner | Delete a schedule template registered by the caller. |
| `extend_stream_end_time` | `env: Env`, `stream_id: u64`, `new_end_time: u64` | — | Sender | Increase `end_time`; existing deposit must cover extended duration. Active or Paused only. |
| `get_auto_claim_destination` | `env: Env`, `stream_id: u64` | `Option<Address>` | None (view) | Return the permissionless auto-claim destination registered by the recipient, if any. |
| `get_auto_claim_status` | `env: Env`, `stream_id: u64` | `AutoClaimStatus` | None (view) | Return whether auto-claim is configured and currently triggerable for the stream. |
| `get_auto_renew` | `env: Env`, `stream_id: u64` | `bool` | None (view) | Return whether the sender has enabled permissionless auto-renew. |
| `get_claimable_at` | `env: Env`, `stream_id: u64`, `timestamp: u64` | `i128` | None (view) | Preview withdrawable amount at an arbitrary timestamp without mutating state. |
| `get_config` | `env: Env` | `Config` | None (view) | Return token and admin addresses from instance storage. |
| `get_delegated_nonce` | `env: Env`, `recipient: Address` | `u64` | None (view) | Return current replay-protection nonce for delegated withdrawals. |
| `get_global_emergency_paused` | `env: Env` | `bool` | None (view) | Return whether the global emergency pause flag is set. |
| `get_id_reservation` | `env: Env`, `caller: Address` | `Option<IdReservation>` | None (view) | Return the active stream-ID reservation for a caller, if any. |
| `get_keeper_fee_split` | `env: Env`, `stream_id: u64` | `(i128, i128)` | None (view) | Preview keeper fee and sender refund that `keeper_cancel` would pay. |
| `get_lookback_window` | `env: Env`, `stream_id: u64` | `Option<u32>` | None (view) | Return the configured per-withdrawal lookback bound in ledgers, if any. |
| `get_pause_info` | `env: Env` | `PauseInfo` | None (view) | Return protocol pause state including reason, timestamp, and admin audit trail. |
| `get_paused_stream_count` | `env: Env` | `u64` | None (view) | Return count of streams currently in Paused status. |
| `get_pending_recipient_update` | `env: Env`, `stream_id: u64` | `Option<PendingRecipientUpdate>` | None (view) | Return a pending recipient rotation awaiting acceptance, if any. |
| `get_protocol_fees_accrued` | `env: Env` | `i128` | None (view) | Return cumulative keeper/protocol fees collected by the contract. |
| `get_recipient_pending_offers` | `env: Env`, `recipient: Address` | `Vec<u64>` | None (view) | List pending offer IDs for a recipient. |
| `get_recipient_stream_count` | `env: Env`, `recipient: Address` | `u64` | None (view) | Return number of active stream IDs indexed for a recipient. |
| `get_recipient_streams` | `env: Env`, `recipient: Address` | `Vec<u64>` | None (view) | Return all stream IDs for a recipient (bounded for large portfolios). |
| `get_recipient_streams_paginated` | `env: Env`, `recipient: Address`, `cursor: u64`, `limit: u32` | `Page` | None (view) | Cursor-paginated recipient stream export capped at `RECIPIENT_STREAMS_PAGE_LIMIT`. |
| `get_sender_portfolio_health` | `env: Env`, `sender: Address`, `cursor: u64`, `limit: u32` | `PortfolioHealthPage` | None (view) | Paginated health summary (underfunded/expired/healthy counts) for a sender. |
| `get_stream_count` | `env: Env` | `u64` | None (view) | Return total streams created (`NextStreamId` counter). |
| `get_stream_health` | `env: Env`, `stream_id: u64` | `StreamHealth` | None (view) | Return underfunding and remaining-balance health metrics for a stream. |
| `get_stream_memo` | `env: Env`, `stream_id: u64` | `Option<Bytes>` | None (view) | Return immutable memo bytes attached at stream creation. |
| `get_stream_metadata` | `env: Env`, `stream_id: u64` | `Option<Map<Bytes, Bytes>>` | None (view) | Return immutable metadata map attached at stream creation. |
| `get_stream_offer` | `env: Env`, `offer_id: u64` | `StreamOffer` | None (view) | Return a pending stream offer by ID. |
| `get_stream_state` | `env: Env`, `stream_id: u64` | `Stream` | None (view) | Return full on-chain stream state. |
| `get_stream_template` | `env: Env`, `template_id: u64` | `StreamScheduleTemplate` | None (view) | Read a registered schedule template by ID. |
| `get_streams_by_id_range` | `env: Env`, `start_id: u64`, `end_id: u64`, `limit: u64` | `Vec<Stream>` | None (view) | Paginated export of streams in an ID range; capped at `MAX_PAGE_SIZE`. |
| `get_total_liabilities` | `env: Env` | `i128` | None (view) | Return aggregate outstanding deposit liabilities across all streams. |
| `get_withdrawable` | `env: Env`, `stream_id: u64` | `i128` | None (view) | Return accrued minus withdrawn at current ledger time. |
| `global_resume` | `env: Env` | — | Admin | Clear the global emergency pause after an incident; emits `GlobalResumed`. |
| `init` | `env: Env`, `token: Address`, `admin: Address` | — | Bootstrap admin | One-time setup: store token and admin; panics if already initialized. |
| `is_paused` | `env: Env` | `bool` | None (view) | Return whether protocol-level stream creation is paused. |
| `keeper_cancel` | `env: Env`, `stream_id: u64`, `keeper: Address` | — | Keeper | Permissionless cancel after grace period; pays keeper fee from sender refund. |
| `pause_protocol` | `env: Env`, `admin: Address`, `reason: Option<String>` | — | Admin | Pause new stream creation with audit trail (reason, timestamp, admin). |
| `pause_stream` | `env: Env`, `stream_id: u64`, `reason: PauseReason` | — | Sender | Set stream status to Paused; Active streams only. |
| `pause_stream_as_admin` | `env: Env`, `stream_id: u64`, `reason: PauseReason` | — | Admin | Admin override to pause any Active stream. |
| `reclaim_expired_id_reservation` | `env: Env`, `holder: Address` | — | Anyone | Permissionlessly release an expired ID reservation and reclaim counter space. |
| `reject_stream_offer` | `env: Env`, `offer_id: u64` | — | Recipient | Reject a pending stream offer and return deposit to sender. |
| `register_stream_template` | `env: Env`, `owner: Address`, `start_delay: u64`, `cliff_delay: u64`, `duration: u64` | `u64` | Owner | Register a reusable relative schedule template; subject to per-owner and global caps. |
| `reject_stream_offer` | `env: Env`, `recipient: Address`, `offer_id: u64` | — | Recipient | Reject a pending stream offer; refund escrowed deposit to the sender. |
| `release_id_reservation` | `env: Env`, `caller: Address` | — | Reservation holder | Voluntarily abandon an unconsumed ID reservation. |
| `renew_stream` | `env: Env`, `stream_id: u64` | `u64` | Anyone | Permissionless renewal of a completed stream when auto-renew is enabled. |
| `reserve_stream_ids` | `env: Env`, `caller: Address`, `count: u32`, `expiry: Option<u64>` | `Vec<u64>` | Caller | Pre-allocate contiguous stream IDs for off-chain orchestration. |
| `set_auto_renew` | `env: Env`, `stream_id: u64`, `enabled: bool` | — | Recipient | Opt in or out of stream auto-renewal. |
| `resume_protocol` | `env: Env`, `admin: Address` | — | Admin | Resume protocol-level stream creation and clear pause audit trail. |
| `resume_stream` | `env: Env`, `stream_id: u64` | — | Sender | Set stream status to Active; Paused streams only. |
| `resume_stream_as_admin` | `env: Env`, `stream_id: u64` | — | Admin | Admin override to resume any Paused stream. |
| `revoke_auto_claim` | `env: Env`, `stream_id: u64` | — | Recipient | Remove a previously registered auto-claim destination. |
| `set_admin` | `env: Env`, `new_admin: Address` | — | Admin | Rotate contract admin address. |
| `set_auto_claim` | `env: Env`, `stream_id: u64`, `destination: Address` | — | Recipient | Register a fixed destination for permissionless `trigger_auto_claim`. |
| `set_auto_renew` | `env: Env`, `stream_id: u64`, `sender: Address`, `enabled: bool` | — | Sender | Enable or disable permissionless auto-renew on a stream. |
| `set_contract_paused` | `env: Env`, `paused: bool` | — | Admin | Toggle creation-only pause (`CreationPaused`); does not block withdrawals. |
| `set_global_emergency_paused` | `env: Env`, `paused: bool` | — | Admin | Toggle global emergency pause blocking operational mutations. |
| `set_lookback_window` | `env: Env`, `stream_id: u64`, `sender: Address`, `max_lookback_ledgers: Option<u32>` | — | Sender | Set or clear the per-stream lookback bound; rejects `Some(0)` and rejects Cancelled streams. |
| `set_max_rate_per_second` | `env: Env`, `max_rate: i128` | — | Admin | Set maximum allowed stream rate for future rate updates. |
| `set_stream_decommissioned` | `env: Env`, `stream_id: u64`, `sender: Address`, `decommissioned: bool` | — | Sender | Flag or clear stream decommission mode; blocks schedule and funding mutations while preserving withdrawals, pause/resume, and cancellation. |
| `shorten_stream_end_time` | `env: Env`, `stream_id: u64`, `new_end_time: u64` | — | Sender | Reduce `end_time` and refund unstreamed tokens to sender; Active or Paused only. |
| `sweep_excess` | `env: Env`, `recipient: Address` | `i128` | Admin | Recover token balance exceeding tracked liabilities to an admin-chosen address. |
| `top_up_stream` | `env: Env`, `stream_id: u64`, `funder: Address`, `amount: i128` | — | Funder | Pull additional tokens into stream deposit; Active or Paused only. |
| `transfer_claim_ownership` | env: Env, stream_id: u64, current_owner: Address, new_owner: Address | — | Current claim owner (or recipient if unset) | Transfer the authorised claim owner for a stream to a new address without affecting recipient entitlement. |
| `trigger_auto_claim` | `env: Env`, `stream_id: u64` | `i128` | Anyone | Permissionlessly withdraw to recipient's registered auto-claim destination. |
| `update_rate` | `env: Env`, `stream_id: u64`, `new_rate_per_second: i128`, `caller: Address` | — | Sender or admin | Update stream rate without deposit adjustment; caller must be sender or admin. |
| `update_rate_per_second` | `env: Env`, `stream_id: u64`, `new_rate_per_second: i128` | — | Sender | Increase rate forward-only; deposit must cover new rate × duration. |
| `update_recipient` | `env: Env`, `stream_id: u64`, `new_recipient: Address` | — | Sender | Propose recipient rotation; finalized by `accept_recipient_update`. |
| `version` | `env: Env` | `u32` | None (view) | Return compile-time contract version (`CONTRACT_VERSION`). |

| `accept_stream_offer` | env: Env, offer_id: u64 | u64 | Recipient | Accept a pending stream offer. |
| `cancel_stream_offer` | env: Env, offer_id: u64 | — | Sender | Cancel a pending stream offer before acceptance. |
| `create_pooled_stream` | env: Env, sender: Address, params: CreateStreamParams | u64 | Sender | Create a stream that distributes to a participant pool. |
| `create_stream_offer` | env: Env, sender: Address, recipient: Address, params: CreateStreamParams | u64 | Sender | Create a stream offer that requires recipient acceptance. |
| `delegate_recipient_share` | env: Env, stream_id: u64, delegate: Address, share_bps: u32 | — | Recipient | Delegate a share of future recipient yield to another address. |
| `get_auto_renew` | env: Env, stream_id: u64 | bool | Anyone (view) | Return whether auto-renew is currently enabled for a stream. |
| `get_lookback_window` | env: Env | u32 | Anyone (view) | Return the protocol-wide claim lookback window setting. |
| `get_recipient_pending_offers` | env: Env, recipient: Address | Vec<u64> | Anyone (view) | Return all pending stream offers for a given recipient. |
| `get_sender_portfolio_health` | env: Env, sender: Address, cursor: u64 | PortfolioHealthPage | Anyone (view) | Return an aggregated health report for a sender's streams. |
| `get_stream_offer` | env: Env, offer_id: u64 | StreamOffer | Anyone (view) | Return the state of a single stream offer. |
| `reject_stream_offer` | env: Env, offer_id: u64 | — | Recipient | Reject a pending stream offer. |
| `renew_stream` | env: Env, stream_id: u64 | u64 | Anyone | Permissionlessly renew an eligible auto-renew stream. |
| `set_auto_renew` | env: Env, stream_id: u64, enabled: bool | — | Sender | Toggle the auto-renew opt-in flag for a stream. |
| `set_lookback_window` | env: Env, window_ledgers: u32 | — | Admin | Set the protocol-wide claim lookback window. |
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

    The contract's primary defence against reentrancy is strict **CEI ordering**
    (Checks → Effects → Interactions): every entrypoint saves all state mutations
    to storage _before_ issuing any external token call. Because Soroban's
    cross-contract calls are synchronous and the host re-enters the contract only
    if explicitly invoked, a correctly ordered CEI sequence is sufficient to
    prevent double-spend or state-corruption reentrancy on all standard paths.

    **Accepted posture — CEI-only for standard token-transfer entrypoints:**

    The following entrypoints rely exclusively on CEI ordering. No explicit
    reentrancy lock is acquired or released:

    - `withdraw`, `withdraw_to`, `batch_withdraw`, `batch_withdraw_to`
    - `cancel_stream`, `cancel_stream_as_admin`, `bulk_cancel_streams`
    - `shorten_stream_end_time`, `decrease_rate_per_second`
    - `delegated_withdraw`
    - `top_up_stream` (pulls tokens _after_ persisting deposit increment)
    - `create_stream`, `create_streams`, `create_streams_partial`,
      `create_streams_relative`, `create_stream_relative`
    - `keeper_cancel`

    This posture is the **accepted design**. The CEI order guarantees that if a
    malicious token contract re-enters any of these entrypoints, the stream state
    already reflects the in-progress operation (e.g. `withdrawn_amount` is
    updated, status may be `Completed`) and the re-entrant call will either find
    nothing withdrawable or be rejected by the terminal-state guard.

    **Explicit reentrancy lock (`DataKey::ReentrancyLock`) — two entrypoints only:**

    Two entrypoints acquire an explicit lock in addition to CEI ordering:

    - `sweep_excess` — admin-callable; prevents concurrent balance sweeps from
      double-counting the excess while a transfer is in flight.
    - `trigger_auto_claim` — permissionless; the explicit lock prevents a race
      between two concurrent auto-claim triggers on the same stream.

    These are the only two entrypoints where the lock is both meaningful and
    correct. Extending the explicit lock to all entrypoints would introduce
    deadlock risk in legitimate same-transaction batch flows (e.g. a sender
    calling `bulk_cancel_streams` inside a composed transaction).

    **Discrepancy with `CEI_ANALYSIS.md` (Issue #262):**

    `CEI_ANALYSIS.md` previously claimed that `withdraw`, `withdraw_to`,
    `batch_withdraw`, `cancel_stream`, and `cancel_stream_as_admin` are wrapped
    in the reentrancy lock. This claim was inaccurate and has been corrected.
    The current code does not wrap those entrypoints with the lock, and this is
    the intentional, documented design. `CEI_ANALYSIS.md` should be treated as
    superseded by this invariant specification and by
    `docs/maintainer-security-checklist.md §14` (resolution note).

    **Summary of invariant requirements auditors should verify:**

    - State for the operation (withdrawn amount, status, deposit, cancelled_at)
      is persisted via `save_stream` or equivalent _before_ any `push_token` or
      `pull_token` call in the same entrypoint.
    - `sweep_excess` and `trigger_auto_claim` call `acquire_reentrancy_lock`
      before any state mutation that precedes a token transfer, and call
      `release_reentrancy_lock` after the transfer completes.
    - No other entrypoint calls `acquire_reentrancy_lock`.
    - `DataKey::ReentrancyLock` is set to `true` before the transfer and cleared
      to `false` (or removed) after; a second concurrent attempt to set it while
      `true` returns `ContractError::InvalidState`.

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
