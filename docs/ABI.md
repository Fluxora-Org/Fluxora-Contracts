# Fluxora ABI — interface of record

**Status: FROZEN as of 2026-08-12, ahead of stage 5.**

This document is the interface contract between `Fluxora-Contracts` and every
consumer — `Fluxora-Backend`, `Fluxora-Frontend`, `fluxora-sdk`, and third-party
integrators. Anything not described here is not part of the interface.

| | |
|---|---|
| Protocol | 27 |
| SDK | `soroban-sdk` 27.0.5 |
| Testnet contract | `CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXW` |
| Wasm hash | `d47c96a344a79c614ab0dcf0eac62cc9384f6dc7f1d45c3f5109fb09658b035e` |
| Interface spec sha256 | `acdfd259c7f9a854d42c5da4cda43138fb71b757b604a21c6dac8a8a5a3a86d1` |

The deployed contract's interface has been verified byte-identical to the local
build:

```bash
stellar contract info interface --wasm target/wasm32v1-none/release/fluxora_stream.wasm
stellar contract info interface --id CBCGTSCJ… --network testnet
```

## What "frozen" means

The core contract is **immutable** — no admin key, no upgrade path (see the
non-goals). The interface therefore cannot change on the deployed contract at
all; a change means a *new deployment at a new address*.

So the freeze is a commitment about how we manage that:

* **Compatible** (no version bump): adding a field to the *end* of an event
  payload; adding a new error discriminant; adding a new entry point in a future
  deployment.
* **Breaking** (new address, new major version, migration note): renaming or
  removing an entry point; changing any parameter's type, order or count;
  renaming a struct field; reordering event topics; renumbering an existing
  error discriminant; changing the meaning of a field.

Consumers should pin the wasm hash above and treat a change in it as requiring
a review of this document.

**Generate, do not hand-write.** Event and function schemas are embedded in the
deployed contract via `#[contractevent]` and `#[contractimpl]`. The SDK and
indexer must codegen from `stellar contract info interface`, not from
hand-rolled topic parsers. The previous frontend's hand-written
`nativeToScVal` encoding is exactly the thing that broke; see
[MIGRATION.md](MIGRATION.md).

The reviewable inventory of every public method, return type, error
discriminant and event is generated from that same spec XDR and committed at
[`contracts/stream/abi/fluxora_stream.json`](../contracts/stream/abi/fluxora_stream.json).
`test::abi` fails the suite if a method is removed, renamed, or type-changed
without bumping [`ABI_VERSION`](../contracts/stream/src/lib.rs). Additive
changes update the snapshot only.

---

## Types

### `Stream`

```rust
struct Stream {
    sender: Address,
    recipient: Address,
    token: Address,        // SEP-41. Per-stream, NOT a contract-wide setting.
    deposited: i128,       // total ever deposited, including top-ups
    withdrawn: i128,       // total ever withdrawn by the recipient
    start_time: u64,       // unix seconds
    end_time: u64,         // unix seconds
    cliff_time: u64,       // in [start_time, end_time]; == start_time for none
    cancellable: bool,     // immutable after creation
    pausable: bool,        // immutable after creation
    transferable: bool,    // immutable after creation
    paused_at: Option<u64>,
    paused_total: u64,     // cumulative paused seconds, excluding any in-progress pause
    status: StreamStatus,
}
```

All amounts are `i128` in the token's smallest unit. **USDC on Stellar has 7
decimals** — not 6, not 18. Amounts cross the JSON-RPC boundary as *strings*
(`"600000000"`), not numbers; a client that parses them as IEEE doubles will
silently lose precision above 2^53.

### `StreamStatus`

Crosses the ABI as its **discriminant**, not its name.

| value | name | terminal | meaning |
|---|---|---|---|
| `0` | `Active` | no | accruing |
| `1` | `Paused` | no | accrual clock frozen; withdrawal still permitted |
| `2` | `Cancelled` | yes | sender clawed back the unvested remainder |
| `3` | `Depleted` | yes | ran to term and was fully withdrawn |

`Cancelled` is **sticky**: a cancelled stream later drained to zero stays
`Cancelled`. It never becomes `Depleted`. This distinction is deliberate and
load-bearing for reporting — see the resolved schema question below.

### Capability flags

Three `bool` fields on `Stream` describe operations that are **not available**
for a given stream. They are supplied by the sender to `create_stream` and are
**fixed for the lifetime of the stream** — no entry point can change them after
creation. `get_stream` returns the live `Stream` struct, which includes all
three fields.

| field | error when `false` | discriminant |
|---|---|---|
| `cancellable` | `NotCancellable` | 8 |
| `pausable` | `NotPausable` | 9 |
| `transferable` | `NotTransferable` | 10 |

**Creation-time semantics.** Flags are part of the initial `Stream` value
written to persistent storage inside `create_stream`. They are never touched
by any subsequent entry point.

**Immutability is structural.** The `Stream` struct exposes no setter for these
fields, and no entry point in the contract's public API assigns to them after
creation. Every mutating operation (`top_up`, `withdraw`, `pause`, `resume`,
`cancel`, `transfer_recipient`, `extend_stream_ttl`, and their delegate
variants) leaves all three flags unchanged.

**Operation enforcement.** When a flag is `false`, the corresponding operation
is rejected before any state change occurs:

* `cancel` and `delegate_cancel` return `NotCancellable` (8).
* `pause` and `delegate_pause` return `NotPausable` (9).
* `transfer_recipient` and `delegate_transfer_recipient` return
  `NotTransferable` (10).

**Trust model.** A recipient can call `get_stream` before accepting a stream
and verify that `cancellable == false`, `pausable == false`, and
`transferable == false`. Because those values cannot change after creation, the
verification is permanent: the sender cannot later claw back, freeze, or
reassign the stream.

### `Error`

Discriminants are ABI and are never renumbered; new variants are appended.

| # | name | | # | name |
|---|---|---|---|---|
| 1 | `StreamNotFound` | | 17 | `NothingToWithdraw` |
| 2 | `InvalidTimeRange` | | 18 | `InvalidAmount` |
| 3 | `InvalidCliff` | | 19 | `BatchTooLarge` |
| 4 | `InvalidDeposit` | | 20 | `EmptyBatch` |
| 5 | `DepositRateTooLow` | | 21 | `DuplicateStreamId` |
| 6 | `SelfStream` | | 22 | `Overflow` |
| 7 | `Unauthorized` | | 23 | `TopUpTooSmall` |
| 8 | `NotCancellable` | | 24 | `StreamIdExhausted` |
| 9 | `NotPausable` | | 25 | `TokenTransferFailed` |
| 10 | `NotTransferable` | | 26 | `TokenMissing` |
| 11 | `StreamNotActive` | | 27 | `DelegateNotPermitted` |
| 12 | `StreamNotPaused` | | 28 | `DelegateExpired` |
| 13 | `StreamAlreadyPaused` | | 29 | `MalformedStreamId` |
| 14 | `StreamTerminated` | | 30 | `RepeatedTransfer` |
| 15 | `StreamMatured` | | 31 | `InvalidTopUp` |
| 16 | `InsufficientWithdrawable` | | 32 | `TokenAmountMismatch` |

`TokenTransferFailed` (25) and `TokenMissing` (26) are **stable stream-level categories** for token sub-invocation failures. The token contract's internal error discriminant is intentionally discarded — forwarding it would produce a value clients decode against Fluxora's error table, yielding a silent misinterpretation. The raw diagnostic is visible in the failed transaction's `diagnosticEvents`.

* `TokenTransferFailed` — the token contract returned a typed contract error: insufficient sender balance, pool underfunded on a payout, or the token's own authorization rules refused the call.
* `TokenMissing` — the token address resolves to nothing (Abort / host trap); the stream references a non-deployed contract.
* `TokenAmountMismatch` (32) — a deposit pull (`create_stream`, `top_up`, `delegate_top_up`) changed the pool's balance by something other than the requested amount. See "Token assumptions" below.

The CLI and RPC render these as `Error(Contract, #N)`.

`DepositRateTooLow` (5) enforces a minimum rate of 1 token unit per second (`deposit >= end_time - start_time`). Below one unit per second, the per-second rate truncates to zero and the recipient accrues literally nothing until very late in the schedule. This is rejected to prevent a footgun where a treasury streams a small grant over a long duration and the recipient cannot withdraw anything.

`StreamNotActive` (11) is reserved in the frozen ABI; current entry points
return the more specific pause/terminated variants instead.

`withdraw` distinguishes empty balances: a live stream with nothing accrued
yet returns `NothingToWithdraw` (17); a `Cancelled` or `Depleted` stream with
nothing left returns `StreamTerminated` (14). Clients must not treat those as
equivalent.

`NothingToWithdraw` (17) and `InsufficientWithdrawable` (16) are also not
interchangeable: on a live stream with zero withdrawable, `withdraw` returns
`NothingToWithdraw` regardless of whether `amount` is `None` or an explicit
value (the zero check runs first). `InsufficientWithdrawable` is returned only
when the withdrawable balance is positive and an explicit `amount` exceeds it;
requesting exactly the available balance succeeds.

---

## Token assumptions

`Stream.token` must be a [SEP-41](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)-conforming
contract. Fluxora is a primitive that trusts its token the way it trusts
nothing else on chain — `Stream.deposited`, and every rate, liability and
refund figure derived from it, is arithmetic over numbers the contract itself
chose, not numbers it re-derives from the token's ledger on every call. That
is fast and simple, and it is only correct if the token behaves as follows.

**1. A transfer moves exactly the amount requested. No fee-on-transfer, no
reflection.** The pool's real token balance is what actually backs every
recipient's claim; `deposited` assumes a `create_stream` or `top_up` pull
grows that balance by precisely the amount passed in.

> **Enforced.** `create_stream`, `top_up` and `delegate_top_up` read the
> contract's own balance immediately before and after the pull and require
> the delta to equal the requested amount exactly ([`pull_deposit`](../contracts/stream/src/lib.rs)).
> A mismatch — a fee-on-transfer token delivering less, or a
> positive-rebasing token delivering more — is rejected with
> [`Error::TokenAmountMismatch`](#error) (32), and the whole invocation,
> including the transfer that already happened, is rolled back by the host.
> No funds move and no phantom entry is written.
>
> This guards the *deposit* leg only. A fee taken on the *outbound* leg
> (`withdraw`'s payout, `cancel`'s refund) is not detected, and does not need
> to be: it is the recipient's or sender's own balance that comes up short,
> not the pool's — the pool still drops by exactly the amount the contract
> sent, so Fluxora's internal accounting stays in sync either way.

**2. The token does not rebase.** Fluxora never re-reads the pool's balance
except immediately around a transfer it initiated itself. An elastic-supply
token that changes the pool's balance out from under the contract — up or
down, on some schedule the contract is not party to — desynchronizes the real
balance from the sum of every stream's `deposited - withdrawn`.

> **Not detectable, not enforced.** There is no transfer to instrument; a
> rebase does not happen inside a Fluxora invocation. If the pool balance
> ever falls short of outstanding liabilities, the failure mode is a
> legitimate `withdraw` or `cancel` refund returning
> [`Error::TokenTransferFailed`](#error) once the shortfall is reached —
> Fluxora fails closed rather than overpaying one recipient at another's
> expense, but it does not compensate for the missing balance. Only fund a
> stream with a token whose balance changes exclusively through transfers
> Fluxora itself is a party to.

**3. Zero-value transfers are never issued — so whether the token treats one
as a no-op or a revert is immaterial.** Every entry point that could reach the
token with a non-positive amount is rejected first, before any token call:

| path | guard |
|---|---|
| `create_stream` | `deposit > 0`, else `InvalidDeposit` (4) |
| `top_up` / `delegate_top_up` | `amount > 0`, else `InvalidAmount` (18); schedule delta must be nonzero, else `TopUpTooSmall` (23) |
| `withdraw` / `delegate_withdraw` | `NothingToWithdraw` (17) short-circuits before any payout of zero |
| `batch_withdraw` | a stream with nothing currently available is skipped, not paid a zero |
| `cancel` / `delegate_cancel` | the refund transfer is only called when `refund > 0` |

`test::token_errors` pins all three assumptions down: a fee-on-transfer
mock is rejected on both `create_stream` and `top_up`; a token that panics on
any zero-value `transfer` call is proven never to be invoked with one, across
`cancel`, `withdraw` and `batch_withdraw`; and an out-of-band balance loss on
the pool (standing in for a negative rebase) is shown to fail closed with
`Error::TokenTransferFailed` rather than corrupting an unrelated stream's
accounting.

---

## Entry points

### Lifecycle

| function | auth | returns |
|---|---|---|
| `create_stream(sender, recipient, token, deposit, start_time, end_time, cliff_time, cancellable, pausable, transferable)` | sender | `u64` stream id |
| `top_up(stream_id, amount)` | sender | — |
| `withdraw(stream_id, amount: Option<i128>)` | recipient | `i128` paid |
| `batch_withdraw(recipient, stream_ids: Vec<u64>)` | recipient | `i128` total — [details](#batch_withdraw) |
| `withdraw(stream_id, amount: Option<i128>)` | recipient | `i128` paid — [details](#withdraw) |
| `batch_withdraw(recipient, stream_ids: Vec<u64>)` | recipient | `i128` total |
| `cancel(stream_id)` | sender | — |
| `pause(stream_id)` / `resume(stream_id)` | sender | — |
| `transfer_recipient(stream_id, new_recipient)` | recipient | — |
| `revoke_delegate(stream_id, grantor, delegate)` | sender or recipient | — |

`withdraw` with `amount = None` draws the full available balance.

#### `resume(stream_id)` — un-pause and fold the paused interval into `paused_total`

`resume` reverses `pause`: it clears `paused_at`, moves the stream back to
`Active`, and adds the elapsed pause (`now - paused_at`) to `paused_total`. The
stored schedule — `start_time`, `end_time`, `cliff_time` — is **not** touched.
Because accrual is driven by `elapsed - paused_total`, the larger `paused_total`
shifts the effective end of the stream forward by exactly the time it spent
paused, so the clock picks up where it stopped and the total value delivered
over the stream's life is unchanged.

**Pausing also moves the cliff, in wall-clock terms.** `cliff_reached` is
evaluated against the same stream clock,
`stream_time(now) = (paused_at ?? now) - paused_total`, so a pause freezes the
cliff gate along with accrual: while the clock is frozen below `cliff_time` the
gate stays shut no matter how far the wall clock advances. After a resume,
`paused_total` has absorbed the whole paused interval, so the gate opens at
wall-clock `cliff_time + paused_total`, not at the stored `cliff_time`. A stream
paused for `P` seconds in total therefore has its cliff — and with it the
instant its recipient can first withdraw — pushed `P` seconds later, exactly as
its `end_time` is. The stored `cliff_time` field is never rewritten; only the
mapping from wall clock to stream clock changes.

The `resumed` event publishes the post-resume `paused_total`, which is enough for
an integrator to recompute the moved instant as `cliff_time + paused_total` from
the `stream_created` schedule (or `get_stream`) alone, without replaying
individual pause intervals. `test::cliff::pause_across_cliff_delays_the_wall_clock_cliff`
and `test::pause::pausing_across_the_cliff_defers_the_cliff_too` assert this.

No value moves: `resume` performs no token sub-invocation and does not change
`deposited`, `withdrawn`, or any balance. It is a pure clock operation.

`paused_duration` is computed with a saturating subtraction, so a ledger
timestamp that has not advanced — or has gone backwards relative to `paused_at`
— yields `0` rather than an error. `resume` is **not** idempotent: a stream that
is already `Active` returns `StreamNotPaused`, not a no-op success. A
non-pausable stream can never satisfy the precondition either, because `pause`
is the only transition into `Paused`.
#### `top_up(stream_id, amount)` — extend duration, keep the rate

`top_up` adds funds to a live stream without changing the per-second rate the
recipient agreed to at creation. `end_time` moves forward by
`floor(amount * duration / deposited)` seconds so the added tokens stream out
at the original pace; `deposited` increases by `amount`. The alternative —
hold `end_time` and raise the rate — is rejected because it would retroactively
re-vest elapsed time.

The duration extension always rounds **down**. Rounding up would lower the rate
and reduce already-vested amounts; rounding down guarantees `vested` never
decreases across a top-up (residual at most one second of schedule, in the
recipient's favour).

Paused streams may be topped up: `Paused` is not terminal. Matured streams
(accrual clock already at `end_time`) and terminal streams (`Cancelled` /
`Depleted`) cannot. On success the sender's token balance is transferred into
the contract via a SEP-41 `transfer`.
#### `revoke_delegate(stream_id, grantor, delegate)` — revoke a delegate grant immediately

`revoke_delegate` removes the delegate grant stored for the pair
`(stream_id, delegate)`, if one exists. Revocation is **immediate and
unconditional**: from the next ledger onward the grant is gone, so a delegate
whose only grant was this one loses access to the stream and its next call fails
with `DelegateNotPermitted` (27). It does **not** unwind anything the delegate
already did — funds moved while the grant was live (for example by
`delegate_withdraw`) stay moved, and `deposited`, `withdrawn` and `status` are
untouched by the revocation itself. No token sub-invocation is made and no
balances change; this call is pure storage plus one event.

`revoke_delegate` is **idempotent**. Revoking a grant that does not exist —
never issued, already revoked, or revoked by the counterparty — is a successful
no-op that still performs the removal and still emits `delegate_revoked`. There
is no "grant not found" error, and a client does not need to read the grant
before revoking it.

Either the stream's `sender` or its `recipient` may revoke, and either party may
revoke a grant the **other** party issued: the grant key is per
`(stream_id, delegate)`, not per issuer. The delegate cannot revoke its own
grant, and an address that is neither party cannot revoke at all.
| `transfer_recipient(stream_id, new_recipient)` | sender | — |

`withdraw` with `amount = None` draws the full available balance.

#### `transfer_recipient(stream_id, new_recipient)` — reassign future payouts

`transfer_recipient` replaces `Stream.recipient` with `new_recipient`. Accrual
schedule, `deposited`, `withdrawn`, status, and token are unchanged: any balance
the old recipient had already accrued but not withdrawn **moves with the
stream** to the new recipient. Integrators should withdraw before transferring
if they need to settle the old payee's claim first.

Transfer is allowed at the cliff and end timestamps; the new recipient receives
whatever is withdrawable at those boundaries. A `Cancelled` stream may still be
transferred while it has an unwithdrawn residual (`withdrawn < deposited`). A
`Depleted` stream, or any stream whose deposit is fully drawn, cannot be
transferred.

Available only when the stream was created with `transferable == true`. A
compliance-bound sender can pin the payee at creation by passing `false`.

No tokens move in this call: there is no token sub-invocation. The recipient
change is a single storage write plus one event.

**Parameters.**

| parameter | type | valid range |
|---|---|---|
| `stream_id` | `u64` | An id of an existing, non-terminal stream whose `status` is `Paused`. Ids are monotonic and run `0..stream_count()`, so any issued id is accepted syntactically; a `stream_id` that was never issued, or whose entry has been archived, fails with `StreamNotFound`. |

**Authorization.** The stream's `sender` must authorise the call
(`sender.require_auth()`). The recipient cannot resume, and there is no admin
key.
| `stream_id` | `u64` | Id of an existing, non-terminal stream whose accrual clock has not yet reached `end_time`. Ids run `0..stream_count()`; an id that was never issued, or whose entry has been archived, fails with `StreamNotFound`. |
| `amount` | `i128` | Strictly positive (`> 0`), in the stream token's smallest unit. Must be large enough that `floor(amount * duration / deposited) >= 1` (otherwise `TopUpTooSmall`). Must also leave `new_deposited >= new_duration` after the extension (`DepositRateTooLow`). |

**Authorization.** The stream's `sender` must authorise the call
(`sender.require_auth()`). The recipient cannot top up, and there is no admin
key. A missing or wrong signature surfaces as a host authentication failure,
not a typed `Error`.
| `stream_id` | `u64` | An id of an existing, readable stream. Ids are monotonic and run `0..stream_count()`, so any issued id is accepted syntactically; a value that was never issued, or whose entry has been archived, fails with `StreamNotFound`. |
| `grantor` | `Address` | Any address equal to the stream's `sender` or `recipient`. Any other address fails with `Unauthorized`. The address must also authorise the call (`grantor.require_auth()`); it is the signing party, not a delegate acting under a grant. |
| `delegate` | `Address` | Any `Address`, used as the grant key to remove. It need not currently hold a grant: a missing grant is a successful no-op, so the full address space is accepted. |

**Authorization.** The `grantor` must be the stream's `sender` or `recipient`,
and must sign (`grantor.require_auth()`). The delegate neither consents nor can
block the revocation, and there is no admin key. The party check runs before the
auth check, so an address that is neither party returns `Unauthorized` whether
or not it signed.
| `stream_id` | `u64` | Id of an existing stream that still has an unwithdrawn claim (`withdrawn < deposited`) and is not `Depleted`. Ids are monotonic in `0..stream_count()`; an id that was never issued, or whose entry has been archived, fails with `StreamNotFound`. |
| `new_recipient` | `Address` | Must differ from the stream's `sender` (`SelfStream`) and from the current `recipient` (`RepeatedTransfer`). Any other address is accepted; there is no on-chain KYC or allow-list check. |

**Authorization.** The stream's **sender** must authorise the call
(`sender.require_auth()`). As of the #1637 hardening, the current recipient
cannot call this entry point directly — recipient-initiated reassignment is the
separate `delegate_transfer_recipient` path, gated on a recipient-issued
`TRANSFER_RECIPIENT` grant. A missing or wrong signature surfaces as a host
authentication failure, not a typed `Error` (there is no `Unauthorized` return
from this function).

**Errors.**

| variant | # | condition |
|---|---|---|
| `StreamNotFound` | 1 | No readable entry for `stream_id`: the id was never issued, or its entry has been archived. |
| `StreamNotPaused` | 12 | The stream is not currently paused — `paused_at` is `None`, or `status` is `Active` (never paused, or already resumed). |
| `StreamTerminated` | 14 | The stream is `Cancelled` or `Depleted`. Checked before the paused-state test, so a terminal stream that still carries a `paused_at` reports `StreamTerminated`, not `StreamNotPaused`. |
| `Overflow` | 22 | Defensive only: `paused_total + paused_duration` does not fit in `u64`. Unreachable for any stream created through the contract. Listed only so an integrator is never surprised by it. |

`NotPausable` (9) is a `pause`-only failure and is never returned by `resume`.
This list was cross-checked against `FluxoraStream::resume` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs); the four
variants above are the complete set it can return.

**Events.** Exactly one `resumed` event on success: topics `stream_id` and
`sender`; payload `paused_duration` (seconds absorbed by this call, i.e. the
elapsed pause) and `paused_total` (cumulative paused seconds after the call).
### `batch_withdraw`

```rust
fn batch_withdraw(env: Env, recipient: Address, stream_ids: Vec<u64>) -> Result<i128, Error>
```

Drains the full currently-withdrawable balance of several streams in one call
and returns the total paid out, in the token's smallest unit (`i128`, a string
over JSON-RPC). Unlike `withdraw` there is **no `amount` parameter**: every
element is drained to its current balance. Use `withdrawable_of(id)` for a
single stream's figure.
### `withdraw`

```rust
fn withdraw(env: Env, stream_id: u64, amount: Option<i128>) -> Result<i128, Error>
```

Pays the stream's current recipient the tokens they have already earned and
returns the `i128` amount transferred (token smallest units). The available
balance is `vested(now) - withdrawn`, where `vested` is zero until
`cliff_time` is reached and then rises linearly (rounding down) to `deposited`
by `end_time`.

Pausing freezes *accrual* only: funds already earned remain withdrawable while
`status == Paused`. A `Cancelled` stream that still holds an unwithdrawn
residual (`withdrawn < deposited`) can also be drained; a `Depleted` stream, or
any terminal stream with nothing left, cannot.

`amount == None` draws the full available balance. An explicit `Some(n)` pays
exactly `n` when `0 < n <= available`. On success the contract transfers tokens
from its pool to the recipient via a SEP-41 `transfer`, increments
`Stream.withdrawn`, and — when `withdrawn` reaches `deposited` on a
non-`Cancelled` stream — flips `status` to `Depleted` (and clears any open
pause). `Cancelled` is sticky: draining a cancelled stream to zero leaves it
`Cancelled`, never `Depleted`.

#### Parameters

| parameter | type | valid range / constraints |
|---|---|---|
| `recipient` | `Address` | The account that authorises the call and receives every payout. It is compared **by value** against each `Stream.recipient`; the batch is rejected with `Unauthorized` (7) unless all of them match. A failed signature surfaces as a host authentication failure, not a typed `Error`. |
| `stream_ids` | `Vec<u64>` | **1 to `MAX_BATCH_SIZE` (16) elements**, inclusive. Every element must decode as a `u64` (`MalformedStreamId` otherwise) and name a distinct existing stream in `0..stream_count()` (`StreamNotFound` otherwise). Empty → `EmptyBatch`; more than 16 → `BatchTooLarge`; a repeated id → `DuplicateStreamId`. |

`MAX_BATCH_SIZE` is 16 because the binding mainnet constraint is the **contract
event budget**, not entry or instruction counts — see the derivation in the
[README](../README.md). Chunk larger id lists client-side; the SDK does this
automatically.

#### Authorisation

`recipient.require_auth()` — the recipient authorises **once** for the whole
batch. The sender, and any other party, cannot call it. Because a stream's
recipient is immutable except through `transfer_recipient`, an authorisation
captured for one recipient cannot be replayed against another.

#### Per-element behaviour

1. The batch is *resolved and validated in full* before any storage write or
   token call.
2. A stream whose withdrawable balance is currently **zero** is **skipped**, not
   rejected: its TTL is extended on the touch and nothing else happens. This
   covers pre-start/pre-cliff streams, fully drawn streams, a drained
   `Cancelled` stream, and a `Depleted` stream. `NothingToWithdraw` (17) and
   `StreamTerminated` (14) are therefore **never returned by
   `batch_withdraw`** — that distinction lives in `withdraw` alone.
3. Streams need not share a token; each non-zero payout uses its own stream's
   token.
4. The returned total is the sum of every element's withdrawable amount,
   including the zero-valued elements. Per-stream amounts exist only in the
   events.

#### Atomicity

All-or-nothing. Any error aborts the entire call, discarding payouts already
applied to earlier elements: no accounting is written, no tokens move, and no
event is observable. Duplicate rejection is deterministic and independent of
where the repeats sit, so a corrected retry always behaves the same way.

#### Events

* One `withdrawn` event **per stream that paid out**, in batch order, with
  topics `stream_id`, `recipient` and payload `amount`, `withdrawn`,
  `deposited`, `status`.
* Skipped (zero-balance) streams emit **nothing**, so a 16-element batch may
  emit fewer than 16 events.
* There is no aggregate or batch-level event; the return value is the only
  total.
* Each non-zero payout also triggers the token contract's own `transfer` event.

#### Failure modes

Cross-checked against `batch_withdraw`'s implementation and the shared helpers
it calls (`validate_batch_ids`, `reject_duplicate_ids`, `accrual::withdrawable`,
`apply_withdrawal` → `token_transfer`). No other discriminant is reachable.

| error | # | condition |
|---|---|---|
| `StreamNotFound` | 1 | An id in `stream_ids` does not exist, or has been archived out of the live ledger (see `stream_exists`). |
| `Unauthorized` | 7 | A resolved stream's `recipient` differs from the `recipient` argument. |
| `BatchTooLarge` | 19 | `stream_ids.len() > MAX_BATCH_SIZE` (16). |
| `EmptyBatch` | 20 | `stream_ids` contains no elements. |
| `DuplicateStreamId` | 21 | The same id appears more than once in the batch. |
| `Overflow` | 22 | Checked arithmetic overflows while accruing a stream's balance or summing the batch total. |
| `TokenTransferFailed` | 25 | A payout's token transfer was rejected by the token contract (pool underfunded, or the token's own authorisation rules refused the call). The raw token discriminant is discarded — see the `Error` table above. |
| `TokenMissing` | 26 | A payout's token address does not resolve to a deployed contract (host `Abort`). No funds moved. |
| `MalformedStreamId` | 29 | A serialized element of `stream_ids` does not decode as a `u64`. |

`StreamNotActive` (11) is reserved and is not returned here.
| `StreamNotFound` | 1 | No readable entry for `stream_id`: the id was never issued, or its entry has been archived. Raised by `load_stream` before any other check. |
| `DepositRateTooLow` | 5 | After applying the extension, `new_deposited < new_duration` (same creation-time guard re-checked against the new figures). |
| `StreamTerminated` | 14 | The stream is `Cancelled` or `Depleted`. Checked before amount/maturity tests. |
| `StreamMatured` | 15 | Accrual clock (`stream_time`) has already reached `end_time`. Extending a matured stream would make the new funds instantly (or near-instantly) withdrawable; create a new stream instead. |
| `InvalidAmount` | 18 | `amount <= 0`. |
| `Overflow` | 22 | Checked arithmetic overflow while computing `delta`, `new_deposited`, `new_end`, `new_duration`, or the creation-time product guard; or `delta` outside `0..=u64::MAX`. |
| `TopUpTooSmall` | 23 | `floor(amount * duration / deposited) == 0` — the top-up cannot buy even one second of schedule, so absorbing it would require raising the rate. |
| `TokenTransferFailed` | 25 | The token contract returned a typed error on the deposit transfer (insufficient sender balance, trustline, or token auth rules). |
| `TokenMissing` | 26 | The stream's token address has no deployed code (host Abort / trap). |

This list was cross-checked against `FluxoraStream::top_up` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs) and the shared
`token_transfer` helper; the nine variants above are the complete set it can
return. `Unauthorized` (7) is **not** reachable here — auth failures abort in
the host before a typed error is produced.

**Events.** Exactly one `topped_up` event on success: topics `stream_id` and
`sender`; payload `amount` (this top-up), `deposited` (total after the call),
and `end_time` (extended schedule end). The token contract also emits its own
`transfer` event for the deposit.
#### `cancel(stream_id)`

Stops accrual and refunds the unvested remainder to the sender. The recipient keeps everything vested up to the current ledger timestamp.

**Authorisation:** `sender`

**Parameters:**
* `stream_id` (`u64`): The ID of the stream to cancel. Valid range: an existing stream ID (`0..stream_count()`).

**Errors:**
* `StreamNotFound` (1): No stream exists with the given `stream_id`.
* `NotCancellable` (8): The stream was created with `cancellable = false`.
* `StreamTerminated` (14): The stream is already in the `Cancelled` or `Depleted` state.
* `Overflow` (22): Integer overflow occurred during the unvested remainder computation.
* `TokenTransferFailed` (25): The token contract refused the refund transfer.
* `TokenMissing` (26): The token contract does not exist.

**Events:**
* `cancelled` — Emitted on success.
| `stream_id` | `u64` | Id of an existing stream. Ids are monotonic in `0..stream_count()`; an id that was never issued, or whose entry has been archived, fails with `StreamNotFound`. |
| `amount` | `Option<i128>` | `None` — pay the entire currently-withdrawable balance. `Some(n)` — `n` must be strictly positive (`> 0`) and at most the currently-withdrawable balance (`<= available`). Values are in the stream token's smallest unit. |

#### Authorisation

`recipient.require_auth()` — only the stream's current recipient may call this
entry point. The sender cannot withdraw, and there is no admin key. A missing
or wrong signature surfaces as a host authentication failure, not a typed
`Error`.

#### Errors

| variant | # | condition |
|---|---|---|
| `StreamNotFound` | 1 | No readable entry for `stream_id`: the id was never issued, or its entry has been archived. Raised by `load_stream` before any other check. |
| `InsufficientWithdrawable` | 16 | An explicit `Some(n)` was supplied, `available > 0`, and `n > available`. Never returned when `available == 0` — that path uses the empty-balance variants below instead. |
| `NothingToWithdraw` | 17 | The stream is still live (`Active` or `Paused`) but the withdrawable balance is zero: pre-start, pre-cliff, fully drawn for the moment, or otherwise accrued-nothing-left-to-pay. |
| `InvalidAmount` | 18 | An explicit `Some(n)` was supplied with `n <= 0`. |
| `StreamTerminated` | 14 | The stream is `Cancelled` or `Depleted` and has nothing left to pay (`available == 0`). Distinct from `NothingToWithdraw` so a client can tell "wait for accrual" apart from "this stream is over" without a second round-trip. |
| `Overflow` | 22 | Checked arithmetic overflow while computing vested/withdrawable amounts, or while updating `withdrawn` / `paused_total` in the shared withdrawal tail. Unreachable for any stream created through the contract under normal schedules. |
| `TokenTransferFailed` | 25 | The token contract returned a typed error on the payout transfer (insufficient pooled balance, deauthorized recipient trustline, or token auth rules). |
| `TokenMissing` | 26 | The stream's token address has no deployed code (host Abort / trap). |

This list was cross-checked against `FluxoraStream::withdraw` and
`FluxoraStream::apply_withdrawal` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs), plus
`accrual::withdrawable` / `accrual::vested` and the shared `token_transfer`
helper; the eight variants above are the complete set the entry point can
return. `Unauthorized` (7) is **not** reachable here — auth failures abort in
the host before a typed error is produced.

#### Events

Exactly one `withdrawn` event on success: topics `stream_id` and `recipient`;
payload `amount` (this payout), `withdrawn` (cumulative after the call),
`deposited`, and `status` (may already be `Depleted` when the payout exhausted
the deposit on a non-cancelled stream). The token contract also emits its own
`transfer` event for the payout. No event is emitted on failure.

| `Unauthorized` | 7 | `grantor` is neither the stream's `sender` nor its `recipient`. |

`DelegateNotPermitted` (27) and `DelegateExpired` (28) are **not** returned by
`revoke_delegate` — they are raised when a delegate *uses* a grant. Revoking an
expired, non-permitted, or non-existent grant all succeed. This list was
cross-checked against `FluxoraStream::revoke_delegate` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs); the two
variants above are the complete set it can return.

**Events.** Exactly one `delegate_revoked` event on success, including the
idempotent no-op case: topics `stream_id`, `grantor`, `delegate`; no payload.
#### `create_stream` — detailed reference

Create a payment stream and transfer `deposit` tokens from `sender` into the contract's pooled balance. Returns the new stream id (monotonic, never reused).

**Signature:**
```rust
fn create_stream(
    env: Env,
    sender: Address,
    recipient: Address,
    token: Address,
    deposit: i128,
    start_time: u64,
    end_time: u64,
    cliff_time: u64,
    cancellable: bool,
    pausable: bool,
    transferable: bool,
) -> Result<u64, Error>
```

**Authorization:** Requires `sender.require_auth()`. The sender's authorization on this invocation covers the nested token transfer; no prior token approval is needed.

**Parameters:**

| parameter | type | description |
|---|---|---|
| `sender` | `Address` | Funding party. Must authorize the call. |
| `recipient` | `Address` | Receiving party. Must differ from `sender`. |
| `token` | `Address` | Token contract address (SEP-41). Per-stream, not contract-wide. |
| `deposit` | `i128` | Initial amount to lock, in the token's smallest unit. Must be positive and satisfy rate constraints. |
| `start_time` | `u64` | Accrual begins (unix seconds). May be past (backdated vesting), present, or future (scheduled stream). |
| `end_time` | `u64` | Accrual ends (unix seconds). Must be strictly greater than `start_time`. |
| `cliff_time` | `u64` | Payout gate (unix seconds). Must be in `[start_time, end_time]`. Set equal to `start_time` for no cliff. **Gates payout, does not delay accrual** — at the cliff instant the recipient becomes entitled to everything accrued since `start_time`. On a `pausable` stream this stored instant is a lower bound, not the wall-clock instant the gate opens: pausing pushes the opening instant forward by the accumulated `paused_total`. See the `resume` entry point. |
| `cancellable` | `bool` | Whether sender may cancel. Immutable after creation. |
| `pausable` | `bool` | Whether sender may pause accrual. Immutable after creation. |
| `transferable` | `bool` | Whether recipient may reassign the stream. Immutable after creation. |

**Valid Ranges and Constraints:**

* **Time Range:** `end_time > start_time` (strictly greater). Duration must be at least 1 second. Zero-duration streams are rejected with `InvalidTimeRange`, not treated as "already vested".
* **Cliff:** `cliff_time` must satisfy `start_time ≤ cliff_time ≤ end_time`. Both boundary values are legal: `cliff_time == start_time` means no cliff, `cliff_time == end_time` means a single lump-sum payout at maturity.
* **Deposit:** Must be positive (`deposit > 0`).
* **Rate Floor:** `deposit ≥ duration_in_seconds`, ensuring the per-second rate does not truncate to zero. For example, a one-year stream requires at least 31,536,000 stroops (~3.16 USDC with 7 decimals).
* **Overflow Guard:** `deposit × duration` must fit in `i128`. This check at creation proves all future accrual multiplications cannot overflow.
* **Self-Stream:** `sender ≠ recipient`.
* **Clock Skew:** No validation limit on past or future timestamps. Backdated streams (past `start_time`) vest immediately for the elapsed portion. Scheduled streams (future `start_time`) accrue nothing until the start instant. Streams extending beyond the network's `max_entry_ttl` are funded to the horizon at creation; the permissionless `extend_stream_ttl` keeper path covers the remainder.

**Errors:**

All validation errors are checked **before** the token transfer. A rejected creation consumes no stream id, increments no counter, pulls no deposit, and leaves no partial state.

| error | condition |
|---|---|
| `SelfStream` (6) | `sender == recipient` |
| `InvalidDeposit` (4) | `deposit ≤ 0` |
| `InvalidTimeRange` (2) | `end_time ≤ start_time` (zero or negative duration) |
| `InvalidCliff` (3) | `cliff_time < start_time` or `cliff_time > end_time` |
| `DepositRateTooLow` (5) | `deposit < (end_time - start_time)`, causing per-second rate to truncate to zero |
| `Overflow` (22) | `deposit × (end_time - start_time)` does not fit in `i128` |
| `StreamIdExhausted` (24) | The stream-id counter has reached `u64::MAX`; no further ids can be allocated. Terminal for new creations. |
| `TokenTransferFailed` (25) | Token contract rejected the transfer (insufficient sender balance, authorization refused, or token's own rules). The token's internal error discriminant is intentionally discarded; see the root diagnostic in the transaction's `diagnosticEvents`. |
| `TokenMissing` (26) | The `token` address has no deployed contract (host `Abort` / trap). |

**Events:**

On success, emits `stream_created` with topics `[stream_id, sender, recipient]` and payload carrying the complete initial state: `token`, `deposited`, `start_time`, `end_time`, `cliff_time`, `cancellable`, `pausable`, `transferable`. This is the canonical event for indexer discovery.

**Atomicity:**

Creation is transactional. The stream-id counter and count are advanced only after all validation and the token transfer succeed. A failed creation leaves the id space contiguous with no gaps, consumes no tokens, and emits no event.

**Special Cases:**

* **Backdated Start** (`start_time < now`): Legitimate for hire-date or grant-award vesting. The elapsed portion vests immediately and is withdrawable on the next ledger.
* **Future Start** (`start_time > now`): Scheduled stream. The deposit is escrowed; accrual begins at `start_time`. Nothing vests or is withdrawable before then.
* **Fully Elapsed Schedule** (`end_time ≤ now`): Accepted. Reads as fully vested immediately. The entry receives the minimum retention TTL floor.
* **No Cliff** (`cliff_time == start_time`): Standard continuous vesting with no payout gate.
* **Cliff at Maturity** (`cliff_time == end_time`): Single lump-sum payout when the stream completes.
* **Pause Moves the Cliff** (`pausable == true`): the stored `cliff_time` is never rewritten, but pausing shifts the wall-clock instant at which the gate opens by the accumulated paused time. A recipient's first withdrawal becomes available at `cliff_time + paused_total`, not at `cliff_time` — see the `resume` entry point. An integrator rendering "funds unlock at &lt;time&gt;" must add the stream's current `paused_total` rather than displaying `cliff_time` directly.

**Example:** A 100-day stream of 1,000 USDC (7 decimals = 10,000,000 stroops per USDC) created with a 10-day cliff:

```
deposit:     10_000_000_000 stroops
duration:    8_640_000 seconds (100 days)
rate:        1_157 stroops/second (truncating division)
start_time:  1609459200 (2021-01-01 00:00:00 UTC, example)
end_time:    1618099200 (2021-04-11 00:00:00 UTC)
cliff_time:  1610323200 (2021-01-11 00:00:00 UTC)
```

At day 9: vested = 900 USDC, but withdrawable = 0 (pre-cliff).  
At day 11: vested = 1,100 USDC, withdrawable = 1,100 USDC (cliff passed, all accrued funds unlocked).  
At day 100: vested = 10,000 USDC, withdrawable = 10,000 USDC (fully matured).
| `StreamNotFound` | 1 | No readable entry for `stream_id`: the id was never issued, or its entry has been archived. Raised by `load_stream` before any other check. |
| `NotTransferable` | 10 | The stream was created with `transferable == false`. |
| `StreamTerminated` | 14 | The stream is `Depleted`, or `withdrawn >= deposited` (covers a sticky `Cancelled` stream whose residual has already been fully drawn). |
| `SelfStream` | 6 | `new_recipient` equals the stream's `sender`. |
| `RepeatedTransfer` | 30 | `new_recipient` equals the current `recipient` (no-op transfers are rejected). |

This list was cross-checked against `FluxoraStream::transfer_recipient` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs); the five
variants above are the complete set it can return. `Unauthorized` (7) is **not**
reachable here — auth failures abort in the host before a typed error is
produced.

**Events.** Exactly one `recipient_transferred` event on success: topics
`stream_id`, `old_recipient`, and `new_recipient`; empty payload.

### Views — read-only, no TTL side effects

| function | returns | extends TTL | writes storage |
|---|---|---|---|
| `get_stream(stream_id)` | `Stream` | no | no |
| `withdrawable_of(stream_id)` | `i128` | no | no |
| `vested_of(stream_id)` | `i128` | no | no |
| `refundable_of(stream_id)` | `i128` | no | no |
| `stream_count()` | `u64` — ids run `0..stream_count()` | no | no |
| `stream_exists(stream_id)` | `bool` | no | no |

#### `vested_of(stream_id)` — total earned by the recipient, withdrawn or not

`vested_of` is the **cumulative** amount the recipient has accrued since
`start_time`, whether or not it has been withdrawn. It is **monotonic
non-decreasing** in the stream clock: paused streams freeze the clock, so a
pause pauses vesting too, and it never moves backwards. The three view
quantities `vested_of`, `withdrawable_of` and `refundable_of` are related by

```
vested_of + refundable_of == deposited          // conservation, at any instant
withdrawable_of == max(0, vested_of - withdrawn) // what the recipient may claim now
```

Two behavioural facts are the point of this entry point and must not be
assumed:

* **Rounding is down.** Vested is computed as

  ```
  vested = floor(deposited * elapsed / duration)     // integer division
  ```

  truncating in the recipient's disfavour. The residue stays in the contract
  and returns to the sender when the stream settles, so the pool can never be
  short. A client must not round up, and must not assume `deposited /
  duration` times `elapsed` is exact.

* **Pre-cliff it is zero.** The cliff *gates* the payout, it does not delay
  accrual: before `cliff_time` the result is exactly `0`, and at the cliff
  instant the recipient becomes entitled to everything accrued since
  `start_time` — not merely what accrues after the cliff. There is no partial
  vesting beforehand. "Before `cliff_time`" is measured **on the stream clock**,
  so on a stream that has been paused the wall-clock instant the gate opens is
  `cliff_time + paused_total`: a recipient whose stream was paused across its
  cliff waits the total paused duration longer before anything is vestable. See
  the `resume` entry point.

The cliff gate is evaluated **first**: while the stream clock is below
`cliff_time` the result is `0`, even for a schedule that has collapsed. Only
once the cliff has opened is the formula applied, and then the result is
`deposited` in full when either the schedule has fully elapsed
(`elapsed >= duration`) or its duration is zero (a `cancel` that collapsed the
schedule onto its start instant). A cancelled stream reads the same way:
`vested_of` returns its settled, rewritten `deposited`.

**Parameters.**

| parameter | type | valid range |
|---|---|---|
| `stream_id` | `u64` | any id ever issued, i.e. `id < stream_count()`. Ids are monotonic and never reused. Any `u64` is accepted syntactically; an id that was never issued, or whose entry has been archived, fails with `StreamNotFound` — it does not signal "no such id" vs "archived" (see Client requirements). |

**Authorization.** None — `vested_of` is a permissionless, read-only view. It
runs in simulation and, like every view, does **not** extend the entry's TTL;
keeping a stream alive is `extend_stream_ttl`'s job.

**Errors.**

| variant | # | condition |
|---|---|---|
| `StreamNotFound` | 1 | No readable entry for `stream_id`: the id was never issued or the entry has been archived. |
| `Overflow` | 22 | Defensive only: the checked `deposited * elapsed` product does not fit in `i128`. It is unreachable for any stream created through the contract — `create_stream` and `top_up` both guard `deposited * duration` fits in `i128`, and `elapsed <= duration` always holds. Listed only so an integrator is never surprised by it. |

**Events.** None. Views emit no events; `vested_of` only reads.

### Maintenance — permissionless

| function | returns | extends TTL | writes storage |
|---|---|---|---|
| `extend_stream_ttl(stream_id)` | `u32` ledgers now funded | **yes** — the stream entry and the contract instance | TTL only; no entry data changes |
| `batch_extend_ttl(stream_ids: Vec<u64>)` | `u32` entries extended | **yes** — each existing listed stream and the contract instance; unknown ids are skipped | TTL only; no entry data changes |

**Read-path TTL rule (#1686).** Every view reads through `storage::peek_stream`
or a plain `has`/`get`, so a view never extends TTL and never writes: calling
one — including in simulation — leaves every ledger entry, and its
`live_until`, exactly as it was. Keeping a stream alive is only ever done by
the two maintenance calls above, or as a side effect of a state-changing
lifecycle call (those go through `storage::load_stream`, which bumps TTL).
`contracts/stream/src/test/read_ttl_matrix.rs` snapshots all ledger entries
around every entry point in these two tables and fails if behaviour and this
table disagree — including if a view is switched to a TTL-bumping read.

`MAX_BATCH_SIZE = 16` for both batch functions. Chunk client-side; the SDK does
this automatically. See [README](../README.md) for why the cap is 16 and why it
is derived from the *event* budget rather than the entry count.

#### `extend_stream_ttl(stream_id) -> u32`

**Authorisation: none - permissionless.**
There is no caller parameter and no `require_auth`: anyone may pay rent for any
stream, and the caller bears the rent cost.
This is deliberate - a recipient's claim must never depend on the sender's
continued goodwill, and a third-party keeper sweeping streams that approach
expiry needs nobody's permission.
The call cannot move funds or change stream state; the caller only ever *pays*.

| parameter | type | valid range |
|---|---|---|
| `stream_id` | `u64` | an issued id whose entry still exists. Ids run `0..stream_count()`. Streams in **any** status are eligible, including terminal `Cancelled` and `Depleted` ones |

**Returns** the `u32` number of ledgers the entry is now funded for.
The target is the stream's remaining effective lifetime (now to `end_time`,
plus accumulated and in-progress pause time) plus a 30-day buffer, converted at
5 seconds per ledger rounding up, floored at `MIN_STREAM_TTL_LEDGERS` (518,400
ledgers, ~30 days) and clamped to the network's `max_entry_ttl`.
Multi-year streams therefore need periodic re-extension no matter how
generously creation funds them.
The contract instance entry is extended to the network maximum in the same
transaction.

**Errors**

| error | condition |
|---|---|
| `StreamNotFound` (1) | `stream_id` was never issued, or its entry has been archived and needs restoring |

`StreamNotFound` is the only typed error this entry point returns.

**Events** - on success emits `ttl_extended`: topics are the event name and
`stream_id`; the payload is `extended_to_ledgers`, equal to the returned value.
### Delegation

`grant_delegate` and `revoke_delegate` manage scoped, expiring permissions for a
third party; the `delegate_*` entry points exercise a grant.

| function | auth | returns |
|---|---|---|
| `grant_delegate(stream_id, grantor, delegate, ops, expires_at)` | sender or recipient, depending on `ops` | — |
| `revoke_delegate(stream_id, grantor, delegate)` | sender or recipient | — |
| `delegate_withdraw(stream_id, delegate, amount: Option<i128>)` | delegate holding `op::WITHDRAW` | `i128` paid |
| `delegate_cancel(stream_id, delegate)` | delegate holding `op::CANCEL` | — |
| `delegate_pause(stream_id, delegate)` | delegate holding `op::PAUSE` | — |
| `delegate_resume(stream_id, delegate)` | delegate holding `op::RESUME` | — |
| `delegate_top_up(stream_id, delegate, amount)` | delegate holding `op::TOP_UP` | — |
| `delegate_transfer_recipient(stream_id, delegate, new_recipient)` | delegate holding `op::TRANSFER_RECIPIENT` | — |

A grant is stored under `(stream_id, delegate)` and covers one stream only. The
delegate entry points verify it before acting: a missing grant or one that does
not cover the requested operation returns `DelegateNotPermitted` (27), and a
grant whose `expires_at` has passed returns `DelegateExpired` (28).

#### `grant_delegate`

Grant a delegate permission to call a scoped set of operations on one stream.
Granting over an existing grant for the same `(stream_id, delegate)` pair
replaces it entirely.

```rust
fn grant_delegate(
    stream_id: u64,
    grantor: Address,
    delegate: Address,
    ops: u32,
    expires_at: Option<u64>,
) -> Result<(), Error>
```

**Parameters**

| parameter | type | valid range | notes |
|---|---|---|---|
| `stream_id` | `u64` | must reference an existing, non-terminal stream | `StreamNotFound` (1) if no such stream; `StreamTerminated` (14) if the stream is `Cancelled` or `Depleted`. |
| `grantor` | `Address` | `stream.sender` for sender-side ops, `stream.recipient` for recipient-side ops | The party the grant is issued by; `require_auth()` runs on it, so the call must be authorised by that address. |
| `delegate` | `Address` | any account or contract address | The party that may later call the `delegate_*` entry points. Not authenticated at grant time. |
| `ops` | `u32` | bitmask of the `op::*` constants: `WITHDRAW` `1`, `CANCEL` `2`, `PAUSE` `4`, `RESUME` `8`, `TOP_UP` `16`, `TRANSFER_RECIPIENT` `32` | Bits 0–5 are defined; higher bits are reserved and never match at authorisation time. OR several constants together for a multi-op grant. `ops == 0` is a no-op: it returns `Ok(())` and stores nothing. |
| `expires_at` | `Option<u64>` | `None` = never expires; `Some(t)` = Unix seconds | The grant is valid while `ledger.timestamp() <= t`. A timestamp already in the past is accepted and produces an immediately-expired grant. |

**Authorisation**

Sender-side ops are `CANCEL`, `PAUSE`, `RESUME` and `TOP_UP`; recipient-side ops
are `WITHDRAW` and `TRANSFER_RECIPIENT`. A purely sender-side set must come from
the stream's sender and a purely recipient-side set from the stream's recipient,
each authorising the call. A set that mixes the two groups — for example
`WITHDRAW | CANCEL` — is rejected with `Unauthorized` (7); issue two grants
instead. `ops == 0` is a no-op and needs no authorisation.

**Errors**

| # | error | condition |
|---|---|---|
| 1 | `StreamNotFound` | No stream exists with `stream_id`. |
| 14 | `StreamTerminated` | The stream is `Cancelled` or `Depleted`; terminal streams accept no new grants. |
| 7 | `Unauthorized` | `ops` mixes sender-side and recipient-side bits, or `grantor` is not the party that owns the requested ops. |

A failed `require_auth()` is a host authorisation trap rather than a typed
contract error, so it is not listed above.

**Events**

On success — that is, whenever a non-empty `ops` mask is stored — emits
`delegate_granted` with topics `stream_id`, `grantor`, `delegate` and payload
`ops`, `expires_at`. `ops == 0` stores nothing and emits nothing.

---

## Events

Declared with `#[contractevent]`; schemas are in the deployed spec. First topic
is the snake_case event name, second is always `stream_id`.

| event | topics after the name | payload |
|---|---|---|
| `stream_created` | `stream_id`, `sender`, `recipient` | `token`, `deposited`, `start_time`, `end_time`, `cliff_time`, `cancellable`, `pausable`, `transferable` |
| `withdrawn` | `stream_id`, `recipient` | `amount`, `withdrawn`, `deposited`, `status` |
| `cancelled` | `stream_id`, `sender`, `recipient` | `refunded`, `vested`, `withdrawn`, `end_time` |
| `paused` | `stream_id`, `sender` | `paused_at`, `paused_total` |
| `resumed` | `stream_id`, `sender` | `paused_duration`, `paused_total` |
| `topped_up` | `stream_id`, `sender` | `amount`, `deposited`, `end_time` |
| `recipient_transferred` | `stream_id`, `old_recipient`, `new_recipient` | — |
| `delegate_revoked` | `stream_id`, `grantor`, `delegate` | — |
| `ttl_extended` | `stream_id` | `extended_to_ledgers` |
| `delegate_granted` | `stream_id`, `grantor`, `delegate` | `ops`, `expires_at` |
| `delegate_revoked` | `stream_id`, `grantor`, `delegate` | — |

Every payload carries enough state to reconstruct the stream without replaying
from genesis. Field order and topic placement are ABI.

`resumed` deserves one note: its `paused_total` is the post-resume cumulative
figure, and it is enough to recompute the stream's moved cliff instant,
`cliff_time + paused_total`, given the `cliff_time` carried by `stream_created`
(or read back from `get_stream`). The event deliberately does not republish
`cliff_time`, which never changes; see the `resume` entry point.

Note that `batch_withdraw` emits one `withdrawn` event **per stream drawn from**,
not one per call, and skips streams with nothing available — so a batch of 16
may emit fewer than 16 events.

---

## Resolved schema questions

Both were open against `Fluxora-Backend` in [MIGRATION.md](MIGRATION.md) §5
and are settled here as part of the freeze.

### 1. `streams.status` — mirror the contract's four values verbatim

The backend's current CHECK constraint is
`('active','paused','completed','cancelled')`. The contract emits
`Active | Paused | Cancelled | Depleted`.

**Resolution: rename `completed` to `depleted` and use the contract's four names
as-is.** Do not map `Depleted` onto `completed`.

The tempting mapping is lossy in a way that matters. `Cancelled` is sticky, so a
cancelled stream that is later drained to zero stays `Cancelled` — it never
becomes `Depleted`. The two terminal states therefore answer different
questions: `Depleted` means "ran to term and the recipient took everything",
`Cancelled` means "the sender clawed back the remainder", regardless of whether
the recipient has since collected their share. Collapsing them, or introducing a
third name that exists only in the database, guarantees the projection and the
chain disagree the first time someone reports on completion rates.

Migration for the backend:

```sql
ALTER TABLE streams DROP CONSTRAINT streams_status_check;
UPDATE streams SET status = 'depleted' WHERE status = 'completed';
ALTER TABLE streams ADD CONSTRAINT streams_status_check
  CHECK (status IN ('active','paused','cancelled','depleted'));
```

The status discriminant in the `withdrawn` event is the authoritative source for
a stream reaching a terminal state; `cancelled` and `stream_created` cover the
rest.

### 2. `rate_per_second` — derived by the client, no on-chain view

**Resolution: the client derives it. We do not add a `rate_of()` view.**

```
rate_per_second = deposited / (end_time - start_time)      // integer, truncating
```

The reasoning, and the cost of being wrong, both matter here because the
contract is immutable — *adding this view later is impossible without
redeploying to a new address*.

Against adding it: it carries zero information. `get_stream` already returns
`deposited`, `start_time` and `end_time`, so a view would be pure convenience
occupying permanent surface on an immutable contract. Worse, "rate" is genuinely
ambiguous while a stream is paused — the instantaneous rate is zero, the
schedule rate is unchanged — and a single view would have to pick one and be
wrong for half its callers.

For deriving it: the formula is stable by construction. `top_up` holds the rate
fixed by design (it extends `end_time` instead of raising the rate), so the
derived value does not move across a top-up. After `cancel` it becomes
meaningless, which is correct — a cancelled stream has no rate.

**The backend's `streams.rate_per_second` column should become a generated or
computed value, not a stored one fed from an event.** No event carries a rate,
and none will.

**Requirement on `fluxora-sdk`, before stage 5.** The SDK must ship the
canonical derivation as a single exported function, and integrators must be
directed to it rather than to the formula. Declining to add an on-chain view
only avoids the ambiguity if exactly one implementation exists downstream; if
three integrators write three slightly different rate calculations — differing
on the paused case, or on cancelled streams, or on truncation — then we have
exported the ambiguity instead of resolving it, which is strictly worse than
having added the view.

The SDK's implementation is normative and must define, at minimum:

| case | value |
|---|---|
| active | `deposited / (end_time - start_time)`, truncating |
| paused | the schedule rate is unchanged; the *instantaneous* rate is zero. The SDK must expose these as two distinct, named quantities rather than one ambiguous `rate`. |
| cancelled | undefined — return `None`, not zero. A cancelled stream has no rate, and zero would be indistinguishable from a paused stream to a caller that ignores status. |
| after `top_up` | unchanged by construction; `top_up` extends `end_time` instead of raising the rate |

Link back to this table from the SDK's own documentation so the two cannot
drift.

---

## Client requirements

Two are non-negotiable for stage 5, both learned the hard way against live
testnet.

**1. Pin multi-view reads to a single ledger.** The public RPC endpoint is
load-balanced across nodes at different heights, and consecutive calls can
observe different ledgers — including apparently going backwards in time. Two
view calls combined into one derived figure (for example checking
`vested_of + refundable_of == deposited`) will intermittently disagree. See
[soroban-rpc-read-skew.md](soroban-rpc-read-skew.md).

**2. Handle archived streams.** `stream_exists(id) == false` while
`id < stream_count()` means the entry has been archived, not that it never
existed. Surface a restore action rather than an error. See
[KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) §1.

