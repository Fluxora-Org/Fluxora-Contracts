# Stream Storage Invariants

Documented guarantees for Fluxora stream contract storage behavior. Regression
coverage: `contracts/stream/tests/storage_invariants.rs`.

**Related:** [storage.md](./storage.md) · [security.md](./security.md)

---

## TTL

- **Instance storage** (`Config`, `NextStreamId`, pause counters, etc.) TTL is
  extended via `bump_instance_ttl()` on every entry-point that touches instance keys.
- **Persistent storage** (`Stream(id)`, recipient/sender indexes) TTL is extended
  on every `load_stream` / `save_stream` and index read/write.
- Closed streams remove persistent entries; inactive streams may expire after ~7
  days without interaction.

## Reentrancy

- `ReentrancyLock` (instance `bool`) is acquired before token transfers that can
  re-enter the contract and released afterward.
- Nested calls while the lock is held return `InvalidState`.

## Liabilities

- `TotalLiabilities` (instance `i128`) is the sum of outstanding deposit
  obligations owed to recipients.
- Incremented on `create_stream`, `top_up_stream`, and similar funding paths.
- Decremented on successful `withdraw`, `cancel_stream`, `keeper_cancel`, and
  refund-modifying operations (`shorten_stream_end_time`, `decrease_rate_per_second`).
- `get_total_liabilities()` exposes the counter read-only.

## Indexes (sorted)

- `RecipientStreams(Address)` and `SenderStreams(Address)` store `Vec<u64>` sorted
  ascending by `stream_id`.
- Inserts use binary-search position; removals preserve order.

## CEI (Checks-Effects-Interactions)

- Stream fields (`withdrawn_amount`, `status`, etc.) are saved before any
  external SEP-41 token transfer.
- Liability counters are updated in the same transaction as the state write or
  immediately before transfer, depending on the entry-point.

## Terminal state

- A stream is **terminal** when `status == Cancelled` or
  `ledger.timestamp() >= end_time`.
- Terminal streams bypass `withdraw_dust_threshold` so recipients can drain
  remaining balances.
- Accrual freezes at `cancelled_at` for cancelled streams.

## Metadata validation

- Optional stream metadata is validated by `validate_metadata` in `storage.rs`
  before stream ID allocation (see [metadata-extension.md](./metadata-extension.md)).
- Metadata is immutable post-creation.

## Append-only DataKey layout

- `DataKey` enum discriminants 0–35 are frozen for deployed instances.
- New variants append at the end only; reordering corrupts live storage.

---

## Regression surface

```bash
cargo test -p fluxora_stream --test storage_invariants --features testutils
```

| Test | Invariant |
|------|-----------|
| `total_liabilities_increments_on_create` | Liabilities |
| `stream_state_round_trips_via_public_api` | CEI / persistence |
| `recipient_index_sorted_after_multiple_creates` | Sorted indexes |
| `terminal_cancelled_stream_bypasses_dust_threshold` | Terminal + dust |
