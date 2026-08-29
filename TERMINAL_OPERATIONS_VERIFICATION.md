# Terminal Operations - Verification Checklist

## Pre-Merge Verification

### 1. Build Verification
```bash
# Verify project builds
cargo build --target wasm32v1-none --release

# Check for any compilation warnings
cargo clippy --all-targets
```

**Expected**: Clean build, no errors or warnings

### 2. Test Execution
```bash
# Run the new terminal operations test suite
cargo test -p fluxora-stream terminal_operations -- --nocapture

# Run full stream contract test suite
cargo test -p fluxora-stream

# Run with release optimizations
cargo test -p fluxora-stream --release
```

**Expected**: 
- All 23 terminal operation tests pass
- Full test suite passes (146+ tests)
- No test timing regressions

### 3. Specific Test Verification

Run each major test category individually:

```bash
# Basic terminal rejection
cargo test -p fluxora-stream cancelled_stream_rejects -- --nocapture
cargo test -p fluxora-stream depleted_stream_rejects -- --nocapture

# Boundary conditions  
cargo test -p fluxora-stream cancelled_stream_with_withdrawable_balance -- --nocapture
cargo test -p fluxora-stream cancel_at_creation -- --nocapture

# Retry/idempotency
cargo test -p fluxora-stream repeated_rejection -- --nocapture

# Comprehensive matrix
cargo test -p fluxora-stream terminal_operation_matrix_comprehensive -- --nocapture
```

**Expected**: Each test passes with clean output

### 4. Invariant Verification

Check that all tests properly verify invariants:

```bash
# Run with verbose output to see invariant checks
cargo test -p fluxora-stream terminal_operations -- --nocapture 2>&1 | grep -i "invariant"
```

**Expected**: No invariant violations logged

### 5. Performance Check

```bash
# Run resource limits test to ensure no regression
cargo test -p fluxora-stream resource_limits -- --nocapture

# Time the test suite
time cargo test -p fluxora-stream terminal_operations --release
```

**Expected**: 
- Resource limits unchanged
- Test suite completes in < 1 second

## Code Review Checklist

### Test Quality
- [ ] All tests have descriptive names
- [ ] Each test has clear documentation
- [ ] Test assertions include failure messages
- [ ] Edge cases are explicitly documented
- [ ] Boundary conditions are tested

### Coverage Completeness
- [ ] All 6 mutating operations covered
- [ ] Both terminal states (Cancelled, Depleted) covered
- [ ] Authorization precedence tested
- [ ] Storage immutability verified
- [ ] Token balance preservation verified
- [ ] TTL non-extension verified

### Error Handling
- [ ] All rejections use `Error::StreamTerminated`
- [ ] Error codes are stable (discriminant = 14)
- [ ] Error messages are clear
- [ ] Failed operations guarantee no side effects

### Integration Points
- [ ] Module added to `mod.rs`
- [ ] No changes to production code (test-only PR)
- [ ] No new dependencies added
- [ ] Documentation updated

## Manual Verification Steps

### 1. Verify Test File Structure

```bash
# Check file exists and is properly formatted
cat contracts/stream/src/test/terminal_operations.rs | head -50

# Check module declaration
grep -n "terminal_operations" contracts/stream/src/test/mod.rs
```

**Expected**: 
- File has proper header documentation
- Module declared in Stage 2 section

### 2. Verify Test Coverage

```bash
# Count tests in the file
grep -c "^fn " contracts/stream/src/test/terminal_operations.rs

# List all test names
grep "^fn " contracts/stream/src/test/terminal_operations.rs | awk '{print $2}' | sed 's/()//'
```

**Expected**: 23 tests listed

### 3. Verify Error Types Used

```bash
# Check all errors are StreamTerminated
grep "Error::" contracts/stream/src/test/terminal_operations.rs | grep -v "StreamTerminated" | grep -v "^//"
```

**Expected**: Only `StreamTerminated` errors in assertions (comments excluded)

### 4. Verify State Checks

```bash
# Check all tests verify state unchanged
grep -c "assert_eq!(before, after)" contracts/stream/src/test/terminal_operations.rs
```

**Expected**: At least 15 occurrences (most tests check this)

## Regression Testing

### Test Against Existing Behavior

```bash
# Run existing cancel tests
cargo test -p fluxora-stream cancel -- --nocapture

# Run existing pause tests  
cargo test -p fluxora-stream pause -- --nocapture

# Run existing withdraw tests
cargo test -p fluxora-stream withdraw -- --nocapture
```

**Expected**: All existing tests still pass

### Cross-Reference Coverage

The new tests complement these existing terminal tests:
- `test/cancel.rs::cancelling_twice_is_rejected` 
- `test/cancel.rs::a_depleted_stream_cannot_be_cancelled`
- `test/pause.rs::terminated_streams_cannot_be_paused_or_resumed`
- `test/withdraw.rs::withdrawing_from_a_depleted_stream_is_a_typed_error`

Verify these still pass:
```bash
cargo test -p fluxora-stream cancelling_twice_is_rejected -- --nocapture
cargo test -p fluxora-stream a_depleted_stream_cannot_be_cancelled -- --nocapture
cargo test -p fluxora-stream terminated_streams_cannot_be_paused_or_resumed -- --nocapture
cargo test -p fluxora-stream withdrawing_from_a_depleted_stream_is_a_typed_error -- --nocapture
```

## Documentation Verification

### 1. Check Module Documentation

```bash
# View module header
head -40 contracts/stream/src/test/terminal_operations.rs
```

**Expected**: Clear explanation of purpose and design decisions

### 2. Verify Summary Document

```bash
# Check summary exists and is complete
ls -lh TERMINAL_OPERATIONS_TEST_SUMMARY.md
wc -l TERMINAL_OPERATIONS_TEST_SUMMARY.md
```

**Expected**: Comprehensive summary document present

### 3. Check Test Documentation

```bash
# Verify each test has doc comments or clear naming
grep -B 3 "^fn " contracts/stream/src/test/terminal_operations.rs | grep "///"
```

**Expected**: Most tests have explanatory comments

## CI/CD Integration

### GitHub Actions Compatibility

```bash
# Ensure tests work with GitHub Actions syntax
cargo test -p fluxora-stream terminal_operations --no-fail-fast

# Check test output format
cargo test -p fluxora-stream terminal_operations -- --format=json --quiet
```

**Expected**: JSON output parseable, no hanging tests

### Test Timing

```bash
# Measure individual test timing
cargo test -p fluxora-stream terminal_operations -- --test-threads=1 --nocapture --show-output
```

**Expected**: No individual test takes > 100ms

## Sign-Off Checklist

Before marking as ready for merge:

- [ ] All 23 tests pass consistently
- [ ] No test flakiness observed (run 5 times)
- [ ] Full test suite passes (146+ tests)
- [ ] No compilation warnings
- [ ] No clippy warnings
- [ ] Documentation is clear and complete
- [ ] Code follows existing test patterns
- [ ] Module properly integrated
- [ ] No performance regressions
- [ ] Summary document reviewed
- [ ] Verification checklist completed

## Post-Merge Verification

After merge, verify in CI:

```bash
# CI should run these automatically
cargo test --workspace
cargo test -p fluxora-stream --release
cargo clippy --all-targets
```

Monitor CI output for:
- [ ] All tests pass
- [ ] No new warnings
- [ ] Test timing stable
- [ ] Coverage metrics maintained

## Troubleshooting

### If Tests Fail

1. **Check environment**:
   ```bash
   rustc --version  # Should be 1.97.1
   cargo --version
   ```

2. **Clean build**:
   ```bash
   cargo clean
   cargo test -p fluxora-stream terminal_operations
   ```

3. **Isolate failure**:
   ```bash
   cargo test -p fluxora-stream <failing_test_name> -- --nocapture
   ```

4. **Check for concurrency issues**:
   ```bash
   cargo test -p fluxora-stream terminal_operations -- --test-threads=1
   ```

### If Performance Regresses

1. **Profile tests**:
   ```bash
   cargo test -p fluxora-stream terminal_operations --release -- --nocapture
   ```

2. **Compare with baseline**:
   ```bash
   git checkout main
   time cargo test -p fluxora-stream --release
   git checkout -
   time cargo test -p fluxora-stream --release
   ```

### If Coverage Gaps Found

1. **Identify missing scenarios**:
   - Review test coverage matrix in summary
   - Check boundary conditions
   - Verify all error paths tested

2. **Add tests as needed**:
   - Follow existing test patterns
   - Use descriptive names
   - Include state verification

## Contact

For questions about this test suite:
- Review design decisions in module header
- Check `TERMINAL_OPERATIONS_TEST_SUMMARY.md`
- Reference existing tests in `test/cancel.rs` and `test/pause.rs`

## Version

- **Test Suite Version**: 1.0
- **Fluxora Version**: Protocol 27
- **Rust Version**: 1.97.1
- **SDK Version**: soroban-sdk 27.0.5
- **Date**: 2026-08-27
