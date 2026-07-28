# Recipient Stream Index

The Fluxora streaming contract maintains a sorted index of stream IDs for each recipient address stored in persistent storage. This feature enables efficient enumeration and count queries for recipient portals, withdrawal workflows, and analytics.

---

## Data Structure & Storage

### Storage Key
```rust
#[contracttype]
pub enum DataKey {
    // ...
    RecipientStreams(Address), // Persistent storage for recipient stream index
}
```

The index maps a recipient's `Address` to a `Vec<u64>` containing stream IDs assigned to that recipient.

### Core Invariants

1. **Sorted Order Invariant**: Stream IDs within a recipient's index are strictly maintained in **ascending order** (`stream_ids[i] < stream_ids[i+1]`). Sorted placement is achieved via binary search during insertion.
2. **Completeness Invariant**: All non-closed streams (`Active`, `Paused`, `Cancelled`, `Completed`) remain in the recipient's index. Streams are only removed when explicitly closed via `close_completed_stream`.
3. **Uniqueness Invariant**: A stream ID appears at most once in a recipient's index.

---

## Public API Reference

### 1. `get_recipient_streams_paginated(env, recipient, cursor, limit) -> Page` (Primary / Recommended)

Returns a paginated slice of stream IDs starting after `cursor`, up to `limit` (capped at `RECIPIENT_STREAMS_PAGE_LIMIT = 100`).

Set `cursor = 0` to begin from the first stream. Pagination is complete when `Page.next_cursor == 0`.

`Page` shape:
```rust
pub struct Page {
    pub stream_ids: Vec<u64>,  // Sorted ascending
    pub next_cursor: u64,      // 0 when no more pages exist
}
```

**Enumeration Pattern:**
```rust
let mut cursor = 0u64;
loop {
    let page = client.get_recipient_streams_paginated(&recipient, &cursor, &100);
    for stream_id in page.stream_ids.iter() {
        // process stream_id …
    }
    cursor = page.next_cursor;
    if cursor == 0 { break; }
}
```

### 2. `get_recipient_streams(env, recipient) -> Vec<u64>` (Deprecated Convenience Wrapper)

Returns **at most `RECIPIENT_STREAMS_PAGE_LIMIT` (100) stream IDs**, sorted ascending.

> **Deprecated convenience wrapper.** Hard-bounded at 100 entries to prevent unbounded memory and gas exhaustion. Callers needing full enumeration **must** use `get_recipient_streams_paginated`.

| Condition | Result |
| --------- | ------ |
| No streams | Empty `Vec` |
| Count ≤ 100 | Full list |
| Count > 100 | First 100 IDs only |

### 3. `get_recipient_stream_count(env, recipient) -> u64`

Returns the total count of streams currently indexed for the given recipient address.

- **Efficiency**: Reads vector length directly without materializing individual stream items in full payload structures.
- **Use Case**: Lightweight UI counters or dashboard metrics.

---

## Lifecycle Integration

The recipient index is updated automatically as part of primary stream lifecycle operations.

| Operation | Index Effect | Description |
|-----------|--------------|-------------|
| `create_stream` | **Add** | Stream ID inserted into recipient index in sorted position. |
| `create_streams` (batch) | **Add** | Each created stream ID inserted into respective recipient index. |
| `pause_stream` | *No change* | Stream remains indexed. |
| `resume_stream` | *No change* | Stream remains indexed. |
| `cancel_stream` | *No change* | Stream remains indexed (allows recipients to inspect cancelled history). |
| `withdraw` / `withdraw_to` | *No change* | Stream remains indexed even after reaching `Completed` status. |
| `close_completed_stream` | **Remove** | Stream ID removed from index and stream data deleted from storage. |

---

## Performance & DoS Prevention

### Operational Complexity

| Operation | Complexity | Description |
|-----------|------------|-------------|
| `get_recipient_streams_paginated` | $O(1)$ read + $O(k)$ slice | Fast paginated slice read ($k \le 100$). |
| `get_recipient_stream_count` | $O(1)$ | Direct storage read of vector length. |
| Add stream to index | $O(n)$ | Binary search ($O(\log n)$) + vector insertion shift ($O(n)$). |
| Remove stream from index | $O(n)$ | Linear scan ($O(n)$) + vector remove shift ($O(n)$). |

*(where $n$ is the recipient's total active stream count, typically small).*

### DoS Prevention & Memory Safeguards

A recipient with thousands of streams could saturate Soroban's per-invocation memory or execution budget if an entire array were materialized at once.

- **Bounded Execution**: `RECIPIENT_STREAMS_PAGE_LIMIT = 100` (exposed as `MAX_RECIPIENT_PAGE_SIZE` in test crates).
- **Pre-allocation Prevention**: Limits are enforced **before** returning memory allocations to prevent gas or buffer attacks.

### Storage & TTL Rules

- **Write Extension**: Any operation updating the index (`create_stream`, `close_completed_stream`) extends the persistent storage TTL.
- **Read Extension**: Non-empty queries (`load_recipient_streams`) extend TTL to prevent expiration while actively queried. Empty indices do not maintain persistent storage keys.

---

## Security & Authorization

1. **Public Read Operations**: `get_recipient_streams`, `get_recipient_streams_paginated`, and `get_recipient_stream_count` require **no authorization**. Stream IDs are public on-chain identifiers and contain no sensitive financial details.
2. **Coupled Operations**: Index modification occurs only inside authorized contract functions (`create_stream`, `close_completed_stream`). Index updates cannot be triggered independently.
3. **Atomicity**: State mutations and index updates execute atomically. If a stream creation fails (e.g., failed token transfer), index updates roll back cleanly.

---

## Common Use Cases

### Recipient Portal
Enables front-end applications to list all incoming streams for a user by fetching paginated stream IDs and querying individual stream states or withdrawable amounts.

### Batch Withdrawal
Allows recipients to discover all claimable stream IDs and execute a `batch_withdraw` call in a single transaction.

```rust
let stream_ids = client.get_recipient_streams(&recipient_address);
client.batch_withdraw(&recipient_address, &stream_ids);
```

---

## Open Work & Future Enhancements

The following follow-up items and optimizations were identified during audit reviews and remain tracked for future releases:

1. **Binary Search Insertion Point Optimization**: Replace linear insertion shifting with optimized binary search position lookups for larger index volumes.
2. **Sender Stream Index**: Introduce a parallel index (`DataKey::SenderStreams(Address)`) to allow senders to enumerate streams they created.
3. **Status Filtered Queries**: Explore indexing streams by status (`Active`, `Paused`, `Cancelled`, `Completed`) for faster UI filtering.
4. **Atomic Recipient Transfer Support**: If stream recipient modification/transfer is added in future protocol versions, index removal from the old recipient and addition to the new recipient must execute atomically.
5. **CEI Refactoring Alignment**: Refactor `create_stream` internal sequence to move external token transfer calls strictly after all internal state and index persistence (reference: Issue #55).

---

## References

- **Main contract**: [`contracts/stream/src/lib.rs`](file:///c:/Users/ICT%20LASIEC/Fluxora-Contracts/contracts/stream/src/lib.rs)
- **Test suite**: [`contracts/stream/src/test.rs`](file:///c:/Users/ICT%20LASIEC/Fluxora-Contracts/contracts/stream/src/test.rs)
- **Streaming specification**: [`docs/streaming.md`](file:///c:/Users/ICT%20LASIEC/Fluxora-Contracts/docs/streaming.md)
- **Storage architecture**: [`docs/storage.md`](file:///c:/Users/ICT%20LASIEC/Fluxora-Contracts/docs/storage.md)
