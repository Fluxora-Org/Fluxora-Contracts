# Archival canary runbook

## Purpose

The archival canary tests one live-network behavior that the Soroban SDK test
host cannot establish: after a **persistent contract-data entry is archived**,
a contract invocation that reads it fails at the network level, and a
`RestoreFootprint` operation can make the same entry readable again with its
value intact.

The probe is a separate, disposable contract. It stores one symbol and
deliberately never extends the entry's TTL, so the network applies its minimum
persistent TTL (120,960 ledgers, approximately seven days at five seconds per
ledger). Fluxora stream entries use their own longer TTL floor; the probe does
not demonstrate that a stream itself has archived. The restore behavior is a
ledger property, which is why this probe is useful evidence for stream
recovery. It is testnet-only and must never be deployed to mainnet.

## Cadence

This is a one-shot canary, not a recurring production monitor. For the current
testnet deployment, the expected live-until ledger is **4,218,293** (about
2026-08-19 09:39 UTC). Run the status check once daily after the expected
archive time until the round trip is complete. Network eviction is a background
scan and may lag the live-until ledger by hours or days; a still-readable entry
is a retry-later result, not a failure. If it remains readable one week after
the live-until ledger, record that finding and escalate it as possible
testnet eviction failure. Do not infer mainnet behavior from that outcome.

After a successful round trip, stop running this canary. Do not replant or
redeploy it as a routine; a new test requires a separately tracked deployment
and a fresh live-until ledger.

## Prerequisites

- Run from the repository root on a machine with `stellar`, `curl`, and
  `python3` available.
- The Stellar CLI must have the configured `testnet` network and the
  `fluxora-deployer` source identity. The deployer must be funded and able to
  submit the restore transaction.
- The defaults target Stellar testnet. Do not set `NETWORK` or `RPC_URL` to
  mainnet for this probe.

The script defaults to the current probe contract, testnet RPC, and deployer
identity. Environment variables `NETWORK`, `RPC_URL`, `SOURCE`, and `PROBE` can
override these values; confirm they still identify the intended testnet probe
before running.

## Run and interpret

From the repository root, first run status-only mode:

```bash
script/archival-canary.sh
```

Status-only mode is safe to run at any time and does not submit a transaction.
Interpret the output as follows:

| Signal | Meaning and action |
|---|---|
| `ALIVE` with ledgers left | The live-until ledger has not passed. No action; continue the scheduled check after the deadline. |
| Past live-until; entry still readable | Archival has not happened yet. Eviction is asynchronous; rerun status-only mode the next day. If still live one week past the deadline, record and escalate the finding. |
| Read failed and invocation failed | This is the expected archived-state signal, but not yet proof of recovery. Immediately run the full round trip below. If output indicates an RPC, CLI, authentication, or network error, resolve that issue and rerun; do not report an archival result based on a generic command failure alone. |
| Full round trip succeeds | The entry was inaccessible, invocation failed, restore succeeded, and the value `canary` was recovered. Record the run and close the live-network acceptance criterion in `docs/KNOWN-LIMITATIONS.md` §1. |
| Restore or value verification fails | Do not close the limitation. Preserve the complete output and transaction hashes, verify the testnet/RPC/source configuration, then escalate for investigation. |

When status-only mode reports that the entry is archived and invocation fails
as expected, run:

```bash
script/archival-canary.sh --restore
```

This submits `RestoreFootprint` for the probe's one persistent key and then
invokes `read` again. Success requires the returned value to be exactly
`canary`; the script also reports the renewed live-until ledger. If the key is
still readable, the script exits with an error instead of restoring it, so
wait for the next scheduled check.

## Report the result

For a successful run, add the date/time, network, observed ledger, restore
transaction hash, and verified value to `docs/KNOWN-LIMITATIONS.md` §1. Retain
the command output as supporting evidence. State the conclusion narrowly:
testnet demonstrated archival, a failed
read, and recovery of the probe's value via `RestoreFootprint`. Do not claim
that a Fluxora stream itself was archived, and retain the explanation that
restore is a ledger-level behavior.

If the entry remains live one week past its live-until ledger, record the
current ledger and the result of the status check in the same limitation
section. Keep the limitation open until there is a verified outcome; a delayed
eviction is not a successful archival test.