# Same-ledger ordering model

Two of the contract's entry points can land in the same ledger. This note states
once, in one place, what the contract guarantees when that happens, lists every
entry-point pair whose result depends on the order the network applies them in,
and names the test that pins each direction.

If you are integrating and about to issue two calls back to back, this is the
document to read: the second call may be ordered before the first, and the
outcome is still well-defined.

## The model

A ledger is a batch of transactions applied in a deterministic order chosen by
the network, not by the contract. The host executes each invocation against the
state left by the invocations ordered before it, so:

- A write committed by an invocation **is visible to every invocation ordered
  after it** — in the same ledger or any later one.
- An invocation **cannot observe, and does not affect**, another invocation
  ordered after it.
- Nothing is buffered, coalesced, or deferred to an "end of ledger" flush. A
  storage write takes effect at the point the invocation returns successfully.

The resulting guarantee is **ordered, not retroactive**:

| The second call is ordered… | Outcome |
|---|---|
| **after** the first | It observes the first call's write. If the first invalidated an authorization or precondition the second needs, the second is rejected. |
| **before** the first | It runs against the state as it was before the first call's write. It is not unwound by the first call, and the first call does not apply retroactively. |

The contract cannot pick the order and cannot see the future, so it cannot
promise "the first operation always wins". It promises the only thing it can
enforce: **whichever side of the pair the network applies second sees the
first's effect.**

The rule is identical across ledgers. The same-ledger case is the interesting
one only because no ledger boundary hides it, so the tests pin it by never
advancing the ledger clock between the pair. (This is the contract-side
counterpart of `docs/soroban-rpc-read-skew.md`, which covers the client-side
read-after-write barrier; this note is about ordering *execution*, not reads.)

## Ordering-sensitive entry points

A pair is ordering-sensitive when one entry point changes an authorization, a
precondition, or a terminal state that the other consumes. Four pairs meet that
test.

### 1. Factory — `set_admin` and any admin-gated setter

`FluxoraFactory::require_admin` reads `DataKey::Admin` from instance storage on
every call and immediately calls `require_auth()` on the freshly-read address.
There is no cached admin, so the setter that runs after `set_admin` is checked
against the new admin, and the one that runs before it against the old admin.
The gated setters are `set_cap`, `set_min_duration`, `set_stream_contract`,
`set_allowlist` and `set_batch_cap_enforcement`.

| Ordering within the ledger | Outcome |
|---|---|
| Setter → `set_admin` | The setter is authorised by the **old** admin and takes effect; the later rotation does not unwind it. |
| `set_admin` → setter | The setter is authorised by the **new** admin. The old admin's call is rejected; the new admin's call succeeds. |

### 2. Stream — `revoke_delegate` and any `delegate_*` call

`revoke_delegate` deletes the `Delegate(stream_id, delegate)` entry with a
single storage write. Every `delegate_*` entry point runs `check_delegate` as
its first step, before any state is read or mutated, so a rejected call leaves
the stream byte-for-byte unchanged. Full detail, including the `Pending` window
and recipient-transfer interaction, is in `docs/delegation-revocation.md`.

| Ordering within the ledger | Outcome |
|---|---|
| Delegate call → `revoke_delegate` | Honoured — the grant was live when the call ran; the later revocation does not unwind it. Funds moved stay moved. |
| `revoke_delegate` → delegate call | Rejected with `Error::DelegateNotPermitted`; the stream is unchanged. |

### 3. Stream — `grant_delegate` and any `delegate_*` call

`grant_delegate` writes the `Delegate(stream_id, delegate)` entry. The same
`check_delegate` first step that makes revocation immediate makes a grant
immediate in the other direction.

| Ordering within the ledger | Outcome |
|---|---|
| Delegate call → `grant_delegate` | Rejected with `Error::DelegateNotPermitted`; the grant is not retroactive and does not authorise the earlier call. |
| `grant_delegate` → delegate call | Honoured — the entry is present when `check_delegate` runs. |

### 4. Stream lifecycle — `withdraw` and `cancel`

`cancel` settles the stream: it pays out nothing by itself, sets `deposited` to
the vested amount and marks the stream `Cancelled`. `withdraw` pays the
currently vested-and-unwithdrawn amount and moves the stream to `Cancelled` or
`Depleted` when nothing remains. Either order conserves the deposit exactly
between recipient and sender; the difference is only *when* each amount moves.

| Ordering within the ledger | Outcome |
|---|---|
| `withdraw` → `cancel` | If something has vested, the recipient is paid it and `cancel` then refunds the remainder to the sender. If nothing has vested the stream is still live, so the `withdraw` is rejected with `Error::NothingToWithdraw` and `cancel` refunds the whole deposit. Terminal status is `Cancelled` (or `Depleted` if fully vested). |
| `cancel` → `withdraw` | `cancel` settles first and marks the stream `Cancelled`. The `withdraw` that follows pays the accrued amount. If **nothing** accrued, it is rejected with `Error::StreamTerminated` — the stream is already terminal when the withdraw runs, and a terminal stream with zero available is `StreamTerminated`, not the live-stream `NothingToWithdraw`. The rejection is a pure precondition failure and leaves the settled stream unchanged. |

Other pairs (for example `pause`/`resume`) are order-insensitive for
authorization purposes and are not listed: whichever runs first, the guard is
re-evaluated from storage on the next call, and neither grants nor revokes an
authorization.

## Verification

Each direction above is asserted by a test. No test advances the ledger clock
between the two calls, which is what pins the *same-ledger* case rather than
merely the cross-ledger one.

| Pair | Ordering | Test |
|---|---|---|
| `set_admin` × setter | Setter → `set_admin` | `test_set_admin_same_ledger_setter_before_rotation_is_honoured` |
| `set_admin` × setter | `set_admin` → setter (old admin) | `test_set_admin_same_ledger_old_admin_fails` |
| `set_admin` × setter | `set_admin` → setter (new admin) | `test_set_admin_same_ledger_new_admin_succeeds` |
| `set_admin` × setter | `set_admin` → several setters | `test_set_admin_same_ledger_multiple_setters` |
| `revoke_delegate` × `delegate_*` | Delegate call → revoke | `delegate_call_ordered_before_revocation_in_the_same_ledger_is_honoured` |
| `revoke_delegate` × `delegate_*` | Revoke → delegate call | `revoked_delegate_cannot_act_later_in_the_same_ledger` |
| `grant_delegate` × `delegate_*` | Delegate call → grant | `delegate_call_ordered_before_a_same_ledger_grant_is_rejected` |
| `grant_delegate` × `delegate_*` | Grant → delegate call | `delegate_call_ordered_before_a_same_ledger_grant_is_rejected` (second half) |
| `withdraw` × `cancel` | Withdraw → cancel | `withdraw_then_cancel_schedule_points` |
| `withdraw` × `cancel` | Cancel → withdraw | `cancel_then_withdraw_schedule_points` |

The delegation pairs loop over `ALL_OPS`, so every permission bit (`WITHDRAW`,
`CANCEL`, `PAUSE`, `RESUME`, `TOP_UP`, `TRANSFER_RECIPIENT`) is covered, and
`all_ops_fixture_covers_every_permission_bit` guards the fixture so a new bit
cannot be added without being threaded through.

Run them from the workspace root:

```bash
cargo test --package fluxora_factory test_set_admin_same_ledger
cargo test --package fluxora-stream same_ledger
cargo test --package fluxora-stream withdraw_cancel_same_ledger
```

## What is *not* guaranteed

- **A particular order.** The contract does not choose it and an invocation
  cannot see sibling transactions in the ledger.
- **Last-write-wins across the pair.** Each call is checked against storage as
  it stands when it runs, so an earlier call's effect is never overwritten by a
  later authorization decision made against stale state.
- **Retroactivity.** No entry point reaches back to undo or re-authorise a call
  that already ran.
