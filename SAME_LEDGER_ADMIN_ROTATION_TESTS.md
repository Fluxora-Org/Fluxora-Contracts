# Same-Ledger Admin Rotation Tests

## Summary

Added three comprehensive tests to `contracts/factory/tests/factory_setters.rs` to verify that admin rotation properly invalidates authorization within the same simulated ledger/transaction.

## Tests Added

### 1. `test_set_admin_same_ledger_old_admin_fails`

**Purpose**: Verify that after rotating admin via `set_admin`, the old admin cannot use residual authorization for a setter call issued immediately after rotation in the same ledger.

**Test Flow**:
1. Initialize factory with `old_admin`
2. Rotate to `new_admin` using `old_admin`'s auth
3. Attempt to call `set_cap` with only `old_admin`'s authorization (same ledger)
4. Assert the call fails auth (using `assert_auth_fails` helper)
5. Verify the cap was NOT changed

**Key Insight**: This test confirms that `require_admin()` reads the admin from storage **fresh** on each call, so there's no stale authorization issue.

### 2. `test_set_admin_same_ledger_new_admin_succeeds`

**Purpose**: Verify that the new admin can successfully call a setter immediately after rotation in the same ledger.

**Test Flow**:
1. Initialize factory with `old_admin`
2. Rotate to `new_admin` using `old_admin`'s auth
3. Call `set_cap` with `new_admin`'s authorization (same ledger)
4. Assert the call succeeds
5. Verify the cap was updated successfully

**Key Insight**: This test confirms that admin rotation takes effect immediately - there's no delay or caching.

### 3. `test_set_admin_same_ledger_multiple_setters`

**Purpose**: Comprehensive test verifying that the old admin cannot call ANY setter after rotation, while the new admin can call multiple different setters.

**Test Flow**:
1. Initialize factory with `old_admin`
2. Rotate to `new_admin`
3. Old admin tries `set_min_duration` → fails
4. New admin calls `set_min_duration` → succeeds
5. Old admin tries `set_batch_cap_enforcement` → fails
6. New admin calls `set_batch_cap_enforcement` → succeeds

**Key Insight**: This test provides additional coverage by exercising multiple different setters to ensure the authorization check is consistent across all admin-gated operations.

## Implementation Analysis

### How Authorization Works

The `require_admin()` function in `contracts/factory/src/lib.rs` (line 99) implements admin authorization:

```rust
fn require_admin(env: &Env) -> Result<Address, FactoryError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(FactoryError::NotInitialized)?;
    admin.require_auth();
    Ok(admin)
}
```

**Key behavior**:
- Reads the admin from instance storage on **every call**
- No caching or memoization
- Immediately calls `require_auth()` on the freshly-read admin

### Why No Stale Authorization Issue Exists

The implementation naturally prevents stale authorization because:

1. **Fresh read on each call**: `require_admin()` reads from `env.storage().instance().get(&DataKey::Admin)` each time
2. **Immediate auth check**: After reading, it immediately calls `admin.require_auth()`
3. **No cached admin**: There's no cached or memoized admin address that could become stale

### What the Tests Verify

Despite the correct implementation, these tests are valuable because they:

1. **Document the expected behavior**: Clear specification that admin rotation is immediate
2. **Regression protection**: Guard against future changes that might introduce caching
3. **MockAuth correctness**: Verify that the test framework's `MockAuth` mechanism properly isolates authorizations
4. **Same-ledger semantics**: Explicitly test the "same transaction" scenario mentioned in the requirements

## Running the Tests

From the workspace root:

```bash
cargo test --package fluxora-factory test_set_admin_same_ledger
```

Or run all factory setter tests:

```bash
cargo test --package fluxora-factory factory_setters
```

## Acceptance Criteria Met

✅ Old admin cannot use residual authorization for a setter call issued right after rotating away admin in the same ledger  
✅ New admin can call a setter successfully immediately after rotation, same ledger  
✅ Existing `test_set_admin_*` tests still pass (no breaking changes)  

## Files Modified

- `contracts/factory/tests/factory_setters.rs` - Added three new test functions after `test_load_policy_equality_is_struct_equality`

## Test Naming Convention

The new tests follow the existing naming pattern in the file:
- Existing: `test_set_admin_updates_config`, `test_set_admin_new_admin_can_call_setters`, `test_set_admin_rejects_non_admin`
- New: `test_set_admin_same_ledger_old_admin_fails`, `test_set_admin_same_ledger_new_admin_succeeds`, `test_set_admin_same_ledger_multiple_setters`

All tests use the `test_set_admin_*` prefix for easy discovery and filtering.
