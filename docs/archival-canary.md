# Archival canary runbook

## Status: run, recorded, retired (2026-09-28)

The canary has been consumed. It was planted on 2026-08-12 and left alone for
seven weeks; the round trip was run on 2026-09-28 and the result is recorded in
[`KNOWN-LIMITATIONS.md` §1](KNOWN-LIMITATIONS.md#1-archival-is-not-a-failure-mode-for-persistent-entries).

**Do not replant or redeploy the probe as a routine.** A new question needs a
separate deployment with its own live-until ledger. This runbook is kept because
it explains what the harness asserts and what the recorded result means.

## What the run established

The probe stores one symbol in a persistent entry and deliberately never extends
its TTL, so it receives the network minimum (`min_persistent_ttl`, 120,960
ledgers, ~7 days) and archives as early as the network allows. Fluxora stream
entries use a longer 30-day floor, so the probe — not a stream — is what can be
archived on a useful timescale.

| | |
|---|---|
| probe contract | `CB4XJYNXQ62TCXI3GKCVBWADTSTFWYL3ZLYS3MKYPWRANOSADRZG4A7N` |
| planted | ledger 4,097,334 |
| live until | ledger 4,218,293 |
| observed at | ledger 4,922,344 (704,051 ledgers, ~40.7 days, past live-until) |
| state at that point | canary and contract instance both returned by `getLedgerEntries` with `liveUntilLedgerSeq: 0` — archived, values still served |
| round trip | tx `32e08f32d30db0f1f1a45786dbe7f8d87ca4f83dbd3e3ced0a0d5b54d807651c`, ledger 4,922,351, **one** operation, `SUCCESS`, returned `canary` |
| `SorobanTransactionData` | `archived_soroban_entries: [0, 1, 2]` — canary, contract instance, contract code, restored by that same invocation |
| cost | 5,912,922 stroops total fee, of which 5,912,822 stroops resource fee |
| after | both entries live again at `liveUntilLedgerSeq: 5,043,310` = 4,922,351 + `min_persistent_ttl` − 1 |

The read did not fail, and no `RestoreFootprint` transaction was submitted
anywhere in the sequence. This is Outcome B of the decision table that was
written into §1 on 2026-08-12, before the result was known.

## Prerequisites

- Run from the repository root on a machine with `stellar`, `python3` (3.8+, for
  the inline RPC helper), `curl`, and `bash`.
- The Stellar CLI must have the configured `testnet` network and a `SOURCE`
  identity that is funded and able to submit a transaction. The defaults target
  Stellar testnet. Do not point `NETWORK` or `RPC_URL` at mainnet.
- Environment variables `NETWORK`, `RPC_URL`, `SOURCE`, `PROBE` and
  `MIN_PERSISTENT_TTL` override the defaults. Confirm they identify the intended
  testnet probe before running.

## Run and interpret

Status-only mode submits nothing, uses no keys, and is safe to run at any time:

```bash
script/archival-canary.sh
```

| Signal | Meaning and action |
|---|---|
| `ABSENT` | The RPC did not return the entry. Check `PROBE` and `RPC_URL`; do not read it as an archival result. |
| `ALIVE` with ledgers left | The live-until ledger has not passed, so there is nothing to assert yet. |
| `ARCHIVED`, value still served | The TTL entry is gone (`liveUntilLedgerSeq: 0`) while the value is still returned. This is the state the round trip needs. |
| Past live-until but still `ALIVE` | Eviction is a background scan and lags live-until by hours or days. Retry later. If it is still live a week past the deadline, record that: it would mean eviction is not running on that network. |

Once the entry is archived, run the round trip:

```bash
script/archival-canary.sh --round-trip
```

It submits `read` and asserts, in order:

1. the invocation **succeeds** and returns the planted value — a failure here is
   the outcome §1 used to predict, and the script stops rather than
   documenting it as a pass;
2. the entry is live again at roughly the network minimum, with
   `lastModifiedLedgerSeq` equal to the restoring transaction's ledger;
3. the restoring transaction's envelope carries a non-empty
   `archived_soroban_entries`, and it prints which footprint entries those
   indices name.

Step 3 is the assertion that matters. A successful call alone could be explained
by the RPC, the CLI, or a stale read; `archived_soroban_entries` is set by the
ledger and names exactly what it resurrected.

## Report the result

Record the date, network, observed ledger, transaction hash, the
`archived_soroban_entries` list, and the verified value in
`KNOWN-LIMITATIONS.md` §1, and state the conclusion narrowly: testnet
demonstrated that an invocation touching an archived persistent entry restores
it and succeeds. Do not claim that a Fluxora stream itself was archived — the
probe is a separate contract — and keep the explanation that restoration is a
property of the ledger entry rather than of the contract that wrote it.

If the entry stays live a week past its live-until ledger, or the round trip
fails where it is expected to succeed, preserve the full output, verify the
testnet/RPC/source configuration, and escalate. Do not close or narrow anything
on the strength of the recorded run alone.
