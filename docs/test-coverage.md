# Contract Test Coverage

## Overview

Fluxora enforces a **minimum 95% line coverage** gate on the `fluxora_stream` contract in CI.
Every pull request and push to `main`/`develop` must meet this threshold or the pipeline fails.

Current baseline: **96.4%** (644/668 lines — see `coverage/cobertura.xml`).

> New stream contract coverage includes boundary tests for `update_rate_per_second`: rejecting equal and zero rates, accepting `i128::MAX` when deposit coverage holds, allowing paused-stream updates, rejecting cancelled streams, and exercising monotonic rate sequences with `proptest`.

---

## Running Coverage Locally

### Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust | 1.94.1 (pinned) | `rustup show` — `rust-toolchain.toml` handles this automatically |
| cargo-tarpaulin | 0.31 | `cargo install cargo-tarpaulin --version 0.31 --locked` |

> cargo-tarpaulin uses Linux `ptrace`-based instrumentation. **macOS and Windows are not
> supported for accurate coverage.** Use a Linux machine, WSL2, or a Docker container
> (see [Soroban/WASM caveats](#sorobanwasm-caveats) below).

### Quick run

```bash
# From the workspace root
cargo tarpaulin \
  --features testutils \
  --out Xml \
  --out Html \
  --output-dir coverage \
  --ignore-tests \
  --skip-clean \
  -p fluxora_stream
```

Reports are written to `coverage/`:
- `coverage/cobertura.xml` — machine-readable (used by CI gate and Codecov)
- `coverage/tarpaulin-report.html` — human-readable, open in a browser

### Check the threshold locally

```bash
LINE_RATE=$(grep -oP '(?<=<coverage )[^>]*' coverage/cobertura.xml | grep -oP 'line-rate="\K[0-9.]+')
PCT=$(awk "BEGIN { printf \"%d\", $LINE_RATE * 100 }")
echo "Coverage: ${PCT}%"
[ "$PCT" -ge 95 ] && echo "PASS" || echo "FAIL — add tests to cover missing lines"
```

### Run with higher proptest case count

Property-based tests default to 256 cases. For deeper fuzz coverage before a release:

```bash
PROPTEST_CASES=10000 cargo tarpaulin \
  --features testutils \
  --out Xml \
  --output-dir coverage \
  --ignore-tests \
  --skip-clean \
  -p fluxora_stream
```

---

## Soroban/WASM Caveats

### Why we do NOT measure WASM coverage

Soroban contracts compile to `wasm32-unknown-unknown`. cargo-tarpaulin instruments
native (`x86_64`) binaries via `ptrace`; it cannot instrument WASM binaries.

This is intentional and correct:

- The Soroban `testutils` feature compiles the contract as a **native `rlib`** and
  links it directly into the test binary. The test harness (`Env`, mock ledger, mock
  auth) runs entirely in-process on the host.
- Coverage is therefore measured against the **same source code** that gets compiled
  to WASM for deployment — only the compilation target differs.
- The WASM build is validated separately in the `build` CI job (reproducible build +
  SHA256 checksum verification).

### `--ignore-tests` flag

We pass `--ignore-tests` so that lines inside `#[cfg(test)]` modules (`test.rs`,
`test_withdrawable_props.rs`, etc.) are excluded from the denominator. Coverage
measures production logic only, not test helpers.

### `--skip-clean` flag

Skips a full `cargo clean` before instrumentation. This speeds up CI significantly
(avoids recompiling the entire dependency tree). Safe to use because tarpaulin
re-instruments object files when source changes.

### `|| true` removed intentionally

The original CI command used `|| true` to swallow tarpaulin failures. This has been
removed. If tarpaulin itself fails (e.g. a test panics, a dependency is missing), CI
now correctly reports the failure rather than silently producing an empty report.

### Soroban `no_std` and `extern crate std`

The contract is `#![no_std]` for WASM compatibility. Test modules re-enable `std`
with `extern crate std;` at the top of each test file. tarpaulin handles this
correctly because it instruments the native build where `std` is always available.

---

## CI Coverage Gate

The `coverage` job in `.github/workflows/ci.yml`:

1. Runs after the `test` job passes.
2. Installs `cargo-tarpaulin 0.31` (cached between runs to avoid slow installs).
3. Generates `coverage/cobertura.xml`.
4. Parses the `line-rate` attribute from the XML root element.
5. Converts to an integer percentage and compares against **95**.
6. Fails with `::error::` if below threshold, annotating the PR.
7. Posts a summary table to the GitHub Actions job summary (visible in the UI).
8. Uploads the HTML + XML report as a downloadable artifact (14-day retention).
9. Uploads to Codecov for trend tracking (non-blocking).

The gate is **blocking**: a PR cannot be merged if coverage drops below 95%.

### Job dependency graph

```
lint ──┬── build
       └── test ── coverage
```

`coverage` only runs when `test` passes, so a broken test suite does not produce
a misleading coverage number.

---

## Interpreting the Report

### Uncovered lines (current baseline)

| File | Line(s) | Reason |
|------|---------|--------|
| `accrual.rs` | 31 | `None` branch of `checked_sub` — requires `current_time < checkpointed_at`, which the contract prevents at call sites |
| `lib.rs` | 1768 | Unreachable branch in stream-close guard |
| `lib.rs` | 1867–1868 | Defensive panic in recipient-index cleanup |
| `lib.rs` | 2003 | Overflow guard in `shorten_stream_end_time` |

These lines are defensive guards. They are intentionally hard to reach because the
contract validates inputs before reaching them. They are tracked here so reviewers
know they are not forgotten.

### Adding tests to improve coverage

1. Identify uncovered lines in `coverage/tarpaulin-report.html` (red highlighting).
2. Write a test in the appropriate module (`test.rs`, `test_issue_39.rs`, etc.) that
   exercises the missing branch.
3. Re-run coverage locally to confirm the line turns green.
4. Open a PR — CI will verify the threshold is still met.

#### Recent Coverage Improvements

- **Batch deposit overflow (lib.rs:316-317)**: Added property-based tests in `integration_suite.rs` that generate batches with cumulative deposits exceeding `i128::MAX`. Tests verify that `ContractError::ArithmeticOverflow` is returned and no partial state is written on overflow. Includes both fuzzing via proptest and exact boundary condition tests.

- **Template cap branches (lib.rs owner-cap and global-cap guards, #521)**: Added
  `test_owner_template_cap_exceeded` and `test_global_template_cap_exceeded` in
  `contracts/stream/tests/stream_templates.rs`.
  - `test_owner_template_cap_exceeded` registers exactly 64 templates for one owner
    and asserts `TemplateLimitExceeded` on the 65th.  It then deletes one template
    and confirms re-registration succeeds (counter decrement verified).
  - `test_global_template_cap_exceeded` fills the 10 000-template global cap using
    156 owners × 64 templates + 16 remainder, asserts `TemplateLimitExceeded` on the
    next call, deletes one template, and confirms the freed slot allows registration.

---

## References

- [cargo-tarpaulin documentation](https://github.com/xd009642/tarpaulin)
- [Snapshot test coverage matrix](./snapshot-test-coverage-matrix.md)
- [Property-based accrual tests](./pr-accrual-property-tests.md)
- [Security guidelines](./security.md)
- [Audit preparation](./audit.md)

## Factory Policy Enforcement Tests (issue #525)

`contracts/stream/tests/factory_policy.rs` covers all six `FactoryError` variants:

| Test | Error variant |
|---|---|
| `test_factory_already_initialized` | `AlreadyInitialized` |
| `test_set_admin_requires_existing_admin` | `Unauthorized` |
| `test_create_stream_recipient_not_allowlisted` | `RecipientNotAllowlisted` |
| `test_create_stream_deposit_exceeds_cap` | `DepositExceedsCap` |
| `test_create_stream_duration_too_short` | `DurationTooShort` |
| `test_factory_not_initialized_returns_error` | `NotInitialized` |
| `test_create_stream_deposit_at_cap_ok` | boundary: cap accepted |
| `test_create_stream_duration_at_minimum_ok` | boundary: min duration accepted |
| `test_set_cap_enforced` | dynamic cap update enforced |
| `test_set_min_duration_enforced` | dynamic min_duration update enforced |
| `test_set_allowlist_remove_enforced` | allowlist removal enforced |
