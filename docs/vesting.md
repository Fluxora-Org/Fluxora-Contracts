# Token Vesting Contract

Soroban vesting contract for Fluxora treasury allocations. Supports cliff + linear vesting with admin revocation.

---

## Overview

The vesting contract locks tokens on behalf of a **benefactor** and releases them linearly to a **beneficiary** over a configurable schedule. A cliff prevents any claims before a specified time. The admin may revoke an active schedule at any time, freezing accrual and refunding unvested tokens to the benefactor.

---

## Accrual formula

```
vested(t) = 0                                                    if t < cliff_time
vested(t) = min((min(t, end_time) − start_time) × rate_per_second, total_amount)   if t >= cliff_time
```

- `claimable(t) = vested(t) − claimed_amount`
- After revocation, `vested(t)` is frozen at `revoked_at`; the beneficiary may still claim the frozen amount.

---

## Lifecycle

```
Active ──claim──► Active (partial)
Active ──claim──► Completed (all tokens claimed)
Active ──revoke─► Revoked ──claim──► Completed (frozen amount claimed)
Completed ──close_schedule──► (removed from storage)
```

---

## Entry-points

| Entry-point | Caller | Description |
|---|---|---|
| `init` | Admin | One-shot initialisation: set token and admin |
| `create_vesting` | Benefactor | Create a schedule and pull `total_amount` tokens |
| `claim` | Beneficiary | Claim all currently vested-but-unclaimed tokens |
| `revoke` | Admin | Freeze accrual; refund unvested tokens to benefactor |
| `close_schedule` | Anyone | Remove a `Completed` schedule from storage |
| `get_schedule` | Anyone | View: full `VestingSchedule` struct |
| `get_claimable` | Anyone | View: claimable amount at current ledger time |
| `get_vested_at` | Anyone | View: simulated vested amount at an arbitrary timestamp |
| `get_config` | Anyone | View: token address and admin address |
| `get_schedule_count` | Anyone | View: total schedules created |
| `version` | Anyone | View: `CONTRACT_VERSION` constant |

---

## Parameters for `create_vesting`

| Parameter | Type | Description |
|---|---|---|
| `benefactor` | `Address` | Funder; receives unvested tokens on revocation |
| `beneficiary` | `Address` | Recipient of vested tokens |
| `total_amount` | `i128` | Total tokens locked (must be >= `rate_per_second × (end_time − start_time)`) |
| `rate_per_second` | `i128` | Tokens released per second after cliff |
| `start_time` | `u64` | Vesting epoch start (must be >= current ledger timestamp) |
| `cliff_time` | `u64` | No tokens claimable before this time (`start_time <= cliff_time <= end_time`) |
| `end_time` | `u64` | Vesting epoch end |

---

## Error codes

| Code | Name | Meaning |
|---|---|---|
| 1 | `ScheduleNotFound` | Requested schedule does not exist |
| 2 | `InvalidState` | Operation invalid for current schedule state |
| 3 | `InvalidParams` | Parameter out of range or logically inconsistent |
| 4 | `AlreadyInitialised` | `init` called more than once |
| 5 | `Unauthorized` | Caller is not authorised |
| 6 | `ArithmeticOverflow` | Overflow in vesting calculations |
| 7 | `InsufficientDeposit` | `total_amount < rate_per_second × duration` |
| 8 | `StartTimeInPast` | `start_time < current ledger timestamp` |

---

## Events

### `created` (schedule_id)

Emitted by `create_vesting`.

```
topic:   ("created", schedule_id: u64)
payload: ScheduleCreated {
    schedule_id, benefactor, beneficiary,
    total_amount, rate_per_second,
    start_time, cliff_time, end_time
}
```

### `claimed` (schedule_id)

Emitted by `claim` when `amount > 0`.

```
topic:   ("claimed", schedule_id: u64)
payload: TokensClaimed { schedule_id, beneficiary, amount }
```

### `revoked` (schedule_id)

Emitted by `revoke`.

```
topic:   ("revoked", schedule_id: u64)
payload: ScheduleRevoked { schedule_id, revoked_at, refund_amount }
```

### `closed` (schedule_id)

Emitted by `close_schedule`.

```
topic:   ("closed", schedule_id: u64)
payload: ScheduleClosed { schedule_id }
```

---

## Storage layout

| Key | Storage type | Value type | Description |
|---|---|---|---|
| `Config` (discriminant 0) | Instance | `Config { token, admin }` | Set at `init` |
| `NextScheduleId` (discriminant 1) | Instance | `u64` | Monotonic counter |
| `Schedule(u64)` (discriminant 2) | Persistent | `VestingSchedule` | One entry per schedule |

TTL constants match the stream contract: threshold = 17 280 ledgers (~1 day), bump = 120 960 ledgers (~7 days).

---

## Security

- **CEI ordering**: State is persisted before every token transfer to prevent reentrancy.
- **Auth boundaries**: `create_vesting` requires benefactor auth; `claim` requires beneficiary auth; `revoke` requires admin auth; `close_schedule` is permissionless.
- **Overflow protection**: `rate_per_second × elapsed` uses `checked_mul`; overflow falls back to `total_amount` (safe upper bound).
- **Revocation safety**: Revoked schedules freeze accrual at `revoked_at`; the beneficiary cannot claim more than was vested at that point.
- **Token model**: Assumes a well-behaved SEP-41 / SAC token (no re-entrancy, no silent failures). See `docs/token-assumptions.md`.

---

## Build and test

```bash
# Unit + integration tests
cargo test -p fluxora_vesting --features testutils

# Release WASM
cargo build --release -p fluxora_vesting --target wasm32-unknown-unknown
```
