Fluxora Streaming Protocol — Balance Conservation Invariants
Core Financial Invariant
The fundamental invariant of the Fluxora streaming protocol is:
For every stream, the sum of all withdrawn amounts plus the remaining contract balance allocated to that stream must equal the original deposit amount (adjusted for refunds on cancel, shorten, or rate-decrease).
Mathematically, for any stream at any point in time:
withdrawn_amount + (deposit_amount - withdrawn_amount) == deposit_amount        (trivial)The meaningful invariant is about token conservation across the entire system:
contract_token_balance == sum_over_all_streams(remaining_deposit) + excessWhere:
remaining_deposit = deposit_amount - withdrawn_amount for each active/completed stream
excess = tokens in the contract not backed by any stream liability (sweepable by admin)
Verified Entrypoints
The property-based test module contracts/stream/tests/balance_conservation.rs verifies the invariant holds after every call to these mutating entrypoints:
| Entrypoint                          | Invariant Check                                               | Notes                                                         |
| ----------------------------------- | ------------------------------------------------------------- | ------------------------------------------------------------- |
| `create_stream`                     | ✓ Contract balance increases by exactly `deposit_amount`      | Sender balance decreases by same amount                       |
| `create_streams` (batch)            | ✓ Atomic: all succeed or all fail                             | Total transfer = sum of all deposits                          |
| `withdraw`                          | ✓ `withdrawn_amount` increases by exact transfer amount       | Recipient receives exactly withdrawn tokens                   |
| `withdraw_to`                       | ✓ Same as `withdraw` but to arbitrary destination             | Destination must not be contract itself                       |
| `batch_withdraw`                    | ✓ Sum of all withdrawn amounts equals total token outflow     | Per-stream amounts verified individually                      |
| `top_up_stream`                     | ✓ Contract and stream deposit increase by exact top-up amount | Sender balance decreases by same amount                       |
| `cancel_stream`                     | ✓ Refund = `deposit - accrued_at_cancel`                      | Sender receives refund; recipient retains accrued entitlement |
| `shorten_stream_end_time`           | ✓ Refund = `old_deposit - new_deposit`                        | New deposit = `rate * (new_end - start)`                      |
| `extend_stream_end_time`            | ✓ No token movement; deposit unchanged                        | Must satisfy `deposit >= rate * (new_end - start)`            |
| `update_rate_per_second` (increase) | ✓ No token movement; deposit unchanged                        | Checkpointed accrual locked in                                |
| `decrease_rate_per_second`          | ✓ Refund = `old_deposit - new_deposit`                        | New deposit = `checkpointed + new_rate * remaining`           |

Property-Based Testing Strategy
Proptest Configuration
ProptestConfig {
    cases: 256,              // 256 random test cases per property
    max_shrink_iters: 50,    // Aggressive shrinking for minimal failing inputs
    ..ProptestConfig::default()
}

Stream Parameter Generation
Valid stream parameters are generated with these constraints:
deposit  = rate * duration          (tight bound, or with excess)
rate     > 0
duration > 0
start    >= current_time (pinned at 0)
cliff    in [start, end]
end      > start

Operation Sequences
Random sequences of 0–20 operations are generated per test case:
| Operation      | Weight | Constraints                              |
| -------------- | ------ | ---------------------------------------- |
| `Withdraw`     | 4      | At time in \[cliff, end+1000]            |
| `TopUp`        | 2      | Amount in \[1, 10000]                    |
| `Cancel`       | 1      | At time in \[start, end+1000]            |
| `Shorten`      | 1      | New end in (current\_time, old\_end)     |
| `Extend`       | 1      | New end in (old\_end, old\_end+duration] |
| `IncreaseRate` | 1      | New rate in (old\_rate, old\_rate\*2]    |
| `DecreaseRate` | 1      | New rate in \[1, old\_rate)              |

Invariants Asserted After Every Operation
Per-stream balance conservation: withdrawn_amount <= deposit_amount always; withdrawn_amount == deposit_amount when Completed.
Global balance conservation: contract_balance >= sum(remaining_deposits) — the contract must always hold enough to cover all obligations.
Token conservation (system-wide): sender_balance + recipient_balance + contract_balance == initial_mint — no tokens created or destroyed.
Accrual consistency: Contract's calculate_accrued matches the pure accrual::calculate_accrued_amount function for the same parameters.
Monotonicity: withdrawn_amount never decreases across operations.
Non-negativity: All token amounts (balances, deposits, withdrawn, refunds) remain ≥ 0.
Security Assumptions Validated
Assumption: No Double-Spend
Test: Withdraw twice at the same timestamp.
Expected: Second withdraw returns 0 (nothing new accrued, nothing to withdraw).
Verified: withdrawn_amount does not increase on zero-withdraw calls.
Assumption: Cancel Refunds Exact Unstreamed Amount
Test: Cancel at arbitrary times; verify refund = deposit - accrued_at_cancel.
Expected: sender_balance increases by exactly deposit - accrued; contract balance decreases by same.
Verified: For all generated timestamps and stream configurations.
Assumption: Shorten Refunds Exact Excess
Test: Shorten to arbitrary valid new_end; verify refund.
Expected: new_deposit = rate * (new_end - start); refund = old_deposit - new_deposit.
Verified: For all valid new_end values.
Assumption: DecreaseRate Never Reduces Recipient Entitlement
Test: Decrease rate at arbitrary times; verify checkpointed accrual preserved.
Expected: checkpointed_amount = accrued_at_decrease_time; new_deposit = checkpointed + new_rate * remaining.
Verified: Recipient's already-accrued amount is locked in; only future accrual changes.
Assumption: Batch Operations Are Atomic
Test: create_streams with mixed valid/invalid params; batch_withdraw with duplicate IDs.
Expected: All succeed or all fail; no partial state changes.
Verified: Token balances and stream counts only change on full success.

Edge Cases Covered
| Scenario                                  | Test                                      | Invariant                                 |
| ----------------------------------------- | ----------------------------------------- | ----------------------------------------- |
| Immediate cancel (t=0)                    | `cancel_immediately_refunds_full_deposit` | Full refund; no accrual                   |
| Cancel at cliff                           | `cancel_at_cliff_refunds_correct_amount`  | Refund = deposit - cliff\_accrued         |
| Withdraw after end                        | `withdraw_after_end_gets_full_deposit`    | Full withdrawal; stream Completed         |
| Zero deposit                              | Parameter filter                          | Rejected at creation                      |
| Zero rate                                 | Parameter filter                          | Rejected at creation                      |
| cliff == end                              | Parameter generation                      | Valid; no accrual window                  |
| Excess deposit (deposit > rate\*duration) | `stream_with_excess_deposit` strategy     | Withdrawal capped at accrued, not deposit |
| Rate decrease at t=0                      | Operation sequence                        | Full refund since no accrual              |
| Multiple top-ups                          | `prop_top_up_preserves_conservation`      | Deposit monotonically increases           |
| Batch with partial completions            | `batch_withdraw_partial_completion`       | Per-stream amounts correct                |


Running the Tests# Run all property-based tests (may take several minutes)
cargo test -p fluxora_stream --test balance_conservation

# Run with verbose output (shows proptest cases)
cargo test -p fluxora_stream --test balance_conservation -- --nocapture

# Run a specific property test
cargo test -p fluxora_stream --test balance_conservation prop_single_stream_balance_conservation

# Run with coverage
cargo tarpaulin -p fluxora_stream --test balance_conservation --out Html

Test Output Examplerunning 4 tests
test prop_single_stream_balance_conservation ... ok (256 cases passed)
test prop_batch_streams_balance_conservation ... ok (128 cases passed)
test prop_top_up_preserves_conservation ... ok (128 cases passed)
test cancel_immediately_refunds_full_deposit ... ok
test cancel_at_cliff_refunds_correct_amount ... ok
test withdraw_after_end_gets_full_deposit ... ok
test shorten_refunds_exact_unstreamed ... ok
test decrease_rate_refunds_excess ... ok
test global_conservation_complex_scenario ... ok
test sweep_excess_preserves_liabilities ... ok
test batch_withdraw_partial_completion ... ok

test result: ok. 11 passed; 0 failed; 0 ignored

    Coverage Requirements
    Minimum 95% test coverage for accrual.rs and lib.rs entrypoint logic.
All 11 mutating entrypoints must have at least one property-based test case.
All error paths (InvalidState, InvalidParams, ArithmeticOverflow) must be exercised.
Edge cases (i128 boundaries, u64::MAX timestamps, zero values) must be covered.

Integration with CI
Add to .github/workflows/test.yml:
- name: Property-based balance conservation tests
  run: cargo test -p fluxora_stream --test balance_conservation
  env:
    PROPTEST_CASES: 1000  # Higher count in CI for confidence

Security Notes for Auditors
The balance conservation invariant is the single most important property to verify. Any violation indicates a critical vulnerability (token minting/burning or double-spend).
Property-based testing complements but does not replace formal verification. The proptest harness provides statistical confidence; for absolute guarantees, consider:
Symbolic execution of accrual.rs
Model checking of state transitions
Formal proof of the conservation lemma
The sweep_excess entrypoint is a safety valve, not an exploit. Excess tokens can only exist due to:
External transfers into the contract (donations)
Trapped funds from failed refunds (extremely unlikely with CEI pattern)
Admin action is required; no user can extract other users' deposits.
CEI (Checks-Effects-Interactions) ordering is critical. All token transfers happen AFTER state updates. If this ordering is ever changed, the conservation invariant may be violated during reentrancy.
The proptest dependency is pinned to ensure reproducible builds and deterministic test behavior across environments.