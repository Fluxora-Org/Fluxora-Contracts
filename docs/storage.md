# Storage Layout

Contract storage architecture, key design, TTL policies, and `DataKey` evolution rules for the Fluxora stream contract.

**Source of truth:** `contracts/stream/src/lib.rs` (`DataKey` enum, TTL constants, storage helpers)

---

## 1. DataKey Enum

All storage keys are defined in the `DataKey` enum:

```rust
#[contracttype]
pub enum DataKey {
    Config,                    // discriminant 0 — instance
    NextStreamId,              // discriminant 1 — instance
    Stream(u64),               // discriminant 2 — persistent
    RecipientStreams(Address), // discriminant 3 — persistent
    GlobalPaused,              // discriminant 4 — instance
}
```

### Current discriminant table

| Discriminant | Variant | Storage type | Value type | Set by | Mutated by |
|---|---|---|---|---|---|
| 0 | `Config` | Instance | `Config { token, admin }` | `init` (one-shot) | `set_admin` |
| 1 | `NextStreamId` | Instance | `u64` (monotonic counter) | `init` (→ 0) | `create_stream`, `create_streams` |
| 2 | `Stream(u64)` | Persistent | `Stream` struct | `create_stream`, `create_streams` | `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `withdraw_to`, `batch_withdraw`, `top_up_stream`, `update_rate_per_second`, `shorten_stream_end_time`, `extend_stream_end_time` |
| 3 | `RecipientStreams(Address)` | Persistent | `Vec<u64>` (sorted) | `create_stream`, `create_streams` | `close_completed_stream` (removes entry) |
| 4 | `GlobalPaused` | Instance | `bool` | `set_contract_paused` | `set_contract_paused` |

---

## 2. DataKey Evolution Policy

`DataKey` is a `#[contracttype]` enum. Soroban serialises enum variants by their **discriminant index** (0-based, declaration order). Changing the order of existing variants, or inserting a new variant anywhere other than the end, silently shifts all subsequent discriminants and makes every existing persistent storage entry unreadable on any live instance.

### Rules (must be followed on every PR that touches `DataKey`)

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
| `GlobalPaused` | Emergency pause flag. `true` blocks `create_stream` and `create_streams`. |

### Persistent storage

Used for per-stream data and per-recipient indexes. Grows linearly with stream count.

| Key | Description |
|---|---|
| `Stream(stream_id)` | Complete stream state: participants, amounts, timing, status, `cancelled_at`. One entry per stream. |
| `RecipientStreams(address)` | Sorted `Vec<u64>` of stream IDs where `address` is the recipient. Maintained in ascending order. |

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

Extended via `bump_instance_ttl()` on **every** entry-point that touches instance storage. This means any contract interaction — read or write — keeps `Config`, `NextStreamId`, and `GlobalPaused` alive.

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
| `set_contract_paused` | `GlobalPaused` | Emergency pause flag |

---

## 6. Security Notes

- **Atomic operations**: All state changes are transactional. No partial updates are possible.
- **Key isolation**: Each stream has independent storage. No cross-stream interference.
- **CEI ordering**: State is always persisted (`save_stream`) before any external token transfer. See `docs/security.md`.
- **No stale reads**: TTL bumps on reads mean monitoring queries keep data fresh.
- **Admin rotation**: `set_admin` writes a new `Config` with the updated admin address. The token address is immutable.
