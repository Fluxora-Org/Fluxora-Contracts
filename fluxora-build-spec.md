# Fluxora — Build Spec (v1.1)

**A continuous payment streaming primitive for Soroban.**

This document is written to be handed to a coding agent. It contains the context needed to
make good decisions, the design decisions already made (and why), and the parts that are
genuinely hard. Read all of section 3 before writing any contract code — several obvious
implementations are wrong on Soroban specifically.

---

## 0. Amendments (v1.0 → v1.1, 2026-08-12)

v1.0 was written before the contract existed. Four things in it turned out to disagree
with what the implementation and its measurements showed. They are corrected inline below;
this section records what changed and why so the reasoning is not lost.

| # | § | v1.0 said | v1.1 says | why |
|---|---|---|---|---|
| 1 | 2.4, 2.5 | `effective_now = min(now, end_time + paused_total)` | a **stream clock** that reads `paused_at` | The v1.0 formula is correct only *after* a resume. During an in-progress pause the current interval has not yet been folded into `paused_total`, so accrual keeps running and the freeze does not freeze. |
| 2 | 1, 3.2 | "roughly 200 ledger entry reads" bounds batch size | the **contract event budget** bounds it; entry limits are far away | The 200-reads figure predates protocol 23. Live Soroban state is now held in memory. Measured: a 16-stream batch uses 43/400 footprint entries but 8,192/16,384 event bytes. |
| 3 | 2.4, 3.3 | dust accumulates; bound it and document it | dust is **exactly zero** at settlement | Computing `vested` from the cumulative formula rather than per-interval deltas means truncation never accumulates. `vested(t) + refundable(t) == deposited` holds exactly, at every instant. |
| 4 | 2.7 | (open question) | `top_up` extends duration, rounding the extension **down** | Discovered after correction 3 was accepted. Rounding *up* lowers the rate and retroactively reduces already-vested value, letting `withdrawn` exceed `vested` and letting `cancel` refund the sender money the recipient already holds. Found by the randomized suite; see §3.3. |

Corrections 1 and 3 were raised and accepted before implementation continued. Correction 2
came out of the stage 3 measurements. Correction 4 came out of the stage 3 randomized suite
and reverses a choice made in stage 2.

---

## 1. Context

### What this is

A Soroban smart contract that lets a sender lock tokens and have them accrue continuously to a
recipient over time. The recipient pulls their accrued balance whenever they like. Think
Sablier or Superfluid, built natively for Stellar.

### What this is not

Not a payroll app. Not a dashboard product. Fluxora is the **layer other things build on** —
payroll tools, grant programs, subscription billing, vesting schedules. The contract is the
product; the UI is a reference implementation that proves the contract works.

This distinction should drive every scoping decision. When in doubt, make the contract more
general and the UI thinner.

### Ecosystem context (why these choices matter)

- **Stellar has no scheduler.** There is no cron, no keeper network, no way for a contract to
  wake itself up. Every state change must be triggered by an external transaction. This is why
  the design is *pull-based*: the recipient calls `withdraw`, and the contract computes what
  they've earned at that instant. Nothing runs in the background.
- **Soroban storage expires.** Persistent entries have a TTL and are archived if not extended.
  A stream running twelve months will outlive its default TTL. This is the single hardest
  problem in this project and the main reason existing attempts are hackathon-grade.
- **Soroban has hard per-transaction resource limits.** **[Amended — see §0.2.]** The
  often-quoted "~200 ledger entry reads" figure is pre-protocol-23. Since protocol 23 live
  Soroban state is held in memory, and the surviving `disk_read_entries` limit (still 200)
  counts only entries restored from disk plus non-Soroban entries such as classic account
  balances — for a contract touching live state it is usually zero. The limits that actually
  bind are enumerated in §3.2. The underlying warning is unchanged and still correct: any
  design that iterates over a growing collection on-chain will work in testing with five
  streams and fail in production with five hundred.
- **Soroban prohibits reentrancy.** A contract cannot call back into itself. This removes a
  whole class of bugs but also means some EVM patterns don't translate.

### Prior art to be aware of

Two unfunded MVP repos exist on GitHub under the name StellarStream, both built as Drips
Stellar Wave bounties. Both implement linear accrual and pull withdrawal. Neither addresses
TTL/rent, resource limits under load, or precision guarantees. **That gap is the entire
opportunity** — do not replicate a basic MVP, build the production-grade version.

---

## 2. Core design

### 2.1 Custody model

One contract instance manages many streams. Tokens are pooled in the contract's own balance;
per-stream accounting is tracked in contract storage.

**Implication to keep in mind:** the pooled balance must always be greater than or equal to the
sum of all unwithdrawn stream balances. Write an invariant test for this and run it after every
operation in the test suite.

### 2.2 Data model

```rust
#[contracttype]
#[derive(Clone)]
pub struct Stream {
    pub sender: Address,
    pub recipient: Address,
    pub token: Address,          // SEP-41 token contract
    pub deposited: i128,         // total ever deposited (incl. top-ups)
    pub withdrawn: i128,         // total ever withdrawn by recipient
    pub start_time: u64,         // unix seconds
    pub end_time: u64,           // unix seconds
    pub cliff_time: u64,         // == start_time when no cliff
    pub cancellable: bool,       // set at creation, immutable
    pub pausable: bool,          // set at creation, immutable
    pub transferable: bool,      // set at creation, immutable  [resolved §7.4]
    pub paused_at: Option<u64>,
    pub paused_total: u64,       // cumulative paused seconds
    pub status: StreamStatus,
}

#[contracttype]
#[derive(Clone, PartialEq)]
pub enum StreamStatus { Active, Paused, Cancelled, Depleted }

#[contracttype]
pub enum DataKey {
    NextStreamId,       // instance storage
    Stream(u64),        // persistent storage, one entry per stream
}
```

`cancellable`, `pausable` and `transferable` are fixed at creation and **must never be mutable
afterwards**. This is a trust feature: a recipient can verify before accepting a stream that the
sender cannot claw it back, freeze it, or reassign it. A stream that can become cancellable
later is worthless as a guarantee.

> **Implementation note.** v1.0 listed a `Config` key in instance storage. With no admin, no
> fees and no upgradeability (all §6 non-goals) there is nothing to configure, so the key was
> dropped rather than stored empty. Instance storage holds only `NextStreamId`.

`StreamStatus` note: `Cancelled` is **sticky**. A cancelled stream that is subsequently drained
to zero stays `Cancelled` rather than becoming `Depleted`, so the indexer can distinguish "ran
to completion" from "sender clawed back the remainder". Both are terminal.

### 2.3 Storage strategy — read this twice

- **Instance storage:** the stream ID counter only. Tiny, shares the contract's TTL.
- **Persistent storage:** one entry per stream, keyed `DataKey::Stream(id)`. Independent TTLs.
- **Temporary storage:** not used.

**Do NOT store a per-user list of stream IDs on-chain.** This is the mistake that kills these
projects. A `Vec<u64>` of a user's streams grows without bound, costs rent forever, and blows
the transaction footprint limit once a treasury has a few hundred recipients.

Stream discovery is an **off-chain concern**. The contract emits events; an indexer consumes
them and answers "show me my streams." This is exactly what the Horizon listener and Postgres
in the Fluxora architecture are for. On-chain, a stream is only ever addressed by its ID.

Consequence: `create_stream` must return the new `u64` ID, and the creation event must carry
sender, recipient, and ID so the indexer can build the mapping.

State this as a test, not just a policy: the Nth stream must cost exactly what the 1st did.

### 2.4 Accrual math

Use `i128` throughout (the SEP-41 token interface uses `i128` for amounts). All arithmetic must
be checked — use `checked_add`, `checked_sub`, `checked_mul`, and panic with a typed error on
overflow rather than wrapping.

**[Amended — see §0.1.]** Everything is expressed against a **stream clock** that stops while
the stream is paused:

```
stream_time     = (paused_at.unwrap_or(now)) - paused_total   // saturating
elapsed         = clamp(stream_time, start_time, end_time) - start_time
duration        = end_time - start_time          // guard: must be > 0 at creation
vested          = if stream_time < cliff_time { 0 }
                  else { deposited * elapsed / duration }   // integer division, rounds down
withdrawable    = vested - withdrawn
```

> **Why not `effective_now = min(now, end_time + paused_total)`.** That formulation, in v1.0,
> is correct only *after* a resume. While a pause is in progress the current interval has not
> yet been added to `paused_total`, so the expression keeps advancing and the stream keeps
> accruing while supposedly frozen. Reading `paused_at` is what makes the freeze actually
> freeze. The same fix is what stops a stream paused across its cliff from silently passing
> the cliff while frozen — the cliff is gated on `stream_time`, not on `now`.

Rules that must hold:

- **Always round down.** Integer division truncating in the recipient's disfavour is correct.
- **Clamp `vested` to `deposited`.** Never let rounding or a clock edge case produce more than
  was deposited. Assert this explicitly, don't rely on the math.
- **`vested` must never decrease.** Monotonicity is load-bearing: if `vested` can fall, then
  `withdrawn` can exceed it, and `cancel` (which sets `deposited = vested`) will drive the
  stream's liability negative and refund the sender funds the recipient already holds. This is
  not hypothetical — see §0.4 and §2.7.
- **Reject `end_time <= start_time` at creation** — division by zero.
- **No bound on clock skew.** `start_time` may be any distance in the past
  (backdated vesting; the elapsed portion vests immediately) or in the future
  (scheduled streams). The ledger timestamp is the only clock on chain, and a
  skew limit would be business policy, not a protocol requirement. A future
  stream may extend beyond the `max_entry_ttl` horizon: creation funds the
  entry for as long as the network allows and the permissionless keeper path
  covers the rest, exactly as for multi-year streams. Only well-formedness is
  validated: `end > start`, and `cliff ∈ [start, end]`.
- **Handle `duration == 0` after a cancel.** Cancelling at the instant of creation collapses
  the schedule to a point. Return `deposited` (which the cancel has already reduced to the
  vested amount) rather than dividing by zero.
- **Enforce a minimum deposit relative to duration.** If `deposited < duration`, the per-second
  rate rounds to zero and the recipient accrues nothing until very late. Reject at creation
  with a clear error; this is a real footgun for a treasury streaming a small grant over a year.
- **Cliff gates, it does not delay.** At `cliff_time` the recipient becomes able to withdraw
  everything accrued since `start_time`, not since the cliff. This matches standard vesting
  semantics. Document it prominently — it surprises people.

**[Amended — see §0.3.] Dust is exactly zero, not merely bounded.** v1.0 said dust accumulates
in the contract and is returned to the sender at settlement. In fact:

```
vested(t) + refundable(t) == deposited      for all t, exactly
```

This falls out of computing `vested` from the cumulative formula rather than by summing
per-interval deltas: truncation error is re-derived from scratch on every call instead of
accumulating, is bounded by one stroop at any instant, and vanishes entirely at settlement
because `refundable` is *defined* as the complement of `vested`. The per-interval
implementation — the obvious one, and the one the existing MVPs use — loses a stroop per
withdrawal and strands it in the pool forever. Keep the cumulative form.

### 2.5 Pause semantics

Pausing is subtle and easy to get wrong. The model: pausing freezes accrual and pushes the
effective end forward by the paused duration. Total value delivered stays constant; the
schedule stretches.

- On `pause`: set `paused_at = Some(now)`, status `Paused`.
- On `resume`: `paused_total += now - paused_at`, `paused_at = None`, status `Active`.
- All accrual math runs on the stream clock in §2.4, which reads `paused_at`.
- The recipient can still `withdraw` while paused — pausing stops *accrual*, it does not freeze
  already-earned funds. Freezing earned funds would make pausable streams unacceptable to any
  serious recipient.
- The cliff is gated on stream time, so a pause spanning the cliff defers the cliff too.
- **Depletion must close out an in-progress pause.** A stream paused after maturity and then
  drained becomes `Depleted`, which is terminal — `resume` is rejected — so leaving `paused_at`
  set strands it reporting "Depleted" and "frozen" at once with nothing able to clear it. Fold
  the interval into `paused_total` and clear `paused_at`, exactly as `cancel` does.

Only the sender may pause, and only if `pausable == true`.

### 2.6 Cancellation

The elegant implementation avoids a new state machine:

1. Compute `vested` at the current instant.
2. `refund = deposited - vested`; transfer `refund` to the sender.
3. Set `deposited = vested` and `end_time = stream_time`, clamped to be no earlier than
   `start_time` so a cancel before the stream opens cannot invert the schedule.
4. Clear `paused_at`; set status `Cancelled`.

The stream now looks like a fully-matured stream whose accrual has stopped. The recipient's
`withdraw` path needs no special-casing — they pull `vested - withdrawn` exactly as before.

Note step 3 uses **stream time**, not wall-clock `now`: cancelling a paused stream must settle
against the frozen clock, or the recipient is credited for time the stream was not running.

Only the sender may cancel, and only if `cancellable == true`.

### 2.7 Function surface

```rust
// Lifecycle
fn create_stream(env, sender, recipient, token, deposit, start, end, cliff,
                 cancellable, pausable, transferable) -> u64
fn top_up(env, stream_id, amount)              // sender auth; extends duration at a fixed rate
fn withdraw(env, stream_id, amount: Option<i128>) -> i128   // None = withdraw max
fn batch_withdraw(env, recipient, stream_ids) -> i128
fn cancel(env, stream_id)
fn pause(env, stream_id)
fn resume(env, stream_id)
fn transfer_recipient(env, stream_id, new_recipient)   // recipient auth

// Views (read-only)
fn get_stream(env, stream_id) -> Stream
fn withdrawable_of(env, stream_id) -> i128
fn vested_of(env, stream_id) -> i128
fn refundable_of(env, stream_id) -> i128
fn stream_count(env) -> u64
fn stream_exists(env, stream_id) -> bool

// Maintenance
fn extend_stream_ttl(env, stream_id) -> u32    // permissionless, see 3.1
fn batch_extend_ttl(env, stream_ids) -> u32    // permissionless
```

**`top_up` semantics: extend the duration, hold the rate.** The per-second rate the recipient
agreed to never changes; `end_time` moves forward by `amount / rate`. The alternative — hold
`end_time` and raise the rate — retroactively re-vests elapsed time, so a top-up at the halfway
point would instantly increase what is already withdrawable. Holding the rate fixed means a
top-up can never accelerate or dilute an existing schedule, which is what makes it safe to
accept a stream from an untrusted sender.

**[Amended — see §0.4.] The duration extension must round DOWN.** This is the opposite of what
was implemented in stage 2 and it is not a cosmetic choice:

- Rounding **up** makes the new duration slightly longer than exact, so the rate falls slightly,
  so `vested` at the current instant *decreases*. A recipient who had already withdrawn at the
  old rate now holds more than `vested`. A subsequent `cancel` sets `deposited = vested`, making
  `deposited < withdrawn` — negative liability, and the sender is refunded money the recipient
  already took. Caught by the randomized suite at a 93-stroop discrepancy.
- Rounding **down** guarantees `vested` never decreases. The residual is at most one second of
  schedule, in the recipient's favour.

Consequence: a top-up too small to buy one second of schedule cannot extend the duration at all,
and absorbing it would require raising the rate. Reject it (`TopUpTooSmall`) rather than
re-vesting retroactively.

Also reject `top_up` on a stream whose accrual clock has reached `end_time`: extending a matured
stream makes the new funds instantly or near-instantly withdrawable, which is never what the
sender means. Create a new stream instead.

### 2.8 Authorization

- `create_stream`, `top_up`, `cancel`, `pause`, `resume` → `sender.require_auth()`
- `withdraw`, `batch_withdraw`, `transfer_recipient` → `recipient.require_auth()`
- `extend_stream_ttl`, `batch_extend_ttl` → no auth (see 3.1)

Both classic keypairs and custom `__check_auth` smart accounts must work. Test with both.
Smart-account compatibility is a selling point — a treasury can wrap `create_stream` in an
OpenZeppelin-style policy that caps how much can be committed per period.

Test authorization from both directions, and **do not hardcode mock sub-invocation trees**:
they drift with every signature change and decay into false failures. Assert positively by
inspecting `env.auths()` for the expected address, and negatively by revoking all mocked auth.

### 2.9 Events

Every state change emits an event. The indexer depends entirely on these, so getting them right
is not optional polish.

`stream_created`, `withdrawn`, `cancelled`, `paused`, `resumed`, `topped_up`,
`recipient_transferred`, `ttl_extended`.

Each event carries `stream_id` plus the fields needed to reconstruct state without replaying
from genesis. `stream_created` must carry sender, recipient, token, deposit, and all timestamps.

Declare events with `#[contractevent]` rather than `env.events().publish()`. The macro is the
current idiom (`publish` is deprecated in SDK 27) and it embeds each event's schema in the
contract's interface spec, so the indexer and TypeScript SDK can generate typed decoders from
the deployed contract instead of hand-rolling topic parsers.

Field order and topic placement are ABI. Adding a field is compatible; reordering or
re-topicking one is not.

---

## 3. The hard parts

These are the differentiators. Existing implementations skip all three.

### 3.1 TTL, rent, and archival

**The problem.** Persistent storage entries have a time-to-live measured in ledgers. When TTL
runs out the entry is archived and becomes unreadable until restored. A stream running twelve
months will outlive its initial TTL unless something extends it.

If a stream entry archives, the *tokens are not lost* — they sit in the contract's pooled
balance. But the accounting entry that says who they belong to is inaccessible until restored.
That is unacceptable UX for a payroll or grant primitive and must be engineered around.

**The strategy:**

1. **Extend on every touch.** Every function that writes a stream calls `extend_ttl` on that
   entry before returning. An actively-used stream never expires. Read-only views deliberately
   do *not*: they run under simulation, where a footprint write is at best noise.
2. **Extend generously at creation.** Target the remaining stream duration plus a large buffer,
   converted from seconds to ledgers (~5s per ledger; do not hardcode, derive and document).
   Clamp to the network `max_entry_ttl` — you cannot exceed it, so a multi-year stream will
   need periodic extension regardless. Read the achievable maximum from the SDK
   (`storage().max_ttl()`), not from `LedgerInfo::max_entry_ttl`; they differ by one.
3. **Permissionless `extend_stream_ttl`.** Anyone can pay to keep any stream alive. This lets
   the Fluxora backend run a cheap keeper that sweeps streams approaching expiry, and it means
   a recipient is never dependent on the sender's goodwill to keep their claim readable.
   There is nothing to grief: the caller only ever *pays* rent, and TTL extension can neither
   move funds nor change stream state.
4. **Document the restore path.** If an entry does archive, it is recoverable via
   `RestoreFootprint` before invocation. The SDK should detect archived streams and surface a
   one-click restore rather than an opaque error. `stream_exists() == false` for an id below
   `stream_count()` is the signal to key on.

**Deliverable:** a test that fast-forwards the ledger past the default TTL and proves a stream
survives via the keeper path, and a second test proving an archived stream restores correctly
with balances intact.

> **Critical caveat on the second deliverable — do not skip.** The SDK test host runs storage
> in *recording* mode, where reading an expired persistent entry is **silently auto-restored**
> (`handle_maybe_expired_entry`) rather than failing. Unit tests can therefore prove that the
> archive→restore boundary preserves accounting, but they **cannot** reproduce the live-network
> flow, where the transaction fails first and the caller must resubmit with a
> `RestoreFootprint` operation. A green suite here does not mean TTL is solved.
>
> Closing that gap on live testnet — let an entry genuinely archive, observe the real failure,
> restore it, prove the stream still pays out — is the **acceptance criterion for stage 4**, not
> a nice-to-have. See `KNOWN-LIMITATIONS.md`.

### 3.2 Resource limits

**[Amended — see §0.2.]** Protocol 27 mainnet limits, as enforced by the SDK:

| limit | value |
|---|---|
| `ledger_entries` (total footprint: disk reads + memory reads + writes) | 400 |
| `disk_read_entries` | 200 |
| `write_entries` | 200 |
| `instructions` | 400,000,000 |
| `contract_events_size_bytes` | 16,384 |

- A single-stream `withdraw` touches the instance entry, one stream entry, and the token
  contract's entries — measured at 13 footprint entries. Comfortably within limits. Good.
- **Batch operations are where this breaks**, but not where v1.0 expected. Entry counts would
  allow well over a hundred streams per batch. The **contract event budget** binds first: each
  stream in a `batch_withdraw` emits a `withdrawn` event *plus the token contract's own
  `transfer` event*, roughly 512 bytes between them, so the hard ceiling is about 32 streams.
- **Design decision:** support a bounded `batch_withdraw(stream_ids: Vec<u64>)` with a hard cap.
  Reject oversized batches with a clear error rather than failing opaquely at the network level.
  The SDK chunks larger requests client-side.

**`MAX_BATCH_SIZE = 16.`**

> **Measured 2026-08-12, protocol 27, soroban-sdk 27.0.5, against the Stellar Asset Contract.**
> At 16 streams: 43/400 footprint entries, 20/200 writes, ~4.6M/400M instructions,
> **8,192/16,384 event bytes**. Sixteen is the ~32 event-budget ceiling with a 2x safety factor.
>
> **This number will drift, and it is not a pure protocol constant.** Roughly half the
> per-stream event cost belongs to the *token*, so a SEP-41 token with a heavier event payload
> moves the ceiling down — which is what the 2x margin is for. Re-derive it, do not adjust it by
> feel. The SDK's enforced limits are a snapshot taken at publication, not a live query, so an
> SDK upgrade can move them silently.

Add a test that measures resource consumption at the batch cap and asserts headroom, and have it
**print** what it measures rather than only asserting. Resource costs change across protocol
versions — this test is your regression alarm, and a number in a CI log is what makes drift
visible before it becomes an outage.

### 3.3 Precision and the invariant

- Write a property test: for random `(deposited, duration, cliff, withdrawal schedule)`, the sum
  of everything the recipient withdraws plus everything refunded to the sender must equal
  `deposited` **exactly** (see §0.3 — the correct bound is zero, not "under a documented
  bound"). Also assert monotonicity of `vested`, that rounding is down and tight to one stroop,
  and that a `top_up` never reduces `vested`.
- Write the pool invariant test described in 2.1 and run it as a post-condition on every
  operation in the suite.
- **Write a randomized operation-sequence test.** Hand-written cases only cover sequences
  somebody thought of. Drive long random sequences of every entrypoint — including calls
  expected to be rejected — through the real contract, and re-check every invariant after
  *every* operation. Seed it deterministically so failures replay. This is not optional polish:
  it found both of the real bugs in v1, and neither was reachable by the hand-written cases.
    - a stream paused after maturity and then drained became `Depleted` with `paused_at` still
      set, permanently stuck (§2.5);
    - `top_up` rounding drove `vested` backwards, letting `withdrawn` exceed it (§0.4, §2.7).
- Wire the randomized and property suites into CI with a larger budget than a local run, and a
  nightly schedule larger still. A fuzzer that only runs on a developer's laptop is a fuzzer
  that stops running.
- Test the adversarial cases explicitly: withdraw at exactly `cliff_time`, withdraw at exactly
  `end_time`, cancel one second after creation, cancel at the instant of creation, cancel after
  full vesting, pause and resume across the cliff, top up a cancelled stream (must reject),
  withdraw from a depleted stream (must be a no-op or typed error, never a panic).

---

## 4. Stack and repo layout

| Repo | Contents |
|---|---|
| `Fluxora-Contracts` | Rust / Soroban. The product. |
| `Fluxora-Backend` | TypeScript, Express, Postgres. Indexer + keeper + API. |
| `Fluxora-Frontend` | React. Reference dashboard and recipient portal. |

Add a fourth as soon as the contract stabilises: **`fluxora-sdk`** (TypeScript). This is what
integrators actually consume, and it is what makes Fluxora a primitive rather than an app. It
should wrap contract calls, handle batch chunking, detect archived entries, and expose typed
stream objects. Generate its types from the deployed contract's interface spec (§2.9) rather
than hand-writing them.

Token interface: SEP-41. USDC on Stellar has **7 decimals** — do not assume 6 or 18. Support any
SEP-41 token, default the UI to USDC. Note that the token is **per stream**, not a single
contract-wide configured token.

Always build against the current stable Soroban SDK, and pin the major version to the protocol
the target network actually runs — `soroban-sdk`'s major version tracks the Stellar protocol
version. As of 2026-08-12 both testnet and mainnet are on **protocol 27**, so `soroban-sdk 27.x`
and the `wasm32v1-none` target. Check before starting; do not trust a version number from
training data or from an older tutorial. Note the `stellar` CLI is versioned the same way and
must match.

See `MIGRATION.md` for the audit of what the pre-v1 contract set exposed and what downstream
repos still call into it.

---

## 5. Build order

Strictly sequential. Do not start a stage before the previous one's tests pass.

1. **Contract core.** Data model, create, withdraw, view functions. Full unit tests including
   the property test and pool invariant. No cliff, no pause, no cancel yet. — **done**
2. **Lifecycle.** Cliff, cancel, pause/resume, top-up, recipient transfer. Extend the test suite
   to every adversarial case in 3.3. — **done**
3. **TTL and limits.** Everything in 3.1 and 3.2, with the two TTL tests as the gate. — **done**
4. **Testnet deploy.** Deploy, publish the contract ID, write a minimal CLI that exercises every
   function against live testnet. Ship this publicly — it is your credibility artifact.
   **Acceptance criterion: the live archival restore round-trip in §3.1.** — *in progress*
5. **Indexer and SDK.** Event consumption into Postgres, keeper for TTL sweeps, TypeScript SDK.
6. **Reference UI.** Last. It is a demo, not the product.

Mapping to SCF tranches, if that submission goes ahead: stages 1–3 are tranche 1 (MVP), 4–5 are
tranche 2 (testnet), and tranche 3 is audit remediation plus mainnet launch.

---

## 6. Explicit non-goals for v1

Say no to these. Each one is a plausible-sounding addition that would double the timeline and
weaken the audit story.

- No admin key, no upgradeability, no pause-everything switch on the core contract.
  Immutability is a *feature* for a primitive — it is what lets another protocol depend on you.
- No fee mechanism. Add it later behind a separate contract if ever.
- No on-chain stream discovery or per-user registries (see 2.3).
- No multi-token streams. One token per stream.
- No unlock curves other than linear. Cliff plus linear covers the real use cases.
- No cross-chain anything.
- No withdrawal rate limiting or claim caps in the core. If wanted, they belong in a policy
  contract wrapping `withdraw` — see `MIGRATION.md` §3.
- No delegated withdrawal with bespoke nonce-and-signature schemes. Smart accounts
  (`__check_auth`) cover the legitimate cases.

---

## 7. Open questions — all resolved

1. **`top_up` semantics — extend duration, or raise rate?** → **Extend the duration at a fixed
   rate**, with the extension rounded **down**. §2.7.
2. **Batch cap for `batch_withdraw`.** → **16**, derived from the contract event budget, not the
   entry count. Measured 2026-08-12 on protocol 27. §3.2.
3. **Minimum deposit-to-duration ratio.** → **`deposited >= duration`**, i.e. at least one
   stroop per second. A year-long USDC stream needs only ~3.16 USDC to clear it, so the floor
   excludes nothing realistic. §2.4.
4. **Should `transfer_recipient` be disableable at creation?** → **Yes**, via an immutable
   `transferable` flag alongside `cancellable` and `pausable`. Compliance-bound senders —
   payroll, KYC'd grant programs — need the payee pinned, and without it they could not use
   Fluxora at all. §2.2.

---

## 8. Instructions for the coding agent

- Read sections 2 and 3 fully before writing code. The Soroban-specific constraints in section 3
  invalidate several standard EVM streaming patterns.
- Verify the current Soroban SDK version and API surface against live documentation before
  starting. Do not rely on remembered APIs — this ecosystem moves fast and tutorials go stale.
  Verify the *live network protocol version* too, and pin the SDK major to match.
- Write tests alongside each function, not at the end. The property test, the pool invariant
  test and the randomized sequence test are not optional.
- When a design choice in this spec seems wrong, stop and raise it rather than silently
  deviating. Several decisions here are non-obvious and were made for reasons stated inline —
  and §0 records four that turned out to be wrong anyway, so the instruction is meant literally.
- Never let a numeric edge case panic. Every arithmetic operation is checked; every failure mode
  returns a typed error.
- When a measurement contradicts a number in this spec, trust the measurement, fix the spec, and
  record the date and protocol version next to the new number.
