# Recipient Stream Index

## Overview

Each recipient address has a persistent sorted list of stream IDs stored under
`DataKey::RecipientStreams(recipient)`. This index powers `get_recipient_streams`
and `get_recipient_stream_count` without scanning all streams.

## Storage key

```
DataKey::RecipientStreams(Address) → Vec<u64>  (persistent, sorted ascending)
```

## Batch-create caching (issue #514)

### Problem

`create_streams` previously called `add_stream_to_recipient_index` once per
stream inside the second pass. Each call independently read and rewrote the
recipient's full stream list from ledger storage, causing **O(n) ledger reads**
for a batch of n streams to the same recipient.

### Solution

`create_streams` now uses a local `Map<Address, Vec<u64>>` cache:

1. **Second pass** — calls `persist_new_stream_skip_index` (identical to
   `persist_new_stream` but omits the index write) and accumulates each
   `(recipient, stream_id)` pair into the cache.
2. **Flush pass** — iterates the cache once, performing **one read + one write
   per unique recipient** regardless of how many streams were created for them.

### Complexity

| Scenario | Before | After |
|---|---|---|
| n streams, 1 recipient | O(n) reads, O(n) writes | O(1) read, O(1) write |
| n streams, n recipients | O(n) reads, O(n) writes | O(n) reads, O(n) writes |
| n streams, k recipients | O(n) reads, O(n) writes | O(k) reads, O(k) writes |

### Security notes

- The cache is a local in-memory `Map` scoped to the transaction; it is never
  persisted and cannot be observed or manipulated by other callers.
- The flush inserts IDs in sorted order using binary search, preserving the
  invariant that `RecipientStreams` is always sorted ascending.
- `create_stream` (single-stream path) is unchanged and still calls
  `add_stream_to_recipient_index` directly.
- `create_streams_relative` delegates to `create_streams` and inherits the
  optimisation automatically.

## TTL policy

`RecipientStreams` entries are bumped on every read and write using
`PERSISTENT_LIFETIME_THRESHOLD` / `PERSISTENT_BUMP_AMOUNT` (see
[storage.md](storage.md) for values). The flush pass calls `save_recipient_streams`
which triggers the TTL bump.
