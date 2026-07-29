# Issue #1122 verification

This PR verifies that the workspace explicitly pins `soroban-env-host` to the same version across the three contract crates, as requested in https://github.com/Fluxora-Org/Fluxora-Contracts/issues/1122.

Findings
- `contracts/factory/Cargo.toml`: `soroban-env-host = "=21.2.1"`
- `contracts/governance/Cargo.toml`: `soroban-env-host = "=21.2.1"`
- `contracts/stream/Cargo.toml`: `soroban-env-host = "=21.2.1"`

Action taken
- Added this verification document so maintainers can review and close the issue.

Recommended next steps
1. Re-run gas-regression test suites for `fluxora_governance` and `fluxora_stream` to confirm recorded baselines under the pinned host version.
   - `cargo test -p fluxora_governance gas_regression`
   - `cargo test -p fluxora_stream gas_regression`
2. If any baselines differ, recalibrate the regression thresholds and commit updated `*.proptest-regressions` or test expectations.

Closes #1122
