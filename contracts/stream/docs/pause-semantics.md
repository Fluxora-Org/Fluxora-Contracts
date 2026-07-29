# Pause Semantics

> **Issue reference:** #1329, #1327  
> **Contract:** `contracts/stream/src/lib.rs`  
> **Status:** Normative — describes current deployed behavior.

---

## 1. Overview

The Fluxora stream contract provides two orthogonal pause mechanisms:

| Mechanism | Scope | Who controls it | What it blocks |
|---|---|---|---|
| **Per-stream pause** | One stream | Sender or admin | Withdrawals from that stream |
| **Global emergency pause** | All streams / whole protocol | Admin only | All user-facing mutations |

There is also a **creation-only pause** (`set_contract_paused`) that blocks new stream creation without affecting existing streams or withdrawals. It is narrower than the global pause and is not covered in depth here — see `global-resume.md` for the full protocol-pause lifecycle.

This document focuses on **per-stream pause** and its interaction with the global emergency pause.

---

## 2. Status model

A stream is a state machine. The full set of valid transitions is:

```
Active ──pause──► Paused ──resume──► Active
  │                  │
  └──cancel──► Cancelled (terminal)
  └──withdraw (full)──► Completed (terminal)
  Paused ──cancel──► Cancelled (terminal)
  Paused ──withdraw (full, time-terminal only)──► Completed (terminal)
```

`Completed` and `Cancelled` are terminal — no further transitions are possible.

---

## 3. Time-terminal override

`is_terminal_state` returns `true` when **either**:

1. `stream.status == Completed || stream.status == Cancelled`, **or**
2. `env.ledger().timestamp() >= stream.end_time`

Condition 2 is the *time-terminal override*. Its effect on pause operations:

| Operation | Time-terminal stream (past end_time) |
|---|---|
| `pause_stream` | **Blocked** — returns `StreamTerminalState` |
| `resume_stream` | **Blocked** — returns `StreamTerminalState` |
| `withdraw` on Paused stream | **Allowed** — time-terminal overrides `Paused` status |

This guarantees that recipients can always claim their full entitlement once a stream's schedule has elapsed, regardless of whether the sender locked the stream in `Paused` state before expiry.

---

## 4. Accrual is never paused

Tokens accrue strictly by wall-clock time (`rate_per_second × elapsed_seconds`). Setting `status = Paused` does **not** stop accrual. A stream paused at `t=300` and resumed at `t=500` will show `200` additional tokens accrued during the pause window.

This is intentional: pausing only restricts **withdrawals** — it does not freeze the recipient's economic entitlement.

Corollary: a sender cannot "pause the clock" to avoid paying a recipient.

---

## 5. Per-stream pause cooldown

To prevent rapid-toggle DoS (where a sender oscillates between `pause` and `resume` to increase gas costs for observers or to manipulate accrual accounting), each toggle is rate-limited by a ledger-sequence cooldown:

```
MIN_PAUSE_INTERVAL_LEDGERS = 17   (~85 seconds at 5 s/ledger)
```

The cooldown applies to **both** `pause_stream` and `resume_stream` (and their `_as_admin` counterparts). It is tracked per-stream on `Stream.last_pause_toggle_ledger`.

**Behavior:**

- On first pause (`last_pause_toggle_ledger == 0`, stream just created): the check uses `current_ledger.saturating_sub(0) == current_ledger`. If the ledger sequence at creation time is ≥ 17 (always true in production), the first pause succeeds.
- After any toggle, the next toggle must wait ≥ 17 ledgers.
- Violation returns `ContractError::PauseCooldownActive`.
- `saturating_sub` prevents underflow if the ledger sequence ever decreases (should be impossible on Stellar, but defensive).

> **Known test harness caveat:** `Env::default()` in unit tests starts the ledger sequence at `0`. A stream created at sequence `0` has `last_pause_toggle_ledger = 0`, making `0.saturating_sub(0) = 0 < 17` — the cooldown trips immediately on the first pause attempt. Tests must advance the sequence by ≥ 17 ledgers before the first toggle. See `Ctx::clear_pause_cooldown()` in `tests/paused_stream_count.rs`.

---

## 6. Pause entrypoints

### 6.1 `pause_stream(stream_id, reason: PauseReason)`

- **Authorization:** `stream.sender.require_auth()`
- **Pre-conditions (checked in order):**
  1. Stream exists — else `StreamNotFound`
  2. `status != Paused` — else `StreamAlreadyPaused`
  3. `!is_terminal_state` — else `StreamTerminalState`
  4. `status == Active` — else `InvalidState`
  5. Cooldown elapsed — else `PauseCooldownActive`
- **Mutations:** `status → Paused`, `last_pause_toggle_ledger = current_ledger`, `PausedStreamCount += 1`
- **Event:** `("paused", stream_id)` → `StreamPaused { stream_id, reason: String }`
- **Global pause:** Not checked. Sender can pause their own stream even during a global emergency pause.

### 6.2 `resume_stream(stream_id)`

- **Authorization:** `stream.sender.require_auth()`
- **Pre-conditions (checked in order):**
  1. Stream exists — else `StreamNotFound`
  2. `status != Active` — else `StreamNotPaused`
  3. `!is_terminal_state` — else `StreamTerminalState`
  4. `status == Paused` — else `StreamNotPaused`
  5. Cooldown elapsed — else `PauseCooldownActive`
- **Mutations:** `status → Active`, `last_pause_toggle_ledger = current_ledger`, `PausedStreamCount -= 1` (saturating)
- **Event:** `("resumed", stream_id)` → `StreamEvent::Resumed(stream_id)`
- **Global pause:** Not checked. Sender can resume a stream even during a global emergency pause.

### 6.3 `pause_stream_as_admin(stream_id, reason: PauseReason)`

Identical behavior to `pause_stream` except:
- **Authorization:** `admin.require_auth()` (bypasses sender check)
- **Extra side-effect:** Writes a `PauseRecord { actor, timestamp, reason }` to `DataKey::LastPauseRecord(PauseKind::Stream)` in instance storage.

### 6.4 `resume_stream_as_admin(stream_id)`

Identical behavior to `resume_stream` except:
- **Authorization:** `admin.require_auth()` (bypasses sender check)

### 6.5 `bulk_resume_streams_as_admin(stream_ids: Vec<u64>)`

Atomically resumes a batch of paused streams. Two-phase: **validate all first, then mutate all**. Any validation failure aborts the entire batch with no mutations applied.

- **Authorization:** `admin.require_auth()`
- **Empty batch:** No-op, returns `Ok(())`.
- **Duplicate IDs:** Returns `DuplicateStreamId` before any mutations.
- **Per-stream validations (phase 1):** Same as `resume_stream` (exists, `Paused`, not time-terminal, cooldown).
- **Mutations (phase 2):** Each stream: `status → Active`, `last_pause_toggle_ledger = current_ledger`, `PausedStreamCount -= 1`.
- **Events:** One `("resumed", stream_id)` per stream, emitted in phase 2.

---

## 7. Global emergency pause

`set_global_emergency_paused(true)` sets `DataKey::GlobalEmergencyPaused` and gates all user-facing mutations behind `require_not_globally_paused`.

### What the global pause blocks

All calls that invoke `require_not_globally_paused` (directly, or transitively via
`require_not_creation_paused` / `batch_withdraw` → `batch_withdraw_to`) fail with
`ContractError::ContractPaused`:

- `create_stream`, `create_streams`, `create_streams_partial`, `clone_stream`
- `withdraw`, `withdraw_to`, `batch_withdraw`, `batch_withdraw_to`, `withdraw_from_pool`, `delegated_withdraw`
- `cancel_stream`, `witnessed_cancel_stream`, `bulk_cancel_streams`
- `update_rate_per_second`, `decrease_rate_per_second`, `delegate_recipient_share`
- `shorten_stream_end_time`, `extend_stream_end_time`
- `top_up_stream`
- `transfer_claim_ownership`
- `update_recipient`, `accept_recipient_update`, `cancel_recipient_update`
- `accept_stream_offer`
- `set_lookback_window`, `set_stream_decommissioned`, `set_auto_renew`, `renew_stream`
- `trigger_auto_claim`

This list is verified against
`grep -n "require_not_globally_paused\|require_not_creation_paused" contracts/stream/src/lib.rs`
— re-run that grep after adding a new mutating entrypoint and update this list in the
same PR, so it does not silently drift the way it previously had (see below).

### What the global pause does NOT block

- `pause_stream`, `resume_stream` — sender can still manage their own streams.
- All `*_as_admin` entrypoints — admin retains full control.
- `global_resume`, `set_global_emergency_paused` — to allow clearing the pause.
- `set_admin` — admin rotation is intentionally left available so a compromised
  or unresponsive admin key can be replaced during an active emergency pause.
- `reject_stream_offer` — recipients can always decline a pending offer; declining
  returns escrowed funds to the sender and cannot be abused to move funds out
  under pause.
- All read-only / view functions.
- `close_completed_stream`.

> **Correction:** an earlier version of this table listed `set_admin` and
> `reject_stream_offer` as blocked by the global pause. Neither entrypoint calls
> `require_not_globally_paused`; both continue to work during an emergency pause.
> `batch_withdraw_to` and several newer entrypoints (`delegate_recipient_share`,
> `witnessed_cancel_stream`, `trigger_auto_claim`, `bulk_cancel_streams`, `clone_stream`,
> and others listed above) were previously missing from this table entirely. Fixed
> as part of #1327.

### Counter orthogonality

`DataKey::PausedStreamCount` tracks **only** individually-paused streams (those with `status == Paused`). Toggling `GlobalEmergencyPaused` has **no effect** on this counter. Consumers that need full pause-state awareness must check both:

```rust
let any_stream_paused = client.get_paused_stream_count() > 0;
let globally_paused   = client.get_global_emergency_paused();
```

---

## 8. PausedStreamCount storage

| Key | Type | Storage | Default |
|---|---|---|---|
| `DataKey::PausedStreamCount` | `u64` | Instance | `0` (absent = `0`) |

`reconcile_paused_stream_count(env, previous, next)` is called after every status transition that involves a `Paused` boundary:

- `!Paused → Paused` → counter `+= 1` (saturating_add)
- `Paused → !Paused` → counter `-= 1` (saturating_sub — safe if key was removed post-upgrade)
- All other transitions → no-op

The counter is set to `0` on contract `init`. Pre-upgrade instances that lack the key return `0` from `read_paused_stream_count` (explicit `unwrap_or(0)`), and `saturating_sub` prevents the counter from wrapping negative when a Paused stream is resumed on such an instance.

---

## 9. Per-stream pause state fields

| Field | Type | Purpose |
|---|---|---|
| `Stream.status` | `StreamStatus` | Current state (`Active`, `Paused`, `Completed`, `Cancelled`) |
| `Stream.last_pause_toggle_ledger` | `u32` | Ledger sequence of the last pause or resume; `0` on creation |

Both are stored in the `DataKey::Stream(stream_id)` persistent entry.

---

## 10. PauseReason

`PauseReason` is an enum argument to `pause_stream` and `pause_stream_as_admin`. It is serialized as a plain `soroban_sdk::String` in the emitted event payload and stored in `PauseRecord` (admin-pause path only). It has **no effect on contract behavior** — all four variants produce the same state transition.

| Variant | Event string |
|---|---|
| `PauseReason::Operational` | `"Operational"` |
| `PauseReason::Administrative` | `"Administrative"` |
| `PauseReason::Emergency` | `"Emergency"` |
| `PauseReason::Compliance` | `"Compliance"` |

---

## 11. Event reference

| Topic | Emitted by | Payload type | Payload |
|---|---|---|---|
| `("paused", stream_id)` | `pause_stream`, `pause_stream_as_admin` | `StreamPaused` | `{ stream_id: u64, reason: String }` |
| `("resumed", stream_id)` | `resume_stream`, `resume_stream_as_admin`, `bulk_resume_streams_as_admin` | `StreamEvent::Resumed(u64)` | `Resumed(stream_id)` |
| `("gl_pause",)` | `set_global_emergency_paused` | `GlobalEmergencyPauseChanged` | `{ paused: bool }` |
| `("gl_resume",)` | `global_resume` | `GlobalResumed` | `{ resumed_at: u64 }` |
| `("ct_pause",)` | `set_contract_paused` | `ContractPauseChanged` | `{ paused: bool }` |
| `("pr_pause", admin)` | `pause_protocol` | `ProtocolPaused` | (see events.rs) |
| `("pr_resume", admin)` | `resume_protocol` | `ProtocolResumed` | (see events.rs) |

---

## 12. Error codes

| Error | Code | Trigger |
|---|---|---|
| `StreamAlreadyPaused` | 14 | `pause_stream` / `pause_stream_as_admin` on a `Paused` stream |
| `StreamNotPaused` | 15 | `resume_stream` / `resume_stream_as_admin` on a non-`Paused` stream |
| `StreamTerminalState` | 16 | Pause or resume on a time-terminal or status-terminal stream |
| `PauseCooldownActive` | 28 | Toggle attempted within 17 ledgers of the previous toggle |
| `ContractPaused` | 4 | User-facing mutation attempted while `GlobalEmergencyPaused == true` |
| `InvalidState` | 2 | `pause_stream` on a non-`Active` stream (catch-all after targeted checks) |
| `DuplicateStreamId` | (see error.md) | Duplicate ID in `bulk_resume_streams_as_admin` batch |

---

## 13. Interaction with other operations

### Cancellation from Paused

`cancel_stream` and `cancel_stream_as_admin` accept streams in **either** `Active` or `Paused` state. Cancelling a paused stream:
- Transitions status `Paused → Cancelled`.
- `reconcile_paused_stream_count` decrements the counter.
- Accrual is frozen at `cancelled_at` (same as Active cancellation).
- Recipient can still withdraw the accrued amount after cancellation.

### Withdrawal from Paused

`withdraw`, `withdraw_to`, `batch_withdraw`, `batch_withdraw_to`, and `withdraw_from_pool` all gate on:

```rust
if stream.status == StreamStatus::Paused && !is_terminal_state(&env, &stream) {
    return Err(ContractError::InvalidState);
}
```

This means:
- Withdrawal is **blocked** on a `Paused` stream that is not yet time-terminal.
- Withdrawal is **allowed** on a `Paused` stream once `current_time >= end_time` (time-terminal override).
- If the time-terminal withdrawal fully drains the stream, status transitions `Paused → Completed` and `PausedStreamCount` is decremented.
- `batch_withdraw_to` (the keeper/auto-claim sweep path) applies the identical
  per-item gate inside its batch loop — a stream cannot bypass its individual
  pause by being swept through the batch entrypoint instead of `withdraw`.
  Regression-pinned by `pause_semantics.rs::batch_withdraw_to_blocked_while_paused`.

### Top-up on Paused

`top_up_stream` does not check the `Paused` status specifically — it accepts `Active` or `Paused` streams (not terminal). A sender can increase the deposit while the stream is paused.

### Rate and schedule changes on Paused

`update_rate_per_second`, `decrease_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time` all operate on non-terminal streams and are allowed while the stream is `Paused` (subject to their own cooldowns and validations).

### Delegation on Paused

`delegate_recipient_share` does not check `stream.status == Paused` — it only rejects `Completed`/`Cancelled` (terminal) and `now >= end_time`. A recipient can therefore split off a delegated child stream from a `Paused` parent:
- The parent's `status` is left untouched (stays `Paused`) — only `checkpointed_amount`, `checkpointed_at`, `rate_per_second`, and `deposit_amount` are updated.
- The child stream is always created with `status: Active`, regardless of the parent's status.

This is consistent with the "rate and schedule changes on Paused" behavior above — pause gates withdrawals, not configuration/allocation changes. Regression-pinned by `pause_semantics.rs::delegate_recipient_share_allowed_while_paused`.

---

## 14. Upgrade / pre-upgrade compatibility

- `Stream.last_pause_toggle_ledger` was added post-genesis. Pre-upgrade stream entries that lack this field will deserialize it as `0`, making `current_ledger.saturating_sub(0) = current_ledger`. On any production ledger (sequence >> 17), this means the first post-upgrade pause/resume on a pre-upgrade stream succeeds immediately — no cooldown penalty for the transition.
- `DataKey::PausedStreamCount` is absent on pre-upgrade instances. `read_paused_stream_count` returns `0` via `unwrap_or(0)`. `reconcile_paused_stream_count` uses `saturating_sub`, so resuming a pre-upgrade stream that was paused before the counter existed cannot underflow.
- `CONTRACT_VERSION` does not need to increment for documentation-only changes.

---

## 15. Regression surface

The following behaviors must not change across refactors. Each has at least one test pinning it:

| Behavior | Covered by |
|---|---|
| Sender can pause own Active stream | `paused_stream_count.rs::paused_stream_count_tracks_sender_pause_resume` |
| Sender can resume own Paused stream | same |
| Admin can pause any Active stream | `paused_stream_count.rs::paused_stream_count_tracks_admin_pause_resume` |
| Admin can resume any Paused stream | same |
| Double-pause returns `StreamAlreadyPaused` | `paused_stream_count.rs::paused_stream_count_ignores_failed_idempotent_calls` |
| Double-resume returns `StreamNotPaused` | same |
| Counter decrements on cancel-from-Paused | `paused_stream_count.rs::paused_stream_count_decrements_on_cancel_from_paused` |
| Counter decrements on time-terminal withdraw-from-Paused | `paused_stream_count.rs::paused_stream_count_decrements_on_terminal_completion_from_paused` |
| Counter saturates at 0 if PausedStreamCount key is absent | `paused_stream_count.rs::paused_stream_count_never_underflows_when_upgrade_key_is_missing` |
| Counter is 0 on fresh init | `paused_stream_count.rs::paused_stream_count_is_initialised_to_zero` |
| Global emergency pause does not affect counter | `paused_stream_count.rs::paused_stream_count_is_zero_during_global_emergency_pause` |
| Global toggle is orthogonal to per-stream counter | `paused_stream_count.rs::paused_stream_count_unaffected_by_global_emergency_toggle` |
| Cooldown blocks rapid toggle | `pause_semantics.rs::pause_cooldown_blocks_rapid_retoggle` |
| Cooldown applies symmetrically to resume | `pause_semantics.rs::resume_cooldown_blocks_rapid_re_resume` |
| Paused stream blocks withdrawal | `pause_semantics.rs::withdraw_blocked_while_paused` |
| Time-terminal Paused stream allows withdrawal | `pause_semantics.rs::withdraw_allowed_on_paused_stream_past_end_time` |
| Time-terminal withdraw-from-Paused sets Completed | `pause_semantics.rs::time_terminal_withdraw_from_paused_completes_stream` |
| pause_stream rejected on time-terminal stream | `pause_semantics.rs::pause_rejected_on_time_terminal_stream` |
| resume_stream rejected on time-terminal Paused stream | `pause_semantics.rs::resume_rejected_on_time_terminal_paused_stream` |
| Cancelled stream cannot be paused | `pause_semantics.rs::pause_rejected_on_cancelled_stream` |
| Completed stream cannot be paused | `pause_semantics.rs::pause_rejected_on_completed_stream` |
| Accrual continues during pause | `pause_semantics.rs::accrual_continues_while_paused` |
| Cancel from Paused refunds correctly | `pause_semantics.rs::cancel_from_paused_refunds_unstreamed` |
| PauseReason variants all succeed and appear in event | `pause_semantics.rs::all_pause_reason_variants_accepted` |
| Global pause blocks withdraw | `pause_semantics.rs::global_pause_blocks_withdraw` |
| Global pause does not block pause_stream | `pause_semantics.rs::global_pause_does_not_block_sender_pause` |
| bulk_resume validates atomically | `bulk_resume_as_admin.rs::bulk_resume_mixed_cancelled_is_atomic` |
| bulk_resume rejects duplicates | `bulk_resume_as_admin.rs::bulk_resume_rejects_duplicate_ids` |
| bulk_resume rejects cooldown violation | `bulk_resume_as_admin.rs::bulk_resume_rejects_pause_cooldown` |
| `batch_withdraw_to` blocks a Paused, non-time-terminal stream | `pause_semantics.rs::batch_withdraw_to_blocked_while_paused` |
| `delegate_recipient_share` is unaffected by parent's Paused status | `pause_semantics.rs::delegate_recipient_share_allowed_while_paused` |
