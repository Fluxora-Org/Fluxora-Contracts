# Delegate revocation: same-ledger ordering guarantee

`revoke_delegate` removes a delegate grant from persistent storage. This note
states exactly when that removal takes effect, and what a recipient transfer
does to grants that have not been revoked, so integrators can rely on both
without reading the implementation.

## Permission model

Each delegation bit is an independent authorization. No bit implies another:

| Bit | Grantor | Permits | Implies another bit? |
|---|---|---|---|
| `WITHDRAW` | recipient | `delegate_withdraw` | No |
| `CANCEL` | sender | `delegate_cancel` | No |
| `PAUSE` | sender | `delegate_pause` | No |
| `RESUME` | sender | `delegate_resume` | No |
| `TOP_UP` | sender | `delegate_top_up` | No |
| `TRANSFER_RECIPIENT` | recipient | `delegate_transfer_recipient` | No |

The model is intentionally orthogonal: a grant for `PAUSE` does not authorize
`RESUME`, a grant for `WITHDRAW` does not authorize `TRANSFER_RECIPIENT`, and a
sender cannot delegate a recipient-owned permission or vice versa. A grant can
only be issued by the party that owns that operation domain, and the same
`grant_delegate` call must not mix sender-side and recipient-side permissions.

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

## Recipient transfer: grants survive

`transfer_recipient` and `delegate_transfer_recipient` reassign who is paid.
They do **not** touch `Delegate(stream_id, delegate)` entries: **a delegate
grant survives a recipient transfer unchanged.**

| Grant | Who issued it | After a recipient transfer |
|---|---|---|
| `CANCEL`, `PAUSE`, `RESUME`, `TOP_UP` | Sender | Unchanged — the sender is still the sender. |
| `WITHDRAW`, `TRANSFER_RECIPIENT` | Recipient | Still live, and now controlled by the **new** recipient. |

The rule follows from where authority already lives. `grant_delegate` and
`revoke_delegate` resolve their recipient half against the stream's *current*
`recipient`, so authority over recipient-issued grants moves with the slot:

- The **new recipient** can revoke a surviving grant immediately:
  `revoke_delegate(stream_id, new_recipient, delegate)` succeeds, and the next
  delegate call is rejected with `Error::DelegateNotPermitted`.
- The **old recipient** is no longer a party to the stream, so
  `grant_delegate` and `revoke_delegate` both reject them with
  `Error::Unauthorized`. They cannot mint new grants for their delegate either.
- The **sender** is unaffected: it can still grant and revoke every sender-side
  op, before or after the transfer.

Because revocation is ordered rather than retroactive (the guarantee above), a
recipient who inherits somebody else's delegate should revoke it before that
delegate acts. Anything the delegate did before the revocation stands, exactly
as for any other revocation.

Clearing grants on transfer was the alternative rule. It was rejected on two
counts: a grant is not exclusively the recipient's instrument — the sender's
`CANCEL` / `PAUSE` / `RESUME` / `TOP_UP` grants have nothing to do with who is
paid — and grants are stored per `(stream, delegate)` with no index a transfer
could sweep, so selective clearing would need a schema change and a migration.
The stated rule needs neither, and it is the behaviour the contract already
exhibits.

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

The same module's section *Recipient transfer* pins the transfer rule above,
again by looping over `ALL_OPS`:

- `delegate_grants_survive_a_recipient_transfer_for_every_permission_bit` —
  grant, transfer the recipient, then invoke the delegate entry point for the
  bit. The call is authorised for all six bits, which can only happen if the
  grant survived.
- `the_new_recipient_can_revoke_a_grant_that_survived_the_transfer` — the new
  recipient revokes a recipient-issued grant; the next delegate call is
  rejected with `DelegateNotPermitted` and the stream is unchanged.
- `the_old_recipient_cannot_revoke_after_a_transfer` — the previous recipient
  is rejected with `Error::Unauthorized`, and the delegate remains authorised,
  so the rejection did not silently clear anything.
