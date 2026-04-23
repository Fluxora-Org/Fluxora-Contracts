# Fluxora Contract Upgrade Strategy

Version policy, migration runbook, and audit notes for operators, integrators, and auditors.

**Source of truth:** `contracts/stream/src/lib.rs` (`CONTRACT_VERSION` constant, `version()` entry-point)

---

## 1. CONTRACT_VERSION Policy

### What it is

`CONTRACT_VERSION` is a compile-time `u32` constant embedded in the WASM binary. It is returned by the permissionless `version()` entry-point with no storage access. Integrators call it to confirm which protocol revision is running before sending state-mutating transactions.

### Current value

```
CONTRACT_VERSION = 3
```

### When to increment

| Change type | Increment required? |
|---|---|
| Remove or rename a public entry-point | Yes |
| Change parameter type or order on any entry-point | Yes |
| Change a `ContractError` discriminant value | Yes |
| Change emitted event topic or payload shape | Yes |
| Change persistent storage key layout (breaks existing entries) | Yes |
| Add a new entry-point (purely additive) | Recommended (conservative) |
| Internal refactor — identical external behaviour | No |
| Documentation-only change | No |
| Gas optimisation — identical observable behaviour | No |
| Tighten validation (reject a previously-accepted edge case) | Document it; increment if integrators depend on the old behaviour |

### What counts as breaking

- Any change that causes a correctly-written v1 client to fail or misinterpret a response when talking to the new contract.
- Storage layout changes that make existing `Stream`, `Config`, or `RecipientStreams` entries unreadable after upgrade.
- Event shape changes that break indexers parsing `StreamCreated`, `Withdrawal`, `StreamEvent`, etc.

### What does NOT require an increment

- Adding new entry-points that old clients can safely ignore.
- Changing TTL bump constants (`INSTANCE_BUMP_AMOUNT`, `PERSISTENT_BUMP_AMOUNT`).
- Changing internal helper functions with no external surface.

> **Note (pause reason codes — v3):** The `"paused"` event data changed from
> `StreamEvent::Paused(stream_id)` to `StreamPaused { stream_id, reason: PauseReason }`.
> This is a **breaking event-shape change**. `CONTRACT_VERSION` was bumped to `3`.
> Indexers must update their pause event parsers. The `pause_stream` and
> `pause_stream_as_admin` entry-points now require a `reason: PauseReason` parameter.
> Callers must pass one of: `Operational`, `Emergency`, `Compliance`, or `Administrative`.

---

## 2. version() Entry-Point Semantics

### Success semantics

- Returns `CONTRACT_VERSION` as a `u32`.
- No storage read, no token interaction, no auth check.
- Works before `init` is called (pre-flight deployment check).
- Idempotent: repeated calls always return the same value for a given deployment.

### Failure semantics

- Cannot fail. There are no error paths in `version()`.

### Authorization

- None. Any caller (wallet, indexer, script, another contract) may call `version()`.

### Gas

- Minimal. No storage access, no external calls.

---

## 3. Migration Runbook

Soroban contracts are **not upgradeable in-place** by default. A new `CONTRACT_VERSION` means deploying a new contract instance.

### Step-by-step

1. **Increment `CONTRACT_VERSION`** in `contracts/stream/src/lib.rs` before merging the breaking change.

2. **Build and deploy** the new WASM:
   ```bash
   cargo build --release -p fluxora_stream --target wasm32-unknown-unknown
   stellar contract deploy --wasm target/wasm32-unknown-unknown/release/fluxora_stream.wasm \
     --network mainnet --source $DEPLOYER_KEY
   ```

3. **Initialise** the new instance:
   ```bash
   stellar contract invoke --id $NEW_CONTRACT_ID -- init \
     --token $TOKEN_ADDRESS --admin $ADMIN_ADDRESS
   ```

4. **Verify version** before announcing migration:
   ```bash
   stellar contract invoke --id $NEW_CONTRACT_ID -- version
   # Must return the new CONTRACT_VERSION value
   ```

5. **Announce migration** with sufficient lead time (recommended: ≥ 14 days for mainnet) so that:
   - Recipients can withdraw accrued funds from the old instance.
   - Senders can cancel and recreate streams on the new instance if desired.
   - Indexers and wallets can update their `CONTRACT_ID` references.

6. **Update all integrations** to point at the new `CONTRACT_ID`. Integrations should assert:
   ```text
   assert version() == EXPECTED_VERSION
   ```
   before sending any state-mutating transaction.

7. **Do not destroy the old instance** until all active streams have been settled (withdrawn or cancelled). Persistent storage entries on the old instance remain readable as long as the instance exists and its TTL has not expired.

### Stream migration

There is no on-chain migration path for stream state between contract versions. Options:

| Stream status | Recommended action |
|---|---|
| Active | Let it run to completion on the old instance, or sender cancels and recreates on new instance |
| Paused | Sender resumes, then withdraws or cancels on old instance |
| Cancelled | Recipient withdraws frozen accrued amount on old instance |
| Completed | Recipient withdraws remaining amount on old instance; optionally close via `close_completed_stream` |

---

## 4. Integrator Checklist

Before interacting with any Fluxora contract instance:

- [ ] Call `version()` and assert it equals the version your client was built against.
- [ ] Call `get_config()` to confirm the token address matches the expected asset.
- [ ] Confirm the `CONTRACT_ID` matches the announced deployment.
- [ ] Subscribe to `StreamCreated` events using the new `CONTRACT_ID` (not the old one).

---

## 5. Residual Risks and Audit Notes

1. **No on-chain enforcement of increment discipline.** If a developer deploys a breaking change without incrementing `CONTRACT_VERSION`, integrators will not detect the incompatibility until a runtime failure occurs. Mitigation: CI check that fails if `CONTRACT_VERSION` is unchanged on a PR that modifies public entry-points, event types, or error codes.

2. **TTL expiry.** Persistent stream entries have a finite TTL. If an old contract instance is abandoned without being bumped, stream entries may expire before recipients withdraw. Operators must ensure recipients are notified well before TTL expiry.

3. **No upgrade path for in-flight streams.** Streams created on v1 cannot be migrated to v2 on-chain. This is a deliberate design choice (simplicity, auditability) but means migration windows must be long enough for all streams to settle.

4. **Admin key continuity.** The admin address is set at `init` time and is immutable via `init`. Use `set_admin` to rotate the admin key before migrating to a new instance, and call `init` on the new instance with the new admin address.

5. **Token address immutability.** The token is fixed at `init` time. A new contract version that needs a different token requires a new `init` call with the new token address — existing streams on the old instance are unaffected.

---

## 6. Paginated Export Views (Issue #429)

Bounded, paginated view entrypoints support off-chain export and migration between contract instances without unbounded loops or memory usage.

### Motivation

Operators need to export stream data for:
- Migration between contract versions (no on-chain upgrade path exists)
- Off-chain analytics and reporting
- Backup and audit trails
- Integration with external systems

Without pagination, `get_recipient_streams` returns **all** streams unbounded, which can exhaust memory or hit gas limits with large portfolios.

### Entrypoints

#### `get_streams_by_id_range(start_id, end_id, limit) -> Vec<Stream>`

Returns streams within an ID range `[start_id, end_id]` with a strict result limit.

**Parameters:**
- `start_id: u64` — First stream ID to include (inclusive)
- `end_id: u64` — Last stream ID to include (inclusive). Use `u64::MAX` for open-ended.
- `limit: u64` — Maximum streams to return (capped at `MAX_PAGE_SIZE = 100`)

**Returns:**
- `Vec<Stream>` — Stream structs in ascending ID order
- Empty vector if `start_id > end_id` or no streams exist in range
- Closed/archived stream IDs are silently skipped

**DoS Protection:**
- `limit` is capped at `MAX_PAGE_SIZE` (100) regardless of input
- Gas cost is O(min(limit, actual_results)), not O(range_size)
- Each stream lookup is O(1)

**Migration Pattern:**
```rust
let total = client.get_stream_count();
let mut start = 0u64;
while start < total {
    let page = client.get_streams_by_id_range(&start, &(start + 99), &100);
    // Export page...
    start += 100;
}
```

#### `get_recipient_streams_paginated(recipient, cursor, limit) -> Vec<u64>`

Cursor-based pagination for recipient stream export.

**Parameters:**
- `recipient: Address` — Address to query
- `cursor: u64` — 0-based starting index in the recipient's stream list
- `limit: u64` — Maximum streams to return (capped at `MAX_PAGE_SIZE = 100`)

**Returns:**
- `Vec<u64>` — Stream IDs in ascending order
- Empty vector if `cursor >= recipient_stream_count`

**Cursor Semantics:**
- Cursor is a 0-based index into the sorted recipient stream list
- After each call: `next_cursor = cursor + result.len()`
- When `result.len() < limit`, you've reached the end
- List mutations (insertions/removals) shift indices naturally

**DoS Protection:**
- `limit` is capped at `MAX_PAGE_SIZE` (100)
- Only loads the requested page, not the entire recipient list
- Gas cost is O(limit), not O(total_recipient_streams)

**Full Export Pattern:**
```rust
let mut cursor = 0u64;
loop {
    let page = client.get_recipient_streams_paginated(&recipient, &cursor, &50);
    if page.is_empty() { break; }
    // Export page...
    cursor += page.len() as u64;
}
```

### Safety Limits

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_PAGE_SIZE` | 100 | Maximum results per paginated query |

These limits prevent:
- Memory exhaustion from unbounded vector returns
- Gas limit violations from excessive storage reads
- DoS via intentionally large limit parameters

### Comparison: Old vs New

| Scenario | Old Approach | New Approach |
|----------|--------------|--------------|
| Export 1000 streams | `get_recipient_streams` → unbounded, may fail | `get_streams_by_id_range` with pagination → reliable |
| Large portfolio query | Risk of gas/memory exhaustion | Bounded pages, predictable gas |
| Migration script | Complex retry logic | Simple cursor iteration |

### Testing Requirements

All paginated views are tested for:
- ✅ Basic pagination (correct items, order)
- ✅ Empty ranges/cursors return empty
- ✅ `MAX_PAGE_SIZE` enforcement (requests > 100 capped)
- ✅ Closed stream handling (gracefully skipped)
- ✅ Open-ended ranges (`u64::MAX`)
- ✅ Zero limit returns empty
- ✅ Cursor beyond end returns empty
- ✅ Multiple recipient isolation
- ✅ Full export workflow (accumulate all pages)

See `contracts/stream/src/test.rs` for the complete test suite.
