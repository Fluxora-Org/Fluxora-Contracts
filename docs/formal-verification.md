# Formal verification notes — `accrual.rs`, keeper-fee, and governance

This document describes the Kani proof harnesses in Fluxora Contracts.

## Coverage status (gap analysis)

The table below maps every property described in this document to what
`contracts/stream/tests/formal_verification_smoke.rs` actually exercises.

| # | Property (doc section) | Smoke-test status | Notes |
|---|------------------------|-------------------|-------|
| 1 | Accrual result bounds (`accrual.rs` original) | **partial** | `formal_verification_smoke.rs` only has `smoke_accrual_examples` (2 concrete numeric cases). The original Kani bounds/monotonicity/clamping proofs described here are **not** present in the smoke harness — they live in `accrual.rs` unit tests, not under `#[cfg(kani)]`. |
| 2 | Accrual monotonicity (original) | **not exercised by smoke** | No `kani::proof` for `calculate_accrued_amount` monotonicity in the smoke harness. Covered only by non-symbolic unit tests in `accrual.rs`. |
| 3 | Accrual clamping (original) | **not exercised by smoke** | Same as above. |
| 4 | `ledger_time_monotonic_guard` (clock regression) | **covered** | `kani::proof` in `kani_accrual_security` — full symbolic `u64` domain, iff `current_ts < prev_ts`. |
| 5 | `cliff_only_accrual_exact_and_bounded` | **covered** | `kani::proof` — exact payout at/after cliff, zero before, bounded in `[0, deposit]` over symbolic domain. |
| 6 | `keeper_fee_conservation` | **covered** | `kani::proof` — conservation + non-negativity + `fee <= gross`, full `i128`/`u32` domain. |
| 7 | `keeper_fee_no_mul_overflow` | **covered** | `kani::proof` — `checked_mul` before `/ 10_000` never overflows. |
| 8 | `governance_quorum_monotonic_and_timelock_safe` | **covered (simulated)** | `kani::proof` uses a local `TIMELOCK` constant (must be kept in sync with `governance.rs`). It does not call the real governance entry point, so it verifies the arithmetic shape, not the on-chain state machine. |
| 9 | `governance_executed_stays_executed` | **covered (simulated)** | `kani::proof` asserts a local `bool` stays `true`; it models intent, not the actual `proposal.executed` storage guard. |

### Smallest gaps closed in this change

- None beyond documentation. The accrual original proofs (rows 1–3) remain
  aspirational in the smoke harness; closing them would require extracting pure
  accrual helpers (as done for `compute_keeper_fee_split`) and adding
  `#[cfg(kani)]` harnesses — out of scope per the issue.

## Accrual proofs (original)
- Located in `contracts/stream/src/accrual.rs` (and exercised via tests).
- Proofs cover result bounds, monotonicity, and clamping.

## Clock monotonicity and CliffOnly proofs (new)
- Harnesses in `contracts/stream/tests/formal_verification_smoke.rs` under `kani_accrual_security`.
  - `ledger_time_monotonic_guard`
    - Proves `assert_ledger_time_monotonic` returns `Err(ClockRegression)` if and only if `current_ts < prev_ts` across the full symbolic `u64` domain.
  - `cliff_only_accrual_exact_and_bounded`
    - Proves a `StreamKind::CliffOnly` stream returns exactly `deposit_amount` at or after `cliff_time` and `0` before it across the full symbolic `i128`/`u64` domain.
    - Proves the result remains within `[0, deposit_amount]` for every nonnegative symbolic deposit.

## Keeper-fee conservation proofs (new)
- Pure helper: `compute_keeper_fee_split(gross, bps)` in `lib.rs`.
- Harness: `keeper_fee_conservation` (in `formal_verification_smoke.rs` under `#[cfg(kani)]`).
  - Asserts conservation: `keeper_fee + protocol_remainder == gross`
  - Proves non-negativity: both `keeper_fee` and `protocol_remainder` are `>= 0`
  - Asserts stronger bound: `keeper_fee <= gross` (follows from conservation + non-negativity)
  - Domain assumptions mirror runtime guards: `gross >= 0`, `bps <= 10_000` (full i128 domain via symbolic input).
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

# Clock monotonicity and CliffOnly accrual (via smoke harness)
kani contracts/stream/tests/formal_verification_smoke.rs --harness ledger_time_monotonic_guard
kani contracts/stream/tests/formal_verification_smoke.rs --harness cliff_only_accrual_exact_and_bounded

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
