# Liability Invariants — Behavior & Regression Surface

Documented guarantees for the Fluxora stream contract's liability-tracking
system (`TotalLiabilities` counter and `sweep_excess` entry-point).

**Related:**
[storage-invariants.md](./storage-invariants.md) ·
[security.md](./security.md) ·
[streaming.md](./streaming.md) ·
[gas.md](./gas.md) ·
[storage.md](./storage.md) ·
[ABI_STABILITY.md](./ABI_STABILITY.md)

---

## Core Invariant

```text
contract_balance >= TotalLiabilities
```

This invariant **must** hold before and after every mutation operation.
If violated, a recipient withdrawal could fail or a parallel stream could
over-withdraw from the same token pool.

### What TotalLiabilities tracks

`TotalLiabilities` (stored as `DataKey::TotalLiabilities`, discriminant 14,
instance storage, `i128`) is the sum of all outstanding deposit obligations
owed to recipients. It is the single source of truth for the contract's
aggregate escrow liability.

---

## Liability Mutation Points

Every code path that touches `TotalLiabilities` is listed below with its
direction, triggering condition, and the internal helper(s) responsible.

Liability is adjusted inline in `lib.rs` using the pair of helpers from
`storage.rs`: [`read_total_liabilities`](../contracts/stream/src/storage.rs#L387)
and [`write_total_liabilities`](../contracts/stream/src/storage.rs#L395).
There is no single `adjust_total_liabilities` function; the
read–modify–write pattern is duplicated at each call site with `checked_add`
or `checked_sub` in between.

| Operation | Direction | Condition | Internal mechanism |
|-----------|-----------|-----------|-------------------|
| `create_stream` | +deposit | On success | `read_total_liabilities` → `checked_add(deposit)` → `write_total_liabilities` (inside `persist_new_stream`) |
| `create_streams` | +sum(deposits) | On success (batch) | Same as `create_stream`, called per stream in batch loop |
| `create_streams_partial` | +deposit | Per successfully-created stream | Same as `create_stream`, called per success inside partial-batch loop |
| `create_stream_relative` | +deposit | On success | Delegates to `create_stream` |
| `create_stream_with_lookback` | +deposit | On success | Delegates to `create_stream_internal` → `persist_new_stream` |
| `create_stream_from_template` | +deposit | On success | Delegates to `create_stream_relative` |
| `create_stream_offer` | +deposit | On success (deposit escrowed) | `read_total_liabilities` → `checked_add(deposit)` → `write_total_liabilities` (inside `persist_new_stream_offer`) |
| `accept_stream_offer` | No change | On success | Liability was already counted at offer creation; stream activation does not re-count |
| `reject_stream_offer` | −deposit | Refund to sender | `read_total_liabilities` → `checked_sub(deposit)` → `write_total_liabilities` |
| `cancel_stream_offer` | −deposit | Refund to sender | Same as `reject_stream_offer` |
| `clone_stream` | +new_deposit | On success | `read_total_liabilities` → `checked_add(new_deposit)` → `write_total_liabilities` |
| `top_up_stream` | +amount | On success | `read_total_liabilities` → `checked_add(amount)` → `write_total_liabilities` |
| `withdraw` | −amount | On success (amount > 0) | `read_total_liabilities` → `checked_sub(amount)` → `write_total_liabilities` |
| `withdraw_to` | −amount | On success (amount > 0) | Delegates to `withdraw` internal |
| `batch_withdraw` | −sum(amounts) | Per stream, on success | In-memory accumulator; single `write_total_liabilities` flush after loop |
| `batch_withdraw_to` | −sum(amounts) | Per stream, on success | Same batch-flush pattern |
| `cancel_stream` | −refund_amount | On success | `read_total_liabilities` → `checked_sub(refund)` → `write_total_liabilities` |
| `cancel_stream_as_admin` | −refund_amount | On success | Delegates to internal cancel (same liability path) |
| `keeper_cancel` | −deposit_amount | On success (full escrow removed) | `read_total_liabilities` → `checked_sub(deposit)` → `write_total_liabilities` |
| `witnessed_cancel` | −refund_amount | On success | Delegates to internal cancel (same liability path) |
| `shorten_stream_end_time` | −refund_amount | On success | `read_total_liabilities` → `checked_sub(refund)` → `write_total_liabilities` |
| `decrease_rate_per_second` | −refund_amount | On success | `read_total_liabilities` → `checked_sub(refund)` → `write_total_liabilities` |
| `init` | Set to 0 | Contract deployment | `.set(&DataKey::TotalLiabilities, &0i128)` |

### Rollback guarantees

Failed or reverted operations do **not** mutate `TotalLiabilities`. Soroban
transaction atomicity ensures that if any step (auth, validation, arithmetic,
token transfer) fails, `TotalLiabilities` is unchanged. Explicit assertions
in `liability_invariant.rs` (`failed_and_unauthorized_operations_do_not_mutate_total_liabilities`)
lock this down.

---

## Sweep Excess Behavior

### Formula

```text
excess = contract_balance.saturating_sub(TotalLiabilities)
```

### Authorization

- **Caller**: Contract admin only (`admin.require_auth()`).
- **Destination**: `recipient` parameter (any address, no co-sign required).
  The admin chooses the destination; requiring recipient co-sign would prevent
  sweeping to cold/offline treasury wallets.

### CEI Ordering

1. Check auth, read balance, read liabilities.
2. Compute excess; if ≤ 0 return 0 (no-op).
3. Emit `ExcessSwept` event **before** token transfer.
4. Acquire reentrancy lock.
5. `push_token` to recipient.
6. Release reentrancy lock.

### Security properties

1. **Excess-only transfer**: `saturating_sub` bounds the transfer — if
   liabilities exceed balance, zero is transferred.
2. **No liability erosion**: `TotalLiabilities` is never modified by
   `sweep_excess`. The counter is read-only during the sweep.
3. **CEI compliance**: Event emitted before transfer.
4. **Reentrancy protection**: Lock acquired before `push_token`.
5. **Zero-excess idempotency**: Calling with zero excess returns 0,
   no state change, no token transfer.
6. **No recipient auth escalation**: Removing recipient co-sign does not
   enable fund draining because the transfer is bounded by liabilities.

### Post-sweep invariant

```text
post_sweep_balance >= TotalLiabilities
```

---

## Overflow Protection

| Scenario | Protection | Error |
|----------|-----------|-------|
| `TotalLiabilities + deposit` overflows `i128` during `create_stream` | `checked_add` | `ArithmeticOverflow` |
| `TotalLiabilities + top_up` overflows `i128` during `top_up_stream` | `checked_add` | `ArithmeticOverflow` |
| `TotalLiabilities - amount` underflows `i128` (should never happen) | `saturating_sub` / `unwrap_or(0)` | Clamped to 0 |

### Near-ceiling regression tests

`contracts/stream/tests/top_up_boundary.rs` contains explicit regression tests:

- **Test 6** (`test_top_up_near_ceiling_total_liabilities_returns_overflow_error`):
  Seeds `TotalLiabilities = i128::MAX - 1`, attempts a top-up of 2, asserts
  `ArithmeticOverflow` is returned and `TotalLiabilities` is **not** mutated.
  *(Currently `#[ignore]` due to event-count drift from soroban-env-host version.)*

- **Test 7** (`test_top_up_just_under_ceiling_total_liabilities_succeeds`):
  Seeds `TotalLiabilities = i128::MAX - 500`, attempts a top-up of 500,
  asserts success and `TotalLiabilities == i128::MAX` after.
  *(Currently `#[ignore]` for the same event-count reason.)*

---

## Gas & Execution Model

### Batch liability flushing

In `batch_withdraw`, `TotalLiabilities` is read once from instance storage
into a local accumulator at the start of the loop. Each successful withdrawal
decrements the local accumulator in-memory. After the loop, the final value
is flushed to instance storage in a single write. This reduces I/O from
`N reads + N writes` to `1 read + 1 write` for the liability slot.

Same pattern applies to `bulk_cancel_streams` and `bulk_resume_streams_as_admin`.

### Determinism

`get_total_liabilities()` and `sweep_excess()` (when excess is 0) are pure
read paths that produce deterministic outputs across repeated calls. Locked
down by `total_liabilities_and_sweep_excess_gas_determinism` in
`liability_invariant.rs` (three iterations — a weak but representative check).

---

## Storage & TTL

- `TotalLiabilities` is stored in **instance** storage (not persistent).
- Every read or write through `read_total_liabilities` / `write_total_liabilities`
  calls `bump_instance_ttl`, keeping the entry alive.
- Discriminant 14 is frozen in the V5 DataKey layout. No variant may be
  inserted before it.

### Upgrade stability

Instance storage survives contract code upgrades via
`update_current_contract_wasm`. The
`total_liabilities_preserves_invariant_across_upgrades` test (marked
`#[ignore]` because the Soroban test environment lacks a deployable WASM
artifact) documents this guarantee conceptually.

---

## Existing Test Coverage

### Primary test file: `contracts/stream/tests/liability_invariant.rs`

| Test | What it verifies |
|------|-----------------|
| `prop_liability_invariant` | Property-based: 256 random sequences, 1–5 streams (Linear + CliffOnly), with operations (withdraw, top-up, decrease-rate, shorten, cancel, pause, resume, inject excess, sweep) — invariant checked at every step |
| `decrease_rate_per_second_reduces_total_liabilities_by_refund_amount` | Rate decrease refund correctly decrements `TotalLiabilities` |
| `shorten_stream_end_time_reduces_total_liabilities_by_refund_amount` | Shorten refund correctly decrements `TotalLiabilities` |
| `keeper_cancel_reduces_total_liabilities_by_unstreamed_amount` | Keeper cancel removes full outstanding obligation |
| `sweep_excess_idempotency_and_retry_determinism` | Repeated sweeps with zero excess return 0 with no state change |
| `total_liabilities_storage_key_discriminant_and_ttl_bumping` | Discriminant 14 round-trips, TTL bump on read/write |
| `total_liabilities_preserves_invariant_across_upgrades` | `#[ignore]` — documents upgrade survival conceptually |
| `failed_and_unauthorized_operations_do_not_mutate_total_liabilities` | Rollback safety for failed top-up and rate decrease |
| `total_liabilities_and_sweep_excess_gas_determinism` | Deterministic output across repeated calls |

### Secondary test files

| File | Relevant tests |
|------|---------------|
| `contracts/stream/tests/storage_invariants.rs` | `total_liabilities_increments_on_create` — verifies +deposit on create |
| `contracts/stream/tests/balance_conservation.rs` | `prop_random_op_sequences_preserve_invariants` — global token conservation (256 cases) |
| `contracts/stream/tests/sweep_liability.rs` | `test_sweep_excess_excludes_liabilities_and_fees` — excess-only transfer; `test_keeper_fee_rounding_*` — rounding invariants |
| `contracts/stream/tests/security_invariants.rs` | §13 Liability Solvency tests (13.1–13.3) |
| `contracts/stream/tests/top_up_boundary.rs` | Tests 6 & 7 — near-ceiling `TotalLiabilities` overflow (both `#[ignore]`) |
| `contracts/stream/tests/storage_key_compat.rs` | `v5_total_liabilities_readable_by_v9`, `discriminant_14_total_liabilities_round_trips` |
| `contracts/stream/tests/keeper_cancel.rs` | Property test verifying `TotalLiabilities` drops by outstanding balance after keeper cancel |
| `contracts/stream/tests/clone_stream.rs` | `clone_correctly_updates_total_liabilities` — +new_deposit on clone |
| `contracts/stream/tests/adversarial_auth.rs` | `test_sweep_excess_admin_to_cold_treasury_succeeds`, `test_sweep_excess_rejects_non_admin`, `test_sweep_excess_zero_excess_is_noop`, `test_sweep_excess_preserves_solvency_invariant` |
| `contracts/stream/tests/integration_suite.rs` | `sweep_excess_returns_zero_when_no_excess`, `sweep_excess_after_stream_cancellation`, `sweep_excess_after_rate_decrease`, `sweep_excess_requires_admin_auth` |
| `contracts/stream/tests/cliff_only_variant.rs` | `TotalLiabilities` tracking for `CliffOnly` streams |
| `contracts/stream/tests/bulk_cancel.rs` | `TotalLiabilities` correctly decremented in bulk cancel |

---

## Regression Surface (Identified Gaps & Edge Cases)

The following areas represent the **expected regression surface** — conditions
under which the liability invariant could be violated and where future changes
should be validated.

### 1. `i128::MAX` boundary conditions

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| `TotalLiabilities` reaches `i128::MAX` with multiple streams | `create_stream` with a large deposit could overflow the counter | `top_up_boundary.rs` tests 6 & 7 (both `#[ignore]`) |
| `TotalLiabilities` + `top_up` overflows after many streams | Panic or silent wrap if unchecked arithmetic were introduced | `../../src/lib.rs` uses `checked_add` — verified |
| `withdraw` when `TotalLiabilities` is at `i128::MIN` | Underflow absurd; `i128::MIN` negative initial state is impossible (init sets to 0) | No test seeds `TotalLiabilities` near `i128::MIN` |

### 2. Multi-token (or future) scenarios

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| Only one token is supported by design | `TotalLiabilities` is a single `i128` counter representing one token; multi-token would need separate counters | N/A (by design) |
| Token contract balance < TotalLiabilities due to direct transfer out | `sweep_excess` would compute `excess = 0` (saturating_sub), but `withdraw` would fail on insufficient balance | No test simulates the token contract losing funds independently |

### 3. Concurrent / reentrancy paths

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| Reentrancy during `push_token` in `sweep_excess` | `ReentrancyLock` protects the sweep; nested call returns `InvalidState` | `sweep_excess` — verified via code review |
| Reentrancy during `pull_token` in `create_stream` | `create_stream` increments `TotalLiabilities` **before** `pull_token`, so a re-entering call observes the updated liability | CEI checks in `security_invariants.rs` |
| Batch liability flush skips write when no withdrawals occur | `liabilities_changed` flag ensures write is skipped; but read-modify-write pattern is correct | Implicit in `batch_withdraw` implementation |

### 4. Edge-case tokens & balances

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| Dust accumulation over many streams | Small excess from rounding builds up — `sweep_excess` handles it correctly | `sweep_liability.rs` tests rounding |
| Inflation token (balance changes without transfers) | Contract balance could go up without liability change — excess increases; no violation | Not tested (token-assumptions.md explicitly lists this as a non-goal) |
| Deflation/rebasing token | Actual contract balance could decrease below liabilities if token burns | Not tested (explicitly out of scope per token-assumptions.md) |

### 5. Batch operation liability tracking

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| `batch_withdraw` with interleaved failures | Liability decremented only on success — the in-memory accumulator tracks correctly | Implicit: per-stream result check in `batch_withdraw` |
| `bulk_cancel_streams` liability flush | Single flush after loop — verified in `gas.md` | Tested in `bulk_cancel.rs` |
| Cross-stream `batch_withdraw` where one stream has 0 withdrawable | Liability unchanged for that stream (no decrement) | In-memory accumulator only flushes if `liabilities_changed` |

### 6. Offer-flow liability (HIGH SEVERITY GAP)

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| `create_stream_offer` increments `TotalLiabilities` | Deposit held in escrow — liability tracked even before stream is active | Covered implicitly via offer creation tests |
| `reject_stream_offer` must decrement `TotalLiabilities` | If the decrement is omitted, `TotalLiabilities` becomes **permanently inflated**, causing `sweep_excess` to under-report excess (treasury loses funds) | **Not tested** in liability-specific tests |
| `cancel_stream_offer` must decrement `TotalLiabilities` | Same permanent-inflation risk as rejection | **Not tested** in liability-specific tests |
| `accept_stream_offer` re-uses the already-counted liability | The liability was counted at offer-creation time — activation does **not** double-count | Not tested in liability-specific tests |

**Why this is high severity:** A missed decrement in the offer-rejection or
offer-cancellation path permanently inflates `TotalLiabilities`. Since
`sweep_excess` uses `contract_balance.saturating_sub(TotalLiabilities)`, an
inflated counter causes the treasury sweep to transfer less excess than it
should. Over many offer rejections/cancellations, this excess accumulates
unrecoverably in the contract. **Recommendation:** Add explicit liability
decrement assertions to the existing offer-flow tests in
`contracts/stream/tests/stream_offer.rs`.

### 7. Clone flow

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| `clone_stream` increments `TotalLiabilities` by new deposit | New stream's deposit added to liability counter | `clone_stream.rs` — locked down |

### 8. Irrevocable streams

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| Irrevocable stream blocks cancel/shorten | Liability cannot be reduced via those paths — must wait for completion or withdrawal | Not tested in liability-specific tests (irrevocable tests exist in `security_invariants.rs`) |

### 9. Paused streams

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| Paused streams still accrue liability | `withdraw` is blocked but liability is not reduced; contract remains solvent | Covered by property tests in `liability_invariant.rs` (pause/resume in sequence) |

### 10. CliffOnly stream interactions

| Gap | Risk | Existing coverage |
|-----|------|------------------|
| `top_up`, decrease_rate, etc. are `UnsupportedStreamKind` for CliffOnly | Liability only changes via `create`, `withdraw`, `cancel` | Covered by `cliff_only_variant.rs` and property tests |

---

## Runbook

### Run all liability-related tests

```bash
# Primary liability invariant suite
cargo test -p fluxora_stream --features testutils --test liability_invariant

# Balance conservation (property-based)
cargo test -p fluxora_stream --features testutils --test balance_conservation

# Sweep liability edge cases
cargo test -p fluxora_stream --features testutils --test sweep_liability

# Storage invariants (includes TotalLiabilities on create)
cargo test -p fluxora_stream --features testutils --test storage_invariants

# Security invariants (includes liability solvency checks)
cargo test -p fluxora_stream --features testutils --test security_invariants

# Top-up boundary regression (near-ceiling overflow)
cargo test -p fluxora_stream --features testutils --test top_up_boundary

# Storage key compatibility (V5 TotalLiabilities reads)
cargo test -p fluxora_stream --features testutils --test storage_key_compat

# Keeper cancel TotalLiabilities invariant
cargo test -p fluxora_stream --features testutils --test keeper_cancel

# Clone liability test
cargo test -p fluxora_stream --features testutils --test clone_stream

# CliffOnly liability tracking
cargo test -p fluxora_stream --features testutils --test cliff_only_variant

# Bulk cancel liability
cargo test -p fluxora_stream --features testutils --test bulk_cancel

# Adversarial auth (sweep authorization)
cargo test -p fluxora_stream --features testutils --test adversarial_auth

# Integration suite (sweep_excess scenarios)
cargo test -p fluxora_stream --features testutils --test integration_suite
```

### Run with higher proptest coverage

```bash
PROPTEST_CASES=10000 cargo test -p fluxora_stream --features testutils --test liability_invariant
PROPTEST_CASES=10000 cargo test -p fluxora_stream --features testutils --test balance_conservation
```

### Run all stream tests

```bash
cargo test -p fluxora_stream --features testutils
```

---

## Version Compatibility

### DataKey discriminant

`TotalLiabilities` is discriminant **14** in the `DataKey` enum. This
discriminant was frozen in the V5 release and must never change. Locked down
by `discriminant_14_total_liabilities_round_trips` in `storage_key_compat.rs`.

### `CONTRACT_VERSION`

`CONTRACT_VERSION` is currently `9`. Any change to `TotalLiabilities` storage
behaviour (new mutation paths, type changes, storage key changes) requires a
`CONTRACT_VERSION` bump per the policy in `lib.rs`.

### V5 backward compatibility

V5-written `TotalLiabilities` entries (discriminant 14) are fully readable
by V9. Locked down by `v5_total_liabilities_readable_by_v9` in
`storage_key_compat.rs`.

---

## Appendix: Proptest Shrinking & Debugging

When a property test fails, `proptest` shrinks the input to the minimal
failing case. To reproduce:

```bash
# Run and capture the seed
cargo test -p fluxora_stream --features testutils --test liability_invariant \
  prop_liability_invariant -- --nocapture

# Re-run with a specific seed
PROPTEST_CASES=1 PROPTEST_SEED=0x<seed> \
  cargo test -p fluxora_stream --features testutils --test liability_invariant
```

The `check_sweep_invariant` helper in `liability_invariant.rs` prints the
exact step label, operation, timestamp, balance, and liabilities at the
point of failure.
