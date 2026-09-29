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

> **Security status:** Automated testing includes property tests, a pool
> invariant checked after every operation, and randomized sequence tests. This
> testing is not an independent security review; no third-party security audit
> of the contracts has been performed.

## Upgrade posture

The deployed stream contract is **not upgradeable in place**. This is a
deliberate property: the contract has no admin key and exposes no upgrade entry
point. A changed implementation must be deployed at a new address; existing
streams remain on the original deployment and are not migrated automatically.
There is therefore no upgrade authorization or in-place migration process.
The ABI inventory below is the deployed surface and contains no upgrade or
admin-rotation method.

The single exception to "no operator" is the **emergency halt** (issue #1818):
an operator address can be installed once, has no rotation path, and can only
stop and restart settlement contract-wide — it holds no key over funds, no
upgrade authority, and no power over any individual stream. See
[Emergency halt](#emergency-halt). A deployment that never installs an operator
is exactly as admin-free as before.

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

### ABI versions

| version | scope |
|---|---|
| `1` | The frozen interface above, as deployed at `CBCGTSCJ…`. |
| `2` | **Release curves (#1815).** `Stream` gained a `curve` field, `StreamCreated` gained a `curve` payload field, and the new `create_stream_with_curve` entry point was added. |

Version 2 is a *new deployment*, not an in-place upgrade (see Upgrade posture).
The version 1 surface is unchanged in meaning and still callable on the version 1
deployment: `create_stream` keeps its exact signature and its exact linear
arithmetic. What makes the change **breaking**, and therefore what requires the
bump, is that `get_stream` returns a `Stream` with one more field and
`stream_created` carries one more payload field — a typed client generated
against version 1 cannot decode either. The new entry point and the appended
event field are individually additive; the struct field is not.

On-chain storage did **not** change: version 1 entries stay readable and are read
back as linear streams. See [Storage compatibility](#storage-compatibility).
`ABI_VERSION` is currently **2**. Version 1 is the interface this repository
froze; version 2 adds `create_stream_with_cliff_mode` and the `CliffMode` type,
and adds one field to the `Stream` UDT and one to the `stream_created` payload.
Every version-1 method keeps its version-1 signature and meaning — `create_stream`
in particular still creates `CliffMode::Schedule` streams — so version 1 is a
prefix of version 2 apart from those two additions. The version-1 record is
preserved as the `frozen_v1()` inventory in
`contracts/stream/src/test/abi.rs` and asserted against by
`test::abi::current_spec_is_compatible_with_frozen_v1`.

**What adding a `Stream` field does to stored data.** `cliff_mode` changes the
`Stream` struct's XDR layout, so a v2 reader cannot decode a v1 entry — the field
count no longer matches and the host rejects the unpack. This is safe for the
same reason every other breaking change here is safe: the contract is not
upgradeable in place, so a v2 build is deployed at a **new address** and v1
entries stay under the v1 contract id, never opened by v2 code. There is no
in-place migration to write, and none is provided — one would be unreachable
code that grows the deployable WASM. Both halves are asserted in
`contracts/stream/src/test/storage_keys.rs`:
`v1_layout_is_no_longer_decodable_and_that_is_deliberate` (the refusal, with the
upgrade-posture argument) and `v2_layout_round_trips` (that the new layout still
reads its own entries).

A consequence worth stating plainly: the mode is a **creation-time** choice that
cannot be changed later, and there is no way to move an existing deployment's
streams onto the new mode other than creating new streams under the new
contract. That is the same trade-off the capability flags already carry, and it
is deliberate — an immutable choice is what makes `cliff_mode` trustworthy to a
recipient reading `get_stream`.

---

## Constants

These numeric limits are part of the frozen ABI. They are guaranteed not to
change in a same-address deployment; any increase requires a new contract at a
new address. Integrators should hard-code these values rather than probing with
trial calls.

| constant | value | applies to | notes |
|---|---|---|---|
| **`MAX_BATCH_SIZE`** | **16** | `batch_withdraw`, `batch_extend_ttl` | Maximum number of stream ids accepted in a single batch call. Requests with **more than 16 ids** return [`BatchTooLarge` (19)](#error). Chunk larger lists client-side; the SDK does this automatically. |
| **`MAX_REFERENCE_LENGTH`** | **64** | `create_stream` | Maximum length in characters for the optional reference string. References exceeding this limit return [`InvalidReferenceLength` (34)](#error). |
| `MIN_RATE_STROOPS_PER_SECOND` | 1 | `create_stream`, `top_up` | Enforced via `DepositRateTooLow` (5). Below 1 token unit per second the per-second rate truncates to zero and the recipient accrues nothing until the last instant. |

### Derivation of `MAX_BATCH_SIZE = 16`

The binding mainnet constraint is the **contract event budget** (16,384 bytes per
transaction), not entry or instruction counts. Each stream in a batch emits a
`withdrawn` event plus the token's `transfer` event. Against the Stellar Asset
Contract that is 512 bytes per stream, so a full batch of 16 spends 8,192 of the
16,384-byte budget — a 2x margin. The other half of that cost belongs to the
*token*, so a heavier transfer event moves the ceiling, which is why the number
is re-derived by measurement rather than assumed.

`test::token_batch_calibration` derives the implied ceiling for four token
profiles of deliberately different cost:

| token profile | per-transfer event bytes | events at 16 | implied ceiling |
| --- | --- | --- | --- |
| no transfer event | 0 | 4,416 / 16,384 | 59 |
| Stellar Asset Contract (baseline) | 236 | 8,192 / 16,384 | 32 |
| ~256-byte transfer event | 412 | 11,008 / 16,384 | 23 |
| ~2 KB transfer event | 2,204 | 39,680 / 16,384 | 6 |

**Sixteen is the measured ceiling for the Stellar Asset Contract and every
lighter token, with the documented 2x safety factor.** A token with a heavier
transfer event shifts the ceiling down — the ~2 KB profile admits 6, not 16 —
and the contract cannot detect that, because the cost lives inside the token's
own event. What it does enforce is the cap itself: more than 16 ids is rejected
with [`BatchTooLarge` (19)](#error) before the token is touched, whatever the
token is. `test::resource_limits` pins the event cost at the cap for the
baseline token; `docs/KNOWN-LIMITATIONS.md` §3 records the per-token ceilings.
`withdrawn` event plus the token's `transfer` event — 624 bytes per stream
between them since issue #1868 appended the sender and the pause bookkeeping to
`withdrawn`. With a heavier token event payload, 26 streams would risk
exhausting the budget. **Sixteen is the measured ceiling leaving 6,400 bytes
spare.** The measurement suite lives in `test::resource_limits` and runs in
every CI build under the `Resource report` step.

Client-side chunking is transparent to integrators: each chunk is a separate,
atomic `batch_withdraw` or `batch_extend_ttl` call, and the SDK retries on
transient network errors per chunk.

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
    cliff_mode: CliffMode, // immutable after creation. How cliff_time is compared.
    cancellable: bool,     // immutable after creation
    pausable: bool,        // immutable after creation
    transferable: bool,    // immutable after creation
    paused_at: Option<u64>,
    paused_total: u64,     // cumulative paused seconds, excluding any in-progress pause
    status: StreamStatus,
    curve: ReleaseCurve,   // release shape above the cliff; immutable after creation
    reference: Option<String>, // optional reference for stream identification, max 64 characters
}
```

### `CliffMode`

```rust
enum CliffMode {
    Schedule,   // 0, default. Gate opens when the stream clock reaches cliff_time.
    WallClock,  // 1.         Gate opens when ledger time reaches cliff_time.
}
```

A `u32`-backed enum, carried on the wire as a bare `u32`. Any other value is
refused by the host while arguments are decoded, as a conversion error, so no
`Error` discriminant is needed for it. The two discriminants are part of the ABI
and will only ever be appended, matching the rule for error codes.

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

### `ReleaseCurve`

The shape of the release schedule **above the cliff**. Crosses the ABI as its
**discriminant**, not its name. Selected once, at creation, and never mutable —
like the capability flags, this is a trust feature: a recipient who accepts a
front-loaded stream has verified on chain that the shape cannot be flattened
afterwards. Defaults to `Linear` when a stream is created with `create_stream`.

| value | name | schedule between `start_time` and `end_time` |
|---|---|---|
| `0` | `Linear` | `floor(deposited × elapsed / duration)`. The original formula, unchanged. |
| `1` | `Step` | Four equal tranches. Tranche *k* (for *k* in `1..=3`) opens once `ceil(k × duration / 4)` seconds have been consumed; the fourth is delivered at maturity. The recipient's claim jumps by a quarter of the deposit at each 25 / 50 / 75 / 100 % boundary and does not move in between. |
| `2` | `FrontLoaded` | `f(u) = 2u − u²` on `u = elapsed / duration`, evaluated on a 1/1000 grid: accelerates early, decelerates into maturity. Ahead of or equal to `Linear` at every grid point while inside the schedule. |

All curves share three properties, and those are what the contract's invariants
rely on:

1. **Monotone non-decreasing** on the stream clock — `vested_of` can never go
   backwards in time, for any curve.
2. **`f(0) == 0`** — nothing vests before the start instant.
3. **`f(duration) == deposited`** — the schedule settles at exactly the deposit,
   so `vested_of + refundable_of == deposited` holds at every instant whichever
   curve is selected.

The variants differ only in *when* the deposit is delivered, never in how much.
A `Step` tranche threshold uses `ceil`, not `floor`, so a schedule shorter than
four seconds does not release a tranche at `start_time`. `Linear` uses the
original `floor` and is byte-for-byte identical to the pre-#1815 arithmetic — a
linear stream created through either entry point vests identically.

### Storage compatibility

Adding a field to the **stored** value of a `#[contracttype]` struct would make
every entry written by an earlier deployment undecodable (the host unpacks the
stored map positionally against the struct's field set, and reports
`Error(Object, UnexpectedSize)` on a mismatch). Release curves therefore do **not**
change the stored layout:

* `DataKey::Stream(id)` still holds the frozen version 1 record — the version 1
  field set, no `curve`.
* A non-linear stream's curve is written to a side-car key,
  `DataKey::StreamCurve(id)`, which is kept alive with the same TTL as the record
  it annotates.
* A **missing** side-car means `Linear`. Version 1 streams have no side-car, so
  they read back as exactly the stream they were created as, and a linear stream
  created today writes none either — it pays no rent for a key that would merely
  restate the default. No migration is required for a live deployment.

`get_stream` stitches the two together, so the `Stream` a caller sees always
carries the effective curve.

### Stream ID allocation

Stream IDs are a zero-based, monotonic sequence scoped to one contract
deployment. A successfully issued ID is never reused, including after its
stream is cancelled, depleted, or its persistent entry is archived. Failed
`create_stream` calls do not consume an ID. When the next ID reaches
`u64::MAX`, creation fails with `StreamIdExhausted` (24) rather than wrapping.

The contract cannot be upgraded in place. A changed implementation requires a
new contract address and starts an independent ID sequence, so IDs are not
globally unique across deployments. Integrators should key records by both
contract address and stream ID. Any future design that adds in-place upgrades
must preserve the allocation counter.

### Capability flags

Three `bool` fields on `Stream` describe operations that are **not available**
for a given stream. They are supplied by the sender to `create_stream` and are
**fixed for the lifetime of the stream** — no entry point can change them after
creation. `get_stream` returns the live `Stream` struct, which includes all
three fields. `cliff_mode` is fixed the same way, but is not a capability: it
selects a rule rather than denying an operation.

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

| # | name | condition | status |
|---:|---|---|---|
| 1 | `StreamNotFound` | Requested stream has no readable storage entry. | reachable |
| 2 | `InvalidTimeRange` | `end_time <= start_time`. | reachable |
| 3 | `InvalidCliff` | `cliff_time` is outside `[start_time, end_time]`. | reachable |
| 4 | `InvalidDeposit` | Deposit is zero or negative. | reachable |
| 5 | `DepositRateTooLow` | Deposit is less than schedule duration in seconds. | reachable |
| 6 | `SelfStream` | Sender and recipient are the same address. | reachable |
| 7 | `Unauthorized` | Caller is not authorized for the requested operation. | reachable |
| 8 | `NotCancellable` | Cancellation is attempted on a non-cancellable stream. | reachable |
| 9 | `NotPausable` | Pause is attempted on a non-pausable stream. | reachable |
| 10 | `NotTransferable` | Recipient transfer is attempted on a non-transferable stream. | reachable |
| 11 | `StreamNotActive` | Reserved; current paths return more specific paused or terminated errors. | reserved |
| 12 | `StreamNotPaused` | Resume is attempted when the stream is not paused. | reachable |
| 13 | `StreamAlreadyPaused` | Pause is attempted when the stream is already paused. | reachable |
| 14 | `StreamTerminated` | An operation requires a live stream but it is cancelled or depleted. | reachable |
| 15 | `StreamMatured` | Top-up is attempted after the accrual end time. | reachable |
| 16 | `InsufficientWithdrawable` | Explicit withdrawal exceeds a positive available balance. | reachable |
| 17 | `NothingToWithdraw` | A live stream has no currently withdrawable balance. | reachable |
| 18 | `InvalidAmount` | Explicit withdrawal or top-up amount is non-positive. | reachable |
| 19 | `BatchTooLarge` | Batch has more than `MAX_BATCH_SIZE = 16` IDs. | reachable |
| 20 | `EmptyBatch` | Batch has no stream IDs. | reachable |
| 21 | `DuplicateStreamId` | Batch contains the same stream ID more than once. | reachable |
| 22 | `Overflow` | Checked arithmetic overflows or underflows. | reachable |
| 23 | `TopUpTooSmall` | Positive top-up cannot extend the schedule by at least one second. | reachable |
| 24 | `StreamIdExhausted` | Next stream ID is `u64::MAX`; allocation cannot wrap. | reachable |
| 25 | `TokenTransferFailed` | Token contract returns a typed transfer error. | reachable |
| 26 | `TokenMissing` | Reserved; missing-token host abort cannot be distinguished in native tests. | reserved |
| 27 | `DelegateNotPermitted` | Delegate uses an operation not included in its grant. | reachable |
| 28 | `DelegateExpired` | Delegate uses a grant after its expiry. | reachable |
| 29 | `MalformedStreamId` | Reserved; typed `Vec<u64>` prevents malformed IDs reaching contract code. | reserved |
| 30 | `RepeatedTransfer` | Recipient transfer targets the current recipient. | reachable |
| 31 | `InvalidTopUp` | Reserved; non-positive top-ups are rejected as `InvalidAmount` first. | reserved |
| 32 | `TokenAmountMismatch` | Deposit pull changes pool balance by an unexpected amount. | reachable |
| 33 | `VestedDecreased` | Reserved; current mutation paths preserve non-decreasing vested value. | reserved |
| 34 | `PoolBalanceDrift` | A funds-moving operation found the pool's real token balance below the total Fluxora accounts for. | reachable |
| 34 | `ContractHalted` | A state-changing entry point was called while the contract-level halt is engaged. | reachable |
| 35 | `HaltOperatorAlreadySet` | `set_halt_operator` was called after an operator was already installed. | reachable |
| 36 | `HaltOperatorNotSet` | `halt`/`resume_contract` was called on a contract with no operator installed. | reachable |
| 37 | `ContractAlreadyHalted` | `halt` was called while the contract was already halted. | reachable |
| 38 | `ContractNotHalted` | `resume_contract` was called while the contract was not halted. | reachable |
| 33 | `VestedDecreased` | Reserved; defensive invariant — the randomized operation-sequence search in `test::vested_decreased` finds no path that lowers vested. | reserved |

`TokenTransferFailed` (25) and `TokenMissing` (26) are **stable stream-level categories** for token sub-invocation failures. The token contract's internal error discriminant is intentionally discarded — forwarding it would produce a value clients decode against Fluxora's error table, yielding a silent misinterpretation. The raw diagnostic is visible in the failed transaction's `diagnosticEvents`.

* `TokenTransferFailed` — the token contract returned a typed contract error: insufficient sender balance, pool underfunded on a payout, the token's own authorization rules refused the call, or the token returned a non-success value such as `false` rather than reverting. Fluxora treats all of those as a failed transfer, not a successful deposit.
* `TokenMissing` — the token address resolves to nothing (Abort / host trap); the stream references a non-deployed contract.
* `TokenAmountMismatch` (32) — a deposit pull (`create_stream`, `top_up`, `delegate_top_up`) changed the pool's balance by something other than the requested amount. See "Token assumptions" below.
* `PoolBalanceDrift` (34) — the pool's real token balance fell below the balance Fluxora accounts for. Raised by the reconciliation at the end of every operation that moves pool funds: `withdraw`, `delegate_withdraw`, `batch_withdraw`, `cancel`, `delegate_cancel`, `top_up` and `delegate_top_up`. A shortfall means the token changed balances outside a transfer Fluxora was party to — an elastic-supply rebase — and the whole invocation reverts rather than paying one recipient out of another's claim. A *surplus* is tolerated, never reported; see "Token assumptions" below.

The CLI and RPC render these as `Error(Contract, #N)`.

`DepositRateTooLow` (5) enforces a minimum rate of 1 token unit per second (`deposit >= end_time - start_time`). Below one unit per second, the per-second rate truncates to zero and the recipient accrues literally nothing until very late in the schedule. This is rejected to prevent a footgun where a treasury streams a small grant over a long duration and the recipient cannot withdraw anything.

`StreamNotActive` (11), `TokenMissing` (26), `MalformedStreamId` (29),
`InvalidTopUp` (31) and `VestedDecreased` (33) are reserved in the frozen ABI:
each is documented in the table above and in `test::error_reachability`, and
none of them has a reachable path through a public entry point. Do not
renumber or remove them.

`VestedDecreased` (33) is the one *defensive invariant* in that list rather than
a superseded error: `test::vested_decreased` searches randomized operation
sequences against paused, cliffed and near-maximum streams, observes the
contract error each call returns, and confirms the guard never fires. It is kept
because the invariant it protects is load-bearing, not because an integrator is
expected to handle it.

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

Fluxora treats amounts as raw integer token units and does not call the token's
`decimals()` method or rescale amounts. The caller must scale each amount using
the token's actual precision. For example, passing `1_000 * 10^7` to a
two-decimal token transfers and records that many smallest units (100,000,000
whole tokens), not 1,000 tokens. Because the requested transfer and received
balance delta still match, a decimal-assumption mismatch is not rejected.

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

**2. The token does not rebase.** An elastic-supply token changes the pool's
balance out from under the contract — up or down, on a schedule the contract
is not party to — desynchronizing the real balance from the sum of every
stream's `deposited - withdrawn`.

> **Shortfalls are enforced, surpluses tolerated.** There is no transfer to
> instrument, so a rebase cannot be caught as it happens. Instead Fluxora
> keeps a running per-token total of the balance it expects to hold
> (`DataKey::PooledBalance`) — every verified pull credits it, every payout
> and refund debits it — and reconciles that total against the token's own
> `balance` at the end of **every** operation that moves pool funds:
> `withdraw`, `delegate_withdraw`, `batch_withdraw`, `cancel`,
> `delegate_cancel`, `top_up` and `delegate_top_up`. A rebase between two
> operations is therefore caught by the next one, which reverts in full with
> [`Error::PoolBalanceDrift`](#error) (34).
>
> The test is `actual < expected`, not `actual != expected`. A **surplus** —
> a positive rebase, a donation, dust — is accepted, because it cannot cause
> an underpayment: every payout is sized by stream accounting and never by
> the pool balance, so the excess simply sits there unclaimed. Demanding
> equality would let anyone freeze the protocol by transferring a single unit
> into the contract, and a solvency check anyone can trip is worse than the
> risk it guards.
>
> Three gaps remain, and all are inherent. A rebase is only detected once an
> operation touches that token — nothing runs while the contract is idle. A
> positive rebase and a negative one on the same token that cancel out before
> the next operation are invisible, because the total is only ever compared
> against the token's own balance. And the expected total is built from
> deposits the contract itself verified, so a pool that already held a balance
> before this change reads as a surplus rather than being reconciled
> retroactively. See `docs/KNOWN-LIMITATIONS.md` §6.

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
the pool (standing in for a negative rebase) is rejected with
`Error::PoolBalanceDrift` rather than corrupting an unrelated stream's
accounting. `test::rebase_drift` builds the dedicated fixture: a token whose
balance can be overwritten with no transfer at all, a rebase between deposit
and withdrawal / top-up / cancel / batch withdrawal, per-token isolation, and
the tolerated-surplus boundary.

---

## Entry points

### Lifecycle

| function | auth | returns |
|---|---|---|
| `create_stream(sender, recipient, token, deposit, start_time, end_time, cliff_time, cancellable, pausable, transferable)` | sender | `u64` stream id |
| `create_stream_with_curve(sender, recipient, token, deposit, start_time, end_time, cliff_time, cancellable, pausable, transferable, curve)` | sender | `u64` stream id — [details](#create_stream_with_curve--detailed-reference) |
| `create_stream(sender, recipient, token, deposit, start_time, end_time, cliff_time, cancellable, pausable, transferable)` | sender | `u64` stream id — [details](#create_stream) |
| `create_stream_with_cliff_mode(sender, recipient, token, deposit, start_time, end_time, cliff_time, cliff_mode, cancellable, pausable, transferable)` | sender | `u64` stream id — [details](#create_stream_with_cliff_mode) |
| `create_stream(sender, recipient, token, deposit, start_time, end_time, cliff_time, cancellable, pausable, transferable, reference)` | sender | `u64` stream id |
| `top_up(stream_id, amount)` | sender | — |
| `withdraw(stream_id, amount: Option<i128>)` | recipient | `i128` paid |
| `batch_withdraw(recipient, stream_ids: Vec<u64>)` | recipient | `i128` total — [details](#batch_withdraw) |
| `withdraw(stream_id, amount: Option<i128>)` | recipient | `i128` paid — [details](#withdraw) |
| `batch_withdraw(recipient, stream_ids: Vec<u64>)` | recipient | `i128` total |
| `cancel(stream_id)` | sender | — |
| `batch_cancel(sender, stream_ids: Vec<u64>)` | sender | `BatchCancelOutcome` — [details](#batch_cancelsender-stream_ids-vecu64) |
| `pause(stream_id)` / `resume(stream_id)` | sender | — |
| `transfer_recipient(stream_id, new_recipient)` | recipient | — |
| `revoke_delegate(stream_id, grantor, delegate)` | sender or recipient | — |

`withdraw` with `amount = None` draws the full available balance.

#### `pause(stream_id)` — freeze accrual and the cliff gate

`pause(stream_id)` requires authorization from the stream's `sender`. The
`stream_id` parameter is an unsigned 64-bit integer in `[0, u64::MAX]`; it must
identify an existing stream. Stream ids are assigned monotonically starting at
`0` and are never reused, so in normal operation the valid ids are
`0..stream_count()`.

The stream must have been created with `pausable = true`, must not be
`Cancelled` or `Depleted`, and must currently be `Active`. On success, the
stream becomes `Paused` and its accrual clock freezes at the ledger timestamp
of the call. Wall-clock time that passes while paused does not increase
`vested_of` or the withdrawable balance, and does not allow a cliff to pass.
The recipient can still withdraw value accrued before the pause — including
value accrued at the pause instant itself: a stream paused exactly at its
cliff instant satisfies `stream_time == cliff_time` while frozen, the gate is
open, and the whole cliff amount is withdrawable
(`test::cliff::paused_exactly_at_the_cliff_instant_leaves_the_gate_open` pins
the boundary). When
`resume` is later called, the paused interval is added to `paused_total` and
the schedule continues from the same stream time, stretching the effective end
date by the paused duration.

**Errors.**

| variant | # | condition |
|---|---|---|
| `StreamNotFound` | 1 | No readable entry for `stream_id`: the id was never issued, or its entry has been archived. |
| `NotPausable` | 9 | The stream was created with `pausable = false`. |
| `StreamAlreadyPaused` | 13 | The stream is already `Paused`. |
| `StreamTerminated` | 14 | The stream is `Cancelled` or `Depleted`. Checked before the paused-state test, so a terminal stream that still carries a `paused_at` reports `StreamTerminated`, not `StreamAlreadyPaused`. |
| `VestedDecreased` | 33 | Defensive only: the post-pause accrual-sanity check observed a lower vested amount. Unreachable for any stream created through the contract. |

This list was cross-checked against `FluxoraStream::pause` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs); the five
variants above are the complete set it can return.

**Events.** Exactly one `paused` event on success: topics `stream_id` and
`sender`; payload `paused_at` (the pause-call ledger timestamp) and
`paused_total` (the cumulative paused seconds before this in-progress pause).

#### `resume(stream_id)` — un-pause and fold the paused interval into `paused_total`

`resume` reverses `pause`: it clears `paused_at`, moves the stream back to
`Active`, and adds the elapsed pause (`now - paused_at`) to `paused_total`. The
stored schedule — `start_time`, `end_time`, `cliff_time` — is **not** touched.
Because accrual is driven by `elapsed - paused_total`, the larger `paused_total`
shifts the effective end of the stream forward by exactly the time it spent
paused, so the clock picks up where it stopped and the total value delivered
over the stream's life is unchanged.

**On a `Schedule` stream, pausing also moves the cliff, in wall-clock terms.**
`cliff_reached` is evaluated against the same stream clock,
`stream_time(now) = (paused_at ?? now) - paused_total`, so a pause freezes the
cliff gate along with accrual: while the clock is frozen below `cliff_time` the
gate stays shut no matter how far the wall clock advances. After a resume,
`paused_total` has absorbed the whole paused interval, so the gate opens at
wall-clock `cliff_time + paused_total`, not at the stored `cliff_time`. A stream
paused for `P` seconds in total therefore has its cliff — and with it the
instant its recipient can first withdraw — pushed `P` seconds later, exactly as
its `end_time` is. The stored `cliff_time` field is never rewritten; only the
mapping from wall clock to stream clock changes.

**On a `WallClock` stream, none of that applies.** `cliff_reached` compares
`now` against `cliff_time` directly, so `paused_total` is not consulted and no
pause can move the gate. Accrual is still frozen while paused, so such a stream
that is paused across its cliff opens the gate on schedule and pays out its whole
pre-pause backlog at once when it resumes.
`test::cliff_mode::pausing_across_the_cliff_moves_a_schedule_cliff_but_not_a_wall_clock_one`
asserts both rules on the same timeline.

The `resumed` event publishes the post-resume `paused_total`, which is enough for
an integrator to recompute the moved instant as `cliff_time + paused_total` from
the `stream_created` schedule (or `get_stream`) alone, without replaying
individual pause intervals. That recomputation is only needed on a `Schedule`
stream. `test::cliff::pause_across_cliff_delays_the_wall_clock_cliff` and
`test::pause::pausing_across_the_cliff_defers_the_cliff_too` assert the
`Schedule` rule.

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

**Non-linear curves (#1815).** The rate-preserving extension above is the
`Linear` rule. A `Step` or `FrontLoaded` stream has no constant per-second rate
to preserve — varying the rate is the point of the curve — so `top_up` takes the
other safe option for those: it **keeps `end_time` where it is** and lets the
added deposit ride the same curve, scaled proportionally. `Step`'s tranches and
`FrontLoaded`'s shape are unchanged; only their amounts grow by the same
proportion as the deposit. `vested` is monotone in `deposited` for a fixed
curve, so this can only raise the recipient's claim, and `top_up` still
re-checks the no-regression guard before committing. The `topped_up` event
reports the (unchanged) `end_time` and the new `deposited`, so an indexer sees
which of the two rules applied without a second call.

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
| `VestedDecreased` | 33 | Defensive only: the post-resume accrual-sanity check observed a lower vested amount. Unreachable for any stream created through the contract; `test::vested_decreased` searches for a trigger and finds none. |

`NotPausable` (9) is a `pause`-only failure and is never returned by `resume`.
This list was cross-checked against `FluxoraStream::resume` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs); the five
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
| `PoolBalanceDrift` | 34 | After every payout landed, one of the batch's tokens held less than the balance Fluxora accounts for — the token changed balances outside a transfer Fluxora was party to. Checked once per distinct token, and the batch reverts in full. |

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
| `PoolBalanceDrift` | 34 | After the pull landed, the token's pool balance was still short of the total Fluxora accounts for — a rebase since the last operation on this token. The top-up reverts in full. |

This list was cross-checked against `FluxoraStream::top_up` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs) and the shared
`token_transfer` helper; the ten variants above are the complete set it can
return. `Unauthorized` (7) is **not** reachable here — auth failures abort in
the host before a typed error is produced.

**Events.** Exactly one `topped_up` event on success: topics `stream_id` and
`sender`; payload `amount` (this top-up), `deposited` (total after the call),
and `end_time` (extended schedule end). The token contract also emits its own
`transfer` event for the deposit.
#### `batch_cancel(sender, stream_ids: Vec<u64>)`

```rust
fn batch_cancel(env: Env, sender: Address, stream_ids: Vec<u64>) -> Result<BatchCancelOutcome, Error>
```

Winds down several cancellable streams in one call, refunding each stream's
unvested remainder to `sender`. The batch counterpart of `cancel`, and the mirror
of `batch_withdraw`: one shared party, one authorization for the whole vector,
and the same all-or-nothing failure model.

The return value is a struct, not a bare `i128`:

```rust
pub struct BatchCancelOutcome {
    pub refunded: i128,          // total refunded across the batch
    pub refused_index: Option<u32>,  // position in `stream_ids` that refused
    pub refused_reason: Option<u32>,  // an `Error` discriminant
}
```

A settled batch has `refused_index == None` and reports the total `refunded`. A
refused batch has `refunded == 0` and names the element that stopped it.

##### Parameters

| parameter | type | valid range / constraints |
|---|---|---|
| `sender` | `Address` | The account that authorises the call and receives every refund. It is compared **by value** against each `Stream.sender`; the batch is rejected with `Unauthorized` (7) unless all of them match. A failed signature surfaces as a host authentication failure, not a typed `Error`. |
| `stream_ids` | `Vec<u64>` | **1 to `MAX_BATCH_SIZE` (16) elements**, inclusive — the same ceiling and the same derivation as `batch_withdraw`: the binding constraint is the contract event budget, not entry or instruction counts. A cap-sized `batch_cancel` settles in 8 832 of 16 384 event bytes (~552 bytes per element, one `cancelled` event plus one token `transfer`), so the event budget alone would allow ~29 streams in a batch; the 16 cap leaves room but not the same 2x margin `batch_withdraw` has on that dimension (`test::resource_limits`). Every element must decode as a `u64` (`MalformedStreamId` otherwise) and name a distinct existing stream in `0..stream_count()` (`StreamNotFound` otherwise). Empty → `EmptyBatch`; more than 16 → `BatchTooLarge`; a repeated id → `DuplicateStreamId`. |

##### Authorisation

`sender.require_auth()` — the sender authorises **once** for the whole batch,
after the structural checks and before any storage write. The recipient cannot
call it, and neither can any other party.

##### Per-element behaviour

1. The batch is *resolved and validated in full* before any storage write or
   token call: existence, then ownership, then a price per element.
2. Every refund is priced against a **single ledger timestamp**, read once at the
   top of the call, and each stream is priced exactly once. Members with
   different deposits, durations, cliffs and pause histories therefore settle
   against one consistent "now" rather than drifting across the vector.
3. Streams need not share a token; each refund uses its own stream's token. The
   returned `refunded` is therefore a sum of amounts in different tokens — read
   per-stream figures from the `cancelled` events, not from the total.
4. An element with **nothing unvested** (already fully vested) still settles: it
   becomes `Cancelled`, and **no token transfer is issued** for it.

##### Refusals are reported by index

A stream that exists, belongs to `sender`, but cannot be cancelled — created with
`cancellable = false`, or already `Cancelled` or `Depleted` — does **not** raise
a typed `Error`. It refuses the batch and reports its own position:

| field | value on a refusal |
|---|---|
| `refused_index` | zero-based offset in `stream_ids` of the **first** element that refused |
| `refused_reason` | `NotCancellable` (8) or `StreamTerminated` (14) — which of the two conditions stopped that element |
| `refunded` | `0` — a refused batch changes nothing |

Elements are checked in batch order, so the report is deterministic for a given
vector: with two pinned streams in the batch, the earlier position is always the
one named. Dropping that element and resubmitting settles the rest.

**Why the index is not in the error.** A Soroban contract error crosses the wire
as a bare `u32` discriminant — `Error(Contract, #N)` — and `#[contracterror]`
enums can only carry unit variants, so no typed error can transport a position
alongside a discriminant. Reporting only the condition would leave a caller
holding a 16-id vector with no way to learn *which* element to drop, and a second
round-trip over the whole vector to find out. The index is data, so it travels
in the data.

##### Atomicity

All-or-nothing, and stricter than `batch_withdraw` in one respect: there are no
skips. Either every element is settled — each schedule collapsed, each refund
paid, each `cancelled` event emitted — or none is. A refused or failed batch
writes no storage, moves no tokens, extends no TTL, and emits no event, including
for elements that had already been processed by the time the refusal was found.

##### Events

* One `cancelled` event **per stream**, in batch order, with topics `stream_id`,
  `sender`, `recipient` and payload `refunded`, `vested`, `deposited`, `status`
  — the same payload a single `cancel` publishes.
* There is no aggregate or batch-level event; the return value is the only total.
* Each non-zero refund also triggers the token contract's own `transfer` event.

##### Failure modes

| error | # | condition |
|---|---|---|
| `StreamNotFound` | 1 | An id in `stream_ids` does not exist, or has been archived out of the live ledger. As with `batch_withdraw`, unknown ids fail the batch rather than being skipped. |
| `Unauthorized` | 7 | A resolved stream's `sender` differs from the `sender` argument. |
| `BatchTooLarge` | 19 | `stream_ids.len() > MAX_BATCH_SIZE` (16). |
| `EmptyBatch` | 20 | `stream_ids` contains no elements. |
| `DuplicateStreamId` | 21 | The same id appears more than once in the batch. |
| `Overflow` | 22 | Checked arithmetic overflows while accruing a stream's figures or summing the batch total. |
| `TokenTransferFailed` | 25 | A refund's token transfer was rejected by the token contract. The raw token discriminant is discarded — see the `Error` table above. |
| `TokenMissing` | 26 | A refund's token address does not resolve to a deployed contract (host `Abort`). No funds moved. |
| `MalformedStreamId` | 29 | A serialized element of `stream_ids` does not decode as a `u64`. |

`NotCancellable` (8) and `StreamTerminated` (14) are reachable only as
`BatchCancelOutcome::refused_reason`, never as a returned `Error`: a caller sees
them as data, with the index attached, rather than as a failure of the call.

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
* `PoolBalanceDrift` (34): After the refund (if any) left the pool, the token held less than the balance Fluxora accounts for — a rebase since this token's last operation. Checked even when the refund is zero, and the cancel reverts in full.

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
| `PoolBalanceDrift` | 34 | After the payout left the pool, the token held less than the balance Fluxora accounts for — a rebase since this token's last operation. The withdrawal reverts in full, so a pool that is merely *short* can never pay a recipient out of another stream's claim. |

This list was cross-checked against `FluxoraStream::withdraw` and
`FluxoraStream::apply_withdrawal` in
[`contracts/stream/src/lib.rs`](../contracts/stream/src/lib.rs), plus
`accrual::withdrawable` / `accrual::vested` and the shared `token_transfer`
helper; the nine variants above are the complete set the entry point can
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
#### `create_stream_with_curve` — detailed reference

Identical to [`create_stream`](#create_stream--detailed-reference) in every
respect — same authorization, same validation, same deposit pull, the same
`stream_created` event — except that the release shape above the cliff is chosen
by the trailing `curve` parameter instead of defaulting to `Linear`.

**Signature:**
```rust
fn create_stream_with_curve(
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
    curve: ReleaseCurve,
) -> Result<u64, Error>
```

**Authorization:** `sender.require_auth()`, exactly as `create_stream`.

**Parameters:** every parameter has the same meaning, valid range and error
behaviour as in `create_stream`; the only addition is:

| parameter | type | description |
|---|---|---|
| `curve` | `ReleaseCurve` | The release shape above the cliff. `0` `Linear`, `1` `Step`, `2` `FrontLoaded` — see [`ReleaseCurve`](#releasecurve). Always one of those three variants; the parameter is never rejected. Immutable after creation and reported by `get_stream` and `stream_created`. |

**Errors:** the error set is exactly `create_stream`'s. `curve` adds no failure
mode: every variant is supported, and the schedule guards (`InvalidDeposit`,
`InvalidTimeRange`, `InvalidCliff`, `DepositRateTooLow`, `Overflow`, …) are
enforced identically. In particular the creation-time guard that
`deposited × duration` fits in `i128` bounds every curve's accrual arithmetic
too, because each curve's release fraction never exceeds 1.

**Events:** one `stream_created`, with the same topics and the same payload as
`create_stream`, plus the `curve` field carrying the value passed here.

**Example.** The same 100-day, 1,000 USDC stream as above, on each curve:

| offset | `Linear` (`0`) | `Step` (`1`) | `FrontLoaded` (`2`) |
|---|---|---|---|
| day 24 | 240 USDC | 0 USDC | 423 USDC |
| day 25 | 250 USDC | 250 USDC | 438 USDC |
| day 50 | 500 USDC | 500 USDC | 750 USDC |
| day 75 | 750 USDC | 750 USDC | 938 USDC |
| day 100 | 1,000 USDC | 1,000 USDC | 1,000 USDC |

`Step` releases nothing until each quarter boundary and then jumps; `FrontLoaded`
leads `Linear` all the way to maturity; all three settle at exactly the deposit.
(`FrontLoaded` is evaluated on a 1/1000 grid, so its interior figures are the
grid values above — 438 rather than 437.5 — and its final instant is the full
deposit via the terminal settlement branch.)

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
    reference: Option<String>,
) -> Result<u64, Error>
```

**Authorization:** Requires `sender.require_auth()`. The sender's authorization on this invocation covers the nested token transfer; no prior token approval is needed.

**Parameters:**

| parameter | type | description |
|---|---|---|
| `sender` | `Address` | Funding party. Must authorize the call. |
| `recipient` | `Address` | Receiving party. Must differ from `sender`. |
| `token` | `Address` | Token contract address (SEP-41). Per-stream, not contract-wide. |
| `deposit` | `i128` | Initial amount to lock, in the token's smallest unit. The caller must scale for the token's actual `decimals()`; Fluxora does not query or normalize precision. Must be positive and satisfy rate constraints. |
| `start_time` | `u64` | Accrual begins (unix seconds). May be past (backdated vesting), present, or future (scheduled stream). |
| `end_time` | `u64` | Accrual ends (unix seconds). Must be strictly greater than `start_time`. |
| `cliff_time` | `u64` | Payout gate (unix seconds). Must be in `[start_time, end_time]`. Set equal to `start_time` for no cliff. **Gates payout, does not delay accrual** — at the cliff instant the recipient becomes entitled to everything accrued since `start_time`. How it is compared to the current time depends on `cliff_mode`, which `create_stream` fixes to `CliffMode::Schedule`: on a `pausable` stream this stored instant is then a lower bound, not the wall-clock instant the gate opens, because pausing pushes the opening instant forward by the accumulated `paused_total`. See the `resume` entry point and [`create_stream_with_cliff_mode`](#create_stream_with_cliff_mode). |
| `cancellable` | `bool` | Whether sender may cancel. Immutable after creation. |
| `pausable` | `bool` | Whether sender may pause accrual. Immutable after creation. |
| `transferable` | `bool` | Whether recipient may reassign the stream. Immutable after creation. |
| `reference` | `Option<String>` | Optional identifier for the stream (e.g., "payroll-001", "grant-xyz-q1"). Maximum 64 characters. Immutable after creation, returned by `get_stream` and included in the `StreamCreated` event. |

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
| `InvalidReferenceLength` (34) | Reference string exceeds `MAX_REFERENCE_LENGTH` (64 characters) |
| `InvalidDeposit` (4) | `deposit ≤ 0` |
| `InvalidTimeRange` (2) | `end_time ≤ start_time` (zero or negative duration) |
| `InvalidCliff` (3) | `cliff_time < start_time` or `cliff_time > end_time` |
| `DepositRateTooLow` (5) | `deposit < (end_time - start_time)`, causing per-second rate to truncate to zero |
| `Overflow` (22) | `deposit × (end_time - start_time)` does not fit in `i128` |
| `StreamIdExhausted` (24) | The stream-id counter has reached `u64::MAX`; no further ids can be allocated. Terminal for new creations. |
| `TokenTransferFailed` (25) | Token contract rejected the transfer (insufficient sender balance, authorization refused, or token's own rules). The token's internal error discriminant is intentionally discarded; see the root diagnostic in the transaction's `diagnosticEvents`. |
| `TokenMissing` (26) | The `token` address has no deployed contract (host `Abort` / trap). |

**Events:**

On success, emits `stream_created` with topics `[stream_id, sender, recipient]` and payload carrying the complete initial state: `token`, `deposited`, `start_time`, `end_time`, `cliff_time`, `cancellable`, `pausable`, `transferable`, `curve`. This is the canonical event for indexer discovery. `create_stream` always reports `curve = 0` (`Linear`).
On success, emits `stream_created` with topics `[stream_id, sender, recipient]` and payload carrying the complete initial state: `token`, `deposited`, `start_time`, `end_time`, `cliff_time`, `cliff_mode`, `cancellable`, `pausable`, `transferable`. This is the canonical event for indexer discovery. `cliff_mode` was appended last, so a decoder that stops after `transferable` still works against a v1 payload.
On success, emits `stream_created` with topics `[stream_id, sender, recipient]` and payload carrying the complete initial state: `token`, `deposited`, `start_time`, `end_time`, `cliff_time`, `cancellable`, `pausable`, `transferable`, `reference`. This is the canonical event for indexer discovery.

**Atomicity:**

Creation is transactional. The stream-id counter and count are advanced only after all validation and the token transfer succeed. A failed creation leaves the id space contiguous with no gaps, consumes no tokens, and emits no event.

**Special Cases:**

* **Backdated Start** (`start_time < now`): Legitimate for hire-date or grant-award vesting. The elapsed portion vests immediately and is withdrawable on the next ledger.
* **Future Start** (`start_time > now`): Scheduled stream. The deposit is escrowed; accrual begins at `start_time`. Nothing vests or is withdrawable before then.
* **Fully Elapsed Schedule** (`end_time ≤ now`): Accepted. Reads as fully vested immediately. The entry receives the minimum retention TTL floor.
* **No Cliff** (`cliff_time == start_time`): Standard continuous vesting with no payout gate.
* **Cliff at Maturity** (`cliff_time == end_time`): Single lump-sum payout when the stream completes.
* **Pause Moves the Cliff** (`pausable == true`): the stored `cliff_time` is never rewritten, but pausing shifts the wall-clock instant at which the gate opens by the accumulated paused time. A recipient's first withdrawal becomes available at `cliff_time + paused_total`, not at `cliff_time` — see the `resume` entry point. An integrator rendering "funds unlock at &lt;time&gt;" must add the stream's current `paused_total` rather than displaying `cliff_time` directly. Call `create_stream_with_cliff_mode` with `CliffMode::WallClock` to get a cliff that pausing cannot move.

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

With `cliff_mode = WallClock` the day-11 unlock instant above is exact even if the stream was paused; with the default `Schedule` mode and 3 days of accumulated pause, the first withdrawal lands on day 14 instead.

#### `create_stream_with_cliff_mode` — detailed reference

Identical to [`create_stream`](#create_stream) except that it takes one extra
parameter, `cliff_mode`, inserted directly after `cliff_time`. It is the only way
to create a stream whose cliff a `pause` cannot move.

**Signature:**
```rust
fn create_stream_with_cliff_mode(
    env: Env,
    sender: Address,
    recipient: Address,
    token: Address,
    deposit: i128,
    start_time: u64,
    end_time: u64,
    cliff_time: u64,
    cliff_mode: CliffMode,
    cancellable: bool,
    pausable: bool,
    transferable: bool,
) -> Result<u64, Error>
```

**Authorization:** Requires `sender.require_auth()`, exactly as `create_stream`.

**Parameters:** All of `create_stream`'s, with the same meanings, constraints and
errors, plus:

| parameter | type | description |
|---|---|---|
| `cliff_mode` | `CliffMode` | How `cliff_time` is compared to the current time. Immutable after creation. `Schedule` (0) gates on the stream clock, so `paused_total` pushes the opening instant forward by exactly the accumulated paused time. `WallClock` (1) gates on the ledger timestamp, so the opening instant is `cliff_time` no matter how much the stream is paused. |

**Errors:** Exactly `create_stream`'s — the mode is not a source of new
outcomes, so there is deliberately no `InvalidCliffMode` discriminant. A value
that is neither 0 nor 1 fails while the host is still decoding the arguments, as
a conversion error that aborts the invocation; it is never a contract-level
`Error`, and because the decode precedes all contract code it consumes no id,
transfers nothing, and leaves no state. `cliff_time` range validation is
unchanged and still applies in both modes: a mode changes *when* the gate is
judged, not *which* `cliff_time` values are legal.

**Relationship to `create_stream`:** `create_stream` is a thin wrapper that
passes `CliffMode::Schedule`. It is not deprecated, and the two are byte-for-byte
equivalent for `Schedule`; `create_stream` exists so a v1 caller and a v2 caller
can create the same stream.

**What the mode does not change.** The cliff remains a gate on *payout*, never on
*accrual*, in both modes: before the gate opens, `withdrawable_of` is `0` even
though `vested_of` has been rising since `start_time`. And a `pause` still stops
accrual in both modes. Choosing `WallClock` therefore removes the sender's
ability to move the unlock instant; it does not let funds accrue while paused, so
a wall-clock stream paused across its cliff releases its whole pre-pause backlog
in one withdrawal on resume.

**Events:** identical to `create_stream`, including `stream_created` carrying the
`cliff_mode` that was actually used.
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
| `halted()` | `bool` | no | no |
| `halt_operator()` | `Option<Address>` | no | no |

> **⚠ RPC read-skew caveat — all view functions**
>
> Public Soroban RPC endpoints are load-balanced across nodes that may be at
> different ledger heights at the same instant. Two consecutive view calls can
> therefore observe different ledgers, and a derived figure that *combines* two
> calls (e.g. `vested_of + refundable_of`) may be arithmetically impossible
> even when the on-chain contract is perfectly correct.
>
> **This is not a contract bug.** Each node returns a valid, internally consistent
> view of a real ledger — it is just a different ledger from the one the previous
> call observed.
>
> **Recommended mitigation for integrators:**
> 1. **Prefer `get_stream`** (single call) over calling `withdrawable_of`,
>    `vested_of`, and `refundable_of` separately. One call is one ledger by
>    construction — there is nothing to pin.
> 2. **When you must combine multiple view calls**, read `latestLedger` from
>    each response and discard the set if any two responses disagree.
> 3. **After a write**, do not read immediately. Sample `getLatestLedger` until
>    several consecutive responses all report a ledger at or beyond your
>    transaction's ledger; only then issue the view call.
> 4. **In tests against a public endpoint**, assert within a justified tolerance
>    rather than demanding exact equality across separate simulation calls.
> 5. **Run your own RPC node** if you need strict read-after-write consistency.
>
> See [`docs/soroban-rpc-read-skew.md`](soroban-rpc-read-skew.md) for a full
> write-up including the original incident, measurement data, and code examples
> for all five mitigations.

#### `vested_of(stream_id)` — total earned by the recipient, withdrawn or not

> **⚠ RPC read-skew:** calling `vested_of` and `refundable_of` (or
> `withdrawable_of`) in separate simulation calls against a public RPC endpoint
> can observe different ledger heights, producing a conservation sum that does
> not equal `deposited`. Use `get_stream` to obtain all three figures from a
> single ledger. See [`docs/soroban-rpc-read-skew.md`](soroban-rpc-read-skew.md)
> and the [Views caveat](#views--read-only-no-ttl-side-effects) above.

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

  The formula above is the `Linear` curve. On a `Step` or `FrontLoaded` stream
  the released fraction is the curve's, but the *direction* is the same: every
  curve floors, so the residue still stays in the pool and returns to the sender
  at settlement. The residue can be up to a full grid step on `FrontLoaded`
  rather than one stroop, which is why a curve-aware client must read `curve`
  from `get_stream` rather than assuming the linear formula. See
  [`ReleaseCurve`](#releasecurve).

* **Pre-cliff it is zero.** The cliff *gates* the payout, it does not delay
  accrual: before `cliff_time` the result is exactly `0`, and at the cliff
  instant the recipient becomes entitled to everything accrued since
  `start_time` — not merely what accrues after the cliff. There is no partial
  vesting beforehand. On a `Schedule` stream (the default) "before `cliff_time`"
  is measured **on the stream clock**, so on a stream that has been paused the
  wall-clock instant the gate opens is `cliff_time + paused_total`: a recipient
  whose stream was paused across its cliff waits the total paused duration
  longer before anything is vestable. On a `WallClock` stream the comparison is
  against the ledger timestamp and `paused_total` is not consulted, so no pause
  can delay the gate. See the `resume` entry point.

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
plus accumulated and in-progress pause time) plus a 30-day buffer, inflated by a 20% close-time safety margin, converted
at 5 seconds per ledger (the measured mean — §5 of KNOWN-LIMITATIONS.md)
rounding up, floored at `MIN_STREAM_TTL_LEDGERS` (622,080 ledgers, a 30-day
floor plus the margin) and clamped to the network's `max_entry_ttl`.
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

`delegate_transfer_recipient` is the authorisation-gated twin of
`transfer_recipient`. Once the grant is verified it repeats the direct path's
four stream-level guards — `NotTransferable` (10), `StreamTerminated` (14),
`SelfStream` (6) and `RepeatedTransfer` (30) — in the same order, so both entry
points reject the same stream states with the same discriminants. The only
variants it can add are the two grant checks above and the host-level
authorisation trap when the delegate does not sign. Because the grant check runs
before the stream is read, an id that was never issued — and therefore has no
grant — is rejected as `DelegateNotPermitted` (27) rather than
`StreamNotFound` (1).

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

### Emergency halt

The stream contract has no admin and no upgrade path. The **emergency halt** is
the single, deliberately narrow exception (issue #1818): an operator can stop
*settlement* across every stream while an exploit or a stuck integration is
handled, without gaining any power over funds.

| function | auth | returns |
|---|---|---|
| `set_halt_operator(operator)` | the named `operator` | — |
| `halt()` | the installed operator | — |
| `resume_contract()` | the installed operator | — |
| `halted()` | none | `bool` |
| `halt_operator()` | none | `Option<Address>` |

**Scope — what the halt does and does not do.**

* It refuses **every state-changing entry point** with `ContractHalted` (34):
  `create_stream`, `top_up`, `withdraw`, `batch_withdraw`, `cancel`, `pause`,
  `resume`, `transfer_recipient`, `grant_delegate`, `revoke_delegate`, all six
  `delegate_*` variants, `extend_stream_ttl` and `batch_extend_ttl`. `set_halt_operator`,
  `halt` and `resume_contract` are the only mutations that still run, which is
  what makes the stop reversible.
* **Reads are unaffected.** `get_stream`, `withdrawable_of`, `vested_of`,
  `refundable_of`, `stream_count`, `stream_exists`, `halted` and
  `halt_operator` answer normally for the whole duration. Integrators can keep
  observing balances and stream state during an incident.
* It **settles nothing**. No funds move, no stream is cancelled or paused, and
  no schedule changes. Accrual is a pure function of ledger time, so streams
  keep vesting while halted — `withdrawable_of` continues to climb; what stops
  is the ability to act on it. Lifting the halt resumes from exactly the state
  that was halted.
* It is **contract-wide**, not per-stream: `pause` remains the per-stream,
  sender-authorised tool that also freezes accrual.

**Operator lifecycle.**

`set_halt_operator` may be called **exactly once per deployment**. There is no
rotation entry point — a second call returns `HaltOperatorAlreadySet` (35), so
an operator cannot be replaced mid-incident, and a leaked operator key cannot
be handed off; recovering means deploying a new contract (the contract is not
upgradeable, so that is true of every change). The setter is opt-in: a
deployment that never calls it has no operator, cannot be halted, and behaves
exactly as it did before this entry point existed.

The operator's powers are exactly `halt()` and `resume_contract()`. It cannot
withdraw, cancel, pause, top up, transfer a recipient, grant a delegation, or
change a stream. It is not consulted by any read method.

**There is no timeout.** The halt ends only when the operator calls
`resume_contract`, which is deliberately explicit: an automatic expiry would
reopen settlement while the incident may still be live.

**Errors.**

| # | error | condition |
|---|---|---|
| 34 | `ContractHalted` | Any state-changing entry point while halted. Checked before authorization and every other precondition. |
| 35 | `HaltOperatorAlreadySet` | `set_halt_operator` after the operator was installed. |
| 36 | `HaltOperatorNotSet` | `halt`/`resume_contract` with no operator installed. |
| 37 | `ContractAlreadyHalted` | `halt` while already halted. |
| 38 | `ContractNotHalted` | `resume_contract` while not halted. |

A failed `require_auth()` on `set_halt_operator`, `halt` or `resume_contract`
is a host authorisation trap rather than a typed contract error, so it is not
listed above.

**Events.**

| event | topics after the name | payload |
|---|---|---|
| `halt_operator_set` | `operator` | — |
| `contract_halted` | `operator` | `halted_at` |
| `contract_resumed` | `operator` | `resumed_at`, `halted_for` |

Each transition emits exactly one event, so an indexer can reconstruct the
halted window from the event stream alone. `test::halt` drives the whole
matrix: unconfigured contract, one-shot install, the operator-only gate on
both transitions, every mutating entry point refused with 34, every read still
answering, and resumption restoring settlement.

---

## Events

Declared with `#[contractevent]`; schemas are in the deployed spec. First topic
is the snake_case event name; second is the routing key — always `stream_id`
for stream-scoped events, and `operator` for the contract-level halt events,
which are not about any one stream.

| event | topics after the name | payload |
|---|---|---|
| `stream_created` | `stream_id`, `sender`, `recipient` | `token`, `deposited`, `start_time`, `end_time`, `cliff_time`, `cancellable`, `pausable`, `transferable`, `curve` |
| `stream_created` | `stream_id`, `sender`, `recipient` | `token`, `deposited`, `start_time`, `end_time`, `cliff_time`, `cliff_mode`, `cancellable`, `pausable`, `transferable` |
| `stream_created` | `stream_id`, `sender`, `recipient` | `token`, `deposited`, `start_time`, `end_time`, `cliff_time`, `cancellable`, `pausable`, `transferable`, `reference` |
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
| `halt_operator_set` | `operator` | — |
| `contract_halted` | `operator` | `halted_at` |
| `contract_resumed` | `operator` | `resumed_at`, `halted_for` |

Every payload carries enough state to reconstruct the stream without replaying
from genesis. Field order and topic placement are ABI.

`resumed` deserves one note: its `paused_total` is the post-resume cumulative
figure, and on a `Schedule` stream it is enough to recompute the moved cliff
instant, `cliff_time + paused_total`, given the `cliff_time` carried by
`stream_created` (or read back from `get_stream`). The event deliberately does
not republish `cliff_time`, which never changes; see the `resume` entry point. A
`WallClock` stream needs no such recomputation — `stream_created` already
carries both the `cliff_time` and the `cliff_mode` that say which rule applies.

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
`vested_of + refundable_of == deposited`) will intermittently disagree.

The primary affected method is `withdrawable_of` — because its return value is
derived from `vested - withdrawn` evaluated at a ledger the caller cannot pin
without reading the full `Stream` struct, any off-chain calculation that calls
`withdrawable_of` and `vested_of` (or `refundable_of`) separately against a
load-balanced endpoint can observe a stale ledger on one of the two calls and
produce an arithmetically impossible result. The same hazard affects
`vested_of` and `refundable_of` when called separately.

**Recommended approach:** use `get_stream` to read the full `Stream` struct in
one call and derive `withdrawable`, `vested`, and `refundable` locally from
`deposited`, `withdrawn`, `start_time`, `end_time`, and `paused_total`. This
is how the Fluxora SDK is written, and it is the canonical mitigation. See
[`docs/soroban-rpc-read-skew.md`](soroban-rpc-read-skew.md) for the full
technical write-up, including measurement data and code examples.

**2. Handle archived streams.** `stream_exists(id) == false` while
`id < stream_count()` means the entry has been archived, not that it never
existed. Surface a restore action rather than an error. See
[KNOWN-LIMITATIONS.md](KNOWN-LIMITATIONS.md) §1.

