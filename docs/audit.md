# Audit Entrypoint Table

This document tracks every public ABI entrypoint in `contracts/stream/src/lib.rs`.
The CI "Audit entrypoint drift check" step verifies this table against the source.

Last verified: 2026-08-29 (PR #1665)

## Stream Contract — `fluxora_stream`

### Lifecycle

| Entrypoint | Description |
|---|---|
| `create_stream` | Create a new payment stream with deposit, schedule, and capability flags |
| `batch_create` | Atomically create multiple payment streams in one transaction |
| `create_stream` | Create a new payment stream with deposit, schedule, and capability flags (`cliff_mode` is `Schedule`) |
| `create_stream_with_curve` | Create a stream with an explicit linear, step, or front-loaded release curve |
| `create_stream_with_cliff_mode` | Same as `create_stream`, plus an explicit `cliff_mode` choosing whether the cliff gate is read on the stream clock or the wall clock |
| `top_up` | Extend stream duration at a fixed rate (sender auth) |
| `withdraw` | Pull accrued balance; `None` = withdraw max |
| `batch_withdraw` | Atomic multi-stream withdrawal |
| `cancel` | Cancel stream, refund unvested to sender (sender auth, `cancellable`) |
| `batch_cancel` | Atomic multi-stream cancellation; a non-cancellable member refuses the batch and is reported by its index in the submitted vector |
| `pause` | Freeze accrual (sender auth, `pausable`) |
| `resume` | Unfreeze accrual (sender auth, `pausable`) |
| `transfer_recipient` | Change stream recipient (recipient auth, `transferable`) |

### Delegation

| Entrypoint | Description |
|---|---|
| `grant_delegate` | Grant per-operation delegation to a third party |
| `revoke_delegate` | Revoke previously granted delegation |
| `delegate_withdraw` | Withdraw on behalf of recipient via delegation |
| `delegate_cancel` | Cancel on behalf of sender via delegation |
| `delegate_pause` | Pause on behalf of sender via delegation |
| `delegate_resume` | Resume on behalf of sender via delegation |
| `delegate_top_up` | Top up on behalf of sender via delegation |
| `delegate_transfer_recipient` | Transfer recipient on behalf of recipient via delegation |

### Views (read-only)

| Entrypoint | Description |
|---|---|
| `get_stream` | Return full stream struct |
| `withdrawable_of` | Return withdrawable amount |
| `vested_of` | Return vested amount |
| `refundable_of` | Return refundable amount |
| `stream_count` | Return total stream count |
| `stream_exists` | Check if a stream ID exists |
| `upgradeable` | Report whether the contract can be replaced in place (always `false`) |
| `halted` | Whether the contract-level emergency halt is engaged |
| `halt_operator` | The installed halt operator, or `None` when the contract is not haltable |

### Maintenance (permissionless)

| Entrypoint | Description |
|---|---|
| `extend_stream_ttl` | Extend a single stream's storage TTL |
| `batch_extend_ttl` | Extend multiple streams' storage TTLs |

### Emergency halt (#1818)

| Entrypoint | Description |
|---|---|
| `set_halt_operator` | Install the one-shot halt operator (named operator auth; no rotation) |
| `halt` | Refuse every state-changing entry point contract-wide (operator auth) |
| `resume_contract` | Lift the halt and restore settlement (operator auth) |
