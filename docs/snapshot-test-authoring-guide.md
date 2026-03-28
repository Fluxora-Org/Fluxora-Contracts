# Snapshot Test Authoring Guide

## Purpose

This guide provides detailed instructions for writing high-quality snapshot tests that capture the complete externally observable behavior of the Fluxora streaming contract.

## Core Principles

### 1. Test One Scenario Per Test

Each test should validate exactly one scenario with clear success or failure semantics.

**Good:**

```rust
#[test]
fn test_withdraw_before_cliff_panics() {
    // Tests one specific failure case
}
```

**Bad:**

```rust
#[test]
fn test_withdraw_various_scenarios() {
    // Tests multiple unrelated scenarios
}
```

### 2. Make Authorization Explicit

Every test must explicitly document who is authorized to perform the operation.

**Good:**

```rust
/// # Authorization
/// - Requires: Recipient signature
/// - Unauthorized: Sender, admin, or third parties cannot withdraw
#[test]
fn test_withdraw_requires_recipient_authorization() {
    let ctx = TestContext::setup_strict(); // No mock_all_auths
    // Explicit authorization testing
}
```

### 3. Capture Complete Observable Behavior

Tests must verify all externally observable effects:

- Storage state changes
- Event emissions
- Token transfers
- Error codes

**Good:**

```rust
#[test]
fn test_cancel_stream_partial_refund() {
    // ... setup ...

    let result = ctx.client().cancel_stream(&stream_id);

    // Verify storage
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify token transfer
    assert_eq!(ctx.token().balance(&ctx.sender), expected_refund);

    // Verify event
    let events = ctx.env.events().all();
    // ... event assertions ...
}
```

## Test Structure Template

```rust
/// [One-line description of what this test validates]
///
/// # Authorization
/// - Requires: [who must authorize this operation]
/// - Proof: [what authorization is needed]
///
/// # Success Semantics
/// - State: [expected state changes]
/// - Events: [expected events emitted]
/// - Tokens: [expected token movements]
/// - Storage: [expected storage updates]
///
/// # Failure Semantics (if applicable)
/// - Error: [expected error code]
/// - Side effects: [none, or specific rollback behavior]
///
/// # Edge Cases Covered
/// - [specific edge case 1]
/// - [specific edge case 2]
#[test]
fn test_operation_scenario_outcome() {
    // 1. SETUP: Create test context and initial state
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create any prerequisite state
    let stream_id = ctx.create_default_stream();

    // 2. ARRANGE: Set up specific conditions for this test
    ctx.env.ledger().set_timestamp(500);

    // 3. ACT: Perform the operation under test
    let result = ctx.client().operation(&stream_id);

    // 4. ASSERT: Verify all observable behavior

    // 4a. Verify return value
    assert_eq!(result, expected_value);

    // 4b. Verify storage state
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.field, expected_value);

    // 4c. Verify token balances
    assert_eq!(ctx.token().balance(&ctx.recipient), expected_balance);

    // 4d. Verify events
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    // ... event assertions ...

    // Snapshot is automatically captured by soroban-sdk
}
```

## Authorization Testing Patterns

### Pattern 1: Permissioned Operation (Sender)

```rust
#[test]
fn test_pause_requires_sender_authorization() {
    let ctx = TestContext::setup_strict(); // No mock_all_auths
    let stream_id = ctx.create_default_stream();

    // Mock authorization from sender
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
}
```

### Pattern 2: Unauthorized Access Attempt

```rust
#[test]
fn test_withdraw_rejects_non_recipient() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();

    let attacker = Address::generate(&ctx.env);

    // Mock authorization from attacker (not recipient)
    ctx.env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = ctx.client().try_withdraw(&stream_id);
    assert!(result.is_err(), "non-recipient must not be able to withdraw");
}
```

### Pattern 3: Admin Override

```rust
#[test]
fn test_admin_can_pause_any_stream() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();

    // Admin pauses stream (not sender)
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
}
```

## Event Verification Patterns

### Pattern 1: Verify Event Emission

```rust
#[test]
fn test_create_stream_emits_event() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1,
        &0,
        &0,
        &1000,
    );

    // Get all events
    let events = ctx.env.events().all();
    let event = events.last().unwrap();

    // Verify event topic
    assert_eq!(event.0, ctx.contract_id);

    // Verify event data
    let event_data = StreamCreated::try_from_val(&ctx.env, &event.2).unwrap();
    assert_eq!(event_data.stream_id, stream_id);
    assert_eq!(event_data.sender, ctx.sender);
    assert_eq!(event_data.recipient, ctx.recipient);
    assert_eq!(event_data.deposit_amount, 1000);
}
```

### Pattern 2: Verify No Event on Failure

```rust
#[test]
fn test_failed_operation_emits_no_event() {
    let ctx = TestContext::setup();

    let events_before = ctx.env.events().all().len();

    // Attempt invalid operation
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &0, // Invalid: zero deposit
        &1,
        &0,
        &0,
        &1000,
    );

    assert!(result.is_err());

    let events_after = ctx.env.events().all().len();
    assert_eq!(events_before, events_after, "failed operation must not emit events");
}
```

## Time-Based Testing Patterns

### Pattern 1: Test at Exact Boundary

```rust
#[test]
fn test_withdraw_exactly_at_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1,
        &0,
        &500, // cliff at t=500
        &1000,
    );

    // Test exactly at cliff time
    ctx.env.ledger().set_timestamp(500);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 500, "accrual at cliff should equal cliff amount");

    // Withdrawal should succeed at cliff
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 500);
}
```

### Pattern 2: Test Before/At/After Sequence

```rust
#[test]
fn test_cliff_boundary_behavior() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Before cliff: accrual is zero
    ctx.env.ledger().set_timestamp(499);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 0);

    // At cliff: accrual starts
    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 500);

    // After cliff: accrual continues
    ctx.env.ledger().set_timestamp(750);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 750);
}
```

## Numeric Edge Case Patterns

### Pattern 1: Maximum Values

```rust
#[test]
fn test_max_deposit_amount() {
    let ctx = TestContext::setup();

    let max_deposit = i128::MAX - 1000;
    let rate = 1_000_000;
    let duration = max_deposit / rate;

    // Mint sufficient tokens
    ctx.sac.mint(&ctx.sender, &max_deposit);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &max_deposit,
        &rate,
        &0,
        &0,
        &(duration as u64),
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, max_deposit);
}
```

### Pattern 2: Overflow Protection

```rust
#[test]
fn test_accrual_overflow_protection() {
    let ctx = TestContext::setup();

    // Create stream with parameters that would overflow without protection
    let deposit = i128::MAX - 100;
    let rate = i128::MAX / 2;
    let duration = 10;

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0,
        &0,
        &duration,
    );

    // Advance time
    ctx.env.ledger().set_timestamp(duration);

    // Accrual should be capped at deposit, not overflow
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued <= deposit, "accrual must not exceed deposit");
}
```

## State Transition Testing Patterns

### Pattern 1: Valid Transition

```rust
#[test]
fn test_active_to_paused_transition() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Verify initial state
    let state_before = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_before.status, StreamStatus::Active);

    // Perform transition
    ctx.client().pause_stream(&stream_id);

    // Verify new state
    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.status, StreamStatus::Paused);

    // Verify event
    let events = ctx.env.events().all();
    let event = events.last().unwrap();
    let event_data = StreamEvent::try_from_val(&ctx.env, &event.2).unwrap();
    assert_eq!(event_data, StreamEvent::Paused(stream_id));
}
```

### Pattern 2: Invalid Transition

```rust
#[test]
fn test_completed_to_active_transition_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Complete the stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Attempt invalid transition
    let result = ctx.client().try_resume_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    // Verify state unchanged
    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.status, StreamStatus::Completed);
}
```

## Error Testing Patterns

### Pattern 1: Structured Error

```rust
#[test]
fn test_withdraw_before_cliff_returns_invalid_state() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Attempt withdrawal before cliff
    ctx.env.ledger().set_timestamp(100);
    let result = ctx.client().try_withdraw(&stream_id);

    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}
```

### Pattern 2: No Side Effects on Error

```rust
#[test]
fn test_failed_create_has_no_side_effects() {
    let ctx = TestContext::setup();

    let balance_before = ctx.token().balance(&ctx.sender);
    let count_before = ctx.client().get_stream_count();

    // Attempt invalid creation
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &0, // Invalid
        &1,
        &0,
        &0,
        &1000,
    );

    assert!(result.is_err());

    // Verify no side effects
    assert_eq!(ctx.token().balance(&ctx.sender), balance_before);
    assert_eq!(ctx.client().get_stream_count(), count_before);
}
```

## Multiple Stream Testing Patterns

### Pattern 1: Stream Independence

```rust
#[test]
fn test_streams_are_independent() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create two streams
    let stream_1 = ctx.create_default_stream();
    let stream_2 = ctx.create_default_stream();

    // Modify stream 1
    ctx.client().pause_stream(&stream_1);

    // Verify stream 2 unaffected
    let state_1 = ctx.client().get_stream_state(&stream_1);
    let state_2 = ctx.client().get_stream_state(&stream_2);

    assert_eq!(state_1.status, StreamStatus::Paused);
    assert_eq!(state_2.status, StreamStatus::Active);
}
```

### Pattern 2: Recipient Index

```rust
#[test]
fn test_recipient_index_tracks_multiple_streams() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create multiple streams for same recipient
    let stream_1 = ctx.create_default_stream();
    let stream_2 = ctx.create_default_stream();
    let stream_3 = ctx.create_default_stream();

    // Query recipient's streams
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);

    assert_eq!(streams.len(), 3);
    assert!(streams.contains(stream_1));
    assert!(streams.contains(stream_2));
    assert!(streams.contains(stream_3));
}
```

## Common Pitfalls

### Pitfall 1: Non-Deterministic Test Data

**Bad:**

```rust
#[test]
fn test_create_stream() {
    let sender = Address::generate(&env); // Random address
    // Test may produce different snapshots on each run
}
```

**Good:**

```rust
#[test]
fn test_create_stream() {
    let ctx = TestContext::setup(); // Fixed addresses
    // Deterministic snapshot
}
```

### Pitfall 2: Incomplete Assertions

**Bad:**

```rust
#[test]
fn test_withdraw() {
    let amount = ctx.client().withdraw(&stream_id);
    assert!(amount > 0); // Incomplete
}
```

**Good:**

```rust
#[test]
fn test_withdraw() {
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 500); // Exact value

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 500);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token().balance(&ctx.recipient), 500);
}
```

### Pitfall 3: Testing Multiple Scenarios

**Bad:**

```rust
#[test]
fn test_withdraw_scenarios() {
    // Test before cliff
    // Test at cliff
    // Test after cliff
    // Multiple scenarios in one test
}
```

**Good:**

```rust
#[test]
fn test_withdraw_before_cliff_panics() {
    // One scenario
}

#[test]
fn test_withdraw_at_cliff_succeeds() {
    // One scenario
}

#[test]
fn test_withdraw_after_cliff_succeeds() {
    // One scenario
}
```

## Checklist for New Tests

Before submitting a new snapshot test:

- [ ] Test name follows `test_<operation>_<scenario>_<outcome>` pattern
- [ ] Documentation comment explains authorization, success, and failure semantics
- [ ] Test uses `TestContext::setup()` or `setup_strict()` appropriately
- [ ] All time values are set explicitly with `env.ledger().set_timestamp()`
- [ ] All observable behavior is verified (storage, events, tokens, errors)
- [ ] Test is deterministic (no random values)
- [ ] Test is isolated (no shared state with other tests)
- [ ] Test is minimal (tests one scenario only)
- [ ] Authorization boundaries are explicit
- [ ] Error cases verify no side effects
- [ ] Snapshot file is reviewed and committed

## References

- [Snapshot Test Documentation](./snapshot-tests.md)
- [Coverage Matrix](./snapshot-test-coverage-matrix.md)
- [Quick Reference](./snapshot-workflow-quick-reference.md)
- [Soroban SDK Testing](https://docs.rs/soroban-sdk/latest/soroban_sdk/testutils/)
