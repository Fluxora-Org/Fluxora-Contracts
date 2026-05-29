# Withdrawal Dust Threshold

Reference guide for the `withdraw_dust_threshold` field on the `Stream` struct.
Covers the enforcement formula, USDC worked examples, a validation table, and
guidance for template authors.

**Related:** [streaming.md §2](./streaming.md#2-accrual-formula) · [token-assumptions.md](./token-assumptions.md) · `contracts/stream/src/lib.rs` (`Stream` struct)

---

## What it does

`withdraw_dust_threshold` is an optional per-stream minimum withdrawal amount
expressed in **raw token units** (the smallest indivisible unit of the token).

When a recipient calls `withdraw`, `withdraw_to`, or `batch_withdraw`, the
contract computes `withdrawable = accrued - withdrawn_amount`. If:

```
withdrawable < withdraw_dust_threshold
```

the call returns `0` immediately — no token transfer, no event, no state change.

This prevents fee and event spam from micro-withdrawals on high-frequency or
long-running streams where the per-second accrual is very small.

---

## Bypass conditions

The threshold is **ignored** in two situations so the recipient can always
recover their full entitlement:

| Condition | Why bypassed |
|-----------|-------------|
| **Terminal state** — `status == Cancelled` or `ledger.timestamp() >= end_time` | Stream is over; the recipient must be able to drain the remaining balance. |
| **Final drain** — `withdrawn_amount + withdrawable == deposit_amount` | The withdrawal that completes the stream is never blocked, regardless of amount. |

---

## USDC on Stellar: decimal context

USDC on Stellar uses **7 decimal places**:

```
1 USDC = 10_000_000 raw units  (1e7)
```

All threshold values in this document are in raw units unless stated otherwise.

---

## Formula for safe threshold selection

Choose a threshold that is smaller than the amount that accrues over your
**minimum acceptable withdrawal interval** (the shortest time between withdrawals
you are willing to support):

```
threshold ≤ rate_per_second × minimum_interval_seconds
```

If you set the threshold higher than this, a recipient who withdraws at that
cadence will always be blocked.

**Example — payroll stream, daily withdrawals:**

```
rate_per_second     = 1_157  raw/s  (≈ 1 USDC/day at 7 decimals)
minimum_interval    = 86_400 s      (1 day)
safe_threshold      = 1_157 × 86_400 = 99_964_800  (≈ 10 USDC)
```

Setting `withdraw_dust_threshold = 10_000_000` (1 USDC) is well within the safe
range for this stream.

---

## Worked USDC examples

### Example 1 — High-frequency micro-stream

| Parameter | Value |
|-----------|-------|
| `deposit_amount` | 700_000_000 (70 USDC) |
| `rate_per_second` | 810 raw/s (≈ 0.07 USDC/day) |
| `duration` | 864_000 s (10 days) |
| `withdraw_dust_threshold` | 100_000 (0.01 USDC) |

At `t = 100 s`, `withdrawable = 81_000` (< 100_000 threshold) → **blocked**.  
At `t = 200 s`, `withdrawable = 162_000` (> 100_000 threshold) → **allowed**, transfers 162_000.

### Example 2 — Monthly payroll stream

| Parameter | Value |
|-----------|-------|
| `deposit_amount` | 30_000_000_000 (3,000 USDC) |
| `rate_per_second` | 11_574 raw/s (≈ 100 USDC/day) |
| `duration` | 2_592_000 s (30 days) |
| `withdraw_dust_threshold` | 10_000_000 (1 USDC) |

A recipient withdrawing every hour accrues `11_574 × 3_600 = 41_666_400` raw
(≈ 4.17 USDC) — well above the 1 USDC threshold. Hourly withdrawals are allowed.

A recipient withdrawing every second accrues `11_574` raw (≈ 0.001 USDC) — below
the threshold. Per-second withdrawals are blocked, reducing event spam.

### Example 3 — Short vesting stream (cliff risk)

| Parameter | Value |
|-----------|-------|
| `deposit_amount` | 10_000_000 (1 USDC) |
| `rate_per_second` | 10 raw/s |
| `duration` | 1_000_000 s (~11.6 days) |
| `withdraw_dust_threshold` | 5_000_000 (0.5 USDC) |

The stream accrues 10 raw/s. To accumulate 5_000_000 raw (the threshold), the
recipient must wait `5_000_000 / 10 = 500_000 s` (≈ 5.8 days) before the first
withdrawal is allowed.

At stream end (`t = 1_000_000 s`), `withdrawable = 10_000_000 - 0 = 10_000_000`
(> threshold) → **allowed** (also a final drain, so bypassed regardless).

---

## Validation table

The table below shows expected behavior for common `(deposit_amount, threshold, withdrawable)` combinations.

| `deposit_amount` | `threshold` | `withdrawable` | `is_terminal` | `is_final_drain` | Result |
|-----------------|-------------|----------------|---------------|-----------------|--------|
| 1_000_000_000 | 0 | 1 | No | No | ✅ Allowed (threshold = 0 is a no-op) |
| 1_000_000_000 | 10_000_000 | 5_000_000 | No | No | ❌ Blocked (below threshold) |
| 1_000_000_000 | 10_000_000 | 15_000_000 | No | No | ✅ Allowed (above threshold) |
| 1_000_000_000 | 10_000_000 | 5_000_000 | Yes | No | ✅ Allowed (terminal bypass) |
| 1_000_000_000 | 10_000_000 | 5_000_000 | No | Yes | ✅ Allowed (final drain bypass) |
| 1_000_000_000 | 10_000_000 | 10_000_000 | No | No | ✅ Allowed (exactly at threshold) |
| 1_000_000_000 | 1_000_000_000 | — | — | — | ❌ Creation rejected (`InvalidDustThreshold`) |
| 1_000_000_000 | 1_000_000_001 | — | — | — | ❌ Creation rejected (`InvalidDustThreshold`) |

> **Note:** `withdrawable == threshold` is **allowed** — the check is strictly less-than (`<`), not less-than-or-equal.

---

## Safety constraint at creation

`withdraw_dust_threshold` must satisfy:

```
0 ≤ withdraw_dust_threshold ≤ deposit_amount
```

If `withdraw_dust_threshold > deposit_amount`, the contract rejects creation with
`ContractError::InvalidDustThreshold` (error code 20). This prevents a
misconfiguration where the threshold can never be reached, permanently locking
the recipient's funds in non-terminal withdrawals.

**Negative values** are also rejected because `withdraw_dust_threshold` is typed
as `i128` and the contract validates `deposit_amount > 0`; a negative threshold
would always pass the `withdrawable < threshold` check (since `withdrawable ≥ 0`),
making it equivalent to `threshold = 0`.

---

## Interaction with `rate_per_second` and short durations

For very short streams (duration < a few minutes), the total deposit may be small
enough that even a modest threshold blocks all withdrawals until the terminal bypass
kicks in.

**Rule of thumb:** For a stream of duration `D` seconds and rate `R` raw/s:

```
maximum_safe_threshold = R × D  (= deposit_amount)
```

Setting `threshold = deposit_amount` is technically valid at creation but means
the recipient can only withdraw at the terminal bypass (after `end_time` or on
cancellation). This is a valid pattern for "lock until end" streams but should be
intentional.

**Short-duration example:**

```
rate_per_second  = 1_000_000  (0.1 USDC/s)
duration         = 60 s
deposit_amount   = 60_000_000 (6 USDC)
threshold        = 10_000_000 (1 USDC)
```

The recipient must wait `10_000_000 / 1_000_000 = 10 s` before the first
withdrawal is allowed. For a 60-second stream this is reasonable. For a 5-second
stream with the same threshold, the recipient would be blocked for the entire
stream duration and could only withdraw at `end_time`.

---

## Guidance for template authors

If you are building a `StreamScheduleTemplate` or a factory contract that sets
`withdraw_dust_threshold` on behalf of users:

1. **Default to `0`** unless you have a specific reason to filter micro-withdrawals.
   A zero threshold is always safe and imposes no restrictions.

2. **Derive from rate, not from a fixed constant.** A threshold that is appropriate
   for a 100 USDC/day stream may lock funds on a 0.01 USDC/day stream.
   Use the formula: `threshold = rate_per_second × expected_withdrawal_interval_seconds`.

3. **Document the threshold in your template's metadata.** Recipients should know
   the minimum amount they need to accumulate before a withdrawal is processed.

4. **Never set `threshold > deposit_amount`.** The contract rejects this at creation,
   but a factory that computes the threshold dynamically should validate this
   invariant before calling `create_stream`.

5. **Consider the cliff interaction.** If a stream has a cliff, the recipient cannot
   withdraw before `cliff_time` regardless of the threshold. After the cliff, the
   full accrual since `start_time` is available — this is often well above any
   reasonable threshold.

6. **Test with the shortest expected stream duration.** A threshold that works for
   a 30-day stream may block all withdrawals on a 1-hour stream with the same rate.

---

## Default value

`withdraw_dust_threshold` defaults to `0` when not supplied:

- `create_stream`: pass `0` explicitly.
- `create_streams` / `create_stream_relative`: `withdraw_dust_threshold` field in
  `CreateStreamParams` / `CreateStreamRelativeParams` is `Option<i128>`; `None`
  is treated as `0`.
- `create_stream_from_template`: pass `0` to disable filtering.

---

## Error reference

| Error | Code | Condition |
|-------|------|-----------|
| `ContractError::InvalidDustThreshold` | 20 | `withdraw_dust_threshold > deposit_amount` at creation |

See [error.md](./error.md) for the full error code reference.
