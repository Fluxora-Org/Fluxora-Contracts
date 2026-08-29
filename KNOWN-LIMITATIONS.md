# Known limitations

What a green test suite here does **not** prove. Read this before treating any
part of Fluxora as production-ready.

---

## 1. The TTL suite does not prove the archival recovery flow

**Status: open. Closing it is the acceptance criterion for stage 4.**

### The claim you might read off a green suite

`test::ttl` passes. It contains
`a_year_long_stream_survives_on_keeper_sweeps_alone` and
`an_archived_stream_restores_with_its_accounting_intact`. It is tempting to
conclude "TTL is solved". **It is not.** Roughly half the problem is untested.

### Why

The Soroban SDK's test host runs storage in *recording* mode. In that mode,
reading an expired persistent entry does not fail. The host calls
`handle_maybe_expired_entry`, which silently restores the entry in place with
its data intact and its TTL reset to `min_persistent_entry_ttl`:

```rust
// soroban-env-host-27.0.1/src/host/storage.rs
if live_until < li.sequence_number {
    match durability {
        ContractDataDurability::Temporary  => { /* entry dropped */ }
        ContractDataDurability::Persistent => {
            // recorded as a ReadWrite access, live_until reset to the minimum
        }
    }
}
```

On a real network the sequence is different, and there is a failure in the
middle of it:

| | test host | live network |
|---|---|---|
| read an archived entry | silently restored, invocation proceeds | **transaction fails** |
| recovery | n/a — never failed | caller must resubmit with a `RestoreFootprint` operation |
| after recovery | entry live at minimum TTL | entry live at minimum TTL |

So the tests exercise the *endpoints* of the journey — a live entry before, a
live entry with intact accounting after — and skip the failure in between.

### What the tests therefore do and do not establish

**Do establish:**

- Rent arithmetic is correct: creation funds a stream for its full remaining
  life plus a 30-day buffer, clamped to `max_entry_ttl`.
- Every mutating call re-extends the entry, so an active stream never decays.
- A year-long stream whose rent cannot be bought in one go survives on
  permissionless keeper sweeps, and pays out in full afterwards.
- Crossing the archive/restore boundary preserves every field of the accounting
  — deposit, withdrawals, schedule, status — with the pool still fully backing
  it, and the pooled tokens are never affected by TTL at all.

**Do not establish:**

- That a client hitting an archived stream gets a recoverable, diagnosable
  failure rather than an opaque one.
- That the `RestoreFootprint` footprint we would build is correct and
  sufficient.
- What the restore actually costs.
- That `stream_exists() == false` while `stream_id < stream_count()` is a
  reliable "needs restoring" signal against a real RPC, as the SDK is intended
  to use it.

### Closing it — in progress, canary planted 2026-08-12

Genuine archival cannot be observed quickly on *any* network. Measured
2026-08-12, testnet and local quickstart carry identical settings:

| setting | ledgers | at 5s/ledger |
|---|---|---|
| `min_persistent_ttl` | 120,960 | **7 days** |
| `max_entry_ttl` | 3,110,400 | 180 days |
| Fluxora's own floor (`MIN_STREAM_TTL_LEDGERS`) | 518,400 | 30 days |

The 7-day figure is a *network* floor applied at entry creation — no contract
can undercut it. Fluxora's 30-day floor sits on top, so a real stream entry
cannot archive for a month. That floor is deliberate and stays: a settled stream
must remain readable for the recipient's unclaimed tail and the indexer's final
state.

Two things are therefore running in parallel.

**1. Testnet canary — clock started 2026-08-12.** `contracts/archival-probe` is a
throwaway contract that writes one persistent entry and *deliberately never
extends its TTL*, so it receives exactly `min_persistent_ttl` and archives as
early as the network allows. The restore mechanism is a property of the ledger,
not of the contract, so proving it there proves it for `DataKey::Stream(id)`.

| | |
|---|---|
| probe contract | `CB4XJYNXQ62TCXI3GKCVBWADTSTFWYL3ZLYS3MKYPWRANOSADRZG4A7N` |
| canary planted | ledger 4,097,334 |
| archives after | ledger 4,218,293 |
| expected | ~2026-08-19 09:39 UTC |

Run `script/archival-canary.sh` any time for status; after that ledger, run it
with `--restore` to perform and verify the round trip. It asserts each step:
the entry stops being readable, invocation *fails* rather than returning stale
data, `RestoreFootprint` recovers it, and the value comes back intact.

**2. Config-upgraded local network — not yet built.** A `min_persistent_ttl`
lowered via a stellar-core config upgrade would make the round trip provable in
minutes and repeatable in CI, rather than a once-a-week manual check. There is
no CLI support for applying a `ConfigUpgradeSet`, so this needs the core admin
endpoint directly. Tracked as the remaining stage 4 work.

Until one of those lands, **this section stays open** and nothing should claim
TTL is solved.

### What we will say in each outcome — decided in advance

Written down before the result is known, so the conclusion cannot be quietly
reshaped to fit whatever happens.

**Outcome A — the entry archives and the restore round trip works.**
This section closes. The claim we then make, and its exact limits:

> Fluxora's archival recovery path is verified end to end against live Stellar
> testnet: an entry was allowed to archive, the subsequent read failed at the
> network level, a `RestoreFootprint` operation recovered it, and the stored
> value came back intact.

That is a headline claim and it is a real differentiator — no other Soroban
streaming implementation has demonstrated it. It still does **not** claim that a
Fluxora *stream* archived: the probe is a separate contract, and the argument
that the result transfers is that restore is a property of the ledger entry, not
of the contract that wrote it. State that reasoning whenever the claim is made
rather than letting the audience assume a stream was involved.

**Outcome B — the network auto-restores, and reads never fail.**
Then the recording-mode behaviour the unit suite relies on turns out to match
the network, and this entire limitation was narrower than we thought. We say so
publicly, in those words, and we **narrow the claim rather than reframing it**:
the honest statement becomes "archival is not a failure mode on this network for
persistent entries", the SDK's restore-detection path becomes dead code and gets
deleted, and this section is rewritten to record that the concern did not
materialise. We do not retro-fit the finding into a success story.

**Outcome C — the entry does not archive on schedule.**
Eviction is a background scan and lags `live_until`, so a delay of hours or days
is expected and is not an outcome in itself. The canary script distinguishes
this case explicitly and exits without a verdict. Re-run rather than concluding
anything. If it is still unarchived a week past `live_until`, that is itself a
finding worth writing up — it would mean testnet eviction is effectively not
running, and mainnet behaviour should not be inferred from it.

In all three cases the result is reported, not just the convenient ones.

### If you are integrating before then

Assume archived streams are reachable and that your first call against one will
fail. Detect it (`stream_exists() == false` with `stream_id < stream_count()`)
and surface a restore action rather than an error toast. Run a keeper against
`batch_extend_ttl` so it rarely comes up.

---

## 2. Resource measurements understate a real deployment

`test::resource_limits` registers contracts **natively**, not as WASM. Wasm
instantiation and execution costs are therefore skipped, so reported
`instructions` are lower than production. Ledger entry counts and event bytes —
the figures `MAX_BATCH_SIZE` is actually derived from — are accurate.

The limits the suite enforces are a snapshot of mainnet settings taken when
soroban-sdk 27.0.5 was published (2026-07-10), not a live query. They can move
under the contract without the tests noticing. Stage 4 should re-measure against
testnet simulation and reconcile.

---

## 3. `MAX_BATCH_SIZE` is calibrated against one token

The cap is bounded by the **contract event budget**, and roughly half of the
per-stream event cost is the *token's* `transfer` event, not Fluxora's
`withdrawn` event. Measured against the Stellar Asset Contract. A SEP-41 token
with a heavier event payload shifts the ceiling down.

The 2x safety factor exists for this reason, but it is a margin, not a proof. An
integrator standardising on an unusual token should re-run
`cargo test resource_limits -- --nocapture` against it.

---

## 4. Not audited

No third-party security audit has been performed. The property tests, the pool
invariant and the randomized sequence suite are evidence of care, not a
substitute for review.

---

## 5. Ledger close time is assumed, not measured

TTL targets convert seconds to ledgers at a nominal 5s close time
(`storage::SECONDS_PER_LEDGER`). Close time is a network property that drifts.
The constant is deliberately conservative — it over-estimates ledgers per unit
time, so entries are funded for longer than strictly needed — but a sustained
slowdown well beyond 5s/ledger would erode the margin. The 30-day buffer and the
keeper path both exist to absorb that.
