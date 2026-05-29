# ABI Stability Guarantees

Canonical reference for what the Fluxora protocol guarantees to remain stable across deployments, and what constitutes a breaking change for indexers, wallets, and other integrators.

**Source of truth:** `contracts/stream/src/lib.rs`
**Current version:** `CONTRACT_VERSION = 3`
**See also:** [`docs/upgrade.md`](./upgrade.md) for the migration runbook and version history.

---

## 1. What "ABI" Means in This Protocol

The Fluxora ABI is the complete external surface that integrators depend on:

| Surface | Covered by this doc |
|---|---|
| Public entrypoints (function names, parameter types, return types) | Yes |
| Error codes (`ContractError` discriminant values) | Yes |
| Event topics and payload schemas | Yes |
| Storage key discriminants (`DataKey` enum order) | Yes |
| `CONTRACT_VERSION` semantics | Yes |
| Internal helper functions | No — not part of the ABI |
| TTL bump constants | No — observable only as liveness, not correctness |
| WASM checksum | No — covered by `wasm/checksums.sha256` |

---

## 2. Stability Guarantees

### 2.1 Entrypoints

Every public function listed in Section 4 is **stable** for the lifetime of a deployed contract instance. Stability means:

- The function name will not change.
- The parameter list (names, types, order) will not change.
- The return type will not change.
- The authorization model (who must sign) will not change.
- The success semantics (what the function does when it succeeds) will not change in a way that breaks a correctly-written client.

A new contract deployment (new `CONTRACT_ID`) is required to change any of the above. The old instance continues to honour its original ABI indefinitely (subject to TTL — see `docs/storage.md`).

### 2.2 Error Codes

`ContractError` variants are identified by their `u32` discriminant value. The mapping between variant name and discriminant is **frozen** for any deployed instance:

```
StreamNotFound      = 1
InvalidState        = 2
InvalidParams       = 3
ContractPaused      = 4
StartTimeInPast     = 5
ArithmeticOverflow  = 6
Unauthorized        = 7
AlreadyInitialised  = 8
InsufficientBalance = 9
InsufficientDeposit = 10
StreamAlreadyPaused = 11
StreamNotPaused     = 12
StreamTerminalState = 13
DuplicateStreamId   = 14
TemplateNotFound    = 15
TemplateLimitExceeded = 16
TemplateUnauthorized  = 17
```

Clients MUST match on the numeric discriminant, not the variant name string, because Soroban encodes errors as `u32` values on the wire.

### 2.3 Event Topics and Payload Schemas

Event topics are `symbol_short!` values (max 9 ASCII characters). The mapping between topic string and payload type is **frozen** for any deployed instance:

| Topic | Payload type |
|---|---|
| `"created"` | `StreamCreated` struct |
| `"withdrew"` | `Withdrawal` struct |
| `"wdraw_to"` | `WithdrawalTo` struct |
| `"paused"` | `StreamPaused` struct (v3+) |
| `"resumed"` | `StreamEvent::Resumed(u64)` |
| `"cancelled"` | `StreamEvent::StreamCancelled(u64)` |
| `"completed"` | `StreamEvent::StreamCompleted(u64)` |
| `"closed"` | `StreamEvent::StreamClosed(u64)` |
| `"rate_upd"` | `RateUpdated` struct |
| `"rate_dec"` | `RateDecreased` struct |
| `"end_shrt"` | `StreamEndShortened` struct |
| `"end_ext"` | `StreamEndExtended` struct |
| `"top_up"` | `StreamToppedUp` struct |
| `"recp_upd"` | `RecipientUpdated` struct |
| `"sndr_xfr"` | `SenderTransferred` struct |
| `"AdminUpdated"` | `(old_admin: Address, new_admin: Address)` tuple |
| `"paused_ctl"` | `ContractPauseChanged { paused: bool }` |
| `"gl_pause"` | `GlobalEmergencyPauseChanged { paused: bool }` |
| `"gl_resume"` | `GlobalResumed { resumed_at: u64 }` |
| `"pr_pause"` | `ProtocolPaused { reason: String, paused_at: u64 }` |
| `"pr_resume"` | `ProtocolResumed { resumed_at: u64 }` |
| `"tmpl_def"` | `StreamScheduleTemplate` struct |

Field names and types within each payload struct are also stable. Adding a new field to an existing struct is a breaking change (Soroban XDR encoding is positional).

### 2.4 Storage Key Discriminants

`DataKey` is a `#[contracttype]` enum. Soroban serialises enum variants by their **0-based declaration-order discriminant**. The following table is **immutable** for any deployed instance:

| Discriminant | Variant | Storage class | Value type |
|---|---|---|---|
| 0 | `Config` | Instance | `Config { token: Address, admin: Address }` |
| 1 | `NextStreamId` | Instance | `u64` |
| 2 | `Stream(u64)` | Persistent | `Stream` struct |
| 3 | `RecipientStreams(Address)` | Persistent | `Vec<u64>` |
| 4 | `GlobalEmergencyPaused` | Instance | `bool` |
| 5 | `CreationPaused` | Instance | `bool` |
| 6 | `GlobalPauseReason` | Instance | `String` |
| 7 | `GlobalPauseTimestamp` | Instance | `u64` |
| 8 | `GlobalPauseAdmin` | Instance | `Address` |
| 9 | `AutoClaimDestination(u64)` | Persistent | `Address` |
| 10 | `NextTemplateId` | Instance | `u64` |
| 11 | `ActiveTemplateCount` | Instance | `u64` |
| 12 | `StreamTemplate(u64)` | Persistent | `StreamScheduleTemplate` struct |
| 13 | `OwnerTemplateIds(Address)` | Persistent | `Vec<u64>` |
| 14 | `TotalLiabilities` | Instance | `i128` |

> **Note on memo storage:** Stream memos are stored as the `memo: Option<Bytes>` field inside the `Stream` struct at `DataKey::Stream(stream_id)` (discriminant 2), not as a separate storage key. `docs/storage.md` references a `StreamMemo(u64)` key that does not exist in the current source; the `Stream` struct field is the canonical location.

Discriminants 0–14 are frozen. Any future variant must be appended at position 15 or higher.

---

## 3. What Counts as a Breaking Change

A change is **breaking** if a correctly-written client targeting the current `CONTRACT_VERSION` would fail, produce wrong results, or misinterpret a response when talking to the updated contract.

### 3.1 Breaking — Always requires `CONTRACT_VERSION` increment and new deployment

| Category | Example |
|---|---|
| Remove a public entrypoint | Deleting `withdraw` |
| Rename a public entrypoint | `cancel_stream` → `terminate_stream` |
| Add, remove, or reorder parameters on any entrypoint | Adding a required `memo` param to `withdraw` |
| Change a parameter type | `stream_id: u64` → `stream_id: u128` |
| Change a return type | `withdraw` returning `()` instead of `i128` |
| Change the authorization model | Allowing anyone to call `cancel_stream` |
| Change a `ContractError` discriminant value | Renumbering `StreamNotFound` from 1 to 99 |
| Add a new `ContractError` variant in the middle of the enum | Shifts all subsequent discriminants |
| Remove a `ContractError` variant | Clients matching on the old value get no match |
| Change an event topic string | `"withdrew"` → `"withdraw"` |
| Add, remove, or reorder fields in an event payload struct | Breaks XDR deserialization |
| Change a field type in an event payload struct | `amount: i128` → `amount: u128` |
| Reorder `DataKey` variants | Corrupts all existing persistent storage entries |
| Insert a `DataKey` variant in the middle | Shifts all subsequent discriminants |
| Change the value type of an existing `DataKey` variant | Existing entries become undecodable |
| Change `StreamStatus` discriminant values | `Active=0`, `Paused=1`, `Completed=2`, `Cancelled=3` are frozen |
| Change `PauseReason` discriminant values | `Operational=0`, `Emergency=1`, `Compliance=2`, `Administrative=3` are frozen |
| Change the `Stream` struct field order or types | Breaks persistent storage deserialization |

### 3.2 Non-Breaking — Does NOT require a version increment

| Category | Example |
|---|---|
| Add a new entrypoint (purely additive) | Adding `get_stream_memo` |
| Append a new `ContractError` variant at the end | Adding `TemplateUnauthorized = 17` |
| Append a new `DataKey` variant at the end | Adding `TotalLiabilities` at discriminant 15 |
| Tighten validation that rejects a previously-accepted edge case | Rejecting `deposit_amount == 0` if it was previously a no-op |
| Change TTL bump constants | Increasing `PERSISTENT_BUMP_AMOUNT` |
| Internal refactor with identical external behaviour | Extracting a helper function |
| Documentation-only change | This file |
| Gas optimisation with identical observable behaviour | Reducing storage reads |
| Change error message strings in panics | Panic messages are not part of the ABI |

> **Conservative policy:** Even non-breaking additive changes (new entrypoints, new error variants) should increment `CONTRACT_VERSION` so that integrators can detect the new capability with a single `version()` call. This is the policy followed in this codebase.

### 3.3 Requires documentation but judgment on version bump

| Change | Guidance |
|---|---|
| Tighten validation (reject previously-accepted input) | Increment if any integrator could be relying on the old behaviour |
| Change success semantics in a subtle way | Increment and document the behaviour change in `CHANGELOG.md` |
| Change event emission conditions (e.g. emit on zero-amount withdrawal) | Increment — indexers depend on emission guarantees |

---

## 4. Complete Entrypoint Reference

### 4.1 Initialization

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `init` | `token: Address, admin: Address` | `Result<(), ContractError>` | `admin` |
| `version` | — | `u32` | None |

### 4.2 Stream Creation

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `create_stream` | `sender, recipient, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64, memo: Option<Bytes>` | `Result<u64, ContractError>` | `sender` |
| `create_streams` | `sender, streams: Vec<CreateStreamParams>` | `Result<Vec<u64>, ContractError>` | `sender` |
| `create_stream_relative` | `sender, params: CreateStreamRelativeParams` | `Result<u64, ContractError>` | `sender` |
| `create_streams_relative` | `sender, streams: Vec<CreateStreamRelativeParams>` | `Result<Vec<u64>, ContractError>` | `sender` |
| `create_stream_from_template` | `sender, template_id: u64, recipient, deposit_amount: i128, rate_per_second: i128` | `Result<u64, ContractError>` | `sender` |

### 4.3 Sender Operations

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `pause_stream` | `stream_id: u64, reason: PauseReason` | `Result<(), ContractError>` | stream `sender` |
| `resume_stream` | `stream_id: u64` | `Result<(), ContractError>` | stream `sender` |
| `cancel_stream` | `stream_id: u64` | `Result<(), ContractError>` | stream `sender` |
| `top_up_stream` | `stream_id: u64, funder: Address, amount: i128` | `Result<(), ContractError>` | `funder` |
| `update_rate_per_second` | `stream_id: u64, new_rate: i128` | `Result<(), ContractError>` | stream `sender` |
| `decrease_rate_per_second` | `stream_id: u64, new_rate: i128` | `Result<(), ContractError>` | stream `sender` |
| `shorten_stream_end_time` | `stream_id: u64, new_end_time: u64` | `Result<(), ContractError>` | stream `sender` |
| `extend_stream_end_time` | `stream_id: u64, new_end_time: u64` | `Result<(), ContractError>` | stream `sender` |
| `transfer_sender` | `stream_id: u64, new_sender: Address` | `Result<(), ContractError>` | stream `sender` |

### 4.4 Recipient Operations

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `withdraw` | `stream_id: u64` | `Result<i128, ContractError>` | stream `recipient` |
| `withdraw_to` | `stream_id: u64, destination: Address` | `Result<i128, ContractError>` | stream `recipient` |
| `batch_withdraw` | `recipient: Address, stream_ids: Vec<u64>` | `Result<Vec<BatchWithdrawResult>, ContractError>` | `recipient` |
| `batch_withdraw_to` | `recipient: Address, withdrawals: Vec<WithdrawToParam>` | `Result<Vec<BatchWithdrawResult>, ContractError>` | `recipient` |
| `update_recipient` | `stream_id: u64, new_recipient: Address` | `Result<(), ContractError>` | stream `recipient` |
| `set_auto_claim` | `stream_id: u64, destination: Address` | `Result<(), ContractError>` | stream `recipient` |
| `revoke_auto_claim` | `stream_id: u64` | `Result<(), ContractError>` | stream `recipient` |

### 4.5 Permissionless Operations

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `trigger_auto_claim` | `stream_id: u64` | `Result<i128, ContractError>` | None |
| `close_completed_stream` | `stream_id: u64` | `Result<(), ContractError>` | None |

### 4.6 Admin Operations

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `set_admin` | `new_admin: Address` | `Result<(), ContractError>` | current `admin` |
| `pause_stream_as_admin` | `stream_id: u64, reason: PauseReason` | `Result<(), ContractError>` | `admin` |
| `resume_stream_as_admin` | `stream_id: u64` | `Result<(), ContractError>` | `admin` |
| `cancel_stream_as_admin` | `stream_id: u64` | `Result<(), ContractError>` | `admin` |
| `set_global_emergency_paused` | `paused: bool` | `()` | `admin` |
| `set_contract_paused` | `paused: bool` | `Result<(), ContractError>` | `admin` |
| `pause_protocol` | `admin: Address, reason: Option<String>` | `Result<(), ContractError>` | `admin` |
| `resume_protocol` | `admin: Address` | `Result<(), ContractError>` | `admin` |
| `global_resume` | — | `Result<(), ContractError>` | `admin` |
| `sweep_excess` | `to: Address` | `Result<(), ContractError>` | `admin` |

### 4.7 Template Management

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `register_stream_template` | `owner: Address, start_delay: u64, cliff_delay: u64, duration: u64` | `Result<u64, ContractError>` | `owner` |
| `delete_stream_template` | `owner: Address, template_id: u64` | `Result<(), ContractError>` | `owner` |
| `get_stream_template` | `template_id: u64` | `Result<StreamScheduleTemplate, ContractError>` | None |

### 4.8 View / Query Functions

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `get_config` | — | `Result<Config, ContractError>` | None |
| `get_stream_state` | `stream_id: u64` | `Result<Stream, ContractError>` | None |
| `get_stream_memo` | `stream_id: u64` | `Result<Option<Bytes>, ContractError>` | None |
| `get_stream_count` | — | `u64` | None |
| `get_recipient_streams` | `recipient: Address` | `Vec<u64>` | None |
| `get_recipient_stream_count` | `recipient: Address` | `u64` | None |
| `get_streams_by_id_range` | `start_id: u64, end_id: u64, limit: u64` | `Vec<Stream>` | None |
| `get_recipient_streams_paginated` | `recipient: Address, cursor: u64, limit: u64` | `Vec<u64>` | None |
| `calculate_accrued` | `stream_id: u64` | `Result<i128, ContractError>` | None |
| `get_withdrawable` | `stream_id: u64` | `Result<i128, ContractError>` | None |
| `get_claimable_at` | `stream_id: u64, timestamp: u64` | `Result<i128, ContractError>` | None |
| `get_global_emergency_paused` | — | `bool` | None |
| `is_paused` | — | `bool` | None |
| `get_pause_info` | — | `PauseInfo` | None |
| `get_auto_claim_destination` | `stream_id: u64` | `Option<Address>` | None |

---

## 5. Frozen Enum Discriminants

The following enums are encoded on-chain. Their discriminant values are **frozen** for any deployed instance. Adding new variants is only allowed by appending at the end.

### StreamStatus

```
Active    = 0
Paused    = 1
Completed = 2
Cancelled = 3
```

### PauseReason

```
Operational    = 0
Emergency      = 1
Compliance     = 2
Administrative = 3
```

### ContractError

See Section 2.2 for the full table.

---

## 6. Event Emission Guarantees

These guarantees are part of the ABI. Changing them is a breaking change.

| Guarantee | Detail |
|---|---|
| `"created"` is emitted exactly once per stream | After tokens are transferred; never on validation failure |
| `"withdrew"` is only emitted when `amount > 0` | Zero-amount withdrawals produce no event |
| `"wdraw_to"` is only emitted when `amount > 0` | Same as above |
| `"completed"` is emitted on the same call as the final `"withdrew"` | Both appear in the same transaction |
| `"cancelled"` is emitted after `status = Cancelled` is persisted | State is durable before the event |
| `"closed"` is emitted before the storage entry is deleted | Indexers see the event before the entry disappears |
| `"pr_pause"` and `"pr_resume"` are NOT emitted on idempotent calls | Only emitted when state actually changes |
| `"paused"` carries a `PauseReason` field (v3+) | Indexers must handle the structured payload, not the bare `u64` |

---

## 7. Integrator Checklist

Before connecting to any Fluxora contract instance:

- [ ] Call `version()` and assert it equals the version your client was built against.
- [ ] Call `get_config()` to confirm the token address matches the expected asset.
- [ ] Confirm the `CONTRACT_ID` matches the announced deployment.
- [ ] Match on `ContractError` discriminant values (numeric), not variant name strings.
- [ ] Parse event payloads using the struct schemas in Section 2.3 and `docs/events.md`.
- [ ] Do not read `DataKey` storage entries directly unless you have verified the discriminant table matches the deployed version.
- [ ] Subscribe to events using the new `CONTRACT_ID` after any migration.

---

## 8. Factory Contract ABI

The `FluxoraFactory` contract is a thin policy wrapper around `FluxoraStream`. Its ABI is versioned independently.

### Factory Error Codes (`FactoryError`)

```
AlreadyInitialized       = 1
NotInitialized           = 2
Unauthorized             = 3
RecipientNotAllowlisted  = 4
DepositExceedsCap        = 5
DurationTooShort         = 6
```

### Factory Entrypoints

| Entrypoint | Parameters | Returns | Auth |
|---|---|---|---|
| `init` | `admin, stream_contract, max_deposit: i128, min_duration: u64` | `Result<(), FactoryError>` | None (first caller) |
| `set_admin` | `new_admin: Address` | `Result<(), FactoryError>` | `admin` |
| `set_stream_contract` | `new_stream_contract: Address` | `Result<(), FactoryError>` | `admin` |
| `set_allowlist` | `recipient: Address, allowed: bool` | `Result<(), FactoryError>` | `admin` |
| `set_cap` | `max_deposit: i128` | `Result<(), FactoryError>` | `admin` |
| `set_min_duration` | `min_duration: u64` | `Result<(), FactoryError>` | `admin` |
| `create_stream` | `sender, recipient, deposit_amount, rate_per_second, start_time, cliff_time, end_time` | `Result<u64, FactoryError>` | `sender` |

> **Important:** Factory policies (allowlist, deposit cap, minimum duration) are only enforced when streams are created through the factory. Direct calls to the `FluxoraStream` contract bypass all factory policies.

### Factory Storage Keys (`DataKey`)

| Discriminant | Variant | Storage class | Value type |
|---|---|---|---|
| 0 | `Admin` | Instance | `Address` |
| 1 | `StreamContract` | Instance | `Address` |
| 2 | `MaxDepositCap` | Instance | `i128` |
| 3 | `MinDuration` | Instance | `u64` |
| 4 | `Allowlist(Address)` | Persistent | `bool` |

---

## 9. WASM Reproducibility

The WASM artifact is deterministically reproducible given the pinned toolchain and dependencies. The reference checksum is stored in `wasm/checksums.sha256`. CI verifies every build against this reference.

Changing the WASM checksum without a `CONTRACT_VERSION` increment is a red flag: it means the binary changed but the version did not, which breaks the version-check guarantee.

See `contracts/stream/src/checksum.rs` for the full determinism contract.

---

## 10. Relationship to `docs/upgrade.md`

This document defines **what** is stable and **what** counts as breaking.
`docs/upgrade.md` defines **how** to handle a breaking change: when to increment `CONTRACT_VERSION`, how to deploy a new instance, and how to migrate streams.

Read both documents before making any change to the contract's external surface.
