# Storage Layout

Contract storage architecture, key design, TTL policies, and `DataKey` evolution rules for the Fluxora stream contract.

**Source of truth:** `contracts/stream/src/lib.rs` (`DataKey` enum, TTL constants, storage helpers)

---

## 1. DataKey Enum

All storage keys are defined in the `DataKey` enum:

```rust
#[contracttype]
pub enum DataKey {
    Config,                    // Instance storage for global settings (admin/token).
    NextStreamId,              // Instance storage for the auto-incrementing ID counter.
    Stream(u64),               // Persistent storage for individual stream data (O(1) lookup).
    RecipientStreams(Address), // Persistent storage for recipient stream index (sorted by stream_id).
    PauseState,                // Instance storage: protocol-wide pause state (enum).
    WithdrawNonce(Address),    // Persistent storage: per-recipient nonce for delegated-withdraw replay protection.
    ReentrancyLock,            // Instance storage: reentrancy guard flag (bool).
}
```

> **Append-only rule**: new variants are always appended at the end to avoid shifting
> existing discriminant values, which would corrupt live storage on mainnet.

## Storage Types and Usage
    Config,                    // discriminant 0 — instance
    NextStreamId,              // discriminant 1 — instance
    Stream(u64),               // discriminant 2 — persistent
    RecipientStreams(Address), // discriminant 3 — persistent
    GlobalEmergencyPaused,     // discriminant 4 — instance (DEPRECATED)
    CreationPaused,            // discriminant 5 — instance (DEPRECATED)
    GlobalPauseReason,         // discriminant 6 — instance
    GlobalPauseTimestamp,      // discriminant 7 — instance
    GlobalPauseAdmin,          // discriminant 8 — instance
    AutoClaimDestination(u64), // discriminant 9 — persistent
    StreamMemo(u64),           // discriminant 10 — persistent
    PauseState,                // discriminant 11 — instance
    ReentrancyLock,            // discriminant 12 — instance
}
```

### Current discriminant table

| Discriminant | Variant | Storage type | Value type | Set by | Mutated by |
|---|---|---|---|---|---|
| 0 | `Config` | Instance | `Config { token, admin }` | `init` (one-shot) | `set_admin` |
| 1 | `NextStreamId` | Instance | `u64` (monotonic counter) | `init` (→ 0) | `create_stream`, `create_streams` |
| 2 | `Stream(u64)` | Persistent | `Stream` struct | `create_stream`, `create_streams` | `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `withdraw_to`, `batch_withdraw`, `top_up_stream`, `update_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time` |
| 3 | `RecipientStreams(Address)` | Persistent | `Vec<u64>` (sorted) | `create_stream`, `create_streams` | `close_completed_stream` (removes entry) |
| 4 | `GlobalEmergencyPaused` | Instance | `bool` | `set_global_emergency_paused` | (DEPRECATED) |
| 5 | `CreationPaused` | Instance | `bool` | `set_contract_paused` | (DEPRECATED) |
| 6 | `GlobalPauseReason` | Instance | `String` | `pause_protocol` | `resume_protocol` (removes) |
| 7 | `GlobalPauseTimestamp` | Instance | `u64` | `pause_protocol` | `resume_protocol` (removes) |
| 8 | `GlobalPauseAdmin` | Instance | `Address` | `pause_protocol` | `resume_protocol` (removes) |
| 9 | `AutoClaimDestination(u64)` | Persistent | `Address` | auto-claim opt-in | auto-claim revoke |
| 10 | `StreamMemo(u64)` | Persistent | `Bytes` (max 64 bytes) | `create_stream`, `create_streams` | `close_completed_stream` (removes) |
| 11 | `PauseState` | Instance | `PauseState` enum | `set_global_emergency_paused`, `set_contract_paused`, `pause_protocol` | `resume_protocol` (Active) |
| 12 | `ReentrancyLock` | Instance | `bool` | `acquire_reentrancy_lock` | `release_reentrancy_lock` |

---

## 2. DataKey Evolution Policy

`DataKey` is a `#[contracttype]` enum. Soroban serialises enum variants by their **discriminant index** (0-based, declaration order). Changing the order of existing variants, or inserting a new variant anywhere other than the end, silently shifts all subsequent discriminants and makes every existing persistent storage entry unreadable on any live instance.

### Rules (must be followed on every PR that touches `DataKey`)

Persistent storage is used for individual stream records and per-recipient nonces:

| Key Pattern | Type | Description | Set By | Modified By |
|-------------|------|-------------|--------|-------------|
| `Stream(stream_id)` | `Stream` struct | Complete stream state including participants, amounts, timing, and status | `create_stream()` | `pause_stream()`, `resume_stream()`, `cancel_stream()`, `withdraw()` |
| `RecipientStreams(address)` | `Vec<u64>` | Sorted list of stream IDs for a recipient | `create_stream()` | `close_completed_stream()` |
| `WithdrawNonce(address)` | `u64` | Monotonically increasing nonce for delegated-withdraw replay protection | `delegated_withdraw()` (first call) | `delegated_withdraw()` (incremented on each successful withdrawal that moves tokens) |
1. **Never reorder** existing variants. The discriminant table above is immutable for the lifetime of any deployed instance.
2. **Never remove** a variant that has ever been written to a live network. Mark it `#[deprecated]` in a doc comment and stop writing to it; do not delete it.
3. **Always append** new variants at the end of the enum.
4. **Increment `CONTRACT_VERSION`** whenever a new variant is added or an existing variant's associated value type changes — both are breaking changes for off-chain tools that read storage directly.
5. **Document the ledger** at which each new variant is first deployed so that migration tooling can determine which entries exist on a given instance.

### What counts as a breaking storage change

| Change | Breaking? | Action |
|---|---|---|
| Reorder existing variants | Yes — corrupts all existing entries | Never do this |
| Insert variant in the middle | Yes — shifts discriminants | Never do this |
| Remove an existing variant | Yes — existing entries become orphaned | Deprecate instead |
| Change the value type of an existing variant | Yes — existing entries become undecodable | Increment `CONTRACT_VERSION` |
| Append a new variant at the end | No — existing entries unaffected | Increment `CONTRACT_VERSION` (conservative) |
| Change TTL constants | No — no effect on stored data | No version bump required |
| Change internal helper logic with identical external behaviour | No | No version bump required |

### Residual risks

- **No on-chain enforcement.** The rules above are enforced by code review and CI only. A developer who reorders variants will not get a compile error — the bug will only surface at runtime when existing entries are read back with the wrong type.
- **Off-chain indexers.** Any tool that reads Soroban storage entries directly (e.g., via RPC `getLedgerEntries`) must be updated whenever a new variant is added, even if it is append-only.
- **Discriminant stability across forks.** If a fork of this contract adds variants in a different order, its discriminant table will diverge. Always use the canonical table above as the reference.

---

## 3. Storage Types

### Instance storage

Used for contract-wide configuration and counters. Shared across all operations, low cardinality (3 keys), TTL extended on every entry-point call.

| Key | Description |
|---|---|
| `Config` | Token address and admin address. Immutable after `init` except for admin rotation via `set_admin`. |
| `NextStreamId` | Monotonically increasing stream ID counter. Never decremented. |
| `GlobalEmergencyPaused` | Emergency pause flag. `true` blocks all operational entrypoints. |
| `CreationPaused` | Soft creation pause flag. `true` blocks `create_stream` and `create_streams`. |

### Persistent storage

Used for per-stream data and per-recipient indexes. Grows linearly with stream count.

| Key | Description |
|---|---|
| `Stream(stream_id)` | Complete stream state: participants, amounts, timing, status, `cancelled_at`. One entry per stream. |
| `RecipientStreams(address)` | Sorted `Vec<u64>` of stream IDs where `address` is the recipient. Maintained in ascending order. |
| `AutoClaimDestination(stream_id)` | Recipient-chosen destination `Address` for permissionless auto-claim. Absent when not opted in. Removed by `revoke_auto_claim`. |

---

## 4. TTL Policy

### Constants

```rust
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;  // ~1 day at 5 s/ledger
const INSTANCE_BUMP_AMOUNT: u32       = 120_960;  // ~7 days
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 17_280;
const PERSISTENT_BUMP_AMOUNT: u32       = 120_960;
```

### Instance TTL

Extended via `bump_instance_ttl()` on **every** entry-point that touches instance storage. This means any contract interaction — read or write — keeps `Config`, `NextStreamId`, `GlobalEmergencyPaused`, and `CreationPaused` alive.

### Persistent TTL

Extended on every `load_stream()` (read) and `save_stream()` (write), and on every `load_recipient_streams()` / `save_recipient_streams()` call.

| Scenario | TTL refreshed? |
|---|---|
| Stream created | Yes (`save_stream` + `save_recipient_streams`) |
| Stream read via `get_stream_state` | Yes (`load_stream`) |
| Stream read via `calculate_accrued` | Yes (`load_stream`) |
| Stream mutated (pause/resume/cancel/withdraw) | Yes (`load_stream` + `save_stream`) |
| Stream closed via `close_completed_stream` | Entry removed (no TTL) |
| Recipient index read via `get_recipient_streams` | Yes (if non-empty) |

### TTL implications for operators

- **Active streams**: TTL refreshed on any interaction.
- **Cancelled streams**: Remain in persistent storage until the recipient withdraws the frozen accrued amount. `close_completed_stream` is blocked while any claimable balance remains. Operators must ensure recipients are notified to withdraw before TTL expiry.
- **Inactive streams**: May expire after ~7 days with zero interaction. Operators must ensure recipients are notified before TTL expiry.
- **Expired entries**: Cannot be recovered. Data is permanently lost.
- **Contract liveness**: Instance storage stays alive as long as any function is called at least once per 7 days.

---

## 5. Storage Access Patterns

### Read-only (view functions)

| Function | Keys read | TTL bumped |
|---|---|---|
| `get_config` | `Config` | Instance |
| `get_stream_count` | `NextStreamId` | Instance |
| `get_stream_state` | `Stream(id)` | Persistent |
| `calculate_accrued` | `Stream(id)` | Persistent |
| `get_withdrawable` | `Stream(id)` | Persistent |
| `get_claimable_at` | `Stream(id)` | Persistent |
| `get_recipient_streams` | `RecipientStreams(addr)` | Persistent (if non-empty) |
| `get_recipient_stream_count` | `RecipientStreams(addr)` | Persistent (if non-empty) |
| `version` | None | Instance (via `bump_instance_ttl`) |

### State-mutating

| Function | Keys written | Notes |
|---|---|---|
| `init` | `Config`, `NextStreamId` | One-shot; fails if `Config` already exists |
| `create_stream` | `NextStreamId`, `Stream(id)`, `RecipientStreams(addr)` | Atomic |
| `create_streams` | `NextStreamId`, `Stream(id)×N`, `RecipientStreams(addr)×N` | Atomic batch |
| `pause_stream` / `resume_stream` | `Stream(id)` | Status field only |
| `cancel_stream` | `Stream(id)` | Sets `status=Cancelled`, `cancelled_at` |
| `withdraw` / `withdraw_to` | `Stream(id)` | Updates `withdrawn_amount`; may set `status=Completed` |
| `top_up_stream` | `Stream(id)` | Updates `deposit_amount` |
| `update_rate_per_second` | `Stream(id)` | Updates `rate_per_second` |
| `shorten_stream_end_time` | `Stream(id)` | Updates `end_time`, `deposit_amount` |
| `extend_stream_end_time` | `Stream(id)` | Updates `end_time` |
| `close_completed_stream` | Removes `Stream(id)`, updates `RecipientStreams(addr)` | Permissionless cleanup |
| `set_admin` | `Config` | Admin key rotation |
| `set_global_emergency_paused` | `GlobalEmergencyPaused` | Global emergency pause flag |
| `set_contract_paused` | `CreationPaused` | Soft creation pause flag |

---

## 6. Security Notes

- **Atomic operations**: All state changes are transactional. No partial updates are possible.
- **Key isolation**: Each stream has independent storage. No cross-stream interference.
- **CEI ordering**: State is always persisted (`save_stream`) before any external token transfer. See `docs/security.md`.
- **No stale reads**: TTL bumps on reads mean monitoring queries keep data fresh.
- **Admin rotation**: `set_admin` writes a new `Config` with the updated admin address. The token address is immutable.

---

## 7. Version History

For a full description of what changed between contract versions and how to migrate, see [DEPLOYMENT.md — Version Migration](./DEPLOYMENT.md#version-migration).

---

## 8. V5 Storage Layout (historical reference)

This section documents the storage layout as it existed in **CONTRACT_VERSION = 5**, before the V6 additions. It is the authoritative reference for:

- Regression tests that seed V5-era ledger state and verify V6 read paths.
- Off-chain indexers that may encounter V5-encoded entries on instances that have not been migrated.
- Auditors verifying that no discriminant was shifted between V5 and V6.

### V5 DataKey discriminant table (frozen — must never change)

| Discriminant | Variant                     | Storage    | Value type                   |
|-------------:|:----------------------------|:-----------|:-----------------------------|
|            0 | `Config`                    | Instance   | `Config { token, admin }`    |
|            1 | `NextStreamId`              | Instance   | `u64`                        |
|            2 | `Stream(u64)`               | Persistent | `Stream` (V5, 14 fields)     |
|            3 | `RecipientStreams(Address)`  | Persistent | `Vec<u64>` (sorted)          |
|            4 | `GlobalEmergencyPaused`     | Instance   | `bool`                       |
|            5 | `CreationPaused`            | Instance   | `bool`                       |
|            6 | `GlobalPauseReason`         | Instance   | `String`                     |
|            7 | `GlobalPauseTimestamp`      | Instance   | `u64`                        |
|            8 | `GlobalPauseAdmin`          | Instance   | `Address`                    |
|            9 | `AutoClaimDestination(u64)` | Persistent | `Address`                    |
|           10 | `NextTemplateId`            | Instance   | `u64`                        |
|           11 | `ActiveTemplateCount`       | Instance   | `u64`                        |
|           12 | `StreamTemplate(u64)`       | Persistent | `StreamScheduleTemplate`     |
|           13 | `OwnerTemplateIds(Address)` | Persistent | `Vec<u64>`                   |
|           14 | `TotalLiabilities`          | Instance   | `i128`                       |

Discriminants 0–14 are **permanently frozen**. No variant at these positions may ever be reordered, renamed, or removed on any instance that has processed at least one transaction.

### V5 Stream struct (14 fields, positional XDR encoding)

| Position | Field                     | Type           | Notes                                      |
|---------:|:--------------------------|:---------------|:-------------------------------------------|
|        0 | `stream_id`               | `u64`          | Monotonically increasing, set at creation  |
|        1 | `sender`                  | `Address`      | Stream creator and controller              |
|        2 | `recipient`               | `Address`      | Token beneficiary                          |
|        3 | `deposit_amount`          | `i128`         | Total escrowed tokens                      |
|        4 | `rate_per_second`         | `i128`         | Streaming speed in raw token units/second  |
|        5 | `start_time`              | `u64`          | Ledger timestamp when accrual begins       |
|        6 | `cliff_time`              | `u64`          | Ledger timestamp when withdrawals unlock   |
|        7 | `end_time`                | `u64`          | Ledger timestamp when accrual stops        |
|        8 | `withdrawn_amount`        | `i128`         | Cumulative tokens already withdrawn        |
|        9 | `status`                  | `StreamStatus` | `Active`, `Paused`, `Completed`, `Cancelled` |
|       10 | `cancelled_at`            | `Option<u64>`  | Set when status transitions to `Cancelled` |
|       11 | `checkpointed_amount`     | `i128`         | Accrued tokens locked at last rate change  |
|       12 | `checkpointed_at`         | `u64`          | Timestamp of last rate change              |
|       13 | `withdraw_dust_threshold` | `i128`         | Minimum withdrawal amount (0 = no filter)  |

**No `memo` field in V5.** The V5 `Stream` struct has exactly 14 fields.

### V5 → V6 transition

V6 appended six new `DataKey` variants (discriminants 15–20) and one new `Stream` field:

| Discriminant | Variant                              | Storage    | Value type  | Notes                                    |
|-------------:|:-------------------------------------|:-----------|:------------|:-----------------------------------------|
|           15 | `WithdrawNonce(Address)`             | Persistent | `u64`       | Per-recipient nonce; absent until first delegated-withdraw |
|           16 | `PauseState`                         | Instance   | `PauseState`| Unified pause state enum                 |
|           17 | `ReentrancyLock`                     | Instance   | `bool`      | Reentrancy guard; absent when not held   |
|           18 | `RecipientStreamPage(Address, u32)`  | Persistent | `Vec<u64>`  | Paged recipient index (page → IDs)       |
|           19 | `RecipientStreamPageCount(Address)`  | Persistent | `u32`       | Number of pages in recipient's index     |
|           20 | `PendingRecipientUpdate(u64)`        | Persistent | `Address`   | Pending recipient rotation proposal      |

V6 `Stream` struct adds one field at the end:

| Position | Field  | Type           | Notes                                                    |
|---------:|:-------|:---------------|:---------------------------------------------------------|
|       14 | `memo` | `Option<Bytes>`| Optional indexer correlation memo (max 64 bytes); `None` in V5 entries |

### Forward-compatibility guarantee

All V5 persistent `Stream` entries remain decodable on a V6 instance. Soroban XDR struct decoding is **positional and forward-compatible**: a V6 decoder reading a V5-encoded struct decodes the first 14 fields correctly and treats the absent 15th field as `None` (for `Option<Bytes>`).

This guarantee holds **only** because:
1. `memo` is `Option`-typed — an absent field decodes as `None`, not a type error.
2. `memo` is appended as the last field — no positional shift occurs for fields 0–13.

A non-`Option` append or a mid-struct insertion would break V5 entries silently.

### Regression test coverage

The file `contracts/stream/tests/storage_key_compat.rs` encodes these invariants as executable tests:

| Test | What it guards |
|:-----|:---------------|
| `v5_stream_readable_by_v6_get_stream_state` | Discriminant 2 stability; `memo == None` on V5 entries |
| `v5_stream_calculate_accrued_correct` | Accrual math on V5 entries |
| `v5_stream_get_withdrawable_correct` | Withdrawable calculation on V5 entries |
| `v5_stream_get_claimable_at_correct` | Claimable-at simulation on V5 entries |
| `v5_multiple_streams_all_readable` | `Stream(u64)` key encoding for multiple IDs |
| `v5_cancelled_stream_readable_accrual_frozen` | `cancelled_at` field decoding; frozen accrual |
| `v5_stream_with_checkpoint_readable` | `checkpointed_amount` field decoding |
| `v5_config_key_readable_by_v6` | Discriminant 0 stability |
| `v5_next_stream_id_readable_by_v6` | Discriminant 1 stability |
| `v5_global_emergency_paused_readable_by_v6` | Discriminant 4 stability |
| `v5_creation_paused_readable_by_v6` | Discriminant 5 stability |
| `v5_total_liabilities_readable_by_v6` | Discriminant 14 stability (last frozen key) |
| `v5_recipient_streams_readable_by_v6` | Discriminant 3 stability |
| `v5_recipient_stream_count_correct` | RecipientStreams count on V5 index |
| `v5_absent_recipient_streams_returns_empty` | No panic on absent V5 index |
| `v6_withdraw_nonce_absent_on_v5_instance` | Discriminant 15 absent on V5 |
| `v6_pause_state_absent_on_v5_instance` | Discriminant 16 absent on V5 |
| `v6_reentrancy_lock_absent_on_v5_instance` | Discriminant 17 absent on V5 |
| `v6_recipient_stream_page_absent_on_v5_instance` | Discriminant 18 absent on V5 |
| `v6_recipient_stream_page_count_absent_on_v5_instance` | Discriminant 19 absent on V5 |
| `v6_pending_recipient_update_absent_on_v5_instance` | Discriminant 20 absent on V5 |
| `discriminant_0_config_round_trips` | Config key round-trip |
| `discriminant_1_next_stream_id_round_trips` | NextStreamId key round-trip |
| `discriminant_2_stream_round_trips` | Stream key round-trip |
| `discriminant_3_recipient_streams_round_trips` | RecipientStreams key round-trip |
| `discriminant_14_total_liabilities_round_trips` | TotalLiabilities key round-trip |
| `version_entry_point_works_on_v5_seeded_instance` | `version()` callable on V5 state |
