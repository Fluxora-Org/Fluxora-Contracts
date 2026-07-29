# Manifest Versioning and Upgrade Compatibility

## Overview

The Fluxora stream contract implements a **versioned storage model** that ensures backward compatibility, deterministic behavior, and safe upgrades. This document formalizes the versioning semantics, edge cases, and upgrade guarantees.

## Current Version

**CONTRACT_VERSION = 9**

The version is:
- Embedded in the contract WASM binary at compile time
- Returned by the permissionless `version()` entrypoint
- Used by integrators, deployment scripts, and indexers to detect protocol revisions

## Versioning Policy

### When to Increment CONTRACT_VERSION

| Change type | Action required |
|---|---|
| **Breaking ABI change** | Increment |
| **New additive entry-point** | Increment (conservative) |
| **Internal refactor** | No increment |
| **Documentation-only** | No increment |

### What Counts as Breaking

- Removing or renaming a public function
- Changing the type or order of any function parameter
- Changing a `ContractError` discriminant value
- Changing the shape of an emitted event payload
- Changing storage key layout (makes existing entries unreadable)

### What Does NOT Require Increment

- Adding a new public function (purely additive)
- Tightening validation (reject previously-accepted edge case)
- Gas optimizations with identical observable behavior
- Changing TTL bump constants

## Version History

| Version | Change | Scope |
|---|---|---|
| 1 | Initial release | Baseline |
| 2 | `Stream` gained `checkpointed_amount` and `checkpointed_at` | Rate decrease safety |
| 3 | `delegated_withdraw` signature now commits to `expected_minimum_amount` | Front-running protection |
| 4 | Accrual paths track last ledger timestamp in instance storage | Clock regression detection |
| 5 | `DataKey::PausedStreamCount` added and maintained; `get_paused_stream_count()` O(1) | Pause tracking |
| 7 | Auto-renewal entrypoints and `DataKey::AutoRenewEnabled` opt-in | Auto-renewal feature |
| 8 | Lookback-bounded creation and claim calculation | Lookback window support |
| 9 | `delegated_withdraw` accepts optional `relayer_fee`; `Withdrawal` event now publishes `net_amount` | Relayer fee support |

## Storage Model

### Append-Only Storage Keys

The `DataKey` enum follows a **frozen discriminant policy**:

```rust
pub enum DataKey {
    // Discriminants 0-35 are frozen and must never be reordered
    Config,                           // 0
    NextStreamId,                     // 1
    Stream(u64),                      // 2
    RecipientStreams(Address),        // 3
    GlobalEmergencyPaused,            // 4
    CreationPaused,                   // 5
    GlobalPauseReason,                // 6
    GlobalPauseTimestamp,             // 7
    GlobalPauseAdmin,                 // 8
    AutoClaimDestination(u64),        // 9
    NextTemplateId,                   // 10
    ActiveTemplateCount,              // 11
    StreamTemplate(u64),              // 12
    OwnerTemplateIds(Address),        // 13
    TotalLiabilities,                 // 14
    WithdrawNonce(Address),           // 15 (deprecated alias for DelegatedWithdrawNonce)
    PauseState,                       // 16
    ReentrancyLock,                   // 17
    RecipientStreamPage(Address, u32), // 18
    RecipientStreamPageCount(Address), // 19
    PendingRecipientUpdate(u64),      // 20
    IdReservation(Address),           // 21
    MaxRatePerSecond,                 // 22
    DelegatedWithdrawNonce(Address),  // 23
    LastPauseRecord(PauseKind),       // 24
    RotationHistory(u64),             // 25
    LastAccrualLedgerTimestamp,       // 26
    PausedStreamCount,                // 27
    TotalKeeperFeesPaid,              // 28
    AutoRenewEnabled(u64),            // 29
    MaxLookbackLedgers(u64),          // 30
    SenderStreams(Address),           // 31
    PendingStreamOffer(u64),          // 32
    RecipientPendingOffers(Address),  // 33
    PooledStreamShares(u64),          // 34
    PooledStreamWithdrawn(u64, Address), // 35
    // NEW variants MUST be appended after discriminant 35
}
```

**Invariant**: Discriminants 0-35 are frozen and must never be reordered. New `DataKey` variants must be appended after discriminant 35 with strictly increasing values.

### Storage Layers

#### Instance Storage (Contract-Wide State)

Bumped on **every** entry-point call:
- `Config` — admin and token address
- `NextStreamId` — auto-incrementing stream ID counter
- `ReentrancyLock` — reentrancy guard flag (bool)
- `LastAccrualLedgerTimestamp` — ledger timestamp for clock regression detection
- Version-specific keys (pause state, template counts, etc.)

**TTL Policy**:
- Threshold: 17,280 ledgers (~1 day at 5 sec/ledger)
- Bump: 120,960 ledgers (~7 days)

#### Persistent Storage (Stream and Index State)

Bumped on load/save and index operations:
- `Stream(u64)` — individual stream data (O(1) lookup)
- `RecipientStreams(Address)` — sorted stream index per recipient
- `SenderStreams(Address)` — sorted stream index per sender
- Version-specific keys (pause records, rotation history, etc.)

**TTL Policy**:
- Adaptive TTL: `min(MAX_TTL, remaining_seconds / LEDGER_CLOSE_TIME + BUFFER_LEDGERS)`
- Scales to the stream's remaining lifetime to keep active entries alive

## Upgrade Mechanism

### Non-Upgradeable Design

Soroban contracts are **not upgradeable in-place** by default. A new version requires:

1. **Deploy New Instance**
   - Create a new contract instance (new `CONTRACT_ID`)
   - Verify `version()` returns the expected value

2. **Initialize**
   - Call `init(token, admin)` on the new instance
   - Use the same token and admin as the old instance

3. **Migrate Streams**
   - **Manual off-chain migration** (no on-chain path)
   - Option A: Let old streams complete, then recreate on new instance
   - Option B: Cancel old streams, withdraw accrued funds, recreate on new instance
   - All stream state is local to the contract instance that created it

4. **Update Integrations**
   - Wallets: point at new `CONTRACT_ID`
   - Indexers: resync from new instance's event stream
   - Treasury tooling: update address and verify `version()`

### Backward Compatibility Guarantees

The contract maintains the following guarantees across upgrades:

- **Event ABI Stability**: Event discriminants and field order are frozen (see `ContractError` discriminant 2.2 table)
- **Error Code Stability**: Error discriminants are frozen and documented in `docs/ABI_STABILITY.md`
- **Storage Key Stability**: `DataKey` discriminants are frozen (append-only pattern)
- **Function Signature Stability**: Public function signatures are append-only (old functions never change)
- **Accrual Determinism**: Accrued amount is timestamp-deterministic (given stream params and current time, result is always the same)

## Edge Cases and Stabilization

### Edge Case 1: Clock Regression on Upgrade

**Scenario**: After upgrading, the test harness or migration tool sets the ledger timestamp to a value earlier than a previous invocation.

**Behavior**: The contract stores `LastAccrualLedgerTimestamp` in instance storage and checks it on every accrual path.

**Safety**: `current_accrual_timestamp()` returns `Err(ContractError::ClockRegression)` if `current_ts < prev_ts`.

**Implications for Retries**:
- Idempotent: calling `current_accrual_timestamp()` multiple times with the same wall clock returns the same value and succeeds.
- Deterministic: if invocation A sets `LastAccrualLedgerTimestamp = 1000` and invocation B retries at `1001`, the check passes and proceeds.
- Safe for batch operations: all streams in a single transaction use the same `current_time` value (read once, reused).

### Edge Case 2: Rate Decrease Across Upgrade

**Scenario**: Stream V1 has rate 100 tokens/sec and checkpoint state `(checkpointed_amount=1000, checkpointed_at=100)`. After upgrade to V9, `decrease_rate_per_second` is called.

**Behavior**: Checkpoint state is preserved exactly as-is. New rate is applied only from the current time forward.

**Accrual Formula**:
```
accrued(now) = checkpointed_amount + (new_rate * (min(now, end_time) - checkpointed_at))
             = 1000 + (new_rate * (now - 100))
```

**Implications**:
- Recipient never loses previously accrued entitlements (checkpoint preservation invariant)
- Safe across upgrades: checkpoint fields are persisted in the `Stream` struct (V2+)
- Deterministic: result depends only on stream params and current time, not on upgrade sequence

### Edge Case 3: Paused Stream Through Upgrade

**Scenario**: Stream is in `Paused` status before upgrade. After upgrade, `resume_stream` is called.

**Behavior**: Pause/resume cooldown applies (`MIN_PAUSE_INTERVAL_LEDGERS = 17` ledgers). Accrual continues normally while paused.

**Implications for Upgrades**:
- Status field is persisted; pause state is visible across contract instances (doesn't transfer, but old instance frozen)
- Cooldown is ledger-sequence-based (not timestamp-based), so upgraded instance starts with fresh ledger sequence
- Deterministic: resuming at ledger N always succeeds if last pause/resume was at ledger ≤ N-17

### Edge Case 4: Batch Operations with Duplicate Ids During Retry

**Scenario**: `batch_withdraw([1, 2, 1])` is called; duplicate detected and rejected. Retried with same parameters.

**Behavior**: `reject_duplicate_ids()` scans the input and returns `Err(DuplicateStreamId)` if any duplicates are found.

**Implications**:
- Idempotent: retry with identical input produces identical output (error or success)
- Deterministic: no state is mutated if duplicates are present
- Safe for retries: operation is all-or-nothing (CEI ensures state is consistent)

### Edge Case 5: Metadata Validation Across Upgrade

**Scenario**: Stream created with `metadata: Some(map)` in V8. After upgrade to V9, `clone_stream` reads the metadata.

**Behavior**: Metadata is stored as-is in the `Stream` struct. Validation happens only at creation time.

**Implications**:
- Metadata bytes are capped at `MAX_METADATA_BYTES = 4_096`
- Persisted metadata is not re-validated on read (immutable after creation)
- Deterministic: metadata shape never changes across upgrades

### Edge Case 6: Keeper Cancel After Stream Completes

**Scenario**: Stream reaches `end_time` and recipient completes a full withdrawal. `keeper_cancel` is then called.

**Behavior**: Stream status is `Completed` (terminal state). `keeper_cancel` checks `is_terminal_state()` and rejects if terminal.

**Implications**:
- Terminal state is idempotent: calling `keeper_cancel` on a completed stream always fails with same error
- Safe for retries: retry produces identical failure
- Deterministic: completed status doesn't change

### Edge Case 7: Global Emergency Pause Across Upgrade

**Scenario**: Global emergency pause is active (`GlobalEmergencyPaused = true`). New instance is deployed and initialized.

**Behavior**: Pause state is **instance-specific**. New instance does NOT inherit pause state from old instance.

**Implications**:
- Pause state is NOT migrated (each instance independent)
- New instance starts with `GlobalEmergencyPaused = false` (default)
- Admin must re-activate pause on new instance if needed
- Deterministic: pause state is predictable from instance initialization

### Edge Case 8: AutoRenew Opt-In Missing on Pre-V7 Streams

**Scenario**: Stream created in V6 (before `AutoRenewEnabled` was added). After upgrade to V9, `set_auto_renew` is called.

**Behavior**: Key `DataKey::AutoRenewEnabled(stream_id)` is created on first write. Read returns `false` (default) if key doesn't exist.

**Implications**:
- Backward compatible: old streams default to non-auto-renewing
- Deterministic: calling `get_auto_renew(stream_id)` on a stream created before V7 always returns `false` until explicitly enabled
- Safe for upgrades: no data loss, all old streams remain functional

## Retry Safety Invariants

The contract maintains the following invariants to ensure deterministic behavior across retries:

1. **Idempotent Entry-Points**
   - Write-only operations that succeed on first invocation return success on retry (CEI ensures idempotency)
   - Example: `create_stream` with identical params returns identical stream_id

2. **Deterministic Error Handling**
   - Validation-only operations (no side effects on error) return identical errors on retry
   - Example: `batch_withdraw` with duplicate IDs always returns `DuplicateStreamId`

3. **Timestamp Determinism**
   - Accrual calculations are deterministic given stream params and current time
   - `calculate_accrued_amount_checkpointed` depends only on struct fields and `now` parameter
   - Retry with same ledger timestamp produces same accrued amount

4. **Storage Determinism**
   - All storage operations are deterministic (no randomness, no UUID generation beyond stream_id)
   - Storage keys are frozen (DataKey discriminants stable)
   - Retry with same parameters produces same storage mutations

5. **CEI Pattern Ensures Atomicity**
   - Check: validation and pre-condition checks
   - Effect: storage mutations (persisted before external calls)
   - Interaction: external token transfers (revert on failure)
   - Retry doesn't encounter different checks because storage is not updated by the same invocation twice

## Gas and Storage Behavior During Upgrades

### Storage Overhead per Stream

**Max stream entry size**: 4,096 bytes (XDR-encoded)

Breakdown:
- Base `Stream` struct: ~400 bytes (stream_id, sender, recipient, deposit_amount, rate, status, etc.)
- Checkpoint fields: 32 bytes (checkpointed_amount: i128, checkpointed_at: u64)
- Index entries: marginal (8 bytes per stream ID in recipient/sender indexes)

**Per-stream TTL cost**: Adaptive TTL bump scales to remaining stream lifetime.

### Storage Mutations During Common Operations

| Operation | Storage Keys Written | Instances |
|---|---|---|
| `create_stream` | `Stream(id)`, `RecipientStreams(recipient)`, `SenderStreams(sender)`, `NextStreamId`, `TotalLiabilities` | 1 per create |
| `withdraw` | `Stream(id)`, `TotalLiabilities` | 1 per withdrawal |
| `pause_stream` | `Stream(id)`, `PausedStreamCount` | 1 per pause |
| `cancel_stream` | `Stream(id)`, `SenderStreams(sender)`, `TotalLiabilities` | 1 per cancel |
| `decrease_rate_per_second` | `Stream(id)` | 1 per rate decrease |

**Upgrade impact**: No new storage is required for old streams. New features use appended `DataKey` variants that don't interfere with existing entries.

## Testing Strategy

### Unit Tests (accrual.rs)

- Checkpoint preservation across rate changes
- Monotonicity of accrued amounts
- Clamping at cliff and end times
- Overflow handling in rate calculations

### Integration Tests (gas_regression.rs)

- Gas baseline for single and batch operations
- Storage entry size (MAX_STREAM_ENTRY_BYTES validation)
- TTL bump behavior
- Keeper cancel incentive scenarios

### Edge-Case Tests (manifest-versioning.test.rs - new)

- Clock regression detection and idempotency
- Paused stream state preservation
- Checkpoint state across rate changes
- Batch operation idempotency
- Metadata immutability
- Terminal state semantics
- Global pause state isolation
- AutoRenew opt-in default behavior

## Documentation References

- `docs/ABI_STABILITY.md` — Frozen discriminants, event shapes, error codes
- `docs/upgrade.md` — Version history and operator migration guide
- `docs/storage.md` — Storage invariants and DataKey layout
- `docs/storage-invariants.md` — CEI pattern, TTL bumping, liability tracking
- `docs/streaming.md` — Accrual formula and checkpoint mechanics

## Rollout Checklist for Future Versions

When incrementing `CONTRACT_VERSION`:

1. [ ] Update version constant and version history in this document
2. [ ] Update `docs/ABI_STABILITY.md` with frozen discriminants
3. [ ] Update `docs/upgrade.md` with version row
4. [ ] Update `docs/storage.md` with DataKey changes
5. [ ] Run `cargo test manifest_versioning` to validate edge cases
6. [ ] Run `cargo test gas_regression` to validate storage overhead
7. [ ] Code review for discriminant stability
8. [ ] Verify all integration tests pass before merge
9. [ ] Announce migration timeline to operators

---

**Last Updated**: Issue #1344
**Status**: Stable
**Backward Compatible**: Yes (V9 fully backward-compatible with all prior versions)
