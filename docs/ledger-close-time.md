# Measured ledger close time

The measurement behind `storage::SECONDS_PER_LEDGER` and
`storage::TTL_SAFETY_MARGIN_PERCENT` (`contracts/stream/src/storage.rs`). It
closes the measurement half of
[`docs/KNOWN-LIMITATIONS.md` §5](KNOWN-LIMITATIONS.md#5-ledger-close-time-is-measured-not-assumed--but-only-on-one-network),
which used to be titled "ledger close time is assumed, not measured". #1806.

## Why this matters

Persistent-entry TTLs are denominated in **ledgers**, but every schedule a user
cares about — a stream's start, end and cliff — is denominated in **seconds**.
The conversion between the two is a network property, not a protocol constant:
a funded TTL of N ledgers spans N × real_close seconds of wall clock. If the
real close time differs from the one the conversion assumes, every funded
window is silently longer or shorter than the schedule it was meant to cover.

## Method

`script/measure-ledger-close.sh` (run against the public Stellar testnet RPC by
default; `RPC_URL` retargets it):

1. **Mean over the full retention window, exactly.** `getLatestLedger` gives
   the newest sequence and its close time; any `getLedgers` response reports
   `oldestLedger` and `oldestLedgerCloseTime` at top level — the oldest ledger
   the node still serves. The mean is then
   `(latest close − oldest close) / (latest seq − oldest seq)` over *every*
   consecutive ledger the node still has. No sampling error, and no need to
   download the ~2,400 nine-megabyte pages of ledger metadata a full sweep
   would cost.
2. **Per-ledger variance, sampled.** 24 pages of 50 ledger headers spread
   evenly across the same window; consecutive gaps inside each page give the
   distribution (min, median, p95, max). This part is the drift detector in
   depth as well as breadth: the mean over a week can sit at nominal while the
   tails move.
3. **Verdict.** The observed mean is compared against
   `SECONDS_PER_LEDGER × (100 + TTL_SAFETY_MARGIN_PERCENT) / 100`. `--verify`
   exits non-zero when the margin does not cover the observation, so a release
   runbook can gate on it. The script parses both constants out of
   `storage.rs` itself, so renaming them fails loudly instead of measuring
   against a stale copy.

## Measurement record

| | |
|---|---|
| network | Stellar testnet (`Test SDF Network ; September 2015`, protocol 28) |
| endpoint | `https://soroban-testnet.stellar.org` |
| measured | 2026-09-28 |
| window | ledgers 4,794,976 → 4,915,935 — **120,960 consecutive ledgers ≈ 6.9 days** |
| mean close time | **5.000 s/ledger** (exact, over the whole window) |
| per-ledger gaps | 1,176 sampled; **min 5 s, median 5 s, p95 5 s, max 5 s** |

The distribution is dead flat: every sampled gap closed in exactly 5 s, zero
gaps above the nominal. Testnet on this window behaved like a metronome.

## What the measurement actually changed

The headline finding was **not** the value — 5.000 s equals the 5 s the
contract already assumed — but what the flat result implies:

> Before #1806 the TTL conversion carried **zero headroom**. A conversion
> funded exactly at the observed mean, so any change in close time — a
> protocol upgrade, a validator-performance shift — would have flowed
> straight into every funded window with nothing in the way.

### Which direction is dangerous

A funded TTL of N ledgers spans N × real_close seconds:

- **Faster network** → every window **shrinks**. An entry can become eligible
  to archive before its schedule ends. This is the unsafe direction, and the
  one the margin guards.
- **Slower network** → every window **lengthens**. Wasteful in rent, never
  unsafe.

### The margin

`TTL_SAFETY_MARGIN_PERCENT = 20` inflates every duration *before* conversion,
so `seconds_to_ledgers` remains fully covering for any sustained real mean
close time at or above

```
assumed × 100 / (100 + margin) = 5 × 100 / 120 ≈ 4.17 s/ledger
```

— a network up to ~17% faster than observed. Under the same margin the
retention floor (`MIN_STREAM_TTL_LEDGERS`, 622,080 ledgers) holds its intended
30-day window for anything at or above ~4.34 s/ledger, and the 30-day keeper
buffer rides along inside every target.

The margin does **not** cover a sustained mean below ~4.17 s. That residual
risk is what the re-measurement procedure below is for; a sustained change in
*either* direction is a signal to re-measure and re-pin, not to silently
rebalance.

## Re-measurement procedure

1. Run `script/measure-ledger-close.sh --verify`. A `COVERED` verdict means
   the current constants still hold — nothing else to do.
2. On `EXPOSED` (or after any protocol upgrade on the target network),
   re-measure on the widest window available and update, **together, in one
   change**:
   - `SECONDS_PER_LEDGER` in `contracts/stream/src/storage.rs` (the measured
     mean, rounded up to a whole second if fractional);
   - the pin in `contracts/stream/src/test/ttl.rs`
     (`seconds_per_ledger_matches_the_measured_close_time`, marked
     `MARKER: observed_mean_seconds`);
   - the table and margin derivation in this file;
   - §5 of `docs/KNOWN-LIMITATIONS.md`.
3. Re-run the script with `--verify` and the full `cargo test` suite.
4. Note the drift in the history table below with the new window and date.

### History

| date | network | window | mean | margin verdict |
|---|---|---|---|---|
| 2026-09-28 | testnet | 120,960 ledgers (6.9 d) | 5.000 s | first measurement; margin added by #1806 |

## Relation to the §1 archival canary

The archival canary (`script/archival-canary.sh`,
`docs/archival-canary.md`) converts ledger counts to wall-clock dates at the
nominal 5 s/ledger. On this measurement the two agree — which is also why the
canary's 7-day estimate was usable at all. The canary reads *observed ledger
sequences* from the network rather than assuming time, so it needs no margin;
this measurement exists for the contract-side conversion, which must commit to
a number ahead of time.
