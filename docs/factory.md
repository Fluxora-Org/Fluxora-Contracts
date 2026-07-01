# Treasury Policy Factory Contract

The `fluxora_factory` contract is an optional wrapper around `FluxoraStream` designed specifically to enforce treasury compliance policies during stream creation.

## Overview

The base `FluxoraStream` contract is highly composable and intentionally un-opinionated about things like maximum stream sizes, minimum durations, and recipient identities. This makes it ideal as a protocol primitive. However, treasuries managing large token reserves often require strict operational policies. 

The `fluxora_factory` acts as a proxy entrypoint to enforce these policies:
- **Recipient Allowlist**: Streams can only be created for recipients explicitly allowlisted by the admin.
- **Deposit Caps**: Enforces a `MaxDepositCap` on the total `deposit_amount` of a single stream.
- **Optional Aggregate Batch Cap**: When enabled, the factory also rejects batches whose total deposit exceeds `MaxDepositCap`, preventing bypass by splitting across entries.
- **Minimum Duration**: Enforces a `MinDuration` (i.e. `end_time - start_time >= min_duration`), preventing overly short or instantaneous streams.
- **Time Relationship Checks**: Rejects invalid schedules before calling `FluxoraStream`. `start_time` must be strictly less than `end_time`, and `cliff_time` must be within the inclusive `[start_time, end_time]` window.

## Initialization & Stream Contract Validation

### Authorization contract

`init` requires the declared `admin` to authorize the call via
`admin.require_auth()`, exactly like every other admin-only entrypoint
(`set_admin`, `set_stream_contract`, `set_allowlist`, `set_cap`,
`set_min_duration`, all of which route through the shared `require_admin`
helper). Without this, any unrelated caller could front-run bootstrap by
calling `init` first and seeding the factory with an admin address they
control, before the intended admin's transaction lands.

`init` checks `AlreadyInitialized` before requiring auth, so a doomed
re-initialization call does not need to pay for, or supply, an authorization
entry — `admin.require_auth()` is only evaluated once it is known the call
could otherwise succeed.

### Stream contract validation

Both `init` and `set_stream_contract` validate the supplied `stream_contract`
address before persisting it. The factory invokes the read-only
`FluxoraStream::version()` entrypoint via `FluxoraStreamClient::try_version`.
Because `try_version` uses `Env::try_invoke_contract` internally, a missing
contract, an EOA (non-contract) address, or a deployed contract that does not
expose `version()` is caught as a typed error and returned as
`FactoryError::InvalidStreamContract`, instead of letting the bad address be
stored and only discovered later when `create_stream` host-traps on the
cross-contract call into `FluxoraStream::create_stream`.

`version()` is intentionally cheap and storage-free, and is documented to
work even before the target `FluxoraStream` contract's own `init` has been
called — so the smoke check never depends on the stream contract's
initialization state.

A failed validation leaves existing state untouched:
- In `init`, no instance storage keys are written if `stream_contract` fails
  validation; a subsequent `init` call with a valid address can still
  succeed.
- In `set_stream_contract`, the previously configured `stream_contract` is
  left in place if the new address fails validation.

## Policy Parameter Validation

Policy parameters are validated before they are written by `init`, `set_cap`,
and `set_min_duration`. Invalid values are rejected at write time so the later
`create_stream` policy checks remain meaningful and cannot be silently bricked by
nonsensical stored configuration. Failed setter calls leave the previously stored
policy unchanged.

| Parameter | Entrypoints | Accepted range | Rejection error | Notes |
|-----------|-------------|----------------|-----------------|-------|
| `max_deposit: i128` | `init`, `set_cap` | `1..=i128::MAX` | `FactoryError::InvalidCap` | `0` and negative caps are rejected because every positive stream deposit would exceed them. |
| `min_duration: u64` | `init`, `set_min_duration` | `0..=3_153_600_000` seconds (`MAX_MIN_DURATION_SECONDS`, 100 365-day years) | `FactoryError::InvalidMinDuration` | `0` is valid and means no additional factory-level minimum duration beyond the required `start_time < end_time` invariant. |

These ranges are also documented in the Rust `///` comments on the factory
entrypoints. Error discriminants are append-only; new variants are added without
renumbering existing values.

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
| `get_factory_config()` | `FactoryConfig { admin, stream_contract, max_deposit, min_duration, batch_cap_enforced }` | Reads all instance policy fields. Returns `FactoryError::NotInitialized` before `init`. |
| `is_allowlisted(recipient)` | `bool` | Returns `true` only when the recipient currently has an allowlist entry. Missing entries return `false`. |

These views are permissionless and do not mutate factory state.

## Important Bypass Warning

> [!WARNING]
> Because the underlying `FluxoraStream` contract does not natively enforce these policies, **they are only enforced if the stream is created by routing through the factory contract.** 
> 
> If a user (e.g. the treasury multi-sig itself) directly calls `create_stream` on the `FluxoraStream` contract, these policies will be bypassed. To truly lock down treasury funds, the token vault or multi-sig must be configured to *only* approve transactions that invoke the `fluxora_factory` contract.

## Stream Kind and Memo

`FluxoraFactory::create_stream` accepts two additional parameters that are
forwarded verbatim to `FluxoraStream::create_stream`:

| Parameter | Type | Description |
|-----------|------|-------------|
| `stream_kind` | `fluxora_stream::StreamKind` | `StreamKind::Linear` (standard time-vesting) or `StreamKind::CliffOnly` (full deposit unlocked at cliff). |
| `memo` | `Option<soroban_sdk::Bytes>` | Optional opaque correlation bytes stored on the stream and readable via `get_stream_memo`. Length validation is delegated to the stream contract. |

All policy checks (allowlist, deposit cap, minimum duration, time invariants,
rate bounds) are enforced **before** the cross-contract call, regardless of
`stream_kind`. A `CliffOnly` stream is subject to exactly the same treasury
policy guards as a `Linear` stream.

For `CliffOnly` streams the `rate_per_second` argument is ignored — the stream
contract sets the effective rate to `0` internally.



The factory contract follows the Checks-Effects-Interactions (CEI) pattern implicitly:
1. **Checks**: Validates the recipient against the allowlist, validates the stream time relationship, and bounds the deposit and duration against the configured caps.
2. **Effects**: No local persistent state changes occur during a successful stream creation.
3. **Interactions**: Makes a cross-contract call to `FluxoraStream::create_stream` or `FluxoraStream::create_streams`.

## Batch creation semantics

`FluxoraFactory::create_streams` is an atomic batch wrapper around `FluxoraStream::create_streams`.
- Each entry is validated against the factory policy individually.
- Each recipient in the batch must be allowlisted.
- Each stream must individually satisfy the per-stream cap, minimum duration, and any configured rate-per-second bounds (MinRatePerSecond and MaxRatePerSecond).
- When `batch_cap_enforced` is enabled, the sum of all `deposit_amount` values in the batch is also checked against `MaxDepositCap`.
- A single invalid entry causes the entire batch to revert, ensuring no partial or policy-violating streams can be created.

> **Note:** The factory intentionally uses the atomic `create_streams` endpoint rather than `create_streams_partial` to ensure strict, all-or-nothing treasury policy compliance. For more details on the difference between atomic and partial batch creation at the stream contract level, see [Batch Creation: Atomic vs Partial](streaming.md#batch-creation-atomic-vs-partial).

### Registry writes for batch creation

After the downstream `FluxoraStream::create_streams` call succeeds, the factory appends every returned stream ID to the `FactoryStreamIds` persistent registry **in creation order** with a single TTL bump for the whole batch. This ensures:

- `get_factory_stream_count` increases by the number of streams in the batch.
- `get_factory_streams_paginated` returns all batch IDs in insertion order.
- An empty batch produces no registry writes and leaves the count unchanged.
- IDs are only written after the cross-contract call succeeds; a downstream failure leaves no orphan index entries.

This mirrors the behaviour of the single `create_stream` path, which appends its one ID immediately after successful creation. The batch path is therefore equivalent to N sequential single-stream creations from the registry's perspective, but O(1) TTL bumps instead of O(N).

## Cross-contract authorization model

Factory-routed creation has one client-facing entrypoint, but the sender authorization
must cover both the wrapper call and the nested stream call:

```mermaid
flowchart TD
    client[Client transaction]
    factory["fluxora_factory.create_stream(sender, recipient, deposit, rate, start, cliff, end, dust_threshold, stream_kind, memo)"]
    stream["fluxora_stream.create_stream(sender, recipient, deposit, rate, start, cliff, end, dust_threshold, memo, stream_kind)"]
    token["token.transfer_from(sender -> fluxora_stream, deposit)"]

    client --> factory
    factory --> stream
    stream --> token
```

The required authorization scopes are:

For `fluxora_factory.create_streams`, the sender must authorize the factory batch call and the nested `fluxora_stream.create_streams` sub-invocation in the same transaction.


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
nested `fluxora_stream.create_stream` sub-invocation with the forwarded
`stream_kind` and `memo` arguments.

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
- Call `set_batch_cap_enforcement` to toggle aggregate batch-cap validation.
- Call `set_stream_contract` to upgrade or switch the underlying stream primitive if a new version is deployed. The new address must pass the same `FluxoraStream::version()` smoke check enforced in `init` (see [Initialization & Stream Contract Validation](#initialization--stream-contract-validation)); a bad address is rejected with `FactoryError::InvalidStreamContract` and the previous stream contract remains active.
- Call `set_rate_bounds` to configure optional inclusive rate-per-second bounds.

The factory admin can shape policy and the target stream contract, but cannot
spend sender funds by itself. A factory-routed stream still needs the `sender`
authorization described above, and the underlying stream contract still enforces
its own authorization table. See the [`docs/security.md` admin powers
section](security.md#admin-powers) for the protocol-wide admin boundary.

## Events

Every state-changing factory entrypoint emits a structured Soroban event so that
indexers, treasury dashboards, and monitoring tools can observe policy changes and
stream creation without re-reading storage. Topic symbols are ≤ 9 characters per
the `symbol_short!` constraint.

| Entrypoint | Topic | Data struct | Notes |
|---|---|---|---|
| `init` | `fct_init` | `FactoryInited { admin, stream_contract, max_deposit, min_duration }` | Emitted once on deployment. |
| `set_admin` | `AdminUpd` | `FactoryAdminUpdated { old_admin, new_admin }` | Mirrors the `AdminUpd` topic used in `FluxoraStream`. |
| `set_stream_contract` | `stm_upd` | `StreamContractUpdated { old_contract, new_contract }` | Emitted after the pointer is updated. |
| `set_allowlist` | `allow_upd` | `AllowlistUpdated { recipient, allowed }` | `allowed: true` = added; `false` = removed. Sufficient for an indexer to reconstruct membership. |
| `set_cap` | `cap_upd` | `CapUpdated { old_cap, new_cap }` | Both old and new values are included. |
| `set_min_duration` | `dur_upd` | `MinDurationUpdated { old_min_duration, new_min_duration }` | Both old and new values are included. |
| `set_rate_bounds` | `rate_bnd` | `RateBoundsUpdated { min_rate, max_rate }` | Carries the arguments passed by the caller; `None` means "unchanged". |
| `set_factory_paused` | `factory` + `paused`/`resumed` | `bool` | Pre-existing event, unchanged. |
| `create_stream` (success) | `fct_strm` | `FactoryStreamCreated { stream_id, sender, recipient, deposit_amount, rate_per_second }` | Emitted only after the cross-contract call succeeds. Lets indexers attribute a stream to the policy-gated factory path. |

See [docs/events.md](events.md) for the complete event catalogue across all contracts.

## Instance Storage TTL Management

The factory's entire configuration (Admin, StreamContract, MaxDepositCap, MinDuration, BatchCapEnforced, rate bounds, CreationPaused) is stored in **instance storage**, not persistent storage. Instance entries are automatically pruned by the Soroban ledger when their TTL (time-to-live) expires. A long-idle factory whose instance entries expire becomes uninitialized and bricks all admin operations — a denial-of-service against the contract itself.

To prevent expiration, the factory implements a `bump_instance` helper that:
- Extends the instance storage TTL to a safe threshold whenever the factory is actively used.
- Mirrors the governance contract's TTL constants for consistency across contracts.

### TTL Constants

```rust
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;   // Ledgers below which a bump is triggered
const INSTANCE_BUMP_AMOUNT: u32 = 120_960;         // Bump target; ~60 days at 5-second ledger close
```

### When Instance TTL is Bumped

- **`init()`**: Bumps TTL during factory initialization.
- **Every admin setter**: `set_admin`, `set_stream_contract`, `set_cap`, `set_min_duration`, `set_batch_cap_enforcement`, `set_rate_bounds`, `set_factory_paused` all bump the instance TTL after a successful update.
- **Stream creation**: Both `create_stream` and `create_streams` bump the instance TTL when called, since they read the factory configuration.

### Why It Matters

A factory that goes idle for longer than the TTL threshold (~60 days) will have its instance entries pruned by the ledger. Subsequent calls to `get_factory_config` or any admin setter will return `FactoryError::NotInitialized`, preventing further operations until the factory is re-initialized—an unrecoverable error on a deployed contract.

By bumping the TTL on every write and read, even a purely inactive factory can survive indefinitely as long as it is occasionally queried or updated. An active factory (regularly creating streams or updating policies) will always keep the instance entries alive.

### Security Note

TTL bumps are performed internally by the factory and do not require additional authorization. They operate on already-protected instance storage without exposing any new attack surface. The bump operation is local to the factory; it does not invoke external contracts or expose any caller-controlled parameters.

## Code alignment checklist

This document is aligned with the current implementation as follows:

- `FluxoraFactory::init`, `set_cap`, and `set_min_duration` validate policy
  ranges before writing factory configuration.
- All setters and stream creation functions call `bump_instance()` to extend
  instance storage TTL and prevent config expiration during idle periods.
- `FluxoraFactory::create_stream` enforces allowlist, cap, and duration checks
  before calling `sender.require_auth()`.
- The factory forwards `stream_kind` and `memo` verbatim to
  `FluxoraStream::create_stream`; all policy gates apply regardless of kind.
- `FluxoraStream::create_stream` calls `sender.require_auth()` before validating
  parameters and pulling `deposit_amount` from `sender`.
- `FluxoraFactory::create_streams` appends all returned stream IDs to the
  `FactoryStreamIds` registry in creation order after the cross-contract call
  succeeds, with a single TTL bump for the whole batch (see `append_stream_ids_batch`
  in `contracts/factory/src/lib.rs`).
- `contracts/stream/tests/factory_policy.rs` covers policy input validation,
  factory policy gates, `CliffOnly` kind forwarding, memo forwarding,
  append-only error discriminants, and admin-guarded policy updates.
