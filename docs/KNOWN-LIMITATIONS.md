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

## 5. Ledger close time is assumed, not measured

**Pinned by:**
`contracts/stream/src/test/ttl.rs::nominal_ledger_close_time_is_five_seconds`
and `contracts/stream/src/test/ttl.rs::seconds_to_ledgers_rounds_up`.
Together they pin both the nominal five-second assumption and the conservative
round-up conversion used by TTL targets.

TTL targets convert seconds to ledgers at a nominal 5s close time
(`storage::SECONDS_PER_LEDGER`). Close time is a network property that drifts.
The constant is deliberately conservative — it over-estimates ledgers per unit
time, so entries are funded for longer than strictly needed — but a sustained
slowdown well beyond 5s/ledger would erode the margin. The 30-day buffer and the
keeper path both exist to absorb that.

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

## 7. Pausing moves the cliff in wall-clock terms

**Status: by design — documented, not fixed.**

`cliff_reached` is evaluated against the stream clock, and `stream_time`
subtracts the cumulative `paused_total`. Pausing a stream therefore freezes the
cliff gate along with accrual, and resuming pushes the wall-clock instant the
gate opens forward by the total time spent paused: the gate opens at
`cliff_time + paused_total`, not at the stored `cliff_time`.

The stored `cliff_time` is never rewritten — `get_stream().cliff_time` still
reports the original instant — so the two values an integrator might read (the
schedule field and the effective instant) disagree by exactly `paused_total`. A
recipient who computes an unlock date from `cliff_time` alone will expect funds
to unlock earlier than they do.

This is a limitation because `pause` is **sender-only** and unbounded. A sender
who wants to defer the recipient's first withdrawal can pause a pausable stream
before its cliff and hold it paused, moving the unlock instant arbitrarily far
into the future. The recipient can still withdraw anything already vested, but
before the cliff nothing has vested, so there is nothing to withdraw. The
`pausable` capability is fixed at creation, so this exposure exists exactly when
the stream was created with `pausable == true`.

**What is documented instead of fixed.** `docs/ABI.md` states the rule and gives
the recomputation (`cliff_time + paused_total`); the `resumed` event publishes
the post-resume `paused_total` so an indexer can derive the new instant without
replaying individual intervals; and
`test::cliff::pause_across_cliff_delays_the_wall_clock_cliff` together with
`test::pause::pausing_across_the_cliff_defers_the_cliff_too` assert it.

**If you are integrating a pausable stream:** treat `cliff_time` as a lower
bound, not the unlock date. Read the stream's current `paused_total` — from
`get_stream`, or from the latest `resumed` event — and display
`cliff_time + paused_total`. Do not cache the unlock instant while a stream is
pausable.
