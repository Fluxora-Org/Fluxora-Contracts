# Snapshot Test Workflow - Quick Reference

## Daily Development Workflow

### Running Snapshot Tests

```bash
# Run all tests (includes snapshot validation)
cargo test -p fluxora_stream

# Run specific snapshot test
cargo test -p fluxora_stream test_create_stream_initial_state

# Run with verbose output
cargo test -p fluxora_stream -- --nocapture
```

### When Tests Fail

```bash
# 1. Review the failure
cargo test -p fluxora_stream 2>&1 | less

# 2. Check what changed
git diff contracts/stream/test_snapshots/

# 3. If change is intentional, update snapshots
SOROBAN_SNAPSHOT_UPDATE=1 cargo test -p fluxora_stream

# 4. Review updated snapshots
git diff contracts/stream/test_snapshots/

# 5. Commit with clear message
git add contracts/stream/test_snapshots/
git commit -m "test: update snapshots for [reason]"
```

## CI/CD Quick Reference

### CI Pipeline Stages

1. **Lint** → Format check + Clippy
2. **Build** → Native + WASM + Optimization
3. **Test** → Unit tests + Integration tests + Snapshot validation
4. **Coverage** → Generate coverage report (95% minimum)
5. **Deploy** → Testnet (auto on main) / Mainnet (manual)

### Snapshot Validation in CI

- **Trigger**: Every push and PR
- **Location**: `test` job in `.github/workflows/ci.yml`
- **Command**: `cargo test -p fluxora_stream --features testutils`
- **Failure**: CI fails if snapshots don't match

### Fixing CI Snapshot Failures

```bash
# Pull latest changes
git pull origin main

# Run tests locally
cargo test -p fluxora_stream

# If intentional change, update snapshots
SOROBAN_SNAPSHOT_UPDATE=1 cargo test -p fluxora_stream

# Push updated snapshots
git add contracts/stream/test_snapshots/
git commit -m "test: update snapshots for [specific change]"
git push
```

## Common Commands

### Update All Snapshots

```bash
SOROBAN_SNAPSHOT_UPDATE=1 cargo test -p fluxora_stream
```

### Update Specific Test Snapshot

```bash
SOROBAN_SNAPSHOT_UPDATE=1 cargo test -p fluxora_stream test_withdraw_mid_stream
```

### Review Snapshot Diff

```bash
# Before updating
cargo test -p fluxora_stream test_withdraw_mid_stream 2>&1 | grep -A 20 "snapshot"

# After updating
git diff contracts/stream/test_snapshots/test/test_withdraw_mid_stream.1.json
```

### Verify Snapshot Coverage

```bash
# Count snapshot files
ls -1 contracts/stream/test_snapshots/test/*.json | wc -l

# List all snapshot tests
cargo test -p fluxora_stream --list | grep "^test_"
```

## PR Checklist

When your PR changes snapshots:

- [ ] Run tests locally before pushing
- [ ] Review every changed `.json` file
- [ ] Verify changes match intended behavior
- [ ] Update documentation if behavior changed
- [ ] Add PR comment explaining snapshot changes
- [ ] Ensure CI passes
- [ ] Request review from maintainer

## Emergency Procedures

### Reverting Snapshot Changes

```bash
# Revert all snapshot changes
git checkout HEAD -- contracts/stream/test_snapshots/

# Revert specific snapshot
git checkout HEAD -- contracts/stream/test_snapshots/test/test_name.1.json

# Re-run tests
cargo test -p fluxora_stream
```

### Debugging Snapshot Failures

```bash
# 1. Enable verbose output
RUST_BACKTRACE=1 cargo test -p fluxora_stream -- --nocapture

# 2. Run single test in isolation
cargo test -p fluxora_stream test_name -- --exact --nocapture

# 3. Check for non-deterministic behavior
for i in {1..10}; do cargo test -p fluxora_stream test_name; done

# 4. Compare with main branch
git diff main -- contracts/stream/test_snapshots/
```

## Environment Variables

| Variable                  | Purpose                | Example                     |
| ------------------------- | ---------------------- | --------------------------- |
| `SOROBAN_SNAPSHOT_UPDATE` | Update snapshots       | `SOROBAN_SNAPSHOT_UPDATE=1` |
| `RUST_BACKTRACE`          | Show full stack traces | `RUST_BACKTRACE=1`          |
| `CARGO_TERM_COLOR`        | Colorize output        | `CARGO_TERM_COLOR=always`   |

## File Locations

| Path                                          | Purpose                   |
| --------------------------------------------- | ------------------------- |
| `contracts/stream/test_snapshots/test/*.json` | Snapshot files            |
| `contracts/stream/src/test.rs`                | Unit tests with snapshots |
| `contracts/stream/tests/integration_suite.rs` | Integration tests         |
| `.github/workflows/ci.yml`                    | CI pipeline configuration |
| `docs/snapshot-tests.md`                      | Full documentation        |

## Getting Help

1. **Read full docs**: `docs/snapshot-tests.md`
2. **Check CI logs**: GitHub Actions → Failed job → Test step
3. **Review test code**: `contracts/stream/src/test.rs`
4. **Ask maintainer**: Open issue or PR comment

## Quick Decision Tree

```
Snapshot test failed?
├─ Expected (I changed behavior)
│  ├─ Review diff carefully
│  ├─ Update: SOROBAN_SNAPSHOT_UPDATE=1 cargo test
│  ├─ Commit with clear message
│  └─ Document in PR
│
└─ Unexpected (I didn't change this)
   ├─ Review what changed: git diff
   ├─ Check recent commits: git log
   ├─ Reproduce locally: cargo test
   └─ Fix code or revert change
```
