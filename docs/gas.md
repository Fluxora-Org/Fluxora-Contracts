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
  }
}
<!-- GAS_BASELINE_END -->

*Note: Baselines are currently initialized to 0 and should be updated after the first successful run of `script/validate_gas.py` once the contract compiles.*
