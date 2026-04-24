    };

    let accrued = match elapsed_seconds.checked_mul(rate_per_second) {
        Some(amount) => amount,
        None => deposit_amount,
    };

    accrued.min(deposit_amount).max(0)
}
```

### Alignment Verification

✅ **Perfect match** with additional safety:
- Overflow protection: `checked_mul` returns `deposit_amount` on overflow (safe upper bound)
- Underflow protection: `checked_sub` returns `0` if `elapsed_now < start_time`
- Both documented in streaming.md §2 "Overflow" and "Rules"

t
pub fn calculate_accrued_amount(
    start_time: u64,
    cliff_time: u64,
    end_time: u64,
    rate_per_second: i128,
    deposit_amount: i128,
    current_time: u64,
) -> i128 {
    if current_time < cliff_time {
        return 0;
    }

    if start_time >= end_time || rate_per_second < 0 {
        return 0;
    }

    let elapsed_now = current_time.min(end_time);
    let elapsed_seconds = match elapsed_now.checked_sub(start_time) {
        Some(elapsed) => elapsed as i128,
        None => return 0,
nvalidState` | None | lib.rs:1991 |
| Cancelled | Cancelled | `cancel_stream` | `InvalidState` | None | lib.rs:1991 |

## Accrual Formula Alignment

### Documentation (streaming.md §2)

```
if current_time < cliff_time           → return 0
if start_time >= end_time or rate < 0  → return 0

elapsed_now = min(current_time, end_time)
elapsed_seconds = elapsed_now - start_time
accrued = elapsed_seconds * rate_per_second
return min(accrued, deposit_amount).max(0)
```

### Implementation (accrual.rs:14-42)

```rus-----|---------------|
| Paused | Paused | `pause_stream` | `InvalidState` | None | lib.rs:911-913 |
| Active | Active | `resume_stream` | `InvalidState` | None | lib.rs:942 |
| Completed | * | Any mutation | `InvalidState` | None | Various |
| Cancelled | * | Any mutation | `InvalidState` | None | Various |
| Completed | Active | `resume_stream` | `InvalidState` | None | lib.rs:942 |
| Cancelled | Active | `resume_stream` | `InvalidState` | None | lib.rs:942 |
| Completed | Cancelled | `cancel_stream` | `Idraw` (full drain) | `status=Completed`, `withdrawn_amount=deposit_amount` | `Withdrawal`, `StreamCompleted` | Remaining tokens to recipient | lib.rs:1033-1099 |
| Paused | Completed | `withdraw` (if fully accrued) | `status=Completed`, `withdrawn_amount=deposit_amount` | `Withdrawal`, `StreamCompleted` | Remaining tokens to recipient | lib.rs:1033-1099 |

### Invalid Transitions (Crisp Failure Semantics)

| From | To | Attempt | Error | Side Effects | Code Location |
|------|-----|---------|-------|---------sed` | `Paused(stream_id)` | None | lib.rs:906-925 |
| Paused | Active | `resume_stream` | `status=Active` | `Resumed(stream_id)` | None | lib.rs:937-955 |
| Active | Cancelled | `cancel_stream` | `status=Cancelled`, `cancelled_at=Some(now)` | `StreamCancelled(stream_id)` | Refund to sender | lib.rs:987-991, 1987-2020 |
| Paused | Cancelled | `cancel_stream` | `status=Cancelled`, `cancelled_at=Some(now)` | `StreamCancelled(stream_id)` | Refund to sender | lib.rs:987-991, 1987-2020 |
| Active | Completed | `withised` | lib.rs:666 |

## State Transition Matrix

### Valid Transitions (Crisp Success Semantics)

| From | To | Trigger | Storage Changes | Events | Token Movements | Code Location |
|------|-----|---------|----------------|--------|----------------|---------------|
| N/A | Active | `create_stream` | New stream persisted, `status=Active`, `withdrawn_amount=0`, `cancelled_at=None` | `StreamCreated` | `deposit_amount` from sender to contract | lib.rs:754-819 |
| Active | Paused | `pause_stream` | `status=Pau_sender` | Auth failure | lib.rs:1975 |
| Non-sender resume | `require_stream_sender` | Auth failure | lib.rs:1975 |
| Non-sender cancel | `require_stream_sender` | Auth failure | lib.rs:1975 |
| Non-recipient withdraw | `recipient.require_auth()` | Auth failure | lib.rs:1033 |
| Non-admin admin operations | `admin.require_auth()` | Auth failure | lib.rs:2033, 2063, 2095 |
| Non-admin set_admin | `old_admin.require_auth()` | Auth failure | lib.rs:1437 |
| Re-init | `has(&DataKey::Config)` check | `AlreadyInitialrequire_auth()` via `require_stream_sender` | lib.rs:1541 | streaming.md §4 |
| `shorten_stream_end_time` | Sender | `sender.require_auth()` via `require_stream_sender` | lib.rs:1598 | streaming.md §4 |
| `extend_stream_end_time` | Sender | `sender.require_auth()` via `require_stream_sender` | lib.rs:1673 | streaming.md §4 |

### Impossible Operations (Enforced by Authorization)

| Attempt | Blocked By | Error | Code Location |
|---------|-----------|-------|---------------|
| Non-sender pause | `require_stream None | lib.rs:1313 | streaming.md §4 |
| `get_stream_state` | Anyone (read-only) | None | lib.rs:1485 | streaming.md §4 |
| `get_withdrawable` | Anyone (read-only) | None | lib.rs:1349 | streaming.md §4 |
| `get_claimable_at` | Anyone (read-only) | None | lib.rs:1378 | streaming.md §4 |
| `close_completed_stream` | Anyone (permissionless) | None | lib.rs:1851 | streaming.md §4 |
| `top_up_stream` | Funder | `funder.require_auth()` | lib.rs:1799 | streaming.md §4 |
| `update_rate_per_second` | Sender | `sender..md §4 |
| `batch_withdraw` | Recipient | `recipient.require_auth()` | lib.rs:1213 | streaming.md §4 |
| `pause_stream_as_admin` | Admin | `admin.require_auth()` | lib.rs:2063 | streaming.md §4 |
| `resume_stream_as_admin` | Admin | `admin.require_auth()` | lib.rs:2095 | streaming.md §4 |
| `cancel_stream_as_admin` | Admin | `admin.require_auth()` | lib.rs:2033 | streaming.md §4 |
| `set_admin` | Current admin | `old_admin.require_auth()` | lib.rs:1437 | streaming.md §4 |
| `calculate_accrued` | Anyone (read-only) | |
| `pause_stream` | Sender | `sender.require_auth()` via `require_stream_sender` | lib.rs:906 | streaming.md §4 |
| `resume_stream` | Sender | `sender.require_auth()` via `require_stream_sender` | lib.rs:937 | streaming.md §4 |
| `cancel_stream` | Sender | `sender.require_auth()` via `require_stream_sender` | lib.rs:987 | streaming.md §4 |
| `withdraw` | Recipient | `recipient.require_auth()` | lib.rs:1033 | streaming.md §4 |
| `withdraw_to` | Recipient | `recipient.require_auth()` | lib.rs:1127 | streamingth Mechanism | Code Location | Doc Reference |
|-----------|----------------|----------------|---------------|---------------|
| `init` | Bootstrap admin | `admin.require_auth()` | lib.rs:665 | streaming.md §4 |
| `create_stream` | Sender | `sender.require_auth()` | lib.rs:754 | streaming.md §4 |
| `create_streams` | Sender | `sender.require_auth()` | lib.rs:827 | streaming.md §4ately with rationale).

## Verification Status

✅ **Complete alignment verified** between `docs/streaming.md` narrative and contract implementation as of 2026-03-27.

## Authorization Boundaries

### Explicit Role Mapping

| Operation | Authorized Role | Aub.rs`, `contracts/stream/src/accrual.rs`). It ensures treasury operators, recipient applications, and auditors can reason about contract behavior using only on-chain observables and published documentation—without inferring hidden rules from implementation details.

## Scope

Everything materially related to protocol semantics, authorization boundaries, state transitions, event emissions, and error classifications. Intentionally excluded: gas costs, TTL behavior, network-specific deployment concerns (documented separis document provides externally visible assurances for the Fluxora streaming contract by mapping protocol documentation (`docs/streaming.md`) to implementation (`contracts/stream/src/li# Protocol Narrative vs Code Alignment

## Purpose

Th