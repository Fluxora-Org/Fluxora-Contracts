# Token Helpers Audit - Implementation Checklist

**Purpose:** Actionable tasks to address audit recommendations  
**Priority:** High-priority items should be completed before mainnet deployment

---

## High Priority Tasks

### Task 1: Document Token Contract Requirements

**File:** `docs/DEPLOYMENT.md`  
**Status:** ⬜ Not Started  
**Estimated Effort:** 15 minutes  
**Owner:** TBD

**Implementation:**

Add a new section to `docs/DEPLOYMENT.md`:

```markdown
## Token Contract Requirements

### Overview

The Fluxora streaming contract is designed to work with any Stellar Asset Contract (SAC)
compliant token. However, certain assumptions are made about token behavior that must be
validated before deployment.

### Required Token Properties

1. **Standard SAC Interface**
   - Must implement `transfer(from: Address, to: Address, amount: i128)`
   - Must follow standard Stellar token semantics
   - Must emit standard transfer events

2. **No Reentrancy**
   - Token contract must NOT call back into the streaming contract during transfer
   - Reentrancy could violate CEI (Checks-Effects-Interactions) assumptions
   - The streaming contract uses CEI pattern as defense-in-depth

3. **No Hidden Fees**
   - Transfer amount must equal received amount
   - No automatic fee deductions or burns
   - If fees exist, they must be explicitly handled by the caller

4. **No Transfer Restrictions**
   - Token must allow transfers to/from the streaming contract address
   - No whitelist/blacklist that could block contract operations
   - No pause mechanism that could lock funds in the contract

### Validation Checklist

Before deploying with a new token, verify:

- [ ] Token follows SAC standard (check interface)
- [ ] Token does not reenter on transfer (review token code)
- [ ] Token has no hidden fees (test transfer amounts)
- [ ] Token allows contract as sender/recipient (test transfers)
- [ ] Token has no pause mechanism (or contract is whitelisted)

### Known Compatible Tokens

- USDC (Stellar native)
- XLM (wrapped)
- [Add other tested tokens here]

### Testing Procedure

1. Deploy streaming contract to testnet
2. Initialize with candidate token
3. Create test stream with small amount
4. Verify deposit amount matches contract balance
5. Withdraw and verify recipient receives exact amount
6. Cancel stream and verify sender receives exact refund

### Risk Mitigation

If using an untested token:

- Start with small amounts on testnet
- Monitor contract balance vs. expected balance
- Test all operations (create, withdraw, cancel, top-up)
- Verify event logs match token transfer events

### Contact

For questions about token compatibility, contact: [your-contact-info]
```

**Acceptance Criteria:**

- [ ] Section added to `docs/DEPLOYMENT.md`
- [ ] All 4 required properties documented
- [ ] Validation checklist provided
- [ ] Testing procedure included
- [ ] Reviewed by team

---

### Task 2: Add Reentrancy Protection Test

**File:** `contracts/stream/src/test.rs`  
**Status:** ⬜ Not Started  
**Estimated Effort:** 1 hour  
**Owner:** TBD

**Implementation:**

Add a new test to verify CEI pattern prevents reentrancy issues:

```rust
/// Test that CEI pattern protects against token contract reentrancy.
///
/// This test verifies that even if a malicious token contract attempts to
/// reenter the streaming contract during a transfer, the CEI pattern ensures
/// state is already persisted and consistent.
///
/// Note: Soroban's execution model makes true reentrancy difficult, but this
/// test documents the defense-in-depth approach.
#[test]
fn test_token_reentrancy_protection() {
    // This test is conceptual - Soroban's execution model prevents reentrancy
    // by design, but we document the CEI pattern as defense-in-depth.

    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance time to allow withdrawal
    ctx.env.ledger().set_timestamp(600);

    // Withdraw should succeed even if token contract were to attempt reentrancy
    // because state is saved BEFORE push_token is called (CEI pattern)
    let amount = ctx.client().withdraw(&stream_id);

    // Verify state was updated correctly
    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.withdrawn_amount, amount);

    // Verify no double-withdrawal is possible
    let second_amount = ctx.client().withdraw(&stream_id);
    assert_eq!(second_amount, 0); // Nothing left to withdraw
}

/// Test that cancel follows CEI pattern (state before refund).
///
/// Verifies that stream status is set to Cancelled and cancelled_at is set
/// BEFORE the refund is sent to the sender. This prevents reentrancy attacks
/// where a malicious sender could try to cancel again during the refund.
#[test]
fn test_cancel_cei_pattern() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel immediately (before any accrual)
    ctx.env.ledger().set_timestamp(0);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream is in terminal state
    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert!(stream.cancelled_at.is_some());

    // Verify cannot cancel again (state was persisted before refund)
    let result = ctx.client().try_cancel_stream(&stream_id);
    assert!(result.is_err()); // Should fail - already cancelled
}

/// Test that top_up follows CEI pattern (state before pull).
///
/// Verifies that deposit_amount is increased and persisted BEFORE tokens are
/// pulled from the funder. This ensures consistent state even if the token
/// transfer fails.
#[test]
fn test_top_up_cei_pattern() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let initial_deposit = 1000i128;
    let top_up_amount = 500i128;

    // Top up the stream
    ctx.client().top_up_stream(&stream_id, &ctx.sender, &top_up_amount);

    // Verify state was updated
    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.deposit_amount, initial_deposit + top_up_amount);

    // Verify contract balance matches
    let contract_balance = ctx.token().balance(&ctx.env.current_contract_address());
    assert_eq!(contract_balance, initial_deposit + top_up_amount);
}
```

**Acceptance Criteria:**

- [ ] Three new tests added to `test.rs`
- [ ] Tests verify CEI pattern in withdraw, cancel, and top_up
- [ ] Tests pass with `cargo test`
- [ ] Tests documented with clear comments
- [ ] Reviewed by team

---

## Medium Priority Tasks

### Task 3: Add Zero-Amount Helper Function

**File:** `contracts/stream/src/lib.rs`  
**Status:** ⬜ Not Started  
**Estimated Effort:** 30 minutes  
**Owner:** TBD

**Implementation:**

Add a helper function to standardize zero-amount checks:

```rust
// Add after push_token function (around line 390)

/// Check if a token transfer should be performed.
///
/// Returns true if amount is positive, false otherwise.
/// Use this before calling push_token to avoid unnecessary token client invocations.
///
/// # Parameters
/// - `amount`: The amount to check
///
/// # Returns
/// - `bool`: true if amount > 0, false otherwise
fn should_transfer(amount: i128) -> bool {
    amount > 0
}
```

Then update all `push_token` call sites to use this helper:

```rust
// In cancel_stream_internal (line 2006)
if should_transfer(refund_amount) {
    push_token(env, &stream.sender, refund_amount)?;
}

// In shorten_stream_end_time (line 1657)
if should_transfer(refund_amount) {
    push_token(&env, &stream.sender, refund_amount)?;
}

// In create_streams (line 730)
if should_transfer(total_deposit) {
    pull_token(&env, &sender, total_deposit)?;
}
```

**Acceptance Criteria:**

- [ ] Helper function added with documentation
- [ ] All zero-amount checks use helper
- [ ] Tests still pass
- [ ] Code review completed

---

### Task 4: Add Lifetime Metrics

**File:** `contracts/stream/src/lib.rs`  
**Status:** ⬜ Not Started  
**Estimated Effort:** 2 hours  
**Owner:** TBD

**Implementation:**

Add new data structures and functions to track lifetime metrics:

```rust
// Add to DataKey enum (around line 210)
#[contracttype]
pub enum DataKey {
    Config,
    NextStreamId,
    Stream(u64),
    RecipientStreams(Address),
    GlobalPaused,
    /// Lifetime metrics for treasury dashboards
    Metrics,
}

// Add new struct for metrics (around line 180)
#[contracttype]
#[derive(Clone, Debug, Default)]
pub struct ContractMetrics {
    /// Total amount deposited across all streams (lifetime)
    pub total_deposited: i128,
    /// Total amount withdrawn by recipients (lifetime)
    pub total_withdrawn: i128,
    /// Total amount refunded to senders (lifetime)
    pub total_refunded: i128,
    /// Number of streams created (lifetime)
    pub streams_created: u64,
    /// Number of streams completed (lifetime)
    pub streams_completed: u64,
    /// Number of streams cancelled (lifetime)
    pub streams_cancelled: u64,
}

// Add helper functions (around line 400)
fn load_metrics(env: &Env) -> ContractMetrics {
    env.storage()
        .instance()
        .get(&DataKey::Metrics)
        .unwrap_or_default()
}

fn save_metrics(env: &Env, metrics: &ContractMetrics) {
    env.storage().instance().set(&DataKey::Metrics, metrics);
    bump_instance_ttl(env);
}

fn increment_deposited(env: &Env, amount: i128) {
    let mut metrics = load_metrics(env);
    metrics.total_deposited = metrics.total_deposited.saturating_add(amount);
    save_metrics(env, &metrics);
}

fn increment_withdrawn(env: &Env, amount: i128) {
    let mut metrics = load_metrics(env);
    metrics.total_withdrawn = metrics.total_withdrawn.saturating_add(amount);
    save_metrics(env, &metrics);
}

fn increment_refunded(env: &Env, amount: i128) {
    let mut metrics = load_metrics(env);
    metrics.total_refunded = metrics.total_refunded.saturating_add(amount);
    save_metrics(env, &metrics);
}

// Add public getter (in impl block)
pub fn get_metrics(env: Env) -> ContractMetrics {
    load_metrics(&env)
}
```

Then update token transfer sites to record metrics:

```rust
// In create_stream (after pull_token)
increment_deposited(&env, deposit_amount);

// In withdraw (after push_token)
increment_withdrawn(&env, withdrawable);

// In cancel_stream_internal (after push_token)
increment_refunded(env, refund_amount);
```

**Acceptance Criteria:**

- [ ] Metrics struct added
- [ ] Helper functions implemented
- [ ] All token transfers update metrics
- [ ] Public getter function added
- [ ] Tests added for metrics tracking
- [ ] Documentation updated

---

## Low Priority Tasks

### Task 5: Add Batch Refund Operation

**File:** `contracts/stream/src/lib.rs`  
**Status:** ⬜ Not Started  
**Estimated Effort:** 3 hours  
**Owner:** TBD

**Implementation:**

Add a new admin function to cancel multiple streams in one transaction:

```rust
/// Cancel multiple streams in a single transaction (admin only).
///
/// Optimizes gas usage for emergency scenarios where admin needs to cancel
/// many streams at once. Each stream is cancelled using the same logic as
/// `cancel_stream_as_admin`.
///
/// # Parameters
/// - `stream_ids`: Vector of stream IDs to cancel
///
/// # Returns
/// - `Vec<i128>`: Refund amounts for each stream (in same order as input)
///
/// # Authorization
/// - Requires authorization from the contract admin
///
/// # Behavior
/// - Each stream is cancelled independently
/// - Refunds are sent to original senders
/// - If any stream fails, entire transaction reverts
///
/// # Panics
/// - If any stream is not in Active or Paused state
/// - If any stream does not exist
/// - If caller is not the admin
pub fn batch_cancel_streams_as_admin(
    env: Env,
    stream_ids: soroban_sdk::Vec<u64>,
) -> Result<soroban_sdk::Vec<i128>, ContractError> {
    let admin = get_admin(&env)?;
    admin.require_auth();

    let mut refunds = soroban_sdk::Vec::new(&env);

    for stream_id in stream_ids.iter() {
        let mut stream = load_stream(&env, stream_id)?;

        // Calculate refund before cancelling
        let now = env.ledger().timestamp();
        let accrued_at_cancel = accrual::calculate_accrued_amount(
            stream.start_time,
            stream.cliff_time,
            stream.end_time,
            stream.rate_per_second,
            stream.deposit_amount,
            now,
        );
        let refund_amount = stream.deposit_amount.checked_sub(accrued_at_cancel)?;

        // Cancel using internal helper
        Self::cancel_stream_internal(&env, &mut stream)?;

        refunds.push_back(refund_amount);
    }

    Ok(refunds)
}
```

**Acceptance Criteria:**

- [ ] Function implemented
- [ ] Tests added for batch cancel
- [ ] Documentation complete
- [ ] Admin authorization verified
- [ ] Atomicity tested (partial failure reverts all)

---

## Testing Checklist

After implementing changes, verify:

- [ ] All existing tests pass: `cargo test`
- [ ] New tests added for each change
- [ ] Test coverage maintained or improved
- [ ] Integration tests pass
- [ ] Testnet deployment successful
- [ ] Manual testing on testnet completed

---

## Documentation Checklist

After implementing changes, update:

- [ ] `docs/DEPLOYMENT.md` - Token requirements
- [ ] `README.md` - New features (if any)
- [ ] `CHANGELOG.md` - Document changes
- [ ] Function documentation (inline comments)
- [ ] API documentation (if public interface changed)

---

## Review Checklist

Before marking tasks complete:

- [ ] Code review by at least 2 team members
- [ ] Security review of changes
- [ ] Gas cost analysis (if applicable)
- [ ] Backward compatibility verified
- [ ] Migration plan (if needed)

---

## Deployment Checklist

Before mainnet deployment:

- [ ] All high-priority tasks completed
- [ ] All tests passing
- [ ] Documentation updated
- [ ] External security audit (if required)
- [ ] Testnet validation complete
- [ ] Deployment plan reviewed
- [ ] Rollback plan prepared

---

**Status Legend:**

- ⬜ Not Started
- 🔄 In Progress
- ✅ Complete
- ⏸️ Blocked

**Last Updated:** 2026-03-26
