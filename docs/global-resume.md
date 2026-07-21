# Global Resume Operation

## Overview

`global_resume` is the dedicated admin entrypoint for clearing the global emergency pause
and restoring normal contract behaviour after an incident. It is the explicit, unambiguous
counterpart to `set_global_emergency_paused(true)`.

## Why a dedicated function?

`set_global_emergency_paused(false)` already clears the flag, but it emits a generic
`GlobalEmergencyPauseChanged { paused: false }` event that is indistinguishable from a
routine toggle. `global_resume` emits a distinct `GlobalResumed { resumed_at }` event so
that incident-response tooling, indexers, and audit logs can unambiguously identify a
deliberate post-incident resume.

## Function signature

```rust
pub fn global_resume(env: Env) -> Result<(), ContractError>
```

### Authorization

Requires authorization from the contract admin (set during `init`). When the stream
admin is the governance contract, clear the flag via a proposal whose calldata is
`CallData::StreamGlobalResume`.

### State changes

- Clears `DataKey::GlobalEmergencyPaused` (sets it to `false`).
- All user-facing mutations blocked by the emergency pause are immediately re-enabled:
  `create_stream`, `create_streams`, `withdraw`, `withdraw_to`, `batch_withdraw`,
  `cancel_stream`, `update_rate_per_second`, `shorten_stream_end_time`,
  `extend_stream_end_time`.
- **Does not** change per-stream status. Streams that were individually paused or
  cancelled during the incident remain in that state until resumed or left terminal.

### Errors

| Error                         | Condition                                                                          |
| ----------------------------- | ---------------------------------------------------------------------------------- |
| `ContractError::InvalidState` | Contract is **not** currently in emergency pause. Prevents spurious resume events. |
| Auth failure (panic)          | Caller is not the contract admin.                                                  |

### Event emitted

Topic: `gl_resume`  
Data: `GlobalResumed { resumed_at: u64 }` — ledger timestamp at which the pause was cleared.

## Per-stream recovery after global resume

Clearing the emergency flag does **not** bulk-resume individual streams. Operators (or
governance) restore paused streams with:

- `resume_stream` / `resume_stream_as_admin` (single stream), or
- `bulk_resume_streams_as_admin(stream_ids)` (batch), also exposed to governance as
  `CallData::StreamBulkResumeAsAdmin(stream_ids)`.

### Mixed-batch / partial-failure semantics — atomic all-or-nothing

`bulk_resume_streams_as_admin` uses **atomic all-or-nothing** semantics (there is **no**
skip-and-report mode):

1. **Validate all** stream IDs first (existence, `Paused` status, cooldown, no duplicates).
2. **Only then** mutate status to `Active` and emit `Resumed` events.

If any target cannot be resumed — for example an already **Cancelled** or **Completed**
stream (`ContractError::StreamTerminalState`), an `Active` stream
(`ContractError::StreamNotPaused`), a missing ID, a duplicate ID, or an active pause
cooldown — the **entire batch fails**, no stream is resumed, and no `Resumed` events are
emitted.

When the batch is dispatched through governance `execute`, a failed target call reverts
the whole transaction (including the proposal's `executed = true` write), so the system
never records a successful execution for a partial resume.

### Governance authorization on the partial-failure path

Admin / governance authorization is checked **before** batch validation. A malformed
batch (e.g. mixing resumable paused streams with a cancelled stream) **cannot** bypass
quorum, timelock, or admin auth. Pre-quorum `execute` still returns
`GovernanceError::QuorumNotReached` and leaves all stream statuses unchanged.

Covered by `contracts/stream/tests/governance_executor_e2e.rs`:

- `test_e2e_global_resume_mixed_batch_partial_failure_is_atomic`
- `test_e2e_bulk_resume_partial_failure_still_requires_governance_auth`
- `test_e2e_global_resume_then_bulk_resume_all_paused_succeeds`
- `test_bulk_resume_as_admin_mixed_batch_atomic_no_partial`

## Expected timeline

```
T+0   Incident detected
T+1   Admin calls set_global_emergency_paused(true)
        → gl_pause event emitted, user mutations blocked
T+?   Root cause identified and mitigated
T+N   Admin (or governance) calls global_resume()
        → gl_resume event emitted, normal operations restored
T+N+… Optionally bulk_resume_streams_as_admin([...]) for streams paused in the window
        → atomic: all resume or none
```

## Post-incident checklist

After calling `global_resume`, operators should complete the following steps before
declaring the incident resolved:

1. **Verify flag cleared** — call `get_global_emergency_paused()` and confirm it returns `false`.
2. **Confirm event** — check the transaction record for the `gl_resume` event with the
   expected `resumed_at` timestamp.
3. **Smoke test** — run a small end-to-end transaction (e.g. a minimal `create_stream`)
   to confirm normal operation is fully restored.
4. **Review incident window** — audit any streams that were paused, cancelled, or otherwise
   affected during the emergency pause period. Resume only streams that are still
   `Paused`; exclude cancelled/completed IDs from any `bulk_resume_streams_as_admin`
   batch (a single terminal ID fails the whole batch).
5. **Communicate** — notify protocol users and downstream integrators that normal operations
   have resumed, referencing the `gl_resume` transaction hash.

## Security notes

- Only the admin can call `global_resume`. There is no time-lock or multi-sig requirement
  at the contract level; those controls belong at the key-management layer (or at
  governance when the stream admin is the governance contract).
- Calling `global_resume` when the contract is not paused returns `InvalidState` and emits
  no event, preventing spurious entries in audit logs.
- Admin override entrypoints (`*_as_admin`) and read-only views are never blocked by the
  emergency pause and remain available throughout an incident.
- Ambiguous partial-failure semantics on a governance-triggered batch resume would leave
  the system in an unexpected state after an incident-response attempt. The implemented
  contract behaviour is therefore **atomic all-or-nothing**, never silent partial success.
