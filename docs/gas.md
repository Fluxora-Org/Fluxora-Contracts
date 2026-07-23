# Gas Profiling and Budget Review

This document describes the gas (CPU and Memory) costs for the Fluxora streaming contract.

---

## WASM Size Budgets

Every CI build compiles all three contracts to `wasm32-unknown-unknown --release` and asserts
that the resulting artifact stays within its byte budget. A contract that exceeds its budget
fails the `wasm-size-budget` CI job.

Budgets were set with **~25% headroom** above the sizes measured during the June 2026 baseline
audit. Soroban's practical upload ceiling is ~100 KiB after Brotli compression; raw WASM budgets
are intentionally more conservative to leave room for future features and keep upload fees low.

| Contract | Budget | Notes |
|---|---|---|
| `fluxora_stream` | 256 KiB (262 144 bytes) | Largest contract; full streaming surface area |
| `fluxora_factory` | 128 KiB (131 072 bytes) | Policy wrapper; should stay small |
| `fluxora_governance` | 128 KiB (131 072 bytes) | Minimal timelock; should stay small |

### Enforcement

The `script/check-wasm-size.sh` script implements the check:

```bash
# Check raw release artifacts (run locally after a WASM build):
bash script/check-wasm-size.sh

# Check optimized artifacts (after running stellar contract optimize):
bash script/check-wasm-size.sh --optimized
```

The `wasm-size-budget` CI job:
1. Builds all three contracts with `cargo build --release --workspace --target wasm32-unknown-unknown`.
2. Runs `stellar contract optimize` on each artifact (best-effort; failures are non-fatal).
3. Calls `script/check-wasm-size.sh` — **fails the job** if any artifact exceeds its budget.

### Updating a Budget

If a deliberate, reviewed feature addition requires more space:

1. Land the feature and measure the new raw size locally.
2. Add ~25% headroom to the measured size, rounding up to the nearest 64 KiB boundary.
3. Update the budget constant in `script/check-wasm-size.sh`.
4. Update the table above with the new value and a note explaining the change.
5. Include the change in the PR description.

### Optimize step

`stellar contract optimize` runs `wasm-opt -Oz` on the artifact, typically reducing binary
size by 10–30%. CI runs this step and checks the resulting `.optimized.wasm` file as an
informational pass. The hard budget gate runs against the **raw** release artifact so that the
check remains reproducible without the Stellar CLI installed.

---

## Safe Batch Limits

| Operation | Batch Size | Recommended CPU Budget |
|-----------|------------|------------------------|
| `create_streams` | 1 | 1.5M |
| `create_streams` | 10 | 10M |
| `create_streams` | 50 | 40M |
| `batch_withdraw` | 1 | 1.0M |
| `batch_withdraw` | 10 | 6M |
| `batch_withdraw` | 50 | 20M |
| `batch_withdraw` | 100 | 35M |

## Hot Path Analysis

### `withdraw`
The `withdraw` function is the most common operation. Its cost is dominated by:
1. Loading the `Stream` state.
2. Accrual calculation.
3. Token transfer (external call).
4. Saving updated `withdrawn_amount`.

### `batch_withdraw`
To reduce gas, `batch_withdraw` optimizes by:
1. Caching the ledger timestamp.
2. Performing a single authorization check.
3. Processing multiple streams in a loop.

## Performance Metrics

The following table provides the CPU instruction counts for core operations.

<!-- GAS_BASELINE_START -->
{
  "create_stream": 0,
  "withdraw": 0,
  "batch_withdraw": {
    "1": 0,
    "10": 0,
    "50": 0,
    "100": 0
  },
  "keeper_cancel": {
    "partial_accrual": 0,
    "fully_accrued": 0
  }
}
<!-- GAS_BASELINE_END -->

*Note: Baselines are currently initialized to 0 and should be updated after the first successful run of `script/validate_gas.py` once the contract compiles.*

## Governance Operations

The governance contract (`fluxora_governance`) handles proposal creation, approval, and execution with bounded costs to prevent DoS attacks.

### Budget Thresholds

These thresholds are enforced by the gas regression tests in `contracts/governance/tests/gas_regression.rs`. CI will fail if any operation exceeds its budget.

#### Propose

Creating a new proposal stores the calldata and proposal metadata. Cost is independent of signer count since we don't iterate over signers during creation.

| Metric | Threshold | Notes |
|--------|-----------|-------|
| CPU Instructions | ≤ 1,000,000 | Independent of calldata size |
| Memory Bytes | ≤ 100,000 | Independent of calldata size |

The calldata is capped at `MAX_CALLDATA_BYTES` (4,096 bytes) to keep storage costs reasonable.

#### Approve

Approving a proposal involves checking the signer's membership (O(1) via Map index) and appending to the approvals list. The cost scales linearly with the number of existing approvals since we store them as a Vec.

| Signer Count | CPU Threshold | Memory Threshold |
|--------------|---------------|------------------|
| 1-5          | ≤ 375,000 + 75,000 per signer | ≤ 37,500 + 7,500 per signer |
| 6-10         | ≤ 750,000 + 75,000 per signer | ≤ 75,000 + 7,500 per signer |
| 11-20        | ≤ 1,125,000 + 75,000 per signer | ≤ 112,500 + 7,500 per signer |
| Max (20)     | ≤ 1,500,000 | ≤ 150,000 |

**Why it matters**: The approvals list is capped at `MAX_SIGNERS` (20), so the maximum cost is bounded. The O(1) duplicate check via the approval index Map prevents additional scanning overhead.

#### Execute

Executing a proposal processes the stored calldata. The cost scales with calldata size since we need to read and process the payload.

| Calldata Size | CPU Threshold | Memory Threshold |
|---------------|---------------|------------------|
| 0-1 KB        | ≤ 5,000,000 | ≤ 500,000 |
| 1-2 KB        | ≤ 6,250,000 | ≤ 625,000 |
| 2-3 KB        | ≤ 7,500,000 | ≤ 750,000 |
| 3-4 KB        | ≤ 8,750,000 | ≤ 875,000 |
| Max (4 KB)    | ≤ 10,000,000 | ≤ 1,000,000 |

**Why it matters**: Calldata is capped at `MAX_CALLDATA_BYTES` (4,096 bytes), so even the worst-case execute cost is bounded. This prevents malicious proposals from being too expensive to execute.

### Worst-Case Scenario

The most expensive governance operation is executing a proposal with:
- `MAX_SIGNERS` (20) approvals
- `MAX_CALLDATA_BYTES` (4,096 bytes) calldata

| Operation | CPU | Memory |
|-----------|-----|--------|
| Propose | ≤ 1,000,000 | ≤ 100,000 |
| Approve (all 20) | ≤ 1,500,000 | ≤ 150,000 |
| Execute | ≤ 10,000,000 | ≤ 1,000,000 |

All operations fit comfortably within Soroban's default budget limits.

### Denial of Service Protection

The governance contract is protected against DoS attacks through:

1. **Bounded approvals**: The signer set is capped at `MAX_SIGNERS` (20), making the approval scan O(n) where n ≤ 20.

2. **Bounded calldata**: The calldata payload is capped at `MAX_CALLDATA_BYTES` (4,096 bytes), limiting storage and processing costs.

3. **O(1) lookups**: Signer membership and duplicate approval checks use Map indices, avoiding linear scans of the signer list.

4. **Proposal expiry**: Proposals expire after `MAX_PROPOSAL_AGE_SECONDS` (30 days), preventing accumulation of stale proposals.

### Regression Testing

The gas regression tests run on every PR and CI build:

```bash
cargo test --test gas_regression -- --nocapture
```

## Baseline Update Process

Gas-regression tests assert that our operations don't unexpectedly increase in CPU instruction count or memory usage. A legitimate baseline bump may be required when intentionally adding features or security checks that increase the cost.

### How the Baseline is Computed and Stored

We currently have two different mechanisms for tracking and asserting gas baselines across our contracts:

1. **Governance (`fluxora_governance`)**:
   - **Stored**: Baselines are stored directly as hardcoded `const` values at the top of `contracts/governance/tests/gas_regression.rs`.
   - **Computed**: These constants represent an absolute threshold. Historically, they were computed by running the tests and adding ~25% headroom. The test suite fails via standard `assert!` statements if the measured budget exceeds these constants.

2. **Stream (`fluxora_stream`)**:
   - **Stored**: Baselines are stored in a JSON block inside `docs/gas.md` (between `<!-- GAS_BASELINE_START -->` and `<!-- GAS_BASELINE_END -->` tags).
   - **Computed**: The test file (`contracts/stream/tests/gas_regression.rs`) prints the costs. A Python script (`script/validate_gas.py`) parses these prints and compares them against the JSON baseline in `docs/gas.md`. It fails the CI if any measurement exceeds the recorded baseline by more than 5%. To update the baseline, run the tests and copy the new measured values into the JSON block in this document.

### Review Bar for Baseline Increases

Baseline increases are not granted automatically. To get a baseline increase approved, the PR must meet the following review bar:

- **Explicit Justification**: The PR description must explicitly justify the gas increase.
- **Root Cause**: The increase must be tied to a specific, legitimate change (e.g., adding a new necessary security check, expanding a feature).
- **No Hidden Costs**: Unintended or unexplainable jumps in gas usage must be optimized or reverted. You cannot blindly bump the baseline to get CI to pass.

---

## Keeper Economics

`keeper_cancel` pays keeper bots a small incentive (fee) to cancel streams that have passed
their `end_time` but whose sender never called `cancel_stream`, preventing unclaimed deposits
from being locked in contract storage indefinitely.  Understanding the relationship between
that fee and the transaction's own resource cost is essential for keeper-bot operators who
need to know which streams are worth cancelling.

### How the fee is calculated

The fee is taken from the unstreamed portion of the deposit (the *sender's gross refund*).
See [docs/cancel-stream-semantics.md](cancel-stream-semantics.md#keeper-initiated-cancellation-keeper_cancel)
for the full distribution formula and [docs/formal-verification.md](formal-verification.md#constants-production-values)
for the constant definitions.

```
accrued          = calculate_accrued_at(end_time)        -- capped at deposit_amount
recipient_amount = accrued - withdrawn_amount
sender_refund_gross = deposit_amount - accrued           -- unstreamed portion
keeper_fee       = sender_refund_gross × KEEPER_FEE_BPS / 10 000
sender_refund    = sender_refund_gross - keeper_fee
```

Production constants:

| Constant                        | Value            | Source                            |
|---------------------------------|------------------|-----------------------------------|
| `KEEPER_FEE_BPS`                | 50 (0.5 %)       | `lib.rs`, `formal-verification.md` |
| `KEEPER_GRACE_PERIOD_SECONDS`   | 604 800 (7 days) | `lib.rs`, `formal-verification.md` |

### CPU-instruction cost

`keeper_cancel` in the common case (partial accrual, 3 token transfers) is more expensive
than a plain `withdraw` because it:

1. Validates grace-period eligibility.
2. Performs the keeper-fee arithmetic.
3. Issues **three** separate token transfers: recipient, sender, and keeper.

The gas regression tests in `contracts/stream/tests/gas_regression.rs` measure two variants:

| Variant           | Description                                              |
|-------------------|----------------------------------------------------------|
| `partial_accrual` | `deposit_amount > rate × duration` → 3 token transfers (common keeper incentive path) |
| `fully_accrued`   | `deposit_amount == rate × duration` → 1 token transfer, `keeper_fee = 0` |

Run the measurements with:

```bash
cargo test -p fluxora_stream gas_regression -- --nocapture
```

The measured CPU instruction counts are recorded in the JSON baseline above
(`keeper_cancel.partial_accrual` and `keeper_cancel.fully_accrued`) and validated
on every CI run by `script/validate_gas.py`.

### Break-even stream size

A keeper-bot only profits when the fee it earns exceeds the Stellar resource fee it pays
to submit the transaction.  The minimum *unstreamed refund* that makes a keeper_cancel
call economically rational is:

```
break_even_unstreamed = (resource_fee_in_tokens × 10 000) / KEEPER_FEE_BPS
                      = resource_fee_in_tokens × 200
```

At `KEEPER_FEE_BPS = 50` (0.5 %), the keeper earns 1 token for every 200 tokens of
unstreamed refund.  Below this threshold the fee is smaller than the cost of the
transaction itself and a rational keeper should not bother.

Representative break-even values for USDC streams (7 decimal places, 1 USDC = 10 000 000
stroops):

| Stellar resource fee (USDC) | Break-even unstreamed refund (USDC) |
|-----------------------------|------------------------------------|
| 0.001                       | 0.20                               |
| 0.01                        | 2.00                               |
| 0.10                        | 20.00                              |
| 1.00                        | 200.00                             |

> **How to read this table**: at a resource fee of 0.01 USDC per transaction, a keeper
> earns nothing (or loses money) on a stream whose unstreamed balance is less than
> **2.00 USDC**.  At a 1.00 USDC resource fee the break-even unstreamed balance rises to
> **200.00 USDC**.

Actual Stellar resource fees vary with network congestion and the fee-market.  Keeper
operators should periodically re-evaluate their configured minimum stream sizes against
current fee levels.

### Implications for stream design

Stream creators can use the break-even formula to reason about keeper incentives:

- **Large, long-running streams** with significant unstreamed balances at expiry will
  always attract keeper cleanup because the incentive exceeds typical transaction costs.
- **Small or tightly-scoped streams** (deposit ≈ rate × duration, little unstreamed
  balance) may not attract keeper cleanup; senders of such streams should call
  `cancel_stream` themselves rather than relying on keeper bots.
- The 7-day grace period (`KEEPER_GRACE_PERIOD_SECONDS`) gives senders a window to
  self-clean before keepers become eligible.

### Security notes

- The keeper fee is taken **only** from the sender's gross refund; the recipient's
  accrued balance is never reduced.  See the security invariants in
  [docs/cancel-stream-semantics.md](cancel-stream-semantics.md#security-invariants).
- Keepers must sign (`keeper.require_auth()`), preventing a third party from
  redirecting the fee to an arbitrary address.
- CEI ordering ensures the stream is marked `Cancelled` before any token transfer,
  preventing re-entrant double-cancellations.
- Formal proofs that `keeper_fee + protocol_remainder == gross` (conservation) and
  that `checked_mul(KEEPER_FEE_BPS)` cannot overflow are described in
  [docs/formal-verification.md](formal-verification.md#keeper-fee-conservation-proofs-new).
