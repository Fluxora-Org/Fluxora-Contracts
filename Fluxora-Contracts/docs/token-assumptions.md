# Token Compatibility Assumptions and Non-Goals

This document defines the token trust model, compatibility assumptions, and explicit non-goals for the Fluxora streaming contract. It serves as the authoritative reference for operators, integrators, and auditors reasoning about token interactions using only on-chain observables and published protocol documentation.

## Scope Boundary

**Issue Caption:** Malicious token assumptions: document non-goals

This document covers all material aspects of token compatibility for the Fluxora streaming protocol. Anything intentionally excluded is called out with rationale and residual risk.

## Token Trust Model

### Core Assumption

The Fluxora streaming contract interacts with exactly one token contract, fixed at initialization time and stored in `Config.token`. This token is assumed to be a **well-behaved SEP-41 / SAC (Stellar Asset Contract) token** that adheres to the following behavioral guarantees:

1. **No reentrancy on transfer**: The token contract does not call back into the streaming contract during `transfer` or `transfer_from` operations.
2. **Explicit failure on insufficient balance**: The token contract panics or returns an error when a transfer is attempted with insufficient balance, rather than silently failing or returning success.
3. **Explicit failure on insufficient allowance**: The token contract panics or returns an error when a transfer is attempted without proper allowance, rather than silently failing.
4. **Deterministic behavior**: Token operations produce consistent, predictable results given the same inputs and contract state.
5. **Standard SEP-41 interface**: The token implements the standard Soroban token interface (`transfer`, `transfer_from`, `balance`, `approve`, `allowance`).

### Why This Assumption Matters

The streaming contract's security model depends on these token behaviors:

- **CEI (Checks-Effects-Interactions) pattern**: State updates are performed before external token transfers to reduce reentrancy risk. This mitigation is only effective if the token does not reenter.
- **Atomic transactions**: The contract assumes that if a token transfer fails, the entire transaction reverts. Silent failures would break this invariant.
- **Balance integrity**: The contract tracks `deposit_amount` and `withdrawn_amount` internally. If tokens are silently minted, burned, or transferred outside the contract's control, these internal accounting invariants diverge from actual token balances.

## Explicit Non-Goals

The following behaviors are **intentionally not mitigated** by the streaming contract. Operators, integrators, and auditors should be aware of these boundaries.

### 1. Malicious Token Contracts

**Non-goal**: The streaming contract does not protect against a malicious token contract that violates SEP-41 behavioral guarantees.

**Specific risks**:

- **Reentrancy**: A malicious token could call back into the streaming contract during `transfer`. The CEI pattern reduces impact (state is already persisted), but does not eliminate all risks if the token can observe intermediate state.
- **Silent failures**: A malicious token could return success without actually transferring tokens, breaking the contract's internal accounting invariants.
- **Arbitrary state manipulation**: A malicious token could modify its own state in ways that affect the streaming contract's balance calculations.
- **Event spoofing**: A malicious token could emit events that mislead indexers about actual token movements.

**Rationale**: Mitigating malicious token behavior would require:

- Reentrancy guards (adding gas overhead and complexity)
- Balance verification after every transfer (adding gas overhead)
- Token contract allowlisting (reducing permissionless composability)

These mitigations conflict with the protocol's goals of gas efficiency, simplicity, and permissionless composability. The token address is fixed at initialization and requires admin authorization, providing a single point of trust verification.

**Residual risk**: If a non-standard token violates SEP-41 expectations, transfer behavior may diverge from documented semantics. CEI ordering reduces but cannot fully eliminate external token risk.

### 2. Token Supply Manipulation

**Non-goal**: The streaming contract does not monitor or restrict token supply changes.

**Specific risks**:

- **Inflationary tokens**: A token could mint new tokens to any address, including the streaming contract, without the contract's knowledge.
- **Deflationary tokens**: A token could burn tokens from the streaming contract's balance without the contract's knowledge.
- **Fee-on-transfer tokens**: A token could deduct fees during transfer, causing the actual received amount to differ from the transferred amount.

**Rationale**: Token supply mechanics are outside the scope of the streaming protocol. The contract tracks internal accounting (`deposit_amount`, `withdrawn_amount`) independently of actual token balances. This design choice:

- Simplifies the contract logic
- Reduces gas costs
- Allows integration with any SEP-41 token
- Places responsibility on the token deployer to ensure correct behavior

**Residual risk**: If a token has non-standard supply mechanics, the contract's internal accounting may diverge from actual token balances. Operators should verify token behavior before initialization.

### 3. Token Upgradeability

**Non-goal**: The streaming contract does not protect against token contract upgrades that change behavior.

**Specific risks**:

- **Proxy tokens**: A token contract behind a proxy could be upgraded to change its behavior after the streaming contract is initialized.
- **Governance-controlled tokens**: A token's governance mechanism could change transfer rules, fees, or other behavior.
- **Admin-controlled tokens**: A token admin could pause transfers, change balances, or modify other parameters.

**Rationale**: Token upgradeability is a property of the token contract, not the streaming contract. The streaming contract:

- Stores the token address at initialization (immutable after `init`)
- Assumes the token at that address continues to behave consistently
- Does not monitor token contract upgrades or governance changes

**Residual risk**: If a token contract is upgraded to violate SEP-41 guarantees, the streaming contract's behavior may become unpredictable. Operators should use stable, non-upgradeable tokens or accept upgrade risk.

### 4. Token Balance Verification

**Non-goal**: The streaming contract does not verify that actual token balances match internal accounting.

**Specific risks**:

- **Direct transfers to contract**: Tokens could be sent directly to the streaming contract address without going through `create_stream` or `top_up_stream`. These tokens would be locked permanently with no recovery path.
- **Token recovery**: The contract has no mechanism to recover tokens sent directly to it outside of stream operations.
- **Balance reconciliation**: The contract does not verify that `contract_balance >= sum(deposit_amount - withdrawn_amount)` across all streams.

**Rationale**: Balance verification would require:

- Tracking total deposits and withdrawals (already done via `deposit_amount` and `withdrawn_amount`)
- Querying the token contract's balance on every operation (adding gas overhead)
- Handling edge cases where balances diverge (adding complexity)

The current design:

- Tracks internal accounting independently
- Assumes token transfers succeed or fail explicitly
- Places responsibility on operators to avoid direct transfers

**Residual risk**: Tokens sent directly to the contract address are permanently locked. Operators should never transfer tokens directly to the streaming contract address.

### 5. Token Allowance Management

**Non-goal**: The streaming contract does not manage token allowances on behalf of users.

**Specific risks**:

- **Insufficient allowance**: If a sender does not approve the streaming contract for the required amount, `create_stream` or `top_up_stream` will fail.
- **Allowance front-running**: A sender's allowance could be consumed by another transaction before the streaming contract's transaction executes.
- **Allowance revocation**: A sender could revoke allowance after creating a stream but before a top-up operation.

**Rationale**: Allowance management is the responsibility of the sender (or their wallet/tooling). The contract:

- Uses `transfer_from` to pull tokens from the sender.
- Assumes the sender has approved the contract for at least the `deposit_amount` (for `create_stream`) or the top-up amount (for `top_up_stream`).
- Fails explicitly if allowance is insufficient (the token contract will panic or return an error).
- Standard wallet flow: `token.approve(contract, amount)` followed by `contract.create_stream(...)`.

**Observable behavior on failure**:
- If `allowance < amount`, the transaction reverts before any state is changed.
- If allowance is valid but expires before the transaction, it reverts.

**Batch operations**:
- In `create_streams` (batch), the total allowance must cover the sum of all `deposit_amount` values in the batch. If any single pull fails due to insufficient allowance, the entire batch reverts (atomicity).

**Residual risk**: Senders must ensure sufficient allowance before calling `create_stream` or `top_up_stream`. Wallets and tooling should handle allowance management transparently.

### 6. Token Decimals and Precision

**Non-goal**: The streaming contract does not enforce or verify token decimal precision.

**Specific risks**:

- **Decimal mismatch**: If a token has different decimal precision than expected (e.g., 6 vs 18 decimals), amounts may be misinterpreted.
- **Rounding errors**: Integer division in accrual calculations could cause rounding errors that accumulate over time.
- **Overflow**: Very large token amounts (close to `i128::MAX`) could cause overflow in arithmetic operations.

**Rationale**: Token decimals are a property of the token contract. The streaming contract:

- Uses `i128` for all amounts (standard Soroban token type)
- Performs checked arithmetic to prevent overflow
- Clamps accrued amounts at `deposit_amount` to prevent over-withdrawal

**Residual risk**: Operators should verify token decimal precision before initialization. The contract's arithmetic is safe for all values within `i128` range, but very large amounts may cause overflow in validation checks.

### 7. Numeric Edge Cases

**Non-goal**: The streaming contract does not prevent all numeric edge cases, but provides explicit failure semantics.

**Specific edge cases**:

- **i128::MAX amounts**: Creating a stream with `deposit_amount = i128::MAX` will fail due to overflow in validation checks (`rate * duration` calculation).
- **Rate overflow**: Updating a stream's rate to `i128::MAX` will fail if `new_rate * remaining_duration` overflows.
- **Top-up overflow**: Topping up a stream with `i128::MAX - deposit_amount + 1` will fail due to overflow in `deposit_amount + amount`.
- **Duration overflow**: Creating a stream with `rate = 1` and `duration = i128::MAX` will fail due to overflow in `rate * duration`.

**Observable behavior**: All overflow scenarios produce explicit `ContractError::InvalidParams` errors. No partial state changes occur (atomic rollback).

**Rationale**: The contract uses checked arithmetic (`checked_mul`, `checked_add`) to prevent silent overflow. This ensures that:

- Failures are explicit and observable
- No partial state changes occur
- Integrators can handle errors predictably

**Residual risk**: Operators should avoid creating streams with amounts near `i128::MAX`. The contract will fail explicitly, but operators should validate amounts before submission.

### 8. Stream Lifecycle Timing Edge Cases

**Non-goal**: The streaming contract does not prevent all timing edge cases, but provides explicit semantics for each scenario.

**Specific edge cases**:

- **Withdraw at exact cliff time**: Returns 0 (cliff not yet passed). Accrual starts at `cliff_time + 1`.
- **Withdraw at exact end time**: Returns full deposit (fully accrued).
- **Withdraw after end time**: Returns full deposit (no over-accrual, amounts clamped at `deposit_amount`).
- **Cancel at exact start time**: Refunds full deposit (nothing accrued yet).
- **Cancel at exact end time**: Refunds 0 (fully accrued).
- **Cancel after end time**: Refunds 0 (fully accrued).

**Observable behavior**: All timing edge cases produce predictable results:

- Withdrawals return the accrued amount (clamped at `deposit_amount`)
- Cancellations refund `deposit_amount - accrued_at(cancelled_at)`
- Events are emitted only on successful operations

**Rationale**: The contract's accrual logic is deterministic and handles edge cases explicitly:

- `accrued_at(time) = min(deposit_amount, rate_per_second * (time - start_time))`
- Cliff time is handled by returning 0 for `time < cliff_time`
- End time is handled by clamping at `deposit_amount`

**Residual risk**: None. All timing edge cases are handled explicitly with predictable behavior.

## Observable Behavior Guarantees

Despite the non-goals above, the streaming contract provides the following observable guarantees that integrators can rely on:

### State Transitions

1. **Atomic operations**: All state changes are persisted before external token transfers (CEI pattern). If a token transfer fails, the entire transaction reverts.
2. **Terminal states**: Streams in `Completed` or `Cancelled` states cannot transition to any other state.
3. **Status consistency**: Stream status is always consistent with `withdrawn_amount` and `cancelled_at` fields.

### Authorization

1. **Sender operations**: Only the stream's `sender` can pause, resume, cancel, update rate, or modify end time.
2. **Recipient operations**: Only the stream's `recipient` can withdraw tokens.
3. **Admin operations**: Only the contract `admin` can pause, resume, or cancel streams as an administrative override.
4. **Permissionless operations**: Anyone can call `close_completed_stream` to clean up completed streams.

### Events

1. **Creation events**: `StreamCreated` is emitted when a stream is created, containing all stream parameters.
2. **State change events**: `Paused`, `Resumed`, `StreamCancelled`, `StreamCompleted`, `StreamClosed` are emitted on state transitions.
3. **Withdrawal events**: `Withdrawal` and `WithdrawalTo` are emitted when tokens are withdrawn, containing the amount and recipient/destination.
4. **Parameter update events**: `RateUpdated`, `StreamEndShortened`, `StreamEndExtended`, `StreamToppedUp` are emitted when stream parameters are modified.

### Error Behavior

1. **Explicit errors**: All failures produce explicit `ContractError` variants that integrators can handle.
2. **No silent failures**: The contract never silently succeeds when it should fail.
3. **Atomic rollback**: Any failure causes the entire transaction to revert, leaving no partial state changes.

## Verification and Evidence

### Automated Tests

The following test scenarios verify token interaction behavior:

1. **Successful transfers**: Tests verify that tokens are transferred correctly on stream creation, withdrawal, cancellation, and top-up.
2. **Insufficient balance**: Tests verify that `create_stream` fails when the sender has insufficient token balance.
3. **Insufficient allowance**: Tests verify that `create_stream` fails when the sender has not approved the contract.
4. **Zero withdrawable**: Tests verify that `withdraw` returns 0 without transferring tokens when nothing is withdrawable.
5. **CEI ordering**: Tests verify that state is persisted before token transfers in all operations.

### Audit Notes

The following scenarios cannot be automatically tested and require manual audit:

1. **Malicious token reentrancy**: Cannot be tested with standard Soroban test utilities. Requires manual review of CEI ordering and state persistence.
2. **Token upgradeability**: Cannot be tested in unit tests. Requires verification that the token contract is not upgradeable or that upgrade risk is accepted.
3. **Fee-on-transfer tokens**: Cannot be tested with standard SAC tokens. Requires manual review of token behavior or integration tests with custom token implementations.

### Residual Risks

1. **Non-standard tokens**: If a token violates SEP-41 guarantees, the streaming contract's behavior may become unpredictable. Mitigation: Use only well-audited, standard SEP-41 tokens.
2. **Direct transfers**: Tokens sent directly to the contract address are permanently locked. Mitigation: Operator education and wallet/tooling warnings.
3. **Token upgrades**: If a token contract is upgraded to violate SEP-41 guarantees, the streaming contract's behavior may change. Mitigation: Use stable, non-upgradeable tokens or accept upgrade risk.

## Integration Guidelines

### For Wallet Developers

1. **Allowance management**: Handle token approval transparently before calling `create_stream` or `top_up_stream`.
2. **Error handling**: Display explicit error messages from `ContractError` variants to users.
3. **Event indexing**: Index all stream events to provide real-time updates to users.
4. **Balance warnings**: Warn users if they attempt to transfer tokens directly to the streaming contract address.

### For Indexer Developers

1. **Event parsing**: Parse all event types defined in `StreamCreated`, `StreamEvent`, `Withdrawal`, `WithdrawalTo`, `RateUpdated`, `StreamEndShortened`, `StreamEndExtended`, and `StreamToppedUp`.
2. **State reconstruction**: Reconstruct stream state from events and on-chain storage to verify consistency.
3. **Error tracking**: Track failed transactions and their error codes for debugging.

### For Treasury Operators

1. **Token selection**: Use only well-audited, standard SEP-41 tokens with known behavior.
2. **Balance monitoring**: Monitor the contract's token balance and compare with internal accounting to detect anomalies.
3. **Upgrade risk**: If using upgradeable tokens, monitor token contract upgrades and their potential impact on streaming behavior.
4. **Direct transfer prevention**: Implement controls to prevent accidental direct transfers to the streaming contract address.

## References

- **SEP-41**: Soroban Token Interface Standard
- **SAC**: Stellar Asset Contract
- **CEI Pattern**: Checks-Effects-Interactions pattern for reentrancy risk reduction
- **Security Documentation**: [`security.md`](security.md) for detailed security patterns
- **Streaming Documentation**: [`streaming.md`](streaming.md) for stream lifecycle and semantics
