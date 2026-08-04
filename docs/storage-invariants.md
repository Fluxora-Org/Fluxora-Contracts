# Stream Storage Invariants

Documented guarantees for Fluxora stream contract storage behavior. Regression
coverage: `contracts/stream/tests/storage_invariants.rs` and
`contracts/stream/tests/storage_invariants_edge_cases.rs`.

**Related:** [storage.md](./storage.md) · [security.md](./security.md)

---

## TTL

- **Instance storage** (`Config`, `NextStreamId`, pause counters, etc.) TTL is
  extended via `bump_instance_ttl()` on every entry-point that touches instance keys.
- **Persistent storage** (`Stream(id)`, recipient/sender indexes) TTL is extended
  on every `load_stream` / `save_stream` and index read/write.
- **Adaptive TTL**: Persistent stream entries use `compute_adaptive_ttl()` which
  scales the bump proportional to the stream's remaining lifetime. When `end_time`
  has passed or `now >= end_time`, the bump falls back to `PERSISTENT_BUMP_AMOUNT`
  (the static floor) so the entry stays alive long enough for the recipient to
  withdraw. The result is clamped to `[PERSISTENT_BUMP_AMOUNT, MAX_TTL]`.
- Closed streams remove persistent entries; inactive streams may expire after ~7
  days without interaction.

### Adaptive TTL edge cases

| Scenario | Behavior |
|----------|----------|
| `end_time` far in future (>~180 days) | Clamped to `MAX_TTL` |
| `end_time` moderately in future | Proportional: `remaining_seconds / 5 + BUFFER_LEDGERS` |
| `end_time == now` | Falls back to `PERSISTENT_BUMP_AMOUNT` |
| `end_time < now` (expired) | Saturating sub yields 0 → `PERSISTENT_BUMP_AMOUNT` |
| `end_time == 0` | `PERSISTENT_BUMP_AMOUNT` floor |
| `end_time == u64::MAX` | Clamped to `MAX_TTL` |

## Reentrancy

- `ReentrancyLock` (instance `bool`) is acquired before token transfers that can
  re-enter the contract and released afterward.
- Nested calls while the lock is held return `InvalidState`.
- Acquire → release → acquire cycles are fully supported (the lock is cleared,
  not sticky).

## Liabilities

- `TotalLiabilities` (instance `i128`) is the sum of outstanding deposit
  obligations owed to recipients.
- Incremented on `create_stream`, `top_up_stream`, and similar funding paths.
- Decremented on successful `withdraw`, `cancel_stream`, `keeper_cancel`, and
  refund-modifying operations (`shorten_stream_end_time`, `decrease_rate_per_second`).
- **Guaranteed non-negative (`>= 0`)**: `write_total_liabilities` clamps any
  negative input to 0 before storage.
- **Overflow-safe**: `increment_total_keeper_fees_paid` uses `checked_add` and
  returns `ArithmeticOverflow` on overflow. The stored value is unchanged after
  a failed increment.
- `get_total_liabilities()` exposes the counter read-only.

## Indexes (sorted & unique)

- `RecipientStreams(Address)`, `SenderStreams(Address)`, and `RecipientPendingOffers(Address)` store `Vec<u64>` sorted
  ascending by `stream_id` / `offer_id`.
- Inserts use binary-search position and early-return if present (idempotent set-uniqueness); removals preserve order.
- Reclaims persistent storage keys when index vectors become empty.
- **Removal of non-existent IDs**: Removing a stream or offer ID that is not
  present in the index is a safe no-op — it does not alter the stored vector or
  emit errors.

## Stream Structural Validation

- `validate_stream_invariants` validates all field constraints prior to storage persistence (`save_stream`).
- `save_stream` **panics** if validation fails — this is a hard invariant; stream
  fields must satisfy all constraints before the save path is reached.

### Validated constraints

| Constraint | Error on violation |
|---|---|
| `start_time <= end_time` | `InvalidParams` |
| `cliff_time >= start_time && cliff_time <= end_time` | `InvalidParams` |
| `deposit_amount >= 0` | `InvalidParams` |
| `withdrawn_amount >= 0 && withdrawn_amount <= deposit_amount` | `InvalidParams` / `InvalidState` |
| `checkpointed_amount >= 0 && checkpointed_amount <= deposit_amount` | `InvalidState` |
| `checkpointed_at <= end_time` | `InvalidState` |
| `rate_per_second >= 0` | `InvalidParams` |
| `withdraw_dust_threshold >= 0` | `InvalidParams` |

### Boundary values accepted

| Boundary | Accepted? |
|---|---|
| `rate_per_second == 0` | ✅ Yes (CliffOnly streams) |
| `start_time == cliff_time == end_time` (zero-duration) | ✅ Yes |
| `checkpointed_at == end_time` | ✅ Yes |
| `withdrawn_amount == deposit_amount` (fully drained) | ✅ Yes |
| `checkpointed_amount == deposit_amount` (fully checkpointed) | ✅ Yes |

## Terminal State

- A stream is **terminal** when `status == Cancelled`, `status == Completed`, or
  `ledger.timestamp() >= end_time`.
- Terminal streams bypass `withdraw_dust_threshold` so recipients can drain
  remaining balances.
- Accrual freezes at `cancelled_at` for cancelled streams.
- **Paused streams** before `end_time` are NOT terminal — they can still be
  resumed.

## CEI (Checks-Effects-Interactions)

- Stream fields (`withdrawn_amount`, `status`, etc.) are saved before any
  external SEP-41 token transfer.
- Liability counters are updated in the same transaction as the state write or
  immediately before transfer, depending on the entry-point.

## Metadata validation

- Optional stream metadata is validated by `validate_metadata` in `storage.rs`
  before stream ID allocation (see [metadata-extension.md](./metadata-extension.md)).
- Metadata is immutable post-creation.
- Aggregate byte size check uses checked arithmetic to prevent overflow on
  adversarial input.

## Paused Stream Counter

- `PausedStreamCount` (instance `u64`) tracks the protocol-wide number of streams
  currently in `StreamStatus::Paused`.
- `reconcile_paused_stream_count` correctly increments/decrements on transitions:
  - `Active → Paused`: +1
  - `Paused → Active/Cancelled/Completed`: -1
  - Same-state transitions (`Paused → Paused`, `Active → Active`): no change
  - Saturating subtraction prevents underflow below 0

## Append-only DataKey layout

- `DataKey` enum discriminants 0–36 are frozen for deployed instances (37 variants total).
- New variants append at the end only; reordering corrupts live storage.
- Version mapping is machine-checked in `storage_key_compat.rs`.

---

## Regression surface

```bash
# Core invariants
cargo test -p fluxora_stream --test storage_invariants --features testutils

# Edge-case invariants
cargo test -p fluxora_stream --test storage_invariants_edge_cases --features testutils

# Storage key compatibility
cargo test -p fluxora_stream --test storage_key_compat --features testutils
```

### `storage_invariants.rs` — core invariant tests

| Test | Invariant |
|------|-----------|
| `total_liabilities_increments_on_create` | Liabilities |
| `stream_state_round_trips_via_public_api` | CEI / persistence |
| `recipient_index_sorted_after_multiple_creates` | Sorted indexes |
| `terminal_cancelled_stream_bypasses_dust_threshold` | Terminal + dust |
| `final_drain_bypasses_dust_threshold` | Final drain + dust |
| `recipient_index_insertion_is_idempotent` | Index uniqueness |
| `stream_structural_invariants_validation` | Field constraints |

### `storage_invariants_edge_cases.rs` — edge-case invariant tests

| Test | Invariant |
|------|-----------|
| `test_adaptive_ttl_computation_boundaries` | TTL floor/ceiling |
| `test_recipient_and_sender_index_storage_reclamation` | Empty-index reclamation |
| `test_instance_ttl_bumping_on_queries` | TTL bump on reads |
| `test_same_ledger_retries_and_monotonicity` | Clock monotonicity |
| `test_id_reservation_and_monotonic_stream_ids` | ID reservation + counter |
| `test_total_liabilities_non_negative_floor` | Liabilities floor |
| `test_offer_index_insertion_is_idempotent` | Offer index uniqueness |
| `test_validate_stream_invariants_boundary_values` | Boundary value acceptance |
| `test_remove_nonexistent_stream_id_is_noop` | Idempotent removal |
| `test_paused_stream_count_reconciliation` | Pause counter transitions |
| `test_total_liabilities_extreme_values` | Extreme value clamping |
| `test_keeper_fee_aggregate_overflow_protection` | Overflow safety |
| `test_is_terminal_state_edge_cases` | Terminal detection |
| `test_remove_nonexistent_offer_is_idempotent` | Offer removal idempotency |
| `test_sender_index_sorted_order_maintenance` | Sender index ordering |
| `test_reentrancy_lock_acquire_release_cycle` | Reentrancy lock lifecycle |
| `test_auto_renew_storage_round_trip` | Auto-renew persistence |
| `test_max_lookback_ledgers_validation` | Lookback constraint enforcement |
