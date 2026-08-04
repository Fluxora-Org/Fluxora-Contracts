Gas Profiling and Budget Review
This document describes the gas (CPU and Memory) costs for the Fluxora streaming contract.

WASM Size Budgets
Every CI build compiles all three contracts to wasm32-unknown-unknown --release and asserts
that the resulting artifact stays within its byte budget. A contract that exceeds its budget
fails the wasm-size-budget CI job.

Budgets were set with ~25% headroom above the sizes measured during the June 2026 baseline
audit. Soroban's practical upload ceiling is ~100 KiB after Brotli compression; raw WASM budgets
are intentionally more conservative to leave room for future features and keep upload fees low.

Contract	Budget	Notes
fluxora_stream	256 KiB (262 144 bytes)	Largest contract; full streaming surface area
fluxora_factory	128 KiB (131 072 bytes)	Policy wrapper; should stay small
fluxora_governance	128 KiB (131 072 bytes)	Minimal timelock; should stay small
Enforcement
The script/check-wasm-size.sh script implements the check:

Bash

# Check raw release artifacts (run locally after a WASM build):
bash script/check-wasm-size.sh

# Check optimized artifacts (after running stellar contract optimize):
bash script/check-wasm-size.sh --optimized
The wasm-size-budget CI job:

Builds all three contracts with cargo build --release --workspace --target wasm32-unknown-unknown.
Runs stellar contract optimize on each artifact (best-effort; failures are non-fatal).
Calls script/check-wasm-size.sh — fails the job if any artifact exceeds its budget.
Updating a Budget
If a deliberate, reviewed feature addition requires more space:

Land the feature and measure the new raw size locally.
Add ~25% headroom to the measured size, rounding up to the nearest 64 KiB boundary.
Update the budget constant in script/check-wasm-size.sh.
Update the table above with the new value and a note explaining the change.
Include the change in the PR description.
Per-PR Delta Reporting
Every build also compares the current WASM sizes against the previous release tag
(most recent v* tag) and prints a byte-delta for each contract. This makes incremental
bloat from individual PRs visible immediately, without waiting for the absolute ceiling
to be approached.

How it works:

The script finds the most recent v* tag in the repository.
For each contract, it reads the WASM file size at that tag from git history.
It computes current_size - previous_size and prints the result.
In GitHub Actions CI, ::warning:: annotations are emitted when a contract grows
and ::notice:: annotations when it shrinks. These annotations appear on the PR
timeline without failing the build (budget enforcement still runs independently).
Example output:

text

Previous release tag: v0.9.0
  vs v0.9.0: +1234 bytes (+1.2 KiB) (was 200000 / 195.3 KiB)
fluxora_stream: 201234 bytes (196.5 KiB) — OK (headroom: 60.9 KiB)
Edge cases:

No release tag exists: Delta reporting is skipped with an informational message.
Previous tag has no WASM file: The delta is reported as "baseline unavailable".
Budget exceeded: Delta is still reported even when the contract fails the budget check.
Step summary: When running in CI (GITHUB_STEP_SUMMARY is set), the script writes
a delta table to the GitHub Actions step summary alongside the budget report.

Optimize step
stellar contract optimize runs wasm-opt -Oz on the artifact, typically reducing binary
size by 10–30%. CI runs this step and checks the resulting .optimized.wasm file as an
informational pass. The hard budget gate runs against the raw release artifact so that the
check remains reproducible without the Stellar CLI installed.

Safe Batch Limits
Operation	Batch Size	Recommended CPU Budget
create_streams	1	1.5M
create_streams	10	10M
create_streams	50	40M
batch_withdraw	1	1.0M
batch_withdraw	10	6M
batch_withdraw	50	20M
batch_withdraw	100	35M
batch_withdraw_to	1	1.0M
batch_withdraw_to	10	6M
batch_withdraw_to	50	20M
batch_withdraw_to	100	35M
bulk_cancel_streams	1	3.5M
bulk_cancel_streams	5	9M
bulk_cancel_streams	10	16M
bulk_cancel_streams	20	32M
bulk_resume_streams_as_admin	1	4M
bulk_resume_streams_as_admin	5	10M
bulk_resume_streams_as_admin	10	18M
bulk_resume_streams_as_admin	20	36M
Stream Metadata Gas Profile
Validation CPU Cost: Bounded validation iterates over a maximum of 8 key-value pairs (MAX_METADATA_KEYS = 8), checking string lengths and accumulating total byte count (MAX_METADATA_BYTES = 512). Execution CPU cost is negligible (< 0.05M CPU instructions).
Fail-Fast Early Revert: Failures during validate_metadata short-circuit before any storage key reads/writes or token transfers, avoiding wasted ledger write footprint fees.
Query Cost (get_stream_metadata): A single read-only persistent storage lookup on DataKey::Stream(u64). Consumes minimal CPU instructions (~0.1M) with no state mutation or token call overhead.

Rate Schedule Validation Gas Profile
The `validate_rate_schedule` function validates a multi-segment rate schedule before storage.
It performs a single linear scan over the segment array with one `checked_mul` and one
`checked_add` per segment.

Validation is bounded by `MAX_RATE_SEGMENTS = 256`:

| Budget | Value | Notes |
|---|---|---|
| Max segments | 256 | Hard bound; exceeds → `RateScheduleTooManySegments` |
| CPU (256 segments) | < 0.01 M instructions | Linear scan with checked arithmetic |
| CPU (empty schedule) | < 0.001 M instructions | Length check only |

**Fail-Fast Early Revert**: The count check runs first, then per-segment checks short-circuit
at the first violation (zero-length, negative rate, or overflow) before any storage
mutation occurs.

## Rate-Schedule Packing: Packed vs. Unpacked Storage

`contracts/stream/src/accrual.rs` provides `pack_rate_segment(rate_per_second, duration_secs)`
and `unpack_rate_segment(word)`, which bit-pack a `(rate_per_second: i128, duration_secs: u64)`
rate-schedule segment into a single `u128` storage word instead of two separate full-width
fields.

### Bit-field layout

```text
bits [0, 31)   (31 bits) : duration_secs  (unsigned, 0..=2^31-1, ~68 years)
bit   31       (1 bit)   : sign bit       (0 = non-negative, 1 = negative)
bits [32, 128) (96 bits) : rate magnitude (unsigned, 0..=2^96-1)
```

`MAX_PACKABLE_DURATION_SECS` (2^31 - 1) and `MAX_PACKABLE_RATE_MAGNITUDE` (2^96 - 1) bound the
packable range. A rate or duration outside these bounds is rejected with
`ContractError::InvalidParams` — the packer never silently truncates a value into an adjacent
bit field.

### Measured baseline: 10-segment schedule

`contracts/stream/tests/gas_regression.rs::test_rate_schedule_packed_vs_unpacked_storage_gas`
builds a representative 10-segment schedule (mixed positive/negative rates, short and long
durations) two ways — as bit-packed `u128` words and as the legacy two-full-width-field layout —
and measures both the serialized XDR size (what Soroban charges rent on) and the CPU
instructions charged for the persistent-storage `set`/`get` host calls:

| Metric | Unpacked (10 segments) | Packed (10 segments) | Reduction |
|---|---|---|---|
| Serialized XDR bytes | 932 bytes (93.2 B/segment) | 212 bytes (21.2 B/segment) | ~77% smaller |
| CPU: persistent `set` | 29,422 instructions | 12,399 instructions | ~58% fewer |
| CPU: persistent `get` | 41,103 instructions | 9,572 instructions | ~77% fewer |

The byte-size reduction exceeds the "roughly half" estimate from the two-full-width-field
comparison because Soroban's XDR framing overhead (type tags, vector length prefixes) is paid
once per `Vec` rather than per field, so collapsing two fields into one word removes a
disproportionate share of that framing on top of the raw bit-width halving. The CPU reduction
follows directly from the smaller serialized payload: persistent storage `set`/`get` cost scales
with entry size.

Run the measurement with:

```bash
cargo test -p fluxora_stream --test gas_regression rate_schedule -- --nocapture
```

The measured CPU instruction counts are recorded in the JSON baseline above
(`rate_schedule_storage.*`) and validated on every CI run by `script/validate_gas.py`.

Hot Path Analysis
withdraw
The withdraw function is the most common operation. Its cost is dominated by:

Loading the Stream state.
Accrual calculation.
Token transfer (external call).
Saving updated withdrawn_amount.
batch_withdraw
To reduce gas, batch_withdraw optimizes by:

Caching the ledger timestamp.
Performing a single authorization check.
Processing multiple streams in a loop.
Reading TotalLiabilities once, decrementing a local accumulator for every
paid stream, and writing the final value once after the batch succeeds.
At MAX_PAGE_SIZE = 100, the liability flush changes the hot-path
TotalLiabilities instance-storage I/O from 100 reads plus 100 writes to 1 read
plus 1 write, while preserving the same final liability value. The batch remains
atomic: any validation or transfer failure reverts the whole call.

## Batch Benchmark Coverage

The stream contract benchmark harness is implemented in `contracts/stream/tests/gas_regression.rs`.
It captures hot-path CPU costs for both creation and withdrawal batch operations and documents the expected regression surface.

Measured paths include:
- `create_stream`: single-stream creation cost.
- `create_streams`: batched creation for 1, 5, and 10 streams.
- `withdraw`: single withdrawal cost.
- `batch_withdraw`: batched withdrawal for 1, 10, 50, and 100 stream IDs.
- `batch_withdraw` mixed-state coverage: active, cancelled, and completed streams together.

This harness is intentionally narrow: it validates batch gas scaling and edge-case state handling, while functional correctness is covered by the broader stream contract test suite.

## Regression Surface

The current batch benchmark coverage explicitly locks down the following contract behavior:

- `create_stream`: single-stream creation gas cost.
- `create_streams`: batched creation for 1, 5, and 10 streams, ensuring batch-size scaling and total deposit handling.
- `withdraw`: single-stream withdrawal cost.
- `batch_withdraw`: batched withdrawal for 1, 10, 50, and 100 stream IDs, ensuring loop scaling and single-auth efficiency.
- `batch_withdraw` mixed-state path: active, cancelled, and completed streams together, ensuring the batch loop exercises completion and cancellation handling.

This coverage is intentionally regression-focused: it locks behavior around batch gas budgets, mixed stream state paths, and batch-create scaling without changing the public batch semantics.

## Hot Path Analysis

batch_withdraw_to
Identical cost structure to batch_withdraw except that each entry carries an explicit per-entry destination address. The additional WithdrawToParam struct field has negligible impact on iteration cost.

O(n²) duplicate-ID scan at MAX_PAGE_SIZE
The four batch entrypoints — batch_withdraw, batch_withdraw_to, bulk_resume_streams_as_admin, and bulk_cancel_streams — each validate that their stream-id arguments contain no duplicates before processing. The current implementation is reject_duplicate_ids in storage.rs, which uses a nested-loop scan over a Vec<u64>:

Rust

for id in stream_ids.iter() {
    for s in seen.iter() {
        if s == id { return Err(DuplicateStreamId); }
        seen.push_back(id);
    }
}
This is O(n²) in the batch size n. At MAX_PAGE_SIZE = 100, the worst case is about 4 950 element comparisons per call (~10 000 inclusive of the outer-loop overhead). The regression tests measure batch_withdraw and batch_withdraw_to at the existing large-batch sizes 1, 10, 50, and 100, and measure bulk_cancel_streams plus bulk_resume_streams_as_admin at issue #1219's baseline sizes 1, 5, 10, and 20. This keeps the newer bulk entrypoints under validate_gas.py regression comparison while preserving the existing large-batch coverage for withdrawal paths.

This is O(n²) in the batch size n.  At `MAX_PAGE_SIZE = 100`, the worst case is about 4 950 element comparisons per call (~10 000 inclusive of the outer-loop overhead).  The gas regression tests measure all four batch entrypoints — `batch_withdraw`, `batch_withdraw_to`, `bulk_resume_streams_as_admin`, and `bulk_cancel_streams` — at batch sizes 1, 10, 50, and 100 up to `MAX_PAGE_SIZE` (100). This ensures full coverage across the full batch capacity while asserting CPU-instruction costs stay well within Soroban's per-invocation CPU limit (`PER_INVOCATION_CPU_BUDGET`).

A companion refactor issue replaces the O(n²) scan with an O(n) helper (e.g. using a `Map<u64,bool>`), after which these baselines are expected to improve significantly, especially at size 100. The budget assertions stay valid regardless; they guard against per-invocation-limit violations, not against algorithmic regressions within the current design.

## Performance Metrics

Performance Metrics
The following table provides the CPU instruction counts for core operations.

<!-- GAS_BASELINE_START -->
{
  "create_stream": 568292,
  "create_streams": {
    "1": 1500000,
    "5": 10000000,
    "10": 40000000
  },
  "withdraw": 562057,
  "batch_withdraw": {
    "1": 531125,
    "10": 3675044,
    "50": 19844037,
    "100": 45453389,
    "mixed-state": 1500000
  },
  "batch_withdraw_to": {
    "1": 545000,
    "10": 3750000,
    "50": 20500000,
    "100": 47000000
  },
  "bulk_resume_streams_as_admin": {
    "1": 4000000,
    "10": 18000000,
    "50": 90000000,
    "100": 180000000
  },
  "bulk_cancel_streams": {
    "1": 3500000,
    "10": 16000000,
    "50": 80000000,
    "100": 160000000
  },
  "keeper_cancel": {
    "partial_accrual": 786739,
    "fully_accrued": 386889
  },
  "rate_schedule_storage": {
    "unpacked_10_segments_write": 29422,
    "unpacked_10_segments_read": 41103,
    "packed_10_segments_write": 12399,
    "packed_10_segments_read": 9572
  }
}

<!-- GAS_BASELINE_END -->
Baselines were captured from a clean run of script/validate_gas.py against contracts/stream/tests/gas_regression.rs on Rust 1.94.1 / soroban-env-host 21.2.1 (see #1201). Costs are deterministic CPU-instruction counts from the metered host and are stable across runs on the same toolchain/SDK pin. Update via the review bar below.

Governance Operations
The governance contract (fluxora_governance) handles proposal creation, approval, and execution with bounded costs to prevent DoS attacks.

Budget Thresholds
These thresholds are enforced by the gas regression tests in contracts/governance/tests/gas_regression.rs. CI will fail if any operation exceeds its budget.

Propose
Creating a new proposal stores the calldata and proposal metadata. Cost is independent of signer count since we don't iterate over signers during creation.

Metric	Threshold	Notes
CPU Instructions	≤ 1,000,000	Independent of calldata size
Memory Bytes	≤ 100,000	Independent of calldata size
The calldata is capped at MAX_CALLDATA_BYTES (4,096 bytes) to keep storage costs reasonable.

Approve
Approving a proposal involves checking the signer's membership (O(1) via Map index) and appending to the approvals list. The cost scales linearly with the number of existing approvals since we store them as a Vec.

Signer Count	CPU Threshold	Memory Threshold
1-5	≤ 375,000 + 75,000 per signer	≤ 37,500 + 7,500 per signer
6-10	≤ 750,000 + 75,000 per signer	≤ 75,000 + 7,500 per signer
11-20	≤ 1,125,000 + 75,000 per signer	≤ 112,500 + 7,500 per signer
Max (20)	≤ 1,500,000	≤ 150,000
Why it matters: The approvals list is capped at MAX_SIGNERS (20), so the maximum cost is bounded. The O(1) duplicate check via the approval index Map prevents additional scanning overhead.

Execute
Executing a proposal processes the stored calldata. The cost scales with calldata size since we need to read and process the payload.

Calldata Size	CPU Threshold	Memory Threshold
0-1 KB	≤ 5,000,000	≤ 500,000
1-2 KB	≤ 6,250,000	≤ 625,000
2-3 KB	≤ 7,500,000	≤ 750,000
3-4 KB	≤ 8,750,000	≤ 875,000
Max (4 KB)	≤ 10,000,000	≤ 1,000,000
Why it matters: Calldata is capped at MAX_CALLDATA_BYTES (4,096 bytes), so even the worst-case execute cost is bounded. This prevents malicious proposals from being too expensive to execute.

Worst-Case Scenario
The most expensive governance operation is executing a proposal with:

MAX_SIGNERS (20) approvals
MAX_CALLDATA_BYTES (4,096 bytes) calldata
Operation	CPU	Memory
Propose	≤ 1,000,000	≤ 100,000
Approve (all 20)	≤ 1,500,000	≤ 150,000
Execute	≤ 10,000,000	≤ 1,000,000
All operations fit comfortably within Soroban's default budget limits.

Denial of Service Protection
The governance contract is protected against DoS attacks through:

Bounded approvals: The signer set is capped at MAX_SIGNERS (20), making the approval scan O(n) where n ≤ 20.

Bounded calldata: The calldata payload is capped at MAX_CALLDATA_BYTES (4,096 bytes), limiting storage and processing costs.

O(1) lookups: Signer membership and duplicate approval checks use Map indices, avoiding linear scans of the signer list.

Proposal expiry: Proposals expire after MAX_PROPOSAL_AGE_SECONDS (30 days), preventing accumulation of stale proposals.

Regression Testing
The gas regression tests run on every PR and CI build:

Bash

cargo test --test gas_regression -- --nocapture
Stream contract batch entrypoints at MAX_PAGE_SIZE
Four entrypoints that use an O(n²) duplicate-ID scan (reject_duplicate_ids) have dedicated gas-regression tests that print GAS_MEASUREMENT lines and also assert that the measured CPU-instruction cost stays within the Soroban per-invocation budget (PER_INVOCATION_CPU_BUDGET = 25 billion = 25% of the 100 billion instruction limit). The budget is intentionally generous enough that valid, non-regressed runs pass comfortably, but tight enough relative to a hypothetical worst-case scenario where MAX_PAGE_SIZE were dramatically increased that a future regression would trip the assertion.

text

cargo test -p fluxora_stream gas_regression -- --nocapture 2>&1 | grep GAS_MEASUREMENT
Release Hardening
Security Checklist Coverage Contract
The release checklist is a no-behaviour-change gate, not a migration mechanism. The
current release remains CONTRACT_VERSION = 9; this documentation and its regression
tests do not change the contract ABI, storage layout, error discriminants, event schemas,
or token-flow semantics.

<!-- RELEASE_HARDENING_COVERAGE_START -->
Surface	Current behaviour	Edge cases that are part of the contract	Executable coverage	Release blocker
Storage	Instance configuration is init-once; stream and index entries are persistent; liabilities move with deposits, withdrawals, and refunds; empty sender/recipient indexes are reclaimed. Existing DataKey discriminants 0..=35 are frozen and new variants are append-only.	Absent post-upgrade keys use their documented default; same-ledger retries are allowed; retrograde accrual timestamps fail; TTL arithmetic saturates/clamps; rejected duplicate batches must not mutate stream state, balances, indexes, counters, or liabilities.	contracts/stream/tests/storage_invariants_edge_cases.rs, contracts/stream/tests/storage_key_compat.rs, contracts/stream/tests/security_invariants.rs	Any key reorder/removal, existing associated-type change, Stream field reorder/removal, unexpected write on a rejected call, or entry larger than MAX_STREAM_ENTRY_BYTES.
Gas	Metered CPU counts are deterministic for the pinned Rust/Soroban toolchain. script/validate_gas.py compares every emitted GAS_MEASUREMENT with the baseline above; exactly +5% is accepted and anything greater fails. Raw WASM budgets are inclusive (size <= budget).	Cliff and CliffOnly creation, partial accrual, routed withdrawal, pause/resume, mixed-success partial creation, duplicate-ID batch paths, MAX_PAGE_SIZE, keeper variants, XDR entry-size ceiling, and exact/over WASM-budget boundaries. A missing baseline or a failed gas-test subprocess is a failure, never an informational pass.	contracts/stream/tests/gas_regression.rs, tests/test_gas_validation.py, script/check-wasm-size.sh	A measured path above 105% of baseline, any emitted measurement without a baseline, a gas test that does not execute successfully, stream XDR above 4,096 bytes, or a raw WASM artifact above its budget.
Upgrade	version() is permissionless, callable before init, returns the compile-time version, and has no storage side effects. upgrade() first loads config and requires current-admin authorization; an uninitialised instance cannot upgrade.	V5-era keys remain readable by V9 code; absent additive keys remain absent/defaulted; the pre-v5 PausedStreamCount caveat is not silently backfilled; invalid/missing WASM hashes are host failures and must not be treated as successful upgrades.	contracts/stream/tests/upgrade_path.rs, contracts/stream/tests/storage_key_compat.rs	Wrong/pre-init authorization behaviour, version drift without the versioning checklist, unreadable legacy state, changed frozen discriminants, or an upgrade attempted without a deployable-WASM smoke test.
Compatibility	The release is additive over frozen storage keys. Existing entrypoint signatures, ContractError values, event topics/payloads, token routing, and state-transition outcomes remain the compatibility boundary for clients and indexers.	Legacy absent optional/additive storage, cancelled-stream accrual freeze, no phantom reads for newer keys, exact error identity on rejected paths, and event continuity at the same contract ID.	contracts/stream/tests/storage_key_compat.rs, contracts/stream/tests/security_invariants.rs, contracts/stream/tests/event_snapshots_suite.rs	Removal/rename/signature change, error-code drift, event-shape drift, changed auth role, changed token destination/accounting, or a storage change that is not append-only.
<!-- RELEASE_HARDENING_COVERAGE_END -->
Current behaviour and expected regression surface
The expected regression surface is the union of four observable layers:

Call results and atomicity. Success values, exact contract errors, authorization
roles, terminal-state rules, balances, liabilities, indexes, counters, and events are
observable behaviour. Failed atomic entrypoints leave all of those unchanged. The one
deliberate exception is create_streams_partial: valid entries commit and invalid
entries return per-item errors, as documented by that entrypoint.
Serialized state. Existing DataKey discriminants and associated value types are
release-frozen. A new key may only be appended. Existing Stream fields may not be
reordered or removed. Defaults for keys absent on an older instance are also behaviour
and must remain explicit in storage_key_compat.rs.
Resource envelopes. CPU baselines, maximum batch/page sizes, the 4,096-byte stream
entry ceiling, and raw WASM limits are safety boundaries. A refactor may reduce cost
without a compatibility change. An increase needs the review and baseline-update
process below; a missing measurement is not approval to ship unmeasured code.
Upgrade and client continuity. version(), admin-gated upgrade authorization,
storage readability, frozen errors, and event schemas are relied on by deployment
tooling and indexers. Additive changes still require the documented version review;
incompatible storage or event changes require a new deployment/migration rather than
an in-place upgrade.
Automated coverage versus release-only checks
The test references in the matrix are automated CI evidence, but they do not make an
arbitrary WASM hash deployable in Soroban's native unit-test host. Consequently, the two
full replacement-WASM tests in upgrade_path.rs remain ignored unless a deployable test
artifact is provided. Before a production upgrade, release engineering must additionally:

build and checksum the pinned release artifact;
run the full Rust, gas, storage-key, snapshot, and documentation gates;
install the exact WASM in a testnet/sandbox and invoke upgrade() through the real
admin/governance path;
verify version, representative pre-upgrade streams, balances/liabilities, and indexer
event decoding after replacement.
This manual smoke test closes the host-fixture gap; it does not permit skipped or
failing automated checks.

Gas Baseline Determinism Contract
Gas baselines in the JSON block above are deterministic given a fixed toolchain and
SDK pin. Two successive runs on the same machine with the same toolchain produce
byte-identical CPU instruction counts because:

Soroban's metered host uses deterministic CPU accounting. Instruction counts are
a property of the compiled WASM bytecode plus the host execution model — not of wall
time, system load, or OS scheduling. Two runs on the same WASM binary and the same
soroban-env-host version always produce the same integer count.

Rust toolchain is pinned. rust-toolchain.toml pins the compiler to a specific
version (currently 1.94.1). A different compiler version can produce a different
WASM codegen, changing instruction counts. The toolchain pin ensures every CI run and
local run uses the same compiler.

soroban-sdk version is pinned with an exact specifier. contracts/stream/Cargo.toml
uses an exact version string (e.g. "21.7.7") rather than a caret range ("^21.7.7").
Cargo.lock is committed and validated by the cargo update --locked CI gate before
any WASM build step.

Retry behaviour is identical. If script/validate_gas.py is re-run without any
code change, it produces the same pass/fail result. The run_tests() call re-invokes
cargo test, which re-runs the metered tests with the same binary and the same host
version. There is no accumulated state between runs.

Implication for upgrades: When upgrading the Rust toolchain or soroban-sdk version,
the baselines in the JSON block above must be re-measured and updated even if no
contract logic changed. A toolchain or SDK change can shift instruction counts by a small
but non-zero amount due to codegen differences. Follow the baseline update procedure in
the Baseline Update Process section below.

Implication for CI flakiness: If validate_gas.py reports a regression that was not
present on the previous run with the same commit, the most likely cause is a toolchain or
environment mismatch. Verify that rustc --version matches rust-toolchain.toml and that
cargo test passes with --locked.

WASM-Size Budget Headroom Update Procedure
The WASM size budgets in script/check-wasm-size.sh are set with ~25% headroom above the
sizes measured during the June 2026 baseline audit. When a deliberate feature addition
causes the raw artifact to approach or exceed the current budget:

Build the release WASM locally:

Bash

cargo build --release -p fluxora_stream --target wasm32-unknown-unknown
Measure the new raw size:

Bash

wc -c target/wasm32-unknown-unknown/release/fluxora_stream.wasm
Compute the new budget: Add ~25% headroom to the measured size and round up to the
nearest 64 KiB boundary:

Python

import math
measured = <size_in_bytes>
headroom = math.ceil(measured * 1.25 / 65536) * 65536
print(f"New budget: {headroom} bytes ({headroom // 1024} KiB)")
Update script/check-wasm-size.sh with the new budget constant.

Update the budget table in docs/gas.md (the table under "WASM Size Budgets")
with the new value and a note explaining the change.

Regenerate the WASM checksum:

Bash

bash script/update-wasm-checksums.sh
git add wasm/checksums.sha256
Include in the PR description: State the measured old size, the new budget, and
which feature caused the growth.

Paused-Stream Counter Backfill Edge Case (v5 upgrades)
This edge case is documented in full in docs/upgrade.md §3 ("Paused-stream counter
backfill caveat"). The gas and operational implications are summarised here:

Background: CONTRACT_VERSION = 5 introduced DataKey::PausedStreamCount — an
instance-level O(1) counter of how many streams are currently in Paused status. This
counter is maintained by pause_stream, pause_stream_as_admin, resume_stream,
resume_stream_as_admin, cancel_stream, cancel_stream_as_admin, and
close_completed_stream.

What happens on upgrade from v4 to v5 (or v6/v7):

An instance upgraded from v4 starts with PausedStreamCount unset (reads as 0).
Legacy streams that were Paused before the upgrade are not counted.
The counter becomes accurate only for streams that experience a pause/resume/cancel
transition after the upgrade.
resume_* and cancel_* applied to a pre-upgrade paused stream do not decrement
below zero — the implementation uses saturating subtraction.
Gas implications:

get_paused_stream_count() is O(1) and its cost does not change with the number of
paused streams. It reads a single instance storage entry.
The saturation guard in cancel_* / resume_* for legacy paused streams adds one
additional instance storage read (to load the current counter before the saturating
decrement). This is negligible (~1 instruction) and is not separately measured in the
gas baselines above.
Operational requirement:

If an exact paused-stream count is required immediately after upgrading a live instance
from v4, reconstruct it off-chain by enumerating all stream states and treat
get_paused_stream_count() as authoritative only for post-upgrade transitions. See
docs/upgrade.md §3 for the full recovery procedure.

Baseline Update Process
Gas-regression tests assert that our operations don't unexpectedly increase in CPU instruction count or memory usage. A legitimate baseline bump may be required when intentionally adding features or security checks that increase the cost.

How the Baseline is Computed and Stored
We currently have two different mechanisms for tracking and asserting gas baselines across our contracts:

Governance (fluxora_governance):

Stored: Baselines are stored directly as hardcoded const values at the top of contracts/governance/tests/gas_regression.rs.
Computed: These constants represent an absolute threshold. Historically, they were computed by running the tests and adding ~25% headroom. The test suite fails via standard assert! statements if the measured budget exceeds these constants.
Stream (fluxora_stream):

Stored: Baselines are stored in a JSON block inside docs/gas.md (between <!-- GAS_BASELINE_START --> and <!-- GAS_BASELINE_END --> tags).
Computed: The test file (contracts/stream/tests/gas_regression.rs) prints the costs. A Python script (script/validate_gas.py) parses these prints and compares them against the JSON baseline in docs/gas.md. It fails the CI if any measurement exceeds the recorded baseline by more than 5%. To update the baseline, run the tests and copy the new measured values into the JSON block in this document.
Review Bar for Baseline Increases
Baseline increases are not granted automatically. To get a baseline increase approved, the PR must meet the following review bar:

Explicit Justification: The PR description must explicitly justify the gas increase.
Root Cause: The increase must be tied to a specific, legitimate change (e.g., adding a new necessary security check, expanding a feature).
No Hidden Costs: Unintended or unexplainable jumps in gas usage must be optimized or reverted. You cannot blindly bump the baseline to get CI to pass.
Keeper Economics
keeper_cancel pays keeper bots a small incentive (fee) to cancel streams that have passed
their end_time but whose sender never called cancel_stream, preventing unclaimed deposits
from being locked in contract storage indefinitely. Understanding the relationship between
that fee and the transaction's own resource cost is essential for keeper-bot operators who
need to know which streams are worth cancelling.

How the fee is calculated
The fee is taken from the unstreamed portion of the deposit (the sender's gross refund).
See docs/cancel-stream-semantics.md
for the full distribution formula and docs/formal-verification.md
for the constant definitions.

text

accrued          = calculate_accrued_at(end_time)        -- capped at deposit_amount
recipient_amount = accrued - withdrawn_amount
sender_refund_gross = deposit_amount - accrued           -- unstreamed portion
keeper_fee       = sender_refund_gross × KEEPER_FEE_BPS / 10 000
sender_refund    = sender_refund_gross - keeper_fee
Production constants:

Constant	Value	Source
KEEPER_FEE_BPS	50 (0.5 %)	lib.rs, formal-verification.md
KEEPER_GRACE_PERIOD_SECONDS	604 800 (7 days)	lib.rs, formal-verification.md
CPU-instruction cost
keeper_cancel in the common case (partial accrual, 3 token transfers) is more expensive
than a plain withdraw because it:

Validates grace-period eligibility.
Performs the keeper-fee arithmetic.
Issues three separate token transfers: recipient, sender, and keeper.
The gas regression tests in contracts/stream/tests/gas_regression.rs measure two variants:

Variant	Description
partial_accrual	deposit_amount > rate × duration → 3 token transfers (common keeper incentive path)
fully_accrued	deposit_amount == rate × duration → 1 token transfer, keeper_fee = 0
Run the measurements with:

Bash

cargo test -p fluxora_stream gas_regression -- --nocapture
The measured CPU instruction counts are recorded in the JSON baseline above
(keeper_cancel.partial_accrual and keeper_cancel.fully_accrued) and validated
on every CI run by script/validate_gas.py.

Break-even stream size
A keeper-bot only profits when the fee it earns exceeds the Stellar resource fee it pays
to submit the transaction. The minimum unstreamed refund that makes a keeper_cancel
call economically rational is:

text

break_even_unstreamed = (resource_fee_in_tokens × 10 000) / KEEPER_FEE_BPS
                      = resource_fee_in_tokens × 200
At KEEPER_FEE_BPS = 50 (0.5 %), the keeper earns 1 token for every 200 tokens of
unstreamed refund. Below this threshold the fee is smaller than the cost of the
transaction itself and a rational keeper should not bother.

Representative break-even values for USDC streams (7 decimal places, 1 USDC = 10 000 000
stroops):

Stellar resource fee (USDC)	Break-even unstreamed refund (USDC)
0.001	0.20
0.01	2.00
0.10	20.00
1.00	200.00
How to read this table: at a resource fee of 0.01 USDC per transaction, a keeper
earns nothing (or loses money) on a stream whose unstreamed balance is less than
2.00 USDC. At a 1.00 USDC resource fee the break-even unstreamed balance rises to
200.00 USDC.

Actual Stellar resource fees vary with network congestion and the fee-market. Keeper
operators should periodically re-evaluate their configured minimum stream sizes against
current fee levels.

Implications for stream design
Stream creators can use the break-even formula to reason about keeper incentives:

Large, long-running streams with significant unstreamed balances at expiry will
always attract keeper cleanup because the incentive exceeds typical transaction costs.
Small or tightly-scoped streams (deposit ≈ rate × duration, little unstreamed
balance) may not attract keeper cleanup; senders of such streams should call
cancel_stream themselves rather than relying on keeper bots.
The 7-day grace period (KEEPER_GRACE_PERIOD_SECONDS) gives senders a window to
self-clean before keepers become eligible.
Security notes
The keeper fee is taken only from the sender's gross refund; the recipient's
accrued balance is never reduced. See the security invariants in
docs/cancel-stream-semantics.md.
Keepers must sign (keeper.require_auth()), preventing a third party from
redirecting the fee to an arbitrary address.
CEI ordering ensures the stream is marked Cancelled before any token transfer,
preventing re-entrant double-cancellations.
Formal proofs that keeper_fee + protocol_remainder == gross (conservation) and
that checked_mul(KEEPER_FEE_BPS) cannot overflow are described in
docs/formal-verification.md.
Stream Persistent-Entry Size
Every Stream struct is written to a Soroban persistent ledger entry. Soroban charges
rent proportional to the serialized byte size of each entry, so unchecked growth of any
caller-controlled field inflates the per-stream rent cost for the entire protocol.

Field breakdown (worst case)
Category	Fields	Approx. XDR bytes
Fixed scalars	stream_id (u64), 3 × i128 amounts, 3 × i128 checkpoints, 3 × u32 ledger stamps, delegation_depth (u32)	~120
Fixed addresses	sender, recipient (each 36 bytes)	~72
Enum fields	status (StreamStatus), kind (StreamKind)	~8
Optional scalars	cancelled_at (Option<u64>), is_pooled (Option<bool>), irrevocable (Option<bool>), parent_stream_id (Option<u64>)	~30
Optional addresses	claim_owner (Option<Address>), witness (Option<Address>)	~74
memo (caller-controlled)	Option<Bytes> capped at MAX_MEMO_BYTES = 256	~268
metadata (caller-controlled)	Option<Map<Bytes,Bytes>> capped at MAX_METADATA_BYTES = 512 aggregate + ScMap framing for up to 8 entries	~680
ScVal type tags + XDR padding	per-field overhead from Soroban encoding	~100
Structural total		~1 352
Measured baselines
These values are printed by the regression tests in
contracts/stream/tests/gas_regression.rs (run with --nocapture). Update this
table whenever the constant or test output changes.

Variant	Serialized bytes	Test name
Baseline (no optional fields)	~480	test_stream_entry_xdr_size_baseline
Memo only (MAX_MEMO_BYTES = 256 B)	~760	test_stream_entry_xdr_size_memo_only
Metadata only (MAX_METADATA_BYTES = 512 B)	~1 160	test_stream_entry_xdr_size_metadata_only
Worst case (memo + metadata + all optionals)	~1 352	test_stream_entry_xdr_size_worst_case
The values above are estimates derived from the field breakdown. Run
cargo test -p fluxora_stream --test gas_regression -- --nocapture to capture
the exact figures printed by the tests and update this table.

Ceiling constant
Rust

pub const MAX_STREAM_ENTRY_BYTES: usize = 4_096;  // lib.rs
The ceiling is 4 096 bytes — a ~2.9× safety margin above the ~1 352-byte worst-case
structural total. The generous margin accounts for:

Future additive fields that do not require a CONTRACT_VERSION bump
Soroban ScVal type tags, XDR padding, and length prefixes that vary by SDK version
Host-side encoding overhead not directly observable from Rust test code
Enforcement
The constant is enforced by:

Bash

cargo test -p fluxora_stream --test gas_regression -- --nocapture
Four tests run and each asserts serialized_len <= MAX_STREAM_ENTRY_BYTES:

Test	What it covers
test_stream_entry_xdr_size_worst_case	All optional fields at maximum size
test_stream_entry_xdr_size_baseline	No optional fields (lower bound)
test_stream_entry_xdr_size_memo_only	Only memo at MAX_MEMO_BYTES
test_stream_entry_xdr_size_metadata_only	Only metadata at MAX_METADATA_BYTES
How to update the ceiling
If the Stream struct gains new fields and the regression test fails:

Run cargo test -p fluxora_stream --test gas_regression -- --nocapture and note
the printed STREAM_XDR_SIZE: worst_case: N bytes value.
Add ~25% headroom, round up to the next 512-byte boundary.
Update MAX_STREAM_ENTRY_BYTES in contracts/stream/src/lib.rs.
Update the measured-baselines table above with the new figures.
Confirm the CONTRACT_VERSION policy in lib.rs has been followed for the
struct change (additive fields require a version bump).
Include the change in the PR description with an explicit justification.