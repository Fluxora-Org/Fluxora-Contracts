# Formal verification notes — `accrual.rs`

This document describes the Kani proof harnesses added to `contracts/stream/src/accrual.rs`.

Summary
- Added `#[cfg(kani)]` proof harnesses exercising `calculate_accrued_amount_checkpointed`.
- Proofs cover:
  - Result bounds: output is always in `[0, deposit_amount]` and no panic occurs.
  - Monotonicity: for non-negative rates, accrual is non-decreasing over time after the cliff.
  - Clamping: returns `0` before `cliff_time` and caps at `deposit_amount` at/after `end_time`.

Assumptions and bounds
- To keep proofs tractable, harnesses constrain numeric ranges via `kani::assume`:
  - `deposit_amount` and `rate_per_second` are bounded to ±1e18-ish in proofs.
  - `checkpointed_amount` is assumed in `[0, deposit_amount]`.
  - `checkpointed_at <= end_time` and `cliff_time <= end_time`.

Unproven/intentional limitations
- The harnesses intentionally bound ranges for tractability. While the proofs
  cover arithmetic logic and clamping, extremely large ranges (e.g. full
  i128 limits) are not exhaustively explored due to state-space explosion.
- Kani proofs are a best-effort complement to unit and property tests, not a
  replacement for manual audits.

How to run

Install Kani (see https://model-checking.github.io/kani/), then run from repo
root:

```bash
# Run Kani on the accrual proofs
kani contracts/stream/src/accrual.rs --recursive
```

If Kani is not installed, CI should skip Kani stages; proofs are gated by
`#[cfg(kani)]` and do not affect normal `cargo test` runs.

Security notes
- Proofs assert no panic/overflow in the core accrual math and reinforce the
  contract's CEI and clamping assumptions. Operators should still validate
  token behaviour and avoid initializing the contract with malicious tokens.
