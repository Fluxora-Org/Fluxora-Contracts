# Delegate revocation: same-ledger ordering guarantee

`revoke_delegate` removes a delegate grant from persistent storage. This note
states exactly when that removal takes effect, so integrators can rely on it
without reading the implementation.

## The guarantee

**Revocation is ordered, not retroactive.**

| Position of the delegate call relative to the revocation | Outcome |
|---|---|
| Ordered **before** the revocation | Honoured. The call runs with the grant live, and the revocation does not unwind it. |
| Ordered **after** the revocation | Rejected with `Error::DelegateNotPermitted`. |

The rule is identical whether the two calls land in the same ledger or in
different ledgers. There is no grace period, no "end of ledger" flush, and no
difference in behaviour between the two cases: `revoke_delegate` deletes the
grant with a single storage write, and every invocation that executes after
that write observes no grant.

## Why the ordering caveat matters

Stellar applies the transactions in a ledger in a deterministic order chosen by
the network — the contract does not select it, and a contract invocation cannot
see other transactions in the same ledger. So the contract cannot promise "the
revocation wins regardless of order". It promises the only thing it can
enforce:

- Once the revocation has executed, **no call ordered after it can use the old
  grant**, even if that call arrives in the same ledger.
- A call ordered before the revocation is not undone. A grant is a permission
  to act, not a claim that can be clawed back; funds moved by a call that
  legitimately preceded the revocation stay moved, exactly as they do for a
  withdrawal made in an earlier ledger.

This mirrors the existing rule that revocation "does not touch already-moved
funds": revocation stops *future* invocations, it does not reverse *past* ones.

## Grant, revoke, and the `Pending` window

There is no intermediate state. `grant_delegate` writes the grant, and
`revoke_delegate` deletes it; a delegate call either finds the entry or it does
not. `check_delegate` runs as the first step of every `delegate_*` entry point,
before any state is read or mutated, so a call rejected for a revoked grant
leaves the stream byte-for-byte unchanged.

`revoke_delegate` is idempotent: revoking an absent or already-revoked grant
succeeds without error. `grant_delegate` on a `(stream, delegate)` pair replaces
any existing grant for that pair, so a re-grant after a revocation restores
access from that point forward.

## Verification

`contracts/stream/src/test/delegation.rs`, section *Same-ledger revocation
ordering*, asserts both directions for **every permission bit**
(`WITHDRAW`, `CANCEL`, `PAUSE`, `RESUME`, `TOP_UP`,
`TRANSFER_RECIPIENT`):

- `revoked_delegate_cannot_act_later_in_the_same_ledger` — grant, revoke, then
  delegate call, with no ledger advance. The call is rejected with
  `DelegateNotPermitted` for each op, and the stream is unchanged.
- `delegate_call_ordered_before_revocation_in_the_same_ledger_is_honoured` —
  grant, delegate call, then revoke, all in one ledger. The call succeeds; the
  next call after the revocation is rejected for each op.
- `all_ops_fixture_covers_every_permission_bit` — guards the fixture itself, so
  a new op bit cannot be added without being threaded through the tests above.

No test advances the ledger clock between the calls, which is what pins the
"same ledger" case rather than merely the cross-ledger one.
