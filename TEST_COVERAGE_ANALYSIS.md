# Rust contract test inventory

> **Note:** An earlier version of this file claimed **Total Tests: 27** with no date. That was a stale, hand-maintained snapshot from when only a handful of multi-stream integration tests existed. Those headline numbers were removed so they are not mistaken for current suite size.

## Current counts (regenerate; do not edit by hand)

From the repository root:

```bash
python3 script/count_rust_tests.py
```

The script counts `#[test]` attributes under `contracts/*/tests/*.rs` and `contracts/*/src/*.rs` (same scope as the issue audit command).

## Pass/fail and line coverage

- **CI** runs `cargo test` (see `.github/workflows/ci.yml`) and enforces the **95% line coverage** gate on `fluxora_stream` via `cargo tarpaulin`.
- For authoritative green/red status and coverage percentage, use the latest CI run—not static markdown totals.

## Historical multi-stream coverage write-up

The prior document listed assertion-level coverage for two integration tests (`integration_same_sender_multiple_streams`, `integration_same_sender_same_recipient_multiple_streams`). That narrative lives in git history if needed; the integration suite in `contracts/stream/tests/` now covers far more scenarios.
