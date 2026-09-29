# Architecture

One place to read how Fluxora fits together: what the components are, who is
trusted to do what, and how a payment actually moves from a sender's token
account to a recipient's.

The document is _checked_. `contracts/stream/src/test/architecture.rs` parses
this file and asserts its claims against the workspace manifest, the committed
ABI inventory (`contracts/stream/abi/fluxora_stream.json`), the `DataKey` enum
in `contracts/stream/src/types.rs`, and the authoritative entry point table in
[`audit.md`](audit.md). A claim here that stops being true fails CI rather than
misleading the next reader, so if you change the contract, change the table that
describes it in the same commit.

---

## 1. What Fluxora is

A single Soroban contract that holds **payment streams**: a sender deposits once,
and the deposit accrues to a recipient linearly over a fixed schedule until it
can be withdrawn. The contract is a payment primitive, not a platform.

Four properties shape everything below:

- **No admin and no upgrade path.** There is no `init`, no owner, no settable
  parameter and no migration entry point. The deployed WASM is frozen, so the
  only authority a stream answers to is the parties named in it.
- **No funds are ever held by discretion.** At every instant, the pool a stream
  is backed by equals the sum of what is still owed — nothing is parked, and
  there is no fee, no sweep and no treasury.
- **A schedule is immutable except by `top_up`.** Only `top_up` may change a
  stream, and only by extending its duration at the existing rate. Nothing can
  accelerate, dilute or retime a stream the recipient already relies on.
- **The ABI is frozen.** `ABI_VERSION` gates additive-only changes; see
  [`ABI.md`](ABI.md) for the compatibility rules the inventory test enforces.

---

## 2. Components

| Component            | Path                                       | Ships?      | Role                                                                                                                                                                                                                                                                                                              |
| -------------------- | ------------------------------------------ | ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Stream contract      | `contracts/stream`                         | **product** | The deployed contract: streams, delegation, TTL maintenance. Everything in §4–§9 describes this crate.                                                                                                                                                                                                            |
| Archival probe       | `contracts/archival-probe`                 | no          | A throwaway canary whose only job is to archive on the network's schedule, so the live archival/restore round trip can be observed. Never released, never deployed to mainnet.                                                                                                                                    |
| Provenance tool      | `tools/provenance`                         | no          | A host-only binary (`fluxora-provenance`) that hashes released wasm and verifies it against the recorded `provenance.json` and `SHASUMS`. A workspace member so it resolves through the root lockfile and is covered by the normal workspace checks, but excluded from wasm-target builds because it needs `std`. |
| Release tooling      | `script/release.sh`                        | no          | The only command that produces release artifacts. Builds the product package alone and fails if a probe wasm appears among its outputs.                                                                                                                                                                           |
| CI and validation    | `script/`, `.github/workflows/ci.yml`      | no          | Wasm size budget, coverage gate, migration/ABI consistency, doc alignment, gas validation, snapshot drift.                                                                                                                                                                                                        |
| Generated bindings   | `contracts/stream/abi/fluxora_stream.json` | **product** | The ABI inventory generated from the contract spec; CI fails when it is stale. This is what downstream code generators consume.                                                                                                                                                                                   |
| Client SDK           | `fluxora-sdk` (separate repo)              | **product** | Transaction construction, stream decoding, archival detection. The contract has no intent of its own about this layer.                                                                                                                                                                                            |
| Backend and frontend | separate repos                             | **product** | Index the `streams` projection from events and render it. The contract does not know they exist.                                                                                                                                                                                                                  |

Two structural consequences:

- the probe is a workspace member precisely so `cargo test --workspace`,
  `cargo fmt --all` and `cargo clippy --all-targets` keep covering its smoke
  test, while `script/release.sh` builds `-p fluxora-stream` and nothing else;
- the ABI inventory is a first-class artifact, because a frozen contract with no
  upgrade path has no second chance to fix a binding.

---

## 3. Trust boundaries

Nothing in the system holds a key that can act on a stream it is not a party to.
There are five addresses that matter, and no sixth:

| Party              | Authority                                                                                                                   | Enforced by                                                                                                             |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| **Sender**         | `top_up`, `pause`, `resume`, `cancel` on its own streams; granting and revoking sender-side delegation                      | `require_auth` on `Stream.sender`, plus the `pausable` / `cancellable` capability flags fixed at creation               |
| **Recipient**      | `withdraw` and `batch_withdraw`; `transfer_recipient` where `transferable`; granting and revoking recipient-side delegation | `require_auth` on the calling recipient and on the stream's current recipient                                           |
| **Delegate**       | Exactly the operations in a live, unexpired `(stream_id, delegate)` grant — no more, never the grant itself                 | Grant check, then `require_auth` on the delegate; a delegate can neither grant to itself nor use a bit it does not hold |
| **Token contract** | Moving the funds a stream pulls or pays out. Trusted by _interface_ (SEP-41) and assumed non-rebasing, non-fee-on-transfer  | Every deposit is asserted to deliver exactly the requested amount; a failing token fails the whole call closed          |
| **Anyone**         | Reading every view, extending any stream's TTL, and planting the archival canary                                            | No auth on views and TTL maintenance: they cannot move value, so they cannot be abused by being permissionless          |

There is deliberately no registration, no allowlist, no pause-the-protocol
switch and no operator: a stream's parties are the whole trust model.

---

## 4. Data flow

### Creating a stream

```
sender ──create_stream(sender, recipient, token, deposit, start, end, cliff,
                        cancellable, pausable, transferable)──▶ contract
                                                                  │
                            1. validate schedule, cliff, deposit │
                            2. pull `deposit` from `sender` ─────┼──▶ token.transfer(sender → contract)
                               and assert the amount arrived    │
                            3. write DataKey::Stream(id)        │
                            4. bump NextStreamId + StreamCount  │
                            5. emit `created`                   ▼
```

The token is named **per stream**. The contract never holds a configured token:
a stream that names a token which does not exist cannot be created at all,
because creation is the first thing that would pull through it.

### Withdrawing

```
recipient ──withdraw(id, Some(n) | None)──▶ contract
                                              │
            1. load Stream(id), check status  │
            2. available = vested(id, now) - withdrawn   (clamped at 0)
            3. push min(n, available) to the recipient ──▶ token.transfer(contract → recipient)
            4. stream.withdrawn += payout; emit `withdrawn`
```

`None` means "everything accrued so far". A payout that the token rejects reverts
the whole invocation — including the `withdrawn` increment and the event — and is
reported as a typed stream error, never as a raw host failure.

### Cancelling, and funding

`cancel` settles the stream against `min(now, end)`, refunds the unvested
remainder to the sender and marks the stream terminal; it is gated on the
`cancellable` flag fixed at creation. `top_up` extends `end_time` by an amount of
time bought at the stream's existing rate:

```
delta = floor(amount × duration ÷ deposited)      // floor, never ceiling
```

Floor keeps the rate from being raised, and a top-up too small to buy a single
second is rejected (`TopUpTooSmall`) rather than absorbed — either alternative
would re-vest elapsed time retroactively.

### Archival and TTL

Every persistent entry — `DataKey::Stream(id)`, `DataKey::Delegate(id, addr)`
and `DataKey::DelegateCount(id)` alike — is written with a TTL floor of
**30 days**, refreshed on write and
opportunistically on read. The network's own minimum (`min_persistent_ttl`,
~7 days) is far below that, so a live Fluxora stream never archives.

The consequence is handled explicitly rather than assumed away:

- if an entry does archive anyway, invoking fails at the network level before the
  contract body runs —
  the caller resubmits with `RestoreFootprint` and reads the same data back;
- `stream_exists` is the predicate that tells _never existed_ from _archived,
  needs restoring_, so a client can offer a restore instead of surfacing a raw
  error;
- the archival probe exists because that failure mode cannot be reproduced
  in-process: the SDK test host auto-restores an expired entry on read. See
  [`KNOWN-LIMITATIONS.md`](KNOWN-LIMITATIONS.md) §1 and
  [`archival-canary.md`](archival-canary.md).

---

## 5. Storage model

Every storage key declared by `DataKey`:

| Key                               | Storage    | Contents                                                                                              |
| --------------------------------- | ---------- | ----------------------------------------------------------------------------------------------------- |
| `DataKey::NextStreamId`           | instance   | Monotonic counter; incremented only on successful creation                                            |
| `DataKey::StreamCount`            | instance   | Streams successfully created; incremented in the same transaction as `NextStreamId` and the new entry |
| `DataKey::PooledBalance(Address)` | instance   | Expected aggregate balance for one token                                                               |
| `DataKey::Stream(u64)`            | persistent | The stream record: parties, token, deposit, schedule, flags, status, `withdrawn`                      |
| `DataKey::StreamCurve(u64)`       | persistent | Non-linear release curve; absent means `Linear`                                                        |
| `DataKey::Delegate(u64, Address)` | persistent | A delegation grant: operation bitmask and optional expiry                                             |
| `DataKey::HaltOperator`           | instance   | Optional one-time emergency halt operator                                                              |
| `DataKey::HaltedAt`               | instance   | Halt start timestamp; absent while the contract is not halted                                          |
| `DataKey::DelegateCount(u64)`     | persistent | Number of stored delegate grants on the stream; bounded at 16 and kept alive with the stream          |

Two properties follow from this layout, and both are load-bearing:

- **There is no per-user index.** Nothing groups streams by sender or recipient.
  Cost is therefore independent of how many streams exist, which is the guarantee
  `test::resource_limits` pins and the reason v1 has no `get_recipient_streams`
  or equivalent.
- **Grants are per `(stream_id, delegate)`.** Two delegates are two entries; the
  permission bits are not a shared budget, so granting one delegate a bit tells
  you nothing about any other. Renaming or reordering a variant silently moves
  every on-chain entry's address, which is why
  `test::storage_keys` snapshots them.
- **Delegate storage is bounded.** `MAX_DELEGATES_PER_STREAM` is 16. The
  side-car count is updated on first grant and revocation; replacing a grant
  does not consume a slot, and expired grants count until their entries are
  removed. Existing immutable deployments are unaffected. A future migration
  importing more than 16 grants must grandfather those entries and initialize
  the exact count; new delegates are blocked until revocation brings the count
  below the cap.

---

## 6. Entry point surface

**16 core entry points plus 8 delegation entry points**, as `MIGRATION.md` §3
states. Grouped by what they touch:

| Group           | Entry points                                                                                                                                                       |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Lifecycle       | `create_stream`, `top_up`, `withdraw`, `batch_withdraw`, `cancel`, `pause`, `resume`, `transfer_recipient`                                                         |
| Delegation      | `grant_delegate`, `revoke_delegate`, `delegate_withdraw`, `delegate_cancel`, `delegate_pause`, `delegate_resume`, `delegate_top_up`, `delegate_transfer_recipient` |
| Views           | `get_stream`, `stream_count`, `stream_exists`, `withdrawable_of`, `vested_of`, `refundable_of`                                                                     |
| TTL maintenance | `extend_stream_ttl`, `batch_extend_ttl`                                                                                                                            |

A batch is capped at `MAX_BATCH_SIZE` (16) ids on every batch entry point, and
the ceiling is checked before ids are resolved and before authorization.

---

## 7. Events

Events are the only channel to any downstream component; there is no on-chain
subscription and no callback. Each one names the `stream_id` so an indexer can
project a single stream's history without a global order, and none of them
carries a party's full record.

| Event                   | Emitted when                                      |
| ----------------------- | ------------------------------------------------- |
| `created`               | A stream is created                               |
| `withdrawn`             | A payout is made, including from `batch_withdraw` |
| `topped_up`             | Duration is extended at the existing rate         |
| `cancelled`             | A stream settles and the remainder is refunded    |
| `paused` / `resumed`    | Accrual is frozen or unfrozen                     |
| `recipient_transferred` | The recipient changes                             |
| `ttl_extended`          | An entry's TTL is refreshed                       |

A failed payout emits nothing at all: the event is written after the token call
returns, so a reverted transfer takes the event with it.

---

## 8. Deliberately not here

Recorded here because their absence is an architectural decision, not an
oversight — each is argued at length in [`MIGRATION.md`](MIGRATION.md) §3 and §7:

- an admin key, an upgrade path, protocol fees, or any global pause;
- on-chain stream discovery and per-user indexes;
- scheduled or keeper-driven withdrawal (the delegated surface covers the
  keeper case without a second scheduler);
- withdrawal rate limiting, which would give a stream a way to reject a
  recipient who is genuinely owed money;
- delegated withdrawal as a _signed message_ (`delegated_withdraw`), which is
  scoped to v1.1 with its own threat model — a smart-account recipient covers the
  same ground today through `__check_auth`.

---

## 9. Keeping this document honest

`contracts/stream/src/test/architecture.rs` asserts:

- the component table matches the workspace members declared in `Cargo.toml`, and
  the release path builds the product package only;
- the storage table names every `DataKey` variant and no others;
- the entry point groups are exactly the committed ABI's function names, each
  listed once, split into the documented 16 core and 8 delegation;
- the trust boundary claims match the per-entry point authority labels in
  [`audit.md`](audit.md); and
- every "not here" entry is genuinely absent from the ABI.

If a change makes one of those claims false, the test names the claim. Update
this file in the same change.
