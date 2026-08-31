# Fix cancellation after a pause so every balance component settles correctly

Closes #1554

## Summary

This change hardens the paused-then-cancel settlement path so cancellation freezes at the paused stream clock, clears the pause metadata, and preserves the exact sender/recipient balance split across the full lifecycle.

The root issue was that a cancel performed while a stream was paused could settle against wall-clock time instead of the stream clock, while also leaving the paused state inconsistent with a terminal cancelled stream. That meant the `refund + vested` conservation identity and the pool invariant could drift away from the actual available balance components.

## What changed

- Cancellation now settles against the frozen stream time while paused, exactly as the accrued value says it should.
- The cancel path clears `paused_at` and leaves the stream in a terminal `Cancelled` state without allowing an accidental paused state to linger.
- The refund/vested split is enforced against the pre-cancel deposit so the contract cannot over-refund or under-settle the sender/recipient balance.
- The pause/cancel sequence is covered with focused regression tests from before-pause, during-pause, after-resume, and repeated-cancel scenarios.

## Design decision

The selected behavior is:

- Cancel while paused settles at the frozen stream clock, not real elapsed wall time.
- The stream is cancelled atomically and the pause state is cleared before the transfer is finalized.
- `refunded + vested == deposited_before_cancel` remains exact, and the recipient keeps only the vested-but-unwithdrawn remainder.
- Repeated cancellation remains rejected with `Error::StreamTerminated` and leaves state unchanged.

This keeps the change narrowly scoped to the accounting semantics without altering unrelated lifecycle behavior.

## Regression coverage

Focused tests cover:

- Cancel before pause
- Cancel while paused
- Cancel after resume
- Cancel after partial withdrawal
- Repeated cancel rejection
- Exact sender and recipient balance reconciliation
- Event payload verification for the settlement figures
- Pause metadata clearing on terminal cancellation

Key checks include:

- `cancel_while_paused_settles_at_the_frozen_clock`
- `state_machine_cancel_after_pause_clears_pause_state`
- `cancel_while_paused_publishes_the_frozen_figures`
- `split_*` balance invariant tests in `contracts/stream/src/test/cancel.rs`

## Why this is the correct fix

Cancellation is terminal and rewrites the schedule so the stream behaves like a fully matured stream from that instant onward. The implementation enforces the invariant:

- `refund + vested == deposited_before_cancel`
- `vested >= withdrawn`
- `withdrawable == vested - withdrawn`
- contract pool balance matches the unclaimed remainder exactly

That ensures the sender receives exactly the unvested remainder, the recipient holds exactly the vested amount still claimable, and no funds remain stranded or double-counted.

## Verification

I verified this with the required focused suite:

```bash
source $HOME/.cargo/env && cd /workspaces/Fluxora-Contracts && cargo test -p fluxora-stream cancel -- --nocapture
```

Evidence from the fresh run:

- `running 69 tests`
- `test result: ok. 69 passed; 0 failed`
- additional event-ordering regression check also passed: `1 passed; 0 failed`

## CI / performance impact

- Runtime impact: none beyond the normal atomic cancel settlement logic already in the stream contract.
- Resource impact: no contract binary or storage schema changes beyond the existing tested accounting semantics.
- CI evidence: the focused cancellation and related regression tests completed successfully in the current environment.

## Out of scope

- unrelated refactors
- dependency churn
- documentation-only cleanup outside this fix
- weakening or skipping regression coverage

## Notes

This patch is intentionally narrow: it preserves the existing stream model outside this pause/cancel settlement path while fixing the bug in the exact transition that was inconsistent. The balance split is now explicit, stable, and regression-tested across the relevant lifecycle states.
