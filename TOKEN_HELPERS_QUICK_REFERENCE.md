# Token Helpers Quick Reference

**Purpose:** Fast lookup for developers working with token transfers in the Fluxora streaming contract

---

## The Two Helper Functions

### `pull_token` - Inbound Transfers

```rust
fn pull_token(env: &Env, from: &Address, amount: i128) -> Result<(), ContractError>
```

**What it does:** Transfers tokens FROM an external address TO the contract  
**When to use:** Stream creation, top-ups, any operation that deposits tokens  
**Authorization:** Caller must have `from.require_auth()` before calling  
**Failure:** Panics if insufficient balance/allowance, reverts transaction

**Usage Pattern:**

```rust
sender.require_auth();                    // 1. Authorize first
validate_params(...)?;                    // 2. Validate inputs
pull_token(&env, &sender, amount)?;       // 3. Pull tokens
persist_state(&env, ...);                 // 4. Save state after
```

---

### `push_token` - Outbound Transfers

```rust
fn push_token(env: &Env, to: &Address, amount: i128) -> Result<(), ContractError>
```

**What it does:** Transfers tokens FROM the contract TO an external address  
**When to use:** Withdrawals, refunds, any operation that sends tokens out  
**Authorization:** Caller must be authorized for the operation (recipient/sender/admin)  
**Failure:** Panics if insufficient contract balance, reverts transaction

**Usage Pattern:**

```rust
recipient.require_auth();                 // 1. Authorize first
let amount = calculate_amount(...)?;      // 2. Calculate amount
save_state(&env, ...);                    // 3. Save state BEFORE transfer (CEI!)
push_token(&env, &recipient, amount)?;    // 4. Push tokens last
emit_event(&env, ...);                    // 5. Emit event after success
```

---

## CEI Pattern (Critical!)

**C**hecks → **E**ffects → **I**nteractions

Always follow this order:

```rust
// ✅ CORRECT
fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
    // CHECKS
    let stream = load_stream(&env, stream_id)?;
    stream.recipient.require_auth();
    if stream.status != Active { return Err(...); }

    // EFFECTS (update state FIRST)
    stream.withdrawn_amount += amount;
    save_stream(&env, &stream);

    // INTERACTIONS (external calls LAST)
    push_token(&env, &stream.recipient, amount)?;
    env.events().publish(...);

    Ok(amount)
}

// ❌ WRONG - Don't do this!
fn withdraw_wrong(env: Env, stream_id: u64) -> Result<i128, ContractError> {
    let stream = load_stream(&env, stream_id)?;

    // ❌ External call before state update
    push_token(&env, &stream.recipient, amount)?;

    // ❌ State update after external call (reentrancy risk!)
    stream.withdrawn_amount += amount;
    save_stream(&env, &stream);

    Ok(amount)
}
```

**Why CEI matters:**

- Prevents reentrancy attacks
- Ensures consistent state on failure
- Makes code easier to audit

---

## Authorization Patterns

### Pattern 1: Sender Authorization (Create/Cancel)

```rust
pub fn create_stream(env: Env, sender: Address, ...) -> Result<u64, ContractError> {
    sender.require_auth();  // Sender must authorize
    // ... validation ...
    pull_token(&env, &sender, deposit_amount)?;
    // ... persist state ...
}
```

### Pattern 2: Recipient Authorization (Withdraw)

```rust
pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
    let stream = load_stream(&env, stream_id)?;
    stream.recipient.require_auth();  // Recipient must authorize
    // ... calculate amount ...
    push_token(&env, &stream.recipient, amount)?;
    // ... emit events ...
}
```

### Pattern 3: Admin Authorization (Admin Operations)

```rust
pub fn cancel_stream_as_admin(env: Env, stream_id: u64) -> Result<(), ContractError> {
    let admin = get_admin(&env)?;
    admin.require_auth();  // Admin must authorize
    // ... cancel logic ...
    push_token(&env, &stream.sender, refund_amount)?;
}
```

### Pattern 4: Flexible Authorization (Top-Up)

```rust
pub fn top_up_stream(env: Env, stream_id: u64, funder: Address, amount: i128) -> Result<(), ContractError> {
    funder.require_auth();  // Any funder can authorize
    // ... validation ...
    pull_token(&env, &funder, amount)?;
}
```

---

## Zero-Amount Handling

### When to Check for Zero

```rust
// ✅ GOOD: Check before push_token to avoid unnecessary calls
if refund_amount > 0 {
    push_token(&env, &sender, refund_amount)?;
}

// ✅ GOOD: Early return for zero withdrawable
let withdrawable = accrued - withdrawn;
if withdrawable == 0 {
    return Ok(0);  // No transfer, no state change
}

// ✅ GOOD: Check before batch pull
if total_deposit > 0 {
    pull_token(&env, &sender, total_deposit)?;
}
```

### Why Zero Checks Matter

- Avoids unnecessary token client invocations (gas savings)
- Prevents confusing events (no event if no transfer)
- Makes code more efficient and clearer

---

## Common Mistakes to Avoid

### ❌ Mistake 1: Forgetting Authorization

```rust
// ❌ WRONG
pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
    let stream = load_stream(&env, stream_id)?;
    // Missing: stream.recipient.require_auth();
    push_token(&env, &stream.recipient, amount)?;
}
```

### ❌ Mistake 2: Wrong CEI Order

```rust
// ❌ WRONG
pub fn cancel_stream(env: Env, stream_id: u64) -> Result<(), ContractError> {
    let stream = load_stream(&env, stream_id)?;
    push_token(&env, &stream.sender, refund)?;  // ❌ Transfer before state update
    stream.status = Cancelled;
    save_stream(&env, &stream);
}
```

### ❌ Mistake 3: Direct Token Client Usage

```rust
// ❌ WRONG - Don't bypass helpers!
pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
    let token_client = token::Client::new(&env, &token_address);
    token_client.transfer(&env.current_contract_address(), &recipient, &amount);  // ❌
}

// ✅ CORRECT - Use helper
pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
    push_token(&env, &recipient, amount)?;  // ✅
}
```

### ❌ Mistake 4: Forgetting to Check Status

```rust
// ❌ WRONG
pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
    let stream = load_stream(&env, stream_id)?;
    // Missing: status checks (Paused? Completed?)
    push_token(&env, &stream.recipient, amount)?;
}
```

---

## Testing Checklist

When adding new token transfer code, test:

- [ ] **Authorization:** Unauthorized calls fail
- [ ] **Insufficient Balance:** Fails gracefully, no state change
- [ ] **Zero Amount:** Handles correctly (early return or skip)
- [ ] **State Consistency:** State updated before transfer
- [ ] **Event Emission:** Events only emitted on success
- [ ] **Balance Tracking:** Contract balance matches expected
- [ ] **Atomicity:** Failed transfer reverts all changes

---

## Quick Debugging Guide

### Problem: "Insufficient balance" panic

**Check:**

1. Does sender have enough tokens? `token.balance(&sender)`
2. Has sender approved contract? `token.allowance(&sender, &contract)`
3. Is amount calculation correct? Add debug logs

### Problem: "Unauthorized" error

**Check:**

1. Is `require_auth()` called on correct address?
2. Is authorization called before helper?
3. Is test using `mock_all_auths()` or specific auth?

### Problem: State inconsistency after failure

**Check:**

1. Is CEI pattern followed? (state before transfer)
2. Are you using `save_stream()` before `push_token()`?
3. Are you testing with transaction rollback?

### Problem: Events not appearing

**Check:**

1. Are events emitted AFTER successful transfer?
2. Is transaction succeeding? (check for panics)
3. Are you checking the right event topic?

---

## Code Review Checklist

When reviewing token transfer code:

- [ ] Uses `pull_token` or `push_token` (no direct token client)
- [ ] Authorization before helper call
- [ ] CEI pattern followed (state before transfer)
- [ ] Zero-amount check (if applicable)
- [ ] Events emitted after success
- [ ] Error handling correct
- [ ] Tests cover failure cases
- [ ] Documentation updated

---

## Emergency Contacts

**For questions about:**

- Token transfer security → [security-team]
- CEI pattern → [architecture-team]
- Test failures → [qa-team]
- Production issues → [on-call-engineer]

---

## Additional Resources

- Full audit report: `TOKEN_HELPERS_AUDIT.md`
- Implementation checklist: `TOKEN_HELPERS_IMPLEMENTATION_CHECKLIST.md`
- Contract documentation: `docs/streaming.md`
- Security guidelines: `docs/security.md`

---

**Last Updated:** 2026-03-26  
**Version:** 1.0
