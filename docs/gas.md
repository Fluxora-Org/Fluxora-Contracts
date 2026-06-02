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
