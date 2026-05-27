## Description
<!-- Provide a clear and concise description of your changes -->

This PR implements comprehensive test coverage for the batch deposit sum overflow guard at lines 316-317 in `contracts/stream/src/lib.rs`. The overflow protection was previously untested, representing a gap in our 96.4% coverage baseline. Added property-based tests using proptest to generate batches with cumulative deposits exceeding `i128::MAX`, ensuring the arithmetic overflow guard fires correctly and returns `ContractError::ArithmeticOverflow` without writing partial state.

## Type of Change
<!-- Mark the relevant option with an 'x' -->

- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update
- [x] Test coverage improvement
- [ ] Refactoring (no functional changes)

## Related Issues
<!-- Link to related issues using #issue_number -->

Closes #520

### Changes Made
<!-- List the specific changes made in this PR -->

**Test Implementation (`contracts/stream/tests/integration_suite.rs`):**
- Added proptest strategy generating 2-10 streams with large deposit amounts that overflow when summed
- Implemented `batch_deposit_sum_overflow_returns_arithmetic_overflow_error` - property-based test with fuzzing
- Implemented `batch_deposit_sum_overflow_exact_boundary` - deterministic test at exact overflow boundary  
- Implemented `batch_deposit_sum_just_under_overflow_succeeds` - validation test for normal operation
- Added proptest import and configuration
- Fixed compilation issues with Soroban SDK error handling and type conversions

**Documentation Updates (`docs/test-coverage.md`):**
- Removed lines 316-317 from uncovered lines table
- Added detailed description of new overflow test coverage
- Documented testing approach and security guarantees

---

## Snapshot Test Changes
<!-- REQUIRED if snapshot files were modified -->

### Did this PR modify snapshot test files?
- [ ] Yes - snapshot files were updated (explain below)
- [x] No - no snapshot changes

## Testing

### Test Coverage
- [x] All tests pass locally: `cargo test -p fluxora_stream`
- [x] New tests added for new functionality
- [ ] Existing tests updated for changed functionality
- [x] Test coverage remains above 95%

### Manual Testing
<!-- Describe any manual testing performed -->

- [x] Tested on local environment
- [x] Tested edge cases
- [x] Tested error conditions

**Specific Test Scenarios Validated:**
- Batch with cumulative deposits > `i128::MAX` returns `ContractError::ArithmeticOverflow`
- Exact boundary condition: `(i128::MAX - 1000) + 2000` triggers overflow
- Valid sums just under `i128::MAX` don't trigger false positives
- Atomicity: no partial state written on overflow (stream count, balances unchanged)
- No events emitted on overflow failure

## Documentation
- [x] Code comments added/updated
- [x] Documentation updated (if behavior changed)
- [ ] README updated (if needed)
- [ ] Snapshot test documentation reviewed

## Security Considerations
<!-- Address any security implications of your changes -->

- [x] No new security concerns introduced
- [x] Authorization boundaries verified
- [x] Input validation added/verified
- [x] Error handling reviewed

**Security Validation:**
- ✅ Overflow protection verified via `checked_add` guard
- ✅ Atomicity guaranteed - no partial state on failure
- ✅ Correct error type returned (`ContractError::ArithmeticOverflow`)
- ✅ Token balances remain unchanged on overflow
- ✅ Stream counter unchanged on overflow
- ✅ No events emitted on failure

## Checklist
- [x] My code follows the project's style guidelines
- [x] I have performed a self-review of my code
- [x] I have commented my code, particularly in hard-to-understand areas
- [x] I have made corresponding changes to the documentation
- [x] My changes generate no new warnings
- [x] I have added tests that prove my fix is effective or that my feature works
- [x] New and existing unit tests pass locally with my changes
- [x] Any dependent changes have been merged and published

## Additional Notes
<!-- Any additional information that reviewers should know -->

**Testing Strategy:**
- **Property-based testing**: Uses proptest with deterministic RNG seeding for reproducible fuzzing
- **Boundary testing**: Exact overflow conditions at `i128::MAX`
- **Negative testing**: Confirms valid sums don't trigger false positives
- **State verification**: Comprehensive atomicity checks

**Coverage Impact:**
- **Before**: Lines 316-317 uncovered (arithmetic overflow path)
- **After**: Full coverage of overflow guard with both property-based and deterministic tests
- **Expected**: Coverage increase from 96.4% baseline

**Commands for Validation:**
```bash
# Run new overflow tests
cargo test -p fluxora_stream batch_deposit_sum_overflow

# Generate coverage report
cargo tarpaulin --features testutils --out Xml --output-dir coverage --ignore-tests --skip-clean -p fluxora_stream
```

## Reviewer Checklist
<!-- For reviewers to complete -->
- [ ] Code quality and style
- [ ] Test coverage adequate
- [ ] Documentation complete
- [ ] Snapshot changes justified and correct
- [ ] Security implications reviewed
- [ ] Breaking changes documented