# Sender-Based Stream Index

## Overview

The sender-based stream index is a secondary index that enables efficient enumeration and analytics queries for streams created by a specific sender. This feature maintains a list of stream IDs for each sender in creation order, allowing applications to:

- List all streams created by a user
- Implement pagination for sender streams
- Perform analytics on sender activity
- Support efficient UI queries without off-chain indexing

## Architecture

### Storage Structure

The index is stored in persistent storage using the `DataKey::SenderStreamIndex(Address)` key pattern:

```rust
#[contracttype]
pub enum DataKey {
    Config,                    // Global configuration
    NextStreamId,              // Stream counter
    Stream(u64),               // Individual stream data
    SenderStreamIndex(Address), // Sender-based stream enumeration
}
```

Each sender has a single index entry containing a vector of stream IDs in creation order.

### Index Maintenance

The index is automatically maintained during stream lifecycle operations:

1. **Creation**: When a stream is created via `create_stream()` or `create_streams()`, the stream ID is appended to the sender's index.
2. **Lifecycle Operations**: Pause, resume, cancel, and withdraw operations do not modify the index (streams remain indexed regardless of status).
3. **Completion**: Completed streams remain in the index until explicitly closed via `close_completed_stream()`.

### TTL Management

The sender index follows the same TTL policy as individual streams:

- **Threshold**: 17,280 ledgers (~24 hours at 5 seconds per ledger)
- **Extension**: 120,960 ledgers (~7 days)
- **Bumping**: TTL is extended on every read and write operation

This ensures that actively-queried indexes remain available while inactive indexes eventually expire.

## API

### `get_sender_streams(sender: Address) -> Vec<u64>`

Retrieves all stream IDs created by a sender in creation order.

**Parameters:**
- `sender`: Address of the stream creator

**Returns:**
- `Vec<u64>`: Vector of stream IDs in creation order (oldest to newest)
- Empty vector if the sender has no streams

**Authorization:** None required (public information)

**Usage Notes:**
- This is a view function (read-only, no state changes)
- Stream IDs are returned in creation order
- Useful for UIs to display all streams created by a user
- Useful for analytics and reporting on sender activity
- Pagination can be implemented by the caller using vector slicing

**Example:**

```rust
// Get all streams created by a sender
let streams = client.get_sender_streams(&sender_address);

// Implement pagination (page size = 10)
let page_size = 10;
let page_num = 0;
let start = page_num * page_size;
let end = (start + page_size).min(streams.len() as u32);

for i in start..end {
    let stream_id = streams.get(i).unwrap();
    let state = client.get_stream_state(&stream_id);
    // Process stream...
}
```

## Consistency Guarantees

### Index Consistency Through Lifecycle

The sender index remains consistent through all stream lifecycle operations:

- **Pause/Resume**: Index unchanged (stream remains indexed)
- **Cancel**: Index unchanged (cancelled streams remain indexed)
- **Withdraw**: Index unchanged (stream remains indexed until completion)
- **Completion**: Index unchanged (completed streams remain indexed)
- **Close**: Stream removed from index only when explicitly closed

### Ordering Guarantee

Stream IDs in the sender index are guaranteed to be in creation order (oldest to newest). This ordering is maintained across all operations and is deterministic.

### Atomicity

Index updates are atomic with stream creation:
- If stream creation succeeds, the stream ID is added to the sender's index
- If stream creation fails (e.g., insufficient balance), the index is not modified
- Batch creation (`create_streams`) atomically updates the index for all streams or none

## Performance Characteristics

### Time Complexity

- **get_sender_streams**: O(1) storage read + O(n) vector iteration where n = number of streams by sender
- **Index update on creation**: O(1) append operation

### Space Complexity

- **Per sender**: O(n) where n = number of streams created by that sender
- **Total contract**: O(m) where m = total number of streams across all senders

### Scalability

The index scales linearly with the number of streams per sender. For senders with thousands of streams, pagination is recommended:

```rust
// Efficient pagination for large sender stream lists
let page_size = 100;
let total_streams = client.get_sender_streams(&sender).len();
let total_pages = (total_streams + page_size - 1) / page_size;

for page in 0..total_pages {
    let start = page * page_size;
    let end = (start + page_size).min(total_streams);
    let page_streams = client.get_sender_streams(&sender);
    // Process page_streams[start..end]
}
```

## Use Cases

### 1. User Dashboard

Display all streams created by a user:

```rust
let sender = user_address;
let streams = client.get_sender_streams(&sender);

for stream_id in streams.iter() {
    let state = client.get_stream_state(&stream_id);
    println!("Stream {}: {} tokens to {}", 
        stream_id, 
        state.deposit_amount, 
        state.recipient
    );
}
```

### 2. Analytics and Reporting

Calculate total value streamed by a sender:

```rust
let sender = user_address;
let streams = client.get_sender_streams(&sender);
let mut total_value = 0i128;

for stream_id in streams.iter() {
    let state = client.get_stream_state(&stream_id);
    total_value += state.deposit_amount;
}

println!("Total value streamed: {}", total_value);
```

### 3. Pagination

Implement efficient pagination for large sender stream lists:

```rust
fn get_sender_streams_page(
    client: &FluxoraStreamClient,
    sender: &Address,
    page: u32,
    page_size: u32,
) -> Vec<u64> {
    let all_streams = client.get_sender_streams(sender);
    let start = (page * page_size) as usize;
    let end = ((page + 1) * page_size).min(all_streams.len() as u32) as usize;
    
    all_streams[start..end].to_vec()
}
```

### 4. Stream Enumeration

Iterate through all streams for a sender with status filtering:

```rust
let sender = user_address;
let streams = client.get_sender_streams(&sender);

let active_streams: Vec<u64> = streams
    .iter()
    .filter(|&stream_id| {
        let state = client.get_stream_state(&stream_id);
        state.status == StreamStatus::Active
    })
    .collect();

println!("Active streams: {}", active_streams.len());
```

## Testing

The sender-based stream index includes comprehensive test coverage:

- **Empty index**: Verify empty vector for new senders
- **Single stream**: Verify index contains single stream ID
- **Multiple streams**: Verify ordering and completeness
- **Independent indexes**: Verify different senders have independent indexes
- **Lifecycle consistency**: Verify index unchanged through pause/resume/cancel
- **Batch creation**: Verify batch operations update index correctly
- **Pagination support**: Verify vector slicing for pagination
- **Interleaved creation**: Verify correct index for multiple senders
- **Analytics use case**: Verify iteration and aggregation
- **Schedule modifications**: Verify index unchanged after rate/schedule updates
- **Consistency with stream count**: Verify sum of sender indexes equals total count

All tests achieve 95%+ coverage of the index functionality.

## Security Considerations

### Authorization

The `get_sender_streams` function requires no authorization (public information). Any address can query streams created by any sender.

### Storage Access

The index is stored in persistent storage with the same TTL and access patterns as individual streams. No special security considerations apply.

### Atomicity

Index updates are atomic with stream creation. If stream creation fails, the index is not modified, preventing inconsistencies.

## Migration and Upgrades

### Backward Compatibility

The sender-based stream index is a new feature that does not affect existing streams or functionality. Existing streams created before this feature was deployed will not have index entries until the contract is upgraded.

### Index Population

After upgrading to a version with the sender-based stream index:
- New streams will automatically be indexed
- Existing streams will not be indexed (they were created before the index existed)
- Off-chain indexing can be used to populate historical data if needed

## Future Enhancements

Potential future enhancements to the sender-based stream index:

1. **Recipient-based index**: Similar index for streams received by a recipient
2. **Status-based filtering**: Built-in filtering by stream status
3. **Time-range queries**: Query streams created within a time range
4. **Composite indexes**: Combine sender + status for efficient filtered queries
5. **Index statistics**: Query total value, count, and other metrics per sender

## References

- [Storage Layout Documentation](./storage.md)
- [Stream Lifecycle Documentation](./streaming.md)
- [Contract Implementation](../contracts/stream/src/lib.rs)
