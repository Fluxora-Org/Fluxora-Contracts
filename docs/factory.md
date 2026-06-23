# Treasury Policy Factory Contract

The `fluxora_factory` contract is an optional wrapper around `FluxoraStream` designed specifically to enforce treasury compliance policies during stream creation.

## Overview

The base `FluxoraStream` contract is highly composable and intentionally un-opinionated about things like maximum stream sizes, minimum durations, and recipient identities. This makes it ideal as a protocol primitive. However, treasuries managing large token reserves often require strict operational policies. 

The `fluxora_factory` acts as a proxy entrypoint to enforce these policies:
- **Recipient Allowlist**: Streams can only be created for recipients explicitly allowlisted by the admin.
- **Deposit Caps**: Enforces a `MaxDepositCap` on the total `deposit_amount` of a single stream.
- **Minimum Duration**: Enforces a `MinDuration` (i.e. `end_time - start_time >= min_duration`), preventing overly short or instantaneous streams.
- **Time Relationship Checks**: Rejects invalid schedules before calling `FluxoraStream`. `start_time` must be strictly less than `end_time`, and `cliff_time` must be within the inclusive `[start_time, end_time]` window.

## Time Validation

The factory mirrors the underlying stream contract's creation-time schedule invariants and returns typed factory errors before making the cross-contract call:

| Condition | Error |
|-----------|-------|
| `start_time >= end_time` | `FactoryError::InvalidTimeRange` |
| `cliff_time < start_time` | `FactoryError::InvalidCliff` |
| `cliff_time > end_time` | `FactoryError::InvalidCliff` |

These checks keep invalid treasury requests on the factory error surface instead of relying on downstream stream-contract panics.

## Read-Only Views

The factory exposes read-only views so UIs, operators, and indexers can inspect policy before routing treasury activity through the wrapper.

| View | Returns | Notes |
|------|---------|-------|
| `get_factory_config()` | `FactoryConfig { admin, stream_contract, max_deposit, min_duration }` | Reads all instance policy fields. Returns `FactoryError::NotInitialized` before `init`. |
| `is_allowlisted(recipient)` | `bool` | Returns `true` only when the recipient currently has an allowlist entry. Missing entries return `false`. |

These views are permissionless and do not mutate factory state.

## Important Bypass Warning

> [!WARNING]
> Because the underlying `FluxoraStream` contract does not natively enforce these policies, **they are only enforced if the stream is created by routing through the factory contract.** 
> 
> If a user (e.g. the treasury multi-sig itself) directly calls `create_stream` on the `FluxoraStream` contract, these policies will be bypassed. To truly lock down treasury funds, the token vault or multi-sig must be configured to *only* approve transactions that invoke the `fluxora_factory` contract.

## Architecture & CEI

The factory contract follows the Checks-Effects-Interactions (CEI) pattern implicitly:
1. **Checks**: Validates the recipient against the allowlist, validates the stream time relationship, and bounds the deposit and duration against the configured caps.
2. **Effects**: No local persistent state changes occur during a successful stream creation.
3. **Interactions**: Makes a cross-contract call to `FluxoraStream::create_stream`.

## Cross-contract authorization model

Factory-routed creation has one client-facing entrypoint, but the sender authorization
must cover both the wrapper call and the nested stream call:

```mermaid
flowchart TD
    client[Client transaction]
    factory["fluxora_factory.create_stream(sender, recipient, deposit, rate, start, cliff, end, dust_threshold)"]
    stream["fluxora_stream.create_stream(sender, recipient, deposit, rate, start, cliff, end, dust_threshold, None, Linear)"]
    token["token.transfer_from(sender -> fluxora_stream, deposit)"]

    client --> factory
    factory --> stream
    stream --> token
```

The required authorization scopes are:

| Signer | Scope | Why it is required |
| --- | --- | --- |
| `sender` | `fluxora_factory.create_stream(...)` with the exact wrapper arguments | `FluxoraFactory::create_stream` calls `sender.require_auth()` after policy checks pass. |
| `sender` | Nested `fluxora_stream.create_stream(...)` with the exact stream arguments the factory forwards | `FluxoraStream::create_stream` also calls `sender.require_auth()` before validating and pulling the deposit. |

This is not two independent user intents. A client should build the Soroban
authorization tree so the `sender` signs the factory invocation and its
`fluxora_stream.create_stream` sub-invocation in the same transaction. The nested
scope is intentionally narrow: it authorizes only the exact stream creation that
the factory forwards after enforcing recipient, cap, and duration policy.

The stream contract, not the factory, pulls `deposit_amount` from `sender` into
the stream contract during `fluxora_stream.create_stream`. The factory never
custodies the sender's tokens and has no standing privilege to spend sender
funds. If a later transaction tries to reuse the factory or a changed set of
arguments, the sender must authorize that new invocation tree again.

### Worked client-signing example

Assume a treasury UI wants to create this routed stream:

```text
sender = G_SENDER
recipient = G_RECIPIENT
deposit_amount = 1_000
rate_per_second = 1
start_time = 1_800_000_000
cliff_time = 1_800_000_000
end_time = 1_800_001_000
withdraw_dust_threshold = 0
```

The client prepares a transaction whose root host function invokes
`fluxora_factory.create_stream` with those values. During simulation/preparation,
the authorization tree must contain `G_SENDER` for the root factory call and the
nested `fluxora_stream.create_stream` sub-invocation with:

```text
memo = None
kind = Linear
```

`G_SENDER` signs that prepared authorization tree. The factory admin does not
sign stream creation unless the admin is also the `sender`. The recipient does
not sign creation. The recipient signs only later recipient-controlled actions
such as `withdraw` or `withdraw_to`.

### Single-auth vs dual-scope auth

For UI and wallet copy, describe the flow as "one sender signing session with two
scopes" rather than "two unrelated signatures":

1. The factory scope lets the sender opt into the treasury policy wrapper.
2. The stream scope lets the stream contract create the stream and pull exactly
   the authorized deposit from the sender.

If the client omits either scope, the transaction fails at the corresponding
`require_auth` call. If the sub-invocation arguments differ from the signed
arguments, the nested authorization is not valid for that call.

## Admin Controls

The factory has an `Admin` key managed via `set_admin`. The admin can:
- Call `set_allowlist` to grant or revoke recipient eligibility.
- Call `set_cap` to update the max deposit limit.
- Call `set_min_duration` to update the minimum duration requirement.
- Call `set_stream_contract` to upgrade or switch the underlying stream primitive if a new version is deployed.

The factory admin can shape policy and the target stream contract, but cannot
spend sender funds by itself. A factory-routed stream still needs the `sender`
authorization described above, and the underlying stream contract still enforces
its own authorization table. See the [`docs/security.md` admin powers
section](security.md#admin-powers) for the protocol-wide admin boundary.

## Factory Stream Registry

Every stream ID that passes all policy checks and is successfully created through
the factory is appended to a persistent on-chain registry. This gives operators,
auditors, and migration tooling a complete, factory-authoritative list of policy-
gated streams that is queryable without relying on off-chain indexers.

### Storage key

The registry is stored under `DataKey::FactoryStreamIds` in persistent storage as
a `Vec<u64>` (ordered by creation time). The key is independent from the stream
contract's own per-recipient index and from the global `StreamCount`.

### TTL

The index TTL is bumped on every write using the same constants as the stream
contract:

| Constant | Value | Purpose |
|---|---|---|
| `PERSISTENT_LIFETIME_THRESHOLD` | 17 280 ledgers (~1 day) | Bump only when below this remaining TTL |
| `PERSISTENT_BUMP_AMOUNT` | 120 960 ledgers (~60 days) | Target TTL after a bump |

A busy factory (frequent `create_stream` calls) will continuously reset the index
to ~60 days. An idle factory will keep the index alive for 60 days after the last
creation without additional ledger fees.

### Consistency guarantee

The registry write happens **after** the cross-contract call to
`FluxoraStream::create_stream` returns successfully. If the downstream call
panics (e.g. insufficient balance, stream contract paused), the entire transaction
is reverted and no entry is written. There are no orphan index entries.

Policy failures (allowlist, cap, duration, time invariants) return early before
`sender.require_auth()` and before the cross-contract call, so they also leave the
registry unchanged.

### Query entry points

| Entry Point | Arguments | Returns | Notes |
|---|---|---|---|
| `get_factory_stream_count(env)` | — | `u32` | Total streams ever created through this factory. |
| `get_factory_streams_paginated(env, start_index, limit)` | `start_index: u32`, `limit: u32` | `Vec<u64>` | Returns up to `min(limit, MAX_PAGE_SIZE)` IDs starting at zero-based `start_index`. Returns an empty list when `start_index ≥ count`. |

Both views are permissionless and read-only.

### Pagination

`get_factory_streams_paginated` enforces a hard cap of **`MAX_PAGE_SIZE = 100`**
entries per call, mirroring the stream contract's `get_recipient_streams_paginated`
convention. Callers requesting `limit > 100` will silently receive at most 100
entries. This prevents unbounded ledger reads that could exhaust the transaction
CPU budget.

Typical pagination loop (pseudocode):

```
page_size = 50
start    = 0
loop:
    ids = get_factory_streams_paginated(start, page_size)
    if ids.is_empty(): break
    process(ids)
    start += ids.len()
```

### Security notes

- The registry can only be written via `create_stream`, which already enforces
  allowlist, cap, duration, and time-invariant checks plus `sender.require_auth()`.
  There is no admin path or direct write that can add entries without passing policy.
- The pagination cap prevents a read-DoS: a caller cannot force the contract to
  iterate an unbounded list in a single invocation.
- The factory admin cannot directly modify the registry; it grows only through
  successful policy-gated creations.

## Code alignment checklist

This document is aligned with the current implementation as follows:

- `FluxoraFactory::create_stream` enforces allowlist, cap, and duration checks
  before calling `sender.require_auth()`.
- The factory forwards a linear `FluxoraStream::create_stream` call with
  `memo = None` and `StreamKind::Linear`.
- `FluxoraStream::create_stream` calls `sender.require_auth()` before validating
  parameters and pulling `deposit_amount` from `sender`.
- `append_stream_id` is called only after the cross-contract call succeeds,
  keeping the registry consistent with actual on-chain stream state.
- `get_factory_stream_count` and `get_factory_streams_paginated` are permissionless
  read-only entry points with a hard `MAX_PAGE_SIZE = 100` cap.
- `contracts/stream/tests/factory_policy.rs` covers the factory policy gates,
  admin-guarded policy updates, and registry invariants (count, pagination,
  no-write-on-failure).
