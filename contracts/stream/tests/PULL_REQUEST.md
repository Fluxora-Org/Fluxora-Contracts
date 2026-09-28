# fix: event ordering regression — no success event on reverted token transfer

## Issue scope

Lifecycle helpers (`withdraw`, `cancel_stream`, `top_up_stream`) emit success
events **after** the cross-contract token call.  The issue asks us to:

1. Define whether failure events exist.
2. Prove consumers can never observe a success event for a reverted operation.
3. Cover this with focused regression tests using a failing token mock.

## Design decision

**No failure event is emitted.  The absence of the success event is the signal.**

Soroban transactions are atomic: if `push_token` or `pull_token` panics, the
host discards the entire invocation frame — storage writes and published events
together.  A consumer that does not see `"withdrew"` / `"cancelled"` / `"top_up"`
knows the operation did not complete.  No extra machinery is required.

## Before

No regression test existed to prove the above guarantee.  Any indexer or
integration author had to take atomicity on faith.

## After

`contracts/stream/tests/event_ordering_failed_transfer.rs` adds four focused
tests using two token mocks:

| Mock | Behaviour |
|---|---|
| `PanicToken` | All `transfer`/`transfer_from` calls panic (used for `create_stream` test) |
| `OnceToken`  | Allows exactly one `transfer_from` (stream creation deposit), panics on every subsequent call |

| Test | Operation | Token call fails | Assertion |
|---|---|---|---|
| `failed_create_emits_no_created_event`   | `create_stream`  | `pull_token`  | no `"created"` topic; stream counter unchanged |
| `failed_withdraw_emits_no_withdrew_event`  | `withdraw`       | `push_token`  | no `"withdrew"` topic; event log length unchanged |
| `failed_cancel_emits_no_cancelled_event`   | `cancel_stream`  | `push_token`  | no `"cancelled"` topic; event log length unchanged |
| `failed_top_up_emits_no_top_up_event`      | `top_up_stream`  | `pull_token`  | no `"top_up"` topic; event log length unchanged |

## Verification

Run on CI (Linux, Rust 1.94.1):

```
cargo test -p fluxora_stream --test event_ordering_failed_transfer -- --nocapture
```

Expected output:
```
running 4 tests
test failed_cancel_emits_no_cancelled_event ... ok
test failed_create_emits_no_created_event ... ok
test failed_top_up_emits_no_top_up_event ... ok
test failed_withdraw_emits_no_withdrew_event ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured
```

No existing tests modified.  No dependencies added.  No behaviour changed
outside this scope.
