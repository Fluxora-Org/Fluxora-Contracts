# Fluxora Streaming Contract — Security Documentation

## cancel_stream_as_admin: Operational Cancel Parity

**Issue:** `cancel_stream_as_admin: operational cancel parity`  
**Branch:** `feature/cancel-admin-parity`  
**Scope:** Authorization boundaries, state transitions, refund semantics, and event observability for the admin cancellation path.

---

## 1. Protocol Semantics

### 1.1 What "parity" means

A cancellation is "in parity" when every **externally observable** outcome of `cancel_stream_as_admin` is identical to `cancel_stream`, with the exception of the authorization check and the global-pause bypass (both intentional and documented below).

"Externally observable" means:

- Persistent fields on the `Stream` struct stored in contract storage
- Token balances of all involved parties (sender, recipient, contract)
- Errors returned to callers
- Events published to the ledger

There is no hidden internal state that differs between the two paths. Both call `cancel_stream_internal`, which is the single authoritative implementation.

### 1.2 Parity table

| Observable                        | `cancel_stream` (sender path) | `cancel_stream_as_admin` (admin path) |
|-----------------------------------|-------------------------------|----------------------------------------|
| `stream.status` after call        | `Cancelled`                   | `Cancelled`                            |
| `stream.cancelled_at` after call  | `Some(ledger.timestamp())`    | `Some(ledger.timestamp())`             |
| Refund formula                    | `deposit - accrued_at_now`    | `deposit - accrued_at_now`             |
| Refund recipient                  | `stream.sender`               | `stream.sender`                        |
| Accrued held for recipient        | `accrued_at_now`              | `accrued_at_now`                       |
| Auto-push to recipient on cancel  | No                            | No                                     |
| Event topic                       | `"cancelled"`                 | `"cancelled"`                          |
| Event payload                     | `StreamCancelled(stream_id)`  | `StreamCancelled(stream_id)`           |
| CEI ordering                      | State saved before transfer   | State saved before transfer            |

### 1.3 Intentional difference: global emergency pause

This is the **one documented asymmetry** between the two paths:

| Caller              | Global emergency pause | Outcome                   |
|---------------------|------------------------|---------------------------|
| Stream sender       | `false`                | Succeeds                  |
| Stream sender       | `true`                 | `"contract is globally paused"` (panic) |
| Contract admin      | `false`                | Succeeds                  |
| Contract admin      | `true`                 | **Succeeds** (intentional bypass) |

**Rationale:** Admin override paths must remain operative during emergency pause so that operators can always intervene. If admin cancel were also blocked by the global pause, an operator could not resolve a stream dispute or perform an emergency intervention while the pause is active. This is a deliberate protocol design choice, not a bug.

**Implementation:** `cancel_stream` calls `require_not_globally_paused(&env)` at the top. `cancel_stream_as_admin` does **not** call it. The comment in the implementation explicitly documents this.

---

## 2. Authorization Boundaries

### 2.1 Role matrix for cancellation

| Role              | `cancel_stream` | `cancel_stream_as_admin` |
|-------------------|-----------------|--------------------------|
| Stream sender     |  Authorized    |  Rejected by auth engine |
| Stream recipient  | Rejected      | Rejected               |
| Contract admin    |  Rejected      |  Authorized             |
| Any third party   | Rejected      |  Rejected               |

### 2.2 What each role must prove

**Stream sender** (`cancel_stream`):
- Must supply a valid Soroban authorization for the `cancel_stream` invocation.
- The contract checks `stream.sender.require_auth()` via `require_stream_sender`.
- No other proof is required.

**Contract admin** (`cancel_stream_as_admin`):
- Must supply a valid Soroban authorization for the `cancel_stream_as_admin` invocation.
- The contract reads the current admin from `Config` (instance storage) and calls `admin.require_auth()`.
- The admin address is set at `init()` and rotatable via `set_admin()`.
- No stream-specific ownership proof is required — the admin can cancel **any** stream.

### 2.3 What is impossible for non-authorized actors

- A recipient cannot cancel any stream via either path.
- An arbitrary third party cannot cancel any stream via either path.
- The sender cannot use the admin entrypoint (even if the sender happens to be the admin — they would need to authorize as admin, not as sender, to use the admin path).
- The admin cannot cancel a stream without an on-chain authorization signature; there is no server-side bypass.

---

## 3. Edge Case Enumeration

### 3.1 Time-driven edge cases

| Scenario                      | Expected behavior                                              |
|-------------------------------|----------------------------------------------------------------|
| Cancel at `t = start_time`   | `accrued = 0`; full refund to sender; contract balance → 0   |
| Cancel before cliff           | `accrued = 0` (cliff not reached); full refund; contract → 0 |
| Cancel exactly at cliff       | `accrued = cliff_time - start_time`; partial refund          |
| Cancel after cliff, mid-stream| `accrued = now - start_time`; partial refund                 |
| Cancel at `t = end_time`     | `accrued = deposit_amount`; zero refund; all held for recipient |
| Cancel past `end_time`       | `accrued` capped at `deposit_amount`; same as at `end_time`  |
| Cancel on Paused stream       | Uses **current** `ledger.timestamp()`, not pause timestamp   |

### 3.2 Stream status combinations

| Stream status at cancel call | `cancel_stream` | `cancel_stream_as_admin` |
|------------------------------|-----------------|--------------------------|
| `Active`                     |  Succeeds      | Succeeds               |
| `Paused`                     | Succeeds      |  Succeeds               |
| `Completed`                  | `InvalidState`|  `InvalidState`         |
| `Cancelled`                  |  `InvalidState`| `InvalidState`         |

### 3.3 Numeric range edge cases

| Scenario                        | Expected behavior                                   |
|---------------------------------|-----------------------------------------------------|
| `accrued_at_cancel = 0`         | `refund = deposit_amount`; no contract balance held |
| `accrued_at_cancel = deposit`   | `refund = 0`; no token sent to sender               |
| `deposit - accrued` underflows  | Protected by `checked_sub`; returns `InvalidState`  |
| Stream ID does not exist        | Returns `StreamNotFound` before auth check*         |

> *Note: Auth check occurs before `load_stream`. If the admin is not authorized, the call fails before any storage read.

### 3.4 Partial withdrawal before cancel

When the recipient has already withdrawn some tokens before an admin cancel:

- `refund = deposit_amount - accrued_at_cancel` — the refund uses total deposit, not remaining balance
- `accrued_at_cancel - withdrawn_amount` is what remains in the contract for the recipient
- The recipient can withdraw the remainder after the cancel

**Example:** deposit=1000, rate=1/s. Recipient withdraws 200 at t=200. Admin cancels at t=600.
- `accrued_at_cancel = 600`
- `refund_to_sender = 1000 - 600 = 400`
- Contract holds `600 - 200 = 400` for recipient to withdraw

---

## 4. Emitted Events

### 4.1 Event shape

Both `cancel_stream` and `cancel_stream_as_admin` emit:

```
topic: ("cancelled", stream_id)
data:  StreamEvent::StreamCancelled(stream_id)
```

The event payload is **identical** regardless of which role triggered the cancellation. Indexers and wallets cannot distinguish the triggering role from on-chain data alone.

### 4.2 Event ordering within the transaction

1. State saved to persistent storage (`save_stream`)
2. Token transfer to sender (`push_token`, if `refund > 0`)
3. Event published (`env.events().publish(...)`)

The event is always the last observable in the transaction for a cancellation call.

### 4.3 No event on auth failure

If the authorization check fails (wrong caller), no event is emitted and no state is changed. The transaction reverts atomically.

---

## 5. CEI (Checks-Effects-Interactions) Analysis

```
cancel_stream_as_admin(stream_id):
│
├─ [CHECK] get_admin()?.require_auth()           ← Auth check, no storage write
├─ [CHECK] load_stream(&env, stream_id)?         ← Read stream; fail if not found
├─ cancel_stream_internal(&env, &mut stream):
│   ├─ [CHECK] require_cancellable_status(status)?  ← Validate status
│   ├─ [CHECK] calculate_accrued_amount(...)        ← Pure computation
│   ├─ [CHECK] checked_sub(...)                     ← Arithmetic check
│   ├─ [EFFECT] stream.status = Cancelled           ← Mutate struct
│   ├─ [EFFECT] stream.cancelled_at = Some(now)     ← Mutate struct
│   ├─ [EFFECT] save_stream(env, stream)            ← Persist to storage ← before interaction
│   ├─ [INTERACTION] push_token(env, sender, refund)  ← External token call
│   └─ [EFFECT] env.events().publish(...)           ← Emit event
```

State is persisted **before** the external `push_token` call. This means:
- If the token transfer fails (e.g., contract balance unexpectedly zero), the stream is already marked `Cancelled` in storage.
- In practice this should not occur under valid contract operation, since the deposit was pulled at `create_stream` and only released via cancel or withdraw.
- The token contract is assumed not to reenter this contract during `push_token`.

---

## 6. Residual Risks and Audit Notes

| Risk | Severity | Status |
|------|----------|--------|
| Admin can cancel any stream without per-stream consent from the sender | Medium | **Intentional** — operator emergency power. Should be governed by off-chain policy and multisig. |
| No time-lock or cooldown before admin cancel takes effect | Low | **Intentional** — emergency interventions must be immediate. |
| Admin cannot confiscate accrued recipient funds | N/A | **Guaranteed by design** — accrued funds remain in contract; recipient retains right to withdraw. |
| If admin key is compromised, attacker can cancel all streams | High | Mitigated by `set_admin` key rotation; recommend multisig admin. |
| Token contract reentrancy during refund transfer | Low | CEI ordering prevents state inconsistency; token contract assumed non-reentrant. |
| Two independent pause flags (`GlobalPaused` and `GlobalEmergencyPaused`) may cause confusion | Medium | **Audit note:** `set_contract_paused` writes `DataKey::GlobalPaused` (read by `require_not_globally_paused`); `set_global_emergency_paused` writes `DataKey::GlobalEmergencyPaused` (read by `is_global_emergency_paused`). These are separate flags. Recommend consolidating to one flag in a future refactor to eliminate the dual-flag confusion risk. |

---

## 7. Verification Evidence

### 7.1 Automated tests

All tests are in `contracts/stream/src/test.rs` under the section:
`// Tests — cancel_stream_as_admin: operational cancel parity`

| Test name | What it proves |
|-----------|---------------|
| `test_admin_cancel_parity_active_stream_status_transition` | Status → Cancelled, cancelled_at recorded |
| `test_admin_cancel_parity_paused_stream_status_transition` | Paused streams can be admin-cancelled |
| `test_admin_cancel_parity_full_refund_at_start` | Full refund when accrued = 0 |
| `test_admin_cancel_parity_partial_refund_at_30_percent` | Partial refund at 30% completion |
| `test_admin_cancel_parity_partial_refund_at_50_percent` | 50/50 split |
| `test_admin_cancel_parity_zero_refund_at_end_time` | Zero refund when fully accrued |
| `test_admin_cancel_parity_zero_refund_past_end_time` | Accrual capped at deposit |
| `test_admin_cancel_parity_before_cliff_full_refund` | Full refund before cliff |
| `test_admin_cancel_parity_at_cliff_boundary` | Accrual at exact cliff time |
| `test_admin_cancel_parity_after_cliff_partial_refund` | Partial refund after cliff |
| `test_admin_cancel_parity_refund_plus_accrued_equals_deposit` | Core invariant: refund + accrued = deposit |
| `test_admin_cancel_parity_accrual_frozen_after_cancel` | Accrual frozen at cancelled_at |
| `test_admin_cancel_parity_recipient_can_withdraw_accrued` | Recipient retains right to withdraw |
| `test_admin_cancel_parity_recipient_withdraw_after_partial_precancel_withdraw` | Post-cancel withdrawal with prior partial withdraw |
| `test_admin_cancel_parity_second_withdraw_returns_zero` | No double-spend after cancel |
| `test_admin_cancel_parity_emits_identical_event_shape` | Event shape parity |
| `test_admin_cancel_parity_cancel_event_payload` | Correct event payload |
| `test_admin_cancel_parity_paused_stream_uses_current_time_for_accrual` | Cancel time = current ledger time, not pause time |
| `test_admin_cancel_parity_both_paths_work_on_paused_stream` | Both paths produce identical refund on paused stream |
| `test_admin_cancel_auth_recipient_cannot_call_as_admin` | Recipient auth rejection |
| `test_admin_cancel_auth_sender_cannot_call_as_admin` | Sender auth rejection on admin path |
| `test_admin_cancel_auth_third_party_cannot_call_as_admin` | Third-party auth rejection |
| `test_admin_cancel_auth_admin_succeeds_strict` | Admin positive auth (strict mode) |
| `test_admin_cancel_rejects_completed_stream` | Terminal state rejection |
| `test_admin_cancel_rejects_already_cancelled_stream` | Double-cancel rejection |
| `test_admin_cancel_rejects_stream_already_cancelled_by_sender` | Cross-path double-cancel rejection |
| `test_admin_cancel_stream_not_found_returns_error` | StreamNotFound error |
| `test_pause_matrix_sender_cancel_blocked_when_paused` | Sender blocked during emergency pause |
| `test_pause_matrix_admin_cancel_succeeds_when_paused` | Admin bypass during emergency pause |
| `test_pause_matrix_sender_cancel_succeeds_when_not_paused` | Sender not blocked without pause |
| `test_pause_matrix_admin_cancel_works_before_and_after_pause_toggle` | Admin bypass consistent through toggle |
| `test_admin_cancel_failed_auth_has_no_side_effects` | Zero side effects on auth failure |
| `test_admin_cancel_parity_token_conservation` | Total token supply conserved |
| `test_admin_cancel_parity_works_on_any_stream_regardless_of_sender` | Admin has no per-sender restriction |

### 7.2 Audit exceptions (not automatically tested)

| Gap | Rationale | Residual risk |
|-----|-----------|---------------|
| Reentrancy during `push_token` in `cancel_stream_internal` | Token contracts on Soroban/Stellar cannot reenter the streaming contract in the current SDK model | Low — would require a malicious token contract deployed to the streaming contract's configured token address, which requires admin key compromise |
| TTL expiry of persistent stream storage | Stream TTL is bumped on every `load_stream` and `save_stream` call; a stream that has never been accessed could expire, but this is a storage hygiene issue, not a cancel-parity issue | Negligible for this scope |
| Two-flag pause system (`GlobalPaused` vs `GlobalEmergencyPaused`) | Both flags exist independently. The cancel paths only check `GlobalPaused` (via `require_not_globally_paused`). `GlobalEmergencyPaused` is surfaced via `get_global_emergency_paused` but not checked in any cancel path. This is a potential source of operator confusion. | Medium — recommend consolidating in a future refactor |

---

## 8. PR Checklist

- [x] `cancel_stream_internal` documented with parity guarantee table
- [x] `cancel_stream_as_admin` doc comment updated with full semantics, role matrix, pause bypass, failure table
- [x] `cancel_stream` doc comment updated with pause-blocked note and cross-reference
- [x] All parity test cases added to `contracts/stream/src/test.rs`
- [x] Pause matrix tests added (4 tests)
- [x] Auth boundary tests added (strict mode, all roles)
- [x] Terminal state rejection tests added
- [x] Token conservation invariant tested
- [x] Zero side-effects on auth failure tested
- [x] `docs/security.md` written (this document)
- [ ] Run `cargo test` in `contracts/stream` — **required before merge**
- [ ] Confirm `cargo llvm-cov` shows ≥95% on modified Rust modules