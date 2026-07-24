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
- [ ] Run the security diff script: `python script/check_snapshot_diff.py`
  - *If it flags security-relevant fields (e.g., auth, events, error codes), ensure these changes receive mandatory extra review per the maintainer security checklist.*
- [ ] Verify changes match intended behavior
- [ ] Update documentation if behavior changed
- [ ] Add PR comment explaining snapshot changes
- [ ] Ensure CI passes
- [ ] Request review from maintainer

## Automated Security Review

To help identify security-relevant changes (authorization requirements, event payloads, error codes) in snapshots, use the diff checker script:

```bash
# Check working tree against HEAD
python script/check_snapshot_diff.py

# Check a specific branch against main
python script/check_snapshot_diff.py --base origin/main --head my-feature-branch
```

**What it does:**
- Diffs snapshot JSON changes between two commits.
- Flags any change to a security-relevant field (`auth`, `events`, `error`, `storage`, etc.).
- Exits with a non-zero code if security fields changed, preventing them from slipping through review unnoticed.


## CI Snapshot Security-Field Diff Gate

### What it is

The **snapshot security-field diff gate** is a hard CI step (no
`continue-on-error`) in the `test` job of `.github/workflows/ci.yml`. It
runs `script/check_snapshot_diff.py` after the snapshot tests pass and
**blocks merging** whenever a pull request (or push) touches a
security-relevant field in any committed snapshot JSON under
`contracts/stream/test_snapshots/`.

### Security-relevant fields

The script monitors changes to any JSON key that is a member of the
`SECURITY_FIELDS` set defined in `check_snapshot_diff.py`:

| Field / prefix | Why it matters |
| -------------- | -------------- |
| `auth`, `auths`, `require_auth`, `signatures` | Authorization requirements — changes here alter who can call a contract entry-point |
| `events`, `topic`, `topics`, `data` | Emitted event payloads — changes affect off-chain indexers and audit trails |
| `error`, `error_code`, `ContractError` | Error discriminants — changes can silently break client error handling |
| `storage`, `state`, `DataKey` | On-chain storage layout — changes can corrupt existing ledger state |

### How it runs in CI

The step resolves the base ref automatically based on the event type:

| Event | Base ref used |
| ----- | ------------- |
| `pull_request` | `origin/${{ github.base_ref }}` (the target branch tip) |
| `push` | `HEAD~1` (the immediate parent commit) |
| All other events | Step is skipped via the `if:` guard |

The effective command executed by CI:

```yaml
# For pull_request events:
python3 script/check_snapshot_diff.py --base "origin/${BASE_REF}"

# For push events:
python3 script/check_snapshot_diff.py --base "HEAD~1"
```

### Exit-code contract

| Exit code | Meaning |
| --------- | ------- |
| `0` | No security-relevant snapshot changes detected — PR may proceed normally |
| `1` | One or more security-relevant fields changed — **mandatory extra review required** |

A non-zero exit code fails the `test` job immediately. The PR cannot be
merged until either the change is reviewed and approved by a maintainer, or
the diff is reverted.

### Running the gate locally before pushing

```bash
# Compare your working branch against main (same as CI does for a PR):
python3 script/check_snapshot_diff.py --base origin/main

# Compare a specific pair of commits:
python3 script/check_snapshot_diff.py --base origin/main --head feature/my-change

# Compare current working tree against HEAD (quick local sanity check):
python3 script/check_snapshot_diff.py
```

### What to do when the gate fires

1. **Read the output** — the script prints which file and which JSON path
   triggered the flag, for example:

   ```
   [WARNING] Security-relevant fields changed in: contracts/stream/test_snapshots/test/test_cancel_stream.1.json
     - events[0].topic
   Mandatory extra review required due to security-relevant snapshot changes.
   ```

2. **Determine intent** — is the change deliberate (a bug fix, new feature)
   or accidental (leftover noise, test pollution)?

3. **If deliberate** — document the change in the PR description, add an
   entry to `CHANGELOG.md`, and request an explicit review from a maintainer
   who can sign off on the security implications.

4. **If accidental** — revert the snapshot changes:

   ```bash
   git checkout HEAD -- contracts/stream/test_snapshots/
   ```

5. **Re-run locally** to confirm the gate now passes before pushing:

   ```bash
   python3 script/check_snapshot_diff.py --base origin/main
   ```

### Security assumptions and design notes

- The script receives only validated git ref strings; no user-controlled
  input is interpolated into shell commands (the underlying `git diff` and
  `git show` calls use argument lists, not shell strings).
- Malformed JSON on either side of the diff is treated as an empty object
  (`{}`); the structural delta is still evaluated, so a file that becomes
  unparseable is not silently ignored.
- The gate runs *after* `cargo test` snapshot validation, so it operates on
  the committed snapshot state — not ephemeral test output.

---

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
