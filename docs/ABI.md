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
[MIGRATION.md](../MIGRATION.md).

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

### `Error`

Discriminants are ABI and are never renumbered; new variants are appended.

| # | name | | # | name |
|---|---|---|---|---|
| 1 | `StreamNotFound` | | 13 | `StreamAlreadyPaused` |
| 2 | `InvalidTimeRange` | | 14 | `StreamTerminated` |
| 3 | `InvalidCliff` | | 15 | `StreamMatured` |
| 4 | `InvalidDeposit` | | 16 | `InsufficientWithdrawable` |
| 5 | `DepositRateTooLow` | | 17 | `NothingToWithdraw` |
| 6 | `SelfStream` | | 18 | `InvalidAmount` |
| 7 | `Unauthorized` | | 19 | `BatchTooLarge` |
| 8 | `NotCancellable` | | 20 | `EmptyBatch` |
| 9 | `NotPausable` | | 21 | `DuplicateStreamId` |
| 10 | `NotTransferable` | | 22 | `Overflow` |
| 11 | `StreamNotActive` | | 23 | `TopUpTooSmall` |
| 12 | `StreamNotPaused` | | | |

The CLI and RPC render these as `Error(Contract, #N)`.

---

## Entry points

### Lifecycle

| function | auth | returns |
|---|---|---|
| `create_stream(sender, recipient, token, deposit, start_time, end_time, cliff_time, cancellable, pausable, transferable)` | sender | `u64` stream id |
| `top_up(stream_id, amount)` | sender | — |
| `withdraw(stream_id, amount: Option<i128>)` | recipient | `i128` paid |
| `batch_withdraw(recipient, stream_ids: Vec<u64>)` | recipient | `i128` total |
| `cancel(stream_id)` | sender | — |
| `pause(stream_id)` / `resume(stream_id)` | sender | — |
| `transfer_recipient(stream_id, new_recipient)` | recipient | — |

`withdraw` with `amount = None` draws the full available balance.

### Views — read-only, no TTL side effects

| function | returns |
|---|---|
| `get_stream(stream_id)` | `Stream` |
| `withdrawable_of(stream_id)` | `i128` |
| `vested_of(stream_id)` | `i128` |
| `refundable_of(stream_id)` | `i128` |
| `stream_count()` | `u64` — ids run `0..stream_count()` |
| `stream_exists(stream_id)` | `bool` |

### Maintenance — permissionless

| function | returns |
|---|---|
| `extend_stream_ttl(stream_id)` | `u32` ledgers now funded |
| `batch_extend_ttl(stream_ids: Vec<u64>)` | `u32` entries extended |

`MAX_BATCH_SIZE = 16` for both batch functions. Chunk client-side; the SDK does
this automatically. See [README](../README.md) for why the cap is 16 and why it
is derived from the *event* budget rather than the entry count.

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
| `ttl_extended` | `stream_id` | `extended_to_ledgers` |

Every payload carries enough state to reconstruct the stream without replaying
from genesis. Field order and topic placement are ABI.

Note that `batch_withdraw` emits one `withdrawn` event **per stream drawn from**,
not one per call, and skips streams with nothing available — so a batch of 16
may emit fewer than 16 events.

---

## Resolved schema questions

Both were open against `Fluxora-Backend` in [MIGRATION.md](../MIGRATION.md) §5
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
[KNOWN-LIMITATIONS.md](../KNOWN-LIMITATIONS.md) §1.
