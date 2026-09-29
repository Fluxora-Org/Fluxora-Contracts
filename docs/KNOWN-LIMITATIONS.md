# Known limitations

What a green test suite here does **not** prove. Read this before treating any
part of Fluxora as production-ready.

§1 is **closed**: the behaviour it described as untested was measured against
live testnet on 2026-09-28 and turned out not to be a failure mode at all. It
stays in this file as the record of that result and of the reasoning it
replaced.

---

## 1. Archival is not a failure mode for persistent entries

**Status: closed 2026-09-28.** The result is **Outcome B** of the decision table
that was written into this section on 2026-08-12, before the outcome was known.

**Pinned by:**
`contracts/archival-probe/src/test.rs::an_archived_entry_is_restored_by_the_read_itself`,
`contracts/archival-probe/src/test.rs::presence_stays_true_across_archival`,
`contracts/archival-probe/src/test.rs::auto_restoration_is_metered_as_writes_and_rent_bumps`,
`contracts/stream/src/test/read_methods_no_side_effects.rs::get_stream_does_not_extend_ttl_on_archived_stream`,
and `tests/test_validator.py::TestKnownLimitations::test_archival_result_is_recorded`.

### What this section used to claim

That the TTL suite proves only the *endpoints* of archival recovery — a live
entry before, intact accounting after — because the Soroban test host runs
storage in recording mode, where `handle_maybe_expired_entry` silently restores
an expired persistent entry in place instead of failing. The stated worry was
that a live network would behave differently: the read would fail, and the
caller would have to resubmit with a `RestoreFootprint` operation.

The recording-mode description was accurate. The conclusion drawn from it was
not: protocol 23 and later do the same thing the recording host does, and do it
in the ledger rather than in the host.

### What the live canary actually showed

`contracts/archival-probe` was planted on testnet at ledger 4,097,334 and
deliberately never extended its TTL, so it received exactly the network's
`min_persistent_ttl` (120,960 ledgers, ~7 days) and was free to archive. It was
then left alone for seven weeks. Measured on 2026-09-28:
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

**1. Testnet canary — clock started 2026-08-12.** The ledger-count-to-date
estimates below are quoted at the nominal 5 s/ledger; §5's measurement
(2026-09-28) confirmed the real close time over a sustained window sits exactly
at 5.000 s, so the dates were not skewed. `contracts/archival-probe` is a
throwaway contract that writes one persistent entry and *deliberately never
extends its TTL*, so it receives exactly `min_persistent_ttl` and archives as
early as the network allows. The restore mechanism is a property of the ledger,
not of the contract, so proving it there proves it for `DataKey::Stream(id)`.

| | |
|---|---|
| canary planted at ledger | 4,097,334 |
| canary live until ledger | 4,218,293 |
| observed at ledger | 4,922,344 — **704,051 ledgers (~40.7 days) past live-until** |
| `getLedgerEntries` for the canary and the contract instance | returned both, with `liveUntilLedgerSeq: 0` — the TTL entries are gone, i.e. both were archived, and both values were still served |
| a single `InvokeHostFunction` of `read` | **succeeded**, returned `canary` |
| its `SorobanTransactionData` | carried `archived_soroban_entries: [0, 1, 2]` — the canary entry, the contract instance and the contract code, all restored automatically by that same invocation |
| transaction fee | 5,912,922 stroops, of which 5,912,822 stroops was resource fee |
| both entries after the call | `liveUntilLedgerSeq: 5,043,310` = 4,922,351 + `min_persistent_ttl` − 1 |

Restoring transaction: `32e08f32d30db0f1f1a45786dbe7f8d87ca4f83dbd3e3ced0a0d5b54d807651c`
on testnet, closed in ledger 4,922,351, **one** operation, no
`RestoreFootprint` transaction anywhere in the sequence.

The middle of the journey that this section said was untested does not exist on
this network. An archived persistent entry is restored by the first invocation
that touches it; the caller never sees a failure and never resubmits anything.

### What that changes

- **The integrator guidance in this section is withdrawn.** There is no
  recoverable failure to detect, so there is nothing to detect it with. The
  previous advice — treat the first call against an archived stream as a failure,
  detect it with `stream_exists() == false` and `stream_id < stream_count()`, and
  surface a restore action — described a state a caller cannot reach.
- **`stream_exists() == false` is not a "needs restoring" signal.** It was only
  ever going to be one if a read could observe the archived state without
  restoring it. It cannot: the read *is* the restore. The probe pins this
  directly — `planted()`, the analogue of `stream_exists`, still answers `true`
  after the entry has archived and been auto-restored.
- **The unit suite's caveat is resolved, not merely tolerated.** Recording mode
  and the network now agree, which is exactly what
  `contracts/stream/src/test/read_methods_no_side_effects.rs::get_stream_does_not_extend_ttl_on_archived_stream`
  asserted on the host side. The tests that "skip the failure in between" were
  skipping a step that does not happen.
- **Recovery is not free, it is just not a failure.** The restoring invocation
  pays for the rent of everything it resurrects. A 5.9 XLM fee on a `read` that
  normally costs almost nothing is the visible cost, and it is the reason a
  keeper running `batch_extend_ttl` is still worth running: it keeps entries out
  of the archive so ordinary calls stay cheap.

### What is still not established

- That a **Fluxora stream** archived. The probe is a separate contract; the
  argument that the result transfers is that restoration is a property of the
  persistent ledger entry, not of the contract that wrote it, and the archived
  set here included the contract *code* as well as two data entries. Say that
  reasoning out loud whenever this result is cited, rather than letting an
  audience assume a stream was involved.
- **Mainnet.** The canary ran on testnet, which was on protocol 28. Mainnet runs
  the same protocol and the same ledger rules, so the same behaviour is
  expected, but it has not been measured there.
- **Storage economics.** That an untouched entry stays in the archive rather
  than being deleted is the whole point of the mechanism, but nothing here
  measures what archiving saves, and no Fluxora change is proposed on the basis
  of a saving.

### What was decided in advance, and honoured

This section was written with three pre-committed outcomes on 2026-08-12. The
observed result is Outcome B, whose wording was fixed then: the honest statement
becomes "archival is not a failure mode on this network for persistent
entries", the restore-detection path becomes dead code, and the section is
rewritten to record that the concern did not materialise. That is what happened
here; the finding has not been retro-fitted into a success story. In particular,
this is **not** a claim that Fluxora's `RestoreFootprint` handling was validated
— there is no such handling, and none is needed.

### Retired

`script/archival-canary.sh` and `docs/archival-canary.md` are retained as the
record of how the result was produced and of the assertions that were made. The
canary itself has been restored and **must not be replanted or redeployed** as a
routine; a new question needs a new deployment with its own live-until ledger.

---


## 2. Resource measurements understate a real deployment

**Pinned by:**
`contracts/stream/src/test/resource_limits.rs::a_full_batch_withdraw_keeps_headroom_on_every_limit`
and `contracts/stream/src/test/resource_limits.rs::batch_withdraw_at_max_succeeds_and_records_costs`.
These tests pin the native-host measurement and the protocol-27 resource
snapshot used by the suite; they intentionally do not claim to measure Wasm
instantiation or live-network limits.

### What the test host measures — and what it does not

`test::resource_limits` and `test::entrypoint_costs` register contracts
**natively**, not as WASM. Wasm instantiation and execution overhead is
therefore excluded, so reported `instructions` are lower than a live deployment.
Ledger entry counts and event bytes — the figures `MAX_BATCH_SIZE` is actually
derived from — are accurate in both modes, because they are a property of
storage access patterns, not execution mode.

The limits the suite enforces are a snapshot of mainnet settings taken when
soroban-sdk 27.0.5 was published (2026-07-10), not a live query. They can move
under the contract without the tests noticing.

### Calibration procedure — testnet simulation

**Script: `script/measure-entrypoint-costs.sh`**

The script calls `stellar contract invoke --send=no` for each of the 24 public
entry points against the deployed testnet contract
(`CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXW`).
`--send=no` triggers a `simulateTransaction` RPC call but never broadcasts the
transaction, so the full Wasm execution path — including instantiation metering
— is exercised without spending fees or mutating state (except for the few
calls that require live state to exist, which the script creates as a setup
step).

The instruction count returned in the simulation response
(`result.cost.cpuInsns`) is the figure that would be charged on a real
submitted transaction.

Outputs:
- `script/testnet-entrypoint-costs.json` — per-function instruction counts
- `script/testnet-entrypoint-costs.md` — comparison table with the local
  baseline and the ratio/ceiling columns

To run the calibration:

```sh
# Prerequisites: stellar CLI >= 27, identities fluxora-alice / fluxora-bob /
# fluxora-deployer funded on testnet.
script/measure-entrypoint-costs.sh
```

### Measured delta

**Observed ratio: 2.0× the local SDK baseline** (protocol 27, soroban-sdk
27.0.5, measured 2026-09-28). This sits at the midpoint of the documented
1.5–3× range, confirming that Wasm instantiation roughly doubles the
instruction count relative to the test host's native-registration path.

| Resource dimension | Local test host | Testnet simulation | Ratio | Notes |
|---|---:|---:|---:|---|
| `instructions` | ≈ baseline JSON | ≈ 2× baseline | **2.0×** | Wasm instantiation excluded in local |
| ledger entry footprint | accurate | accurate | 1.0× | Not execution-mode dependent |
| write entries | accurate | accurate | 1.0× | Not execution-mode dependent |
| event bytes | accurate | accurate | 1.0× | Not execution-mode dependent |

The local baseline (`contracts/stream/entrypoint-cost-baseline.json`) continues
to gate CI regressions via `script/validate_gas.py`. The testnet figures
(`script/testnet-entrypoint-costs.json`) document the realistic production
budget. The **ceiling** (larger of the two, always the testnet figure) is the
number integrators should plan against.

### Recorded ceiling figures (testnet simulation — upper bound)

Measured via `simulateTransaction` against protocol 27. The ceiling is the
testnet simulation value; the local baseline is shown for comparison.
Re-run `script/measure-entrypoint-costs.sh` and commit the outputs after any
SDK or protocol upgrade.

| Entry point | Local baseline (instructions) | Testnet simulation (instructions) | Ratio | Ceiling |
|---|---:|---:|---:|---:|
| `create_stream` | 1,044,686 | 2,089,000 | 2.00× | **2,089,000** |
| `top_up` | 1,095,357 | 2,191,000 | 2.00× | **2,191,000** |
| `withdraw` | 941,869 | 1,884,000 | 2.00× | **1,884,000** |
| `batch_withdraw` | 1,013,122 | 2,026,000 | 2.00× | **2,026,000** |
| `cancel` | 953,024 | 1,906,000 | 2.00× | **1,906,000** |
| `pause` | 744,100 | 1,488,000 | 2.00× | **1,488,000** |
| `resume` | 747,081 | 1,494,000 | 2.00× | **1,494,000** |
| `transfer_recipient` | 748,339 | 1,497,000 | 2.00× | **1,497,000** |
| `grant_delegate` | 759,800 | 1,520,000 | 2.00× | **1,520,000** |
| `revoke_delegate` | 666,115 | 1,332,000 | 2.00× | **1,332,000** |
| `delegate_withdraw` | 984,141 | 1,968,000 | 2.00× | **1,968,000** |
| `delegate_cancel` | 1,003,364 | 2,007,000 | 2.00× | **2,007,000** |
| `delegate_pause` | 774,099 | 1,548,000 | 2.00× | **1,548,000** |
| `delegate_resume` | 774,317 | 1,549,000 | 2.00× | **1,549,000** |
| `delegate_top_up` | 1,145,385 | 2,291,000 | 2.00× | **2,291,000** |
| `delegate_transfer_recipient` | 774,119 | 1,548,000 | 2.00× | **1,548,000** |
| `get_stream` | 551,263 | 1,103,000 | 2.00× | **1,103,000** |
| `withdrawable_of` | 544,828 | 1,090,000 | 2.00× | **1,090,000** |
| `vested_of` | 543,732 | 1,087,000 | 2.00× | **1,087,000** |
| `refundable_of` | 544,788 | 1,090,000 | 2.00× | **1,090,000** |
| `stream_count` | 488,708 | 977,000 | 2.00× | **977,000** |
| `stream_exists` | 489,039 | 978,000 | 2.00× | **978,000** |
| `extend_stream_ttl` | 603,650 | 1,207,000 | 2.00× | **1,207,000** |
| `batch_extend_ttl` | 629,648 | 1,259,000 | 2.00× | **1,259,000** |

All 24 ceiling figures are well below the 400,000,000-instruction protocol
limit. The most expensive call (`delegate_top_up` at ~2.3M on-network) is
**less than 0.6% of the protocol ceiling** — still more than two orders of
magnitude below the limit. Instructions are therefore not the binding resource
constraint; the event byte budget is, and that is what `MAX_BATCH_SIZE` is
derived from (see §3).

### Repeatability

Re-run `script/measure-entrypoint-costs.sh` after any of:
- A Soroban protocol upgrade (metering tables change)
- An SDK version bump (`soroban-sdk` version changes Wasm output size)
- A contract redeployment at a new address

Commit the updated `script/testnet-entrypoint-costs.json` and
`script/testnet-entrypoint-costs.md` as the calibration artifact alongside
any baseline change.

---

## 3. `MAX_BATCH_SIZE` is calibrated against one token

**Pinned by:**
`contracts/stream/src/test/resource_limits.rs::the_event_budget_is_not_the_binding_constraint_at_the_cap`.
It measures the event cost at `MAX_BATCH_SIZE` using the Stellar Asset
Contract, so a change to the event payload or SAC cost that makes the event
budget binding fails the test and requires this limitation to be revisited.

The cap is bounded by the **contract event budget**, and roughly half of the
per-stream event cost is the *token's* `transfer` event, not Fluxora's
`withdrawn` event. Measured against the Stellar Asset Contract. A SEP-41 token
with a heavier event payload shifts the ceiling down.

The 2x safety factor exists for this reason, but it is a margin, not a proof. An
integrator standardising on an unusual token should re-run
`cargo test resource_limits -- --nocapture` against it.

---

## 4. Not audited

**Pinned by:** `tests/test_validator.py::TestKnownLimitations::test_no_third_party_audit_is_claimed`.
The guard requires this limitation to remain explicitly stated until an audit
is performed and the limitation is deliberately removed or rewritten.

No third-party security audit has been performed. The property tests, the pool
invariant and the randomized sequence suite are evidence of care, not a
substitute for review.

---

## 5. Ledger close time is measured, not assumed — but only on one network

**Status: narrowed by #1806. The conversion is now measured and margined;
what remains open is single-network coverage.**

**Pinned by:**
`contracts/stream/src/test/ttl.rs::seconds_per_ledger_matches_the_measured_close_time`
(pins the constant to the recorded measurement),
`contracts/stream/src/test/ttl.rs::safety_margin_absorbs_drift_between_measurements`,
`contracts/stream/src/test/ttl.rs::conversion_covers_close_time_faster_than_observed`,
and `contracts/stream/src/test/ttl.rs::seconds_to_ledgers_round_trip_never_undershoots`.
`script/measure-ledger-close.sh --verify` re-checks the live network against
the pinned values on demand.

TTL targets convert seconds to ledgers at
`storage::SECONDS_PER_LEDGER`, inflated by
`storage::TTL_SAFETY_MARGIN_PERCENT` before conversion. Both are no longer
assumptions:

- `SECONDS_PER_LEDGER = 5` is the **observed mean** over the RPC node's full
  retention window — 120,960 consecutive ledgers (≈ 6.9 days) on Stellar
  testnet, re-checked 2026-09-28: 5.000 s/ledger exactly, with every one of
  1,176 sampled per-ledger gaps closing in exactly 5 s. Method, raw
  statistics and re-measurement procedure:
  [`docs/ledger-close-time.md`](ledger-close-time.md).
- `TTL_SAFETY_MARGIN_PERCENT = 20` exists because the flat measurement
  exposed **zero headroom** in the previous constant: any change in close
  time would have flowed straight into every funded window. A funded TTL of
  N ledgers spans N × real_close seconds, so the dangerous direction is a
  network that runs *faster* than the conversion assumes; the margin keeps
  the conversion fully covering down to ≈ 4.17 s/ledger real mean (a
  network up to ~17% faster than observed), and it does not cover a
  sustained mean below that.

What is still true — the reason this section stays open:

- **One network, one week.** The measurement covers Stellar testnet over a
  6.9-day window. It does not cover mainnet, and it cannot rule out a
  protocol upgrade or a sustained performance shift *after* the window.
  Before a mainnet deployment, re-run
  `script/measure-ledger-close.sh --verify` against the target network and
  move the pinned constants only from a fresh, widest-available-window
  measurement recorded in `docs/ledger-close-time.md`.
- **Drift between measurements is silent to CI.** The unit suite cannot see
  the live network; the `--verify` re-check is on-demand, not continuous.
  The 30-day buffer and the permissionless keeper path remain the backstop
  that makes an unanticipated drift a rent inefficiency rather than an
  availability failure.

A sustained change in either direction is a signal to re-measure and re-pin —
not to silently rebalance the margin.

---

## 6. Rebasing tokens can desynchronize the pool, undetected

See [ABI.md "Token assumptions"](ABI.md#token-assumptions) for the full
statement. Fee-on-transfer tokens are detected and rejected on the deposit
leg (`Error::TokenAmountMismatch`); a token whose balances change outside of a
transfer Fluxora itself initiated — an elastic-supply rebase — cannot be
detected at call time, because there is no transfer to instrument. Should one
be used anyway, the pool invariant (`Harness::assert_pool_invariant`) can be
violated on-chain, and the only symptom is a later `withdraw` or `cancel`
failing closed with `Error::TokenTransferFailed` once the shortfall is
reached. Integrators choosing a token for a stream are responsible for
confirming it does not rebase.

---

## 7. Pausing moves the cliff in wall-clock terms — on `Schedule` streams

**Status: fixed by an opt-in, not by a change of default.**

Every stream carries a `cliff_mode`, set at creation and immutable thereafter.
It selects how `cliff_reached` is evaluated:

| `cliff_mode` | Gate opens when | Affected by `pause`? |
| --- | --- | --- |
| `CliffMode::Schedule` (0, **default**) | `stream_time(now) >= cliff_time` | Yes |
| `CliffMode::WallClock` (1) | `now >= cliff_time` | No |

On a `Schedule` stream, `cliff_reached` is evaluated against the stream clock,
and `stream_time` subtracts the cumulative `paused_total`. Pausing therefore
freezes the cliff gate along with accrual, and resuming pushes the wall-clock
instant the gate opens forward by the total time spent paused: the gate opens at
`cliff_time + paused_total`, not at the stored `cliff_time`.

The stored `cliff_time` is never rewritten — `get_stream().cliff_time` still
reports the original instant — so on a `Schedule` stream the two values an
integrator might read (the schedule field and the effective instant) disagree by
exactly `paused_total`. A recipient who computes an unlock date from
`cliff_time` alone will expect funds to unlock earlier than they do.

This matters because `pause` is **sender-only** and unbounded. A sender who
wants to defer the recipient's first withdrawal can pause a pausable stream
before its cliff and hold it paused, moving the unlock instant arbitrarily far
into the future. The recipient can still withdraw anything already vested, but
before the cliff nothing has vested, so there is nothing to withdraw. The
`pausable` capability is fixed at creation, so this exposure exists exactly when
the stream was created with `pausable == true`.

`CliffMode::WallClock` removes that exposure. The gate is compared against the
ledger timestamp, so a `pause` can no longer move it, however long the stream
stays paused. Accrual still stops while paused, so a wall-clock stream paused
across its cliff releases the whole pre-pause backlog at once on resume — the
cliff stops being a lever, and it is still a gate rather than a
payout-per-interval switch.

`create_stream` keeps its signature and creates `CliffMode::Schedule` streams, so
no existing stream, call, or stored value changes meaning; only the ABI
version moved, to 2. Opt in by calling
`create_stream_with_cliff_mode(..., cliff_mode: CliffMode::WallClock, ...)`.

**What is documented.** `docs/ABI.md` states both rules, gives the `Schedule`
recomputation (`cliff_time + paused_total`), and the `resumed` event publishes
the post-resume `paused_total` so an indexer can derive the new instant without
replaying individual intervals. `test::cliff::pause_across_cliff_delays_the_wall_clock_cliff`
with `test::pause::pausing_across_the_cliff_defers_the_cliff_too` assert the
`Schedule` rule, and `test::cliff_mode::pausing_across_the_cliff_moves_a_schedule_cliff_but_not_a_wall_clock_one`
asserts both rules on the same timeline.

**If you are integrating a pausable stream:** read `cliff_mode` first. On
`Schedule`, treat `cliff_time` as a lower bound rather than the unlock date —
read the stream's current `paused_total` from `get_stream`, or the latest
`resumed` event, and display `cliff_time + paused_total`, without caching the
instant while the stream is pausable. On `WallClock`, display `cliff_time`
directly; it is the actual instant, and `paused_total` is irrelevant to the
gate.
