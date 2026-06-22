# Gas Profiling and Budget Review

This document describes the gas (CPU and Memory) costs for the Fluxora streaming contract.

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

## WASM Size Budgets

Fluxora CI builds every deployable contract as release WASM with the
`wasm32-unknown-unknown` target, optionally runs `stellar contract optimize`,
and then enforces a per-contract byte budget with
`script/check-wasm-size-budget.sh`. The check fails if any required raw WASM
artifact is missing or if either the raw or optimized artifact exceeds the
documented budget.

| Contract | Budget bytes | Budget rationale |
|----------|--------------|------------------|
| `fluxora_stream` | 262,144 | Largest contract; caps growth at the 256 KiB review threshold while the current stream source is still being consolidated. |
| `fluxora_factory` | 98,304 | Thin treasury policy wrapper; allows policy and event growth without approaching stream size. |
| `fluxora_governance` | 65,536 | Timelock and signer-management contract; local `wasm32` build measured 35,666 bytes before the stream package stopped the workspace build. |

The CI report is written to
`target/wasm32-unknown-unknown/release/wasm-size-report.md` and uploaded with
the WASM artifacts. If a contract intentionally grows beyond its budget, update
this table in the same PR as the budget change and explain the additional
deployment cost or liveness trade-off.

Local commands:

```bash
cargo build --release --target wasm32-unknown-unknown \
  -p fluxora_stream -p fluxora_factory -p fluxora_governance
bash script/check-wasm-size-budget.sh --no-build
```

The default budgets can be overridden for exploratory local runs with
`FLUXORA_STREAM_WASM_BUDGET_BYTES`,
`FLUXORA_FACTORY_WASM_BUDGET_BYTES`, and
`FLUXORA_GOVERNANCE_WASM_BUDGET_BYTES`. Do not rely on overrides in CI unless
the documented table is updated to match.
