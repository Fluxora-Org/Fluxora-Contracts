# Recipient Stream Index

## Overview

The recipient stream index is a secondary data structure that enables efficient enumeration of all streams for a given recipient address. This feature is essential for recipient portals and withdraw workflows where users need to see all their incoming streams.

**Key characteristics:**
- **Sorted by stream_id**: All streams for a recipient are maintained in ascending order by stream_id
- **Deterministic**: Same recipient always returns streams in the same order
- **Lifecycle-aware**: Streams are added on creation, removed on close
- **Separate per recipient**: Each recipient has an independent index
- **Efficient**: O(log n) insertion/removal, O(1) lookup by recipient

## Data Structure

### Storage Key

```rust
DataKey::RecipientStreams(Address)  // Persistent storage for recipient stream index
```

Each recipient address maps to a `Vec<u64>` of stream IDs, maintained in sorted ascending order.

### Invariants

1. **Sorted order**: `streams[i] < streams[i+1]` for all valid indices
2. **Uniqueness**: No duplicate stream IDs in a recipient's index
3. **Completeness**: All active streams for a recipient are in the index
4. **Consistency**: Index is updated atomically with stream creation/closure

## API Reference

### Query Functions

#### `get_recipient_streams(recipient: Address) -> Vec<u64>`

Retrieve all stream IDs for a given recipient in sorted ascending order.

**Parameters:**
- `recipient`: Address to query streams for

**Returns:**
- `Vec<u64>`: Vector of stream IDs (sorted ascending by stream_id)
  - Empty vector if the recipient has no streams
  - Includes streams in all statuses (Active, Paused, Completed, Cancelled)
  - Does not include closed streams (removed via `close_completed_stream`)

**Behavior:**
- This is a view function (read-only, no state changes)
- No authorization required (public information)
- Extends TTL on the recipient's index to prevent expiration
- Useful for recipient portals to enumerate all streams

**Usage:**
```rust
// Get all streams for a recipient
let streams = client.get_recipient_streams(&recipient_address);

// Iterate through streams
for stream_id in streams.iter() {
    let state = client.get_stream_state(&stream_id);
    // Process stream...
}
```

#### `get_recipient_stream_count(recipient: Address) -> u64`

Count the total number of streams for a recipient.

**Parameters:**
- `recipient`: Address to query stream count for

**Returns:**
- `u64`: Number of streams for the recipient (0 if none)

**Behavior:**
- This is a view function (read-only, no state changes)
- No authorization required (public information)
- Extends TTL on the recipient's index to prevent expiration
- More gas-efficient than `get_recipient_streams` when only count is needed

**Usage:**
```rust
// Get stream count for UI display
let count = client.get_recipient_stream_count(&recipient_address);
println!("You have {} active streams", count);
```

## Lifecycle Management

### Stream Creation

When a stream is created via `create_stream` or `create_streams`:

1. Stream is persisted in `DataKey::Stream(stream_id)`
2. Stream ID is added to recipient's index in sorted order
3. TTL is extended on the recipient's index

**Example:**
```
Before: recipient has streams [0, 2, 5]
Create stream 3
After: recipient has streams [0, 2, 3, 5]  (sorted)
```

### Stream Closure

When a completed stream is closed via `close_completed_stream`:

1. Stream is removed from recipient's index
2. Stream data is deleted from persistent storage
3. TTL is extended on the recipient's index

**Example:**
```
Before: recipient has streams [0, 2, 3, 5]
Close stream 3
After: recipient has streams [0, 2, 5]
```

### Stream Status Changes

Status changes (pause, resume, cancel, withdraw) do **not** affect the index:

- **Pause/Resume**: Stream remains in index
- **Cancel**: Stream remains in index (not removed until closed)
- **Withdraw**: Stream remains in index (even when completed)
- **Close**: Stream is removed from index

## Consistency Guarantees

### Sorted Order

The index is always maintained in ascending order by stream_id. This enables:

- **Deterministic pagination**: Same recipient always returns streams in the same order
- **Efficient binary search**: O(log n) lookup for insertion point
- **UI consistency**: Predictable display order across sessions

### Completeness

All active streams for a recipient are in the index:

- Streams added on creation
- Streams removed only on close
- No streams are "lost" or orphaned
- Index is atomic with stream operations

### Recipient Updates

If recipient updates are supported in the future (e.g., stream transfer), the index must be updated atomically:

1. Remove stream from old recipient's index
2. Add stream to new recipient's index
3. Update stream's recipient field

This ensures consistency across all indices.

## Protocol Semantics & Audit Notes

To provide crisp assurances to integrators, the recipient index observes strict invariants across all execution paths.

### Success Semantics
- The index strictly maintains `stream_id` in ascending order (enforced internally via `binary_search`).
- Index mutation is completely atomic with stream status changes. A `create_stream` guarantees the recipient index includes the new stream. A `close_completed_stream` guarantees it is removed.
- State views emit deterministic, deduplicated, sorted output regardless of the creation order of individual underlying stream persistence.

### Failure Semantics
- Any failure during insertion (e.g., Soroban environment memory limit on highly unbounded indices) will revert the entire transaction. There is no silent drift where a stream is persisted but the index is not.
- Indexing code correctly cascades errors back to the caller instead of isolating failures.
- Direct non-authorized mutation is impossible; index updates are intentionally unexposed to cross-contract endpoints except via internally triggered side-effects of authorized stream lifecycle functions.

### Roles and Authorization
- **Sender**: Authorizes stream creation (`create_stream`, `create_streams`), implicitly appending the resulting `stream_id` to the recipient index.
- **Requester (Recipient/Admin/Anyone)**: Anyone executing `close_completed_stream` implicitly proves the stream is fully depleted. This authorization-free action triggers the internal removal of the `stream_id` from the index.
- **Indexers / Third Parties**: Any caller can query the index via `get_recipient_streams`. No proof of identity is required.

### Edge Cases and Residual Risks
1. **Time Boundaries**: The index is agnostic to start times, cliff times, or end times. Progressing ledger timestamps will **not** alter index composition. A stream remains in the index before its start and long after its end until explicitly closed.
2. **Stream Status Configurations**: Active, Paused, and Cancelled streams persist within the index. Pause or Cancel operations neither remove nor re-append items to the index.
3. **Numeric Bounds & Host Limits (Audit Note)**: The protocol does not actively bounded the maximum number of streams a single recipient can harbor. Extreme volumes of incoming streams to a single recipient could exceed Soroban's local storage read/write operational budgets, leading to out-of-gas (`HostError`) during insertion/lookup. Senders are responsible for ensuring they do not grief recipients by deliberately bloating their indexes.

## Performance Characteristics

### Time Complexity

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| `get_recipient_streams` | O(1) | Direct storage read |
| `get_recipient_stream_count` | O(1) | Direct storage read |
| Add stream to index | O(n) | Binary search + insert |
| Remove stream from index | O(n) | Linear search + remove |

Where n = number of streams for the recipient (typically small).

### Storage Complexity

- **Per recipient**: O(n) where n = number of streams
- **Total contract**: O(S) where S = total number of streams across all recipients
- **Overhead**: ~8 bytes per stream ID per recipient

## Use Cases

### Recipient Portal

Display all streams for a user:

```rust
let streams = client.get_recipient_streams(&user_address);
for stream_id in streams.iter() {
    let state = client.get_stream_state(&stream_id);
    let accrued = client.calculate_accrued(&stream_id);
    let withdrawable = client.get_withdrawable(&stream_id);
    
    println!("Stream {}: {} accrued, {} withdrawable",
        stream_id, accrued, withdrawable);
}
```

### Batch Withdraw

Withdraw from all streams:

```rust
let streams = client.get_recipient_streams(&recipient);
let results = client.batch_withdraw(&recipient, &streams);
```

### Stream Analytics

Analyze recipient's portfolio:

```rust
let count = client.get_recipient_stream_count(&recipient);
let total_accrued: i128 = client.get_recipient_streams(&recipient)
    .iter()
    .map(|id| client.calculate_accrued(&id).unwrap_or(0))
    .sum();
```

### Pagination

Paginate through large recipient portfolios:

```rust
let all_streams = client.get_recipient_streams(&recipient);
let page_size = 10;
let page_num = 0;

let start = page_num * page_size;
let end = (start + page_size).min(all_streams.len() as usize);

for i in start..end {
    let stream_id = all_streams.get(i as u32).unwrap();
    // Process stream...
}
```

## Testing

The recipient stream index is tested comprehensively:

### Test Coverage

- **Creation**: Streams added to index on creation
- **Sorted order**: Multiple streams maintain sorted order
- **Separate indices**: Different recipients have independent indices
- **Closure**: Streams removed from index on close
- **Lifecycle**: Index remains consistent through pause/resume/cancel
- **Batch operations**: Batch withdraw works with indexed streams
- **Large portfolios**: Handles 50+ streams per recipient
- **Multiple senders**: Correctly indexes streams from different senders

### Test Files

- `contracts/stream/src/test.rs`: Comprehensive test suite (95%+ coverage)
- Test snapshots: `contracts/stream/test_snapshots/test/`

## Migration & Upgrades

### Backward Compatibility

The recipient stream index is a new feature that does not affect existing streams:

- Existing streams continue to work unchanged
- New streams automatically get indexed
- No migration required for existing data

### Future Enhancements

Potential future improvements:

1. **Sender index**: Similar index for senders to enumerate their streams
2. **Status filter**: Separate indices by status (Active, Paused, Completed, Cancelled)
3. **Time-based index**: Index streams by start_time or end_time for scheduling
4. **Recipient transfer**: Support changing recipient with atomic index updates

## Security Considerations

### Authorization

- Index queries require no authorization (public information)
- Index is read-only from the contract perspective
- Index updates are internal to stream creation/closure

### Storage Limits

- No hard limit on streams per recipient
- Storage grows linearly with number of streams
- TTL management prevents index expiration

### Consistency

- Index updates are atomic with stream operations
- No race conditions (Soroban is single-threaded)
- Index is always consistent with stream data

## Documentation Sync Checklist

When modifying the recipient stream index:

- [ ] Update this document if behavior changes
- [ ] Update API documentation in `lib.rs`
- [ ] Add/update tests in `test.rs`
- [ ] Update test snapshots if needed
- [ ] Run `cargo test -p fluxora_stream` before committing
- [ ] Verify 95%+ test coverage maintained
- [ ] Update `QUICK_REFERENCE.md` if adding new queries

## Examples

### Example 1: Display Recipient Dashboard

```rust
pub fn display_recipient_dashboard(
    env: &Env,
    client: &FluxoraStreamClient,
    recipient: &Address,
) {
    let streams = client.get_recipient_streams(recipient);
    let count = client.get_recipient_stream_count(recipient);
    
    println!("Recipient: {}", recipient);
    println!("Total streams: {}", count);
    println!("\nStreams:");
    
    for stream_id in streams.iter() {
        let state = client.get_stream_state(&stream_id);
        let accrued = client.calculate_accrued(&stream_id).unwrap_or(0);
        let withdrawable = client.get_withdrawable(&stream_id).unwrap_or(0);
        
        println!("  Stream {}: {} accrued, {} withdrawable, status: {:?}",
            stream_id, accrued, withdrawable, state.status);
    }
}
```

### Example 2: Batch Withdraw from All Streams

```rust
pub fn withdraw_all_streams(
    env: &Env,
    client: &FluxoraStreamClient,
    recipient: &Address,
) -> i128 {
    let streams = client.get_recipient_streams(recipient);
    
    if streams.is_empty() {
        return 0;
    }
    
    let results = client.batch_withdraw(recipient, &streams);
    
    let total: i128 = results.iter()
        .map(|r| r.amount)
        .sum();
    
    println!("Withdrew {} total from {} streams", total, results.len());
    total
}
```

### Example 3: Find Streams by Status

```rust
pub fn get_active_streams(
    env: &Env,
    client: &FluxoraStreamClient,
    recipient: &Address,
) -> Vec<u64> {
    let streams = client.get_recipient_streams(recipient);
    let mut active = Vec::new();
    
    for stream_id in streams.iter() {
        let state = client.get_stream_state(&stream_id);
        if state.status == StreamStatus::Active {
            active.push(stream_id);
        }
    }
    
    active
}
```

## References

- **Main contract**: `contracts/stream/src/lib.rs`
- **Tests**: `contracts/stream/src/test.rs`
- **Streaming docs**: `docs/streaming.md`
- **Storage docs**: `docs/storage.md`
