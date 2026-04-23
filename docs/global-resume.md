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

Requires authorization from the contract admin (set during `init`).

### State changes

- Clears `DataKey::GlobalEmergencyPaused` (sets it to `false`).
- All user-facing mutations blocked by the emergency pause are immediately re-enabled:
  `create_stream`, `create_streams`, `withdraw`, `withdraw_to`, `batch_withdraw`,
  `cancel_stream`, `update_rate_per_second`, `shorten_stream_end_time`,
  `extend_stream_end_time`.

### Errors

| Error                         | Condition                                                                          |
| ----------------------------- | ---------------------------------------------------------------------------------- |
| `ContractError::InvalidState` | Contract is **not** currently in emergency pause. Prevents spurious resume events. |
| Auth failure (panic)          | Caller is not the contract admin.                                                  |

### Event emitted

Topic: `gl_resume`  
Data: `GlobalResumed { resumed_at: u64 }` — ledger timestamp at which the pause was cleared.

## Expected timeline

```
T+0   Incident detected
T+1   Admin calls set_global_emergency_paused(true)
        → gl_pause event emitted, user mutations blocked
T+?   Root cause identified and mitigated
T+N   Admin calls global_resume()
        → gl_resume event emitted, normal operations restored
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
   affected during the emergency pause period.
5. **Communicate** — notify protocol users and downstream integrators that normal operations
   have resumed, referencing the `gl_resume` transaction hash.

## Security notes

- Only the admin can call `global_resume`. There is no time-lock or multi-sig requirement
  at the contract level; those controls belong at the key-management layer.
- Calling `global_resume` when the contract is not paused returns `InvalidState` and emits
  no event, preventing spurious entries in audit logs.
- Admin override entrypoints (`*_as_admin`) and read-only views are never blocked by the
  emergency pause and remain available throughout an incident.
