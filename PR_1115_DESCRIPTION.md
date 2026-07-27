# Pull Request: #1115 - Wire irrevocable parameter to Stream struct in persist_new_stream

## Description

This PR fixes issue #1115 by ensuring that `persist_new_stream` and `persist_new_stream_skip_index` correctly assign the caller-supplied `irrevocable` parameter (`Option<bool>`) directly into the constructed `Stream.irrevocable` field. Previously, parameter mapping drift could cause caller-supplied irrevocability flags to be silently ignored or defaulted.

In addition, this PR audits all internal and external call sites of `persist_new_stream` (`create_stream`, `create_stream_by_duration`, `create_streams`, `renew_stream`, `split_stream`, `clone_stream`) to guarantee that `irrevocable` flags are deliberately propagated end-to-end. Dedicated unit tests are included to verify that streams created with `Some(true)`, `Some(false)`, and `None` properly store and enforce the intended irrevocability state.

## Type of Change

- [x] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [x] Documentation update
- [x] Test coverage improvement
- [ ] Refactoring (no functional changes)

## Related Issues

Closes #1115

## Changes Made

- **`contracts/stream/src/lib.rs`**:
  - Confirmed and enforced direct assignment of parameter `irrevocable` to field `Stream.irrevocable` inside `persist_new_stream` and `persist_new_stream_skip_index`.
  - Conducted line-by-line parameter alignment audit of `Stream` struct instantiation in `persist_new_stream` functions to prevent unused or misplaced parameter drop-offs.
  - Audited all call sites (`create_stream`, `create_stream_by_duration`, `create_streams`, `renew_stream`, `split_stream`, `clone_stream`) ensuring explicit passing of `irrevocable` state.
- **`contracts/stream/tests/`**:
  - Added unit test coverage checking `persist_new_stream` with `irrevocable: Some(true)`, `Some(false)`, and `None`.
  - Added test cases verifying that cancellation behavior (`cancel_stream`, `cancel_stream_as_admin`, `keeper_cancel`) respects stored irrevocability.

## Snapshot Test Changes

### Did this PR modify snapshot test files?

- [ ] Yes - snapshot files were updated (explain below)
- [x] No - no snapshot changes

## Testing

### Test Coverage

- [x] All unit tests pass locally: `cargo test`
- [x] New unit tests added for `irrevocable` setting propagation (`Some(true)`, `Some(false)`, `None`).
- [x] Test coverage remains above 95%

### Security Considerations

An irrevocable stream guarantees that sender-side cancellation mechanisms (`cancel_stream`, `cancel_stream_as_admin`, `keeper_cancel`) refuse to cancel the stream. Ensuring `irrevocable` is strictly stored from caller parameters maintains this critical security guarantee without silent parameter dropping.
