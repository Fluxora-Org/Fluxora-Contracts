# Fix: Structured InsufficientDeposit on extend_stream_end_time Underfunding

## Summary

`extend_stream_end_time` returns `ContractError::InsufficientDeposit` (error code 10)
when the existing deposit does not cover the extended duration. No panic occurs.

## Error Condition

```
deposit_amount < rate_per_second × (new_end_time - start_time)
```

## Client Handling

```rust
match client.try_extend_stream_end_time(&stream_id, &new_end_time) {
    Ok(()) => { /* extension succeeded */ }
    Err(Ok(ContractError::InsufficientDeposit)) => {
        // Top up first, then retry
        client.top_up_stream(&stream_id, &funder, &extra_amount);
        client.extend_stream_end_time(&stream_id, &new_end_time);
    }
    Err(e) => { /* handle other errors */ }
}
```

## Test Coverage

| Test | Scenario |
|------|----------|
| `test_extend_end_time_deposit_one_short_rejected` | Deposit 1 token short |
| `test_extend_end_time_deposit_far_below_new_requirement_rejected` | Large extension, far below requirement |
| `test_extend_end_time_after_top_up_succeeds` | Top-up then extend succeeds |
| `integration_extend_end_time_insufficient_deposit_rejected_no_side_effects` | No state change on failure |

## Error Code Stability

`InsufficientDeposit = 10` — stable, guaranteed not to change.
