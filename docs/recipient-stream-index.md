# Recipient Stream Index

The contract maintains a sorted index of stream IDs per recipient address.
Two entry-points expose this index; they differ in safety guarantees.

## Entry-points

### `get_recipient_streams(env, recipient) → Vec<u64>` — bounded convenience wrapper

Returns **at most `RECIPIENT_STREAMS_PAGE_LIMIT` (100) stream IDs**, sorted ascending.

> **Deprecated convenience wrapper.** This function is hard-bounded at
> `RECIPIENT_STREAMS_PAGE_LIMIT` to prevent unbounded memory and gas exhaustion.
> Callers that need the full index **must** use `get_recipient_streams_paginated`
> instead, iterating until `next_cursor == 0`.

Behavior:

| Condition | Result |
| --------- | ------ |
| No streams | Empty `Vec` |
| Count ≤ 100 | Full list (backward-compatible) |
| Count > 100 | First 100 IDs only |

### `get_recipient_streams_paginated(env, recipient, cursor, limit) → Page`

Returns up to `limit` IDs (itself capped at `RECIPIENT_STREAMS_PAGE_LIMIT`) starting
after `cursor`.  Set `cursor = 0` to begin from the first stream.  Pagination is
complete when `Page.next_cursor == 0`.

`Page` shape:

```rust
pub struct Page {
    pub stream_ids: Vec<u64>,  // sorted ascending
    pub next_cursor: u64,      // 0 when no more pages
}
```

## Full enumeration pattern

```rust
let mut cursor = 0u64;
loop {
    let page = client.get_recipient_streams_paginated(&recipient, &cursor, &100);
    // process page.stream_ids …
    cursor = page.next_cursor;
    if cursor == 0 { break; }
}
```

## DoS-prevention rationale

A recipient with thousands of streams can saturate Soroban's per-invocation
read/return budget.  The cap is enforced **before** the return `Vec` is
materialised in contract memory, so an oversized buffer is never allocated.

- `RECIPIENT_STREAMS_PAGE_LIMIT = 100` (see `contracts/stream/src/lib.rs`)
- Matches `MAX_PAGE_SIZE` used by other paginated views
- Exposed as `MAX_RECIPIENT_PAGE_SIZE` for test crates

## Security notes

- The bound in `get_recipient_streams` is applied on the slice of the internal
  sorted `Vec`, so the contract never constructs an over-sized vector.
- `get_recipient_streams_paginated` independently enforces the same cap via
  `limit.min(RECIPIENT_STREAMS_PAGE_LIMIT)`.
- Neither entry-point requires authorisation (public read).
