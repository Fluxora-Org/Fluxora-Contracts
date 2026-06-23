# Formal verification notes — `accrual.rs`, keeper-fee, and governance

This document describes the Kani proof harnesses in Fluxora Contracts.

## Accrual proofs (original)
- Located in `contracts/stream/src/accrual.rs` (and exercised via tests).
- Proofs cover result bounds, monotonicity, and clamping.

## Keeper-fee conservation proofs (new)
- Pure helper: `compute_keeper_fee_split(gross, bps)` in `lib.rs`.
- Harness: `keeper_fee_conservation` (in `formal_verification_smoke.rs` under `#[cfg(kani)]`).
  - Asserts: `keeper_fee + sender_refund == sender_refund_gross`
  - Asserts: `keeper_fee <= sender_refund_gross`
  - Domain: `gross >= 0`, `bps <= 10_000` (full i128 domain via symbolic input).
- Harness: `keeper_fee_no_mul_overflow`
  - Proves the `checked_mul(KEEPER_FEE_BPS)` before `/ 10_000` cannot overflow in production path.

## Governance proofs (new)
- Harnesses in `formal_verification_smoke.rs` (`kani_governance`).
  - `governance_quorum_monotonic_and_timelock_safe`
    - Proves `quorum_at + GOVERNANCE_TIMELOCK_SECONDS` is overflow-safe.
    - Proves approval-count → quorum transition is monotonic (once reached, stays reached).
  - `governance_executed_stays_executed`
    - Proves an executed proposal remains executed (cannot be cancelled or re-executed).

## Constants (production values)
- `KEEPER_GRACE_PERIOD_SECONDS = 604_800` (7 days)
- `KEEPER_FEE_BPS = 50` (0.5%)
- `GOVERNANCE_TIMELOCK_SECONDS = 172_800` (48 hours)

## How to run

```bash
# Accrual (original)
kani contracts/stream/src/accrual.rs --recursive

# Fee + governance (via smoke harness)
kani contracts/stream/tests/formal_verification_smoke.rs --harness keeper_fee_conservation
kani contracts/stream/tests/formal_verification_smoke.rs --harness governance_quorum_monotonic_and_timelock_safe
kani contracts/stream/tests/formal_verification_smoke.rs --harness governance_executed_stays_executed
```

All proofs are gated by `#[cfg(kani)]`:
- `cargo test -p fluxora_stream` unaffected.
- `cargo build --target wasm32-unknown-unknown -p fluxora_stream` unaffected.

## Security notes
- Proofs target the **exact** production arithmetic path (via extracted pure helper for fee).
- Full symbolic domains (not sampled values) for gross refunds and BPS.
- Timelock addition uses checked arithmetic + explicit overflow proof.
- Governance monotonicity + executed-stays-executed prevent replay / double-execution vectors.
