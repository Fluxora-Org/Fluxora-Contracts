# Storage Layout

This document describes the contract's storage architecture, key design, and TTL (Time To Live) policies.

## Overview

The Fluxora Stream contract uses Soroban's storage system with two storage types:
- **Instance storage** for global configuration and counters
- **Persistent storage** for individual stream data

## DataKey Enum

All storage keys are defined in the `DataKey` enum:

```rust
#[contracttype]
pub enum DataKey {
    Config,       // Instance storage for global settings (admin/token).
    NextStreamId, // Instance storage for the auto-incrementing ID counter.
    Stream(u64),  // Persistent storage for individual stream data (O(1) lookup).
}
```

## Storage Types and Usage

### Instance Storage

Instance storage is used for contract-wide configuration that applies to all streams:

| Key | Type | Description | Set By | Modified By |
|-----|------|-------------|--------|-------------|
| `Config` | `Config` struct | Contains `token` address and `admin` address | `init()` | Never (immutable after init) |
| `NextStreamId` | `u64` | Auto-incrementing counter for stream IDs | `init()` (set to 0) | `create_stream()` (incremented) |

**Characteristics:**
- Shared across all contract operations
- Low cardinality (only 2 keys)
- Extended TTL on initialization: 17,280 ledgers threshold, 120,960 ledgers max
- Accessed frequently by most contract functions

### Persistent Storage

Persistent storage is used for individual stream records:

| Key Pattern | Type | Description | Set By | Modified By |
|-------------|------|-------------|--------|-------------|
| `Stream(stream_id)` | `Stream` struct | Complete stream state including participants, amounts, timing, and status | `create_stream()` | `pause_stream()`, `resume_stream()`, `cancel_stream()`, `withdraw()` |

**Characteristics:**
- One entry per stream (unbounded growth)
- O(1) lookup by stream ID
- Contains all stream metadata and state
- TTL extended on every write operation

## TTL (Time To Live) Policy

### Instance Storage TTL

Set during contract initialization (`init()`):
```rust
env.storage().instance().extend_ttl(17280, 120960);
```

- **Threshold**: 17,280 ledgers (~24 hours at 5s/ledger)
- **Max extension**: 120,960 ledgers (~7 days)
- **Rationale**: Ensures contract configuration remains accessible for active operations

### Persistent Storage TTL

Extended on every stream save operation (`save_stream()`):
```rust
env.storage().persistent().extend_ttl(&key, 17280, 120960);
```

- **Threshold**: 17,280 ledgers (~24 hours)
- **Max extension**: 120,960 ledgers (~7 days)
- **Trigger**: Every state modification (create, pause, resume, cancel, withdraw)
- **Rationale**: Active streams remain accessible; inactive streams may expire after ~7 days of no activity

### TTL Implications

- **Active streams**: TTL automatically extended on withdrawals and state changes
- **Inactive streams**: May expire after max TTL period without interaction
- **Completed/Cancelled streams**: No automatic TTL extension after terminal state
- **Recovery**: Expired streams cannot be recovered; data is permanently lost

## Storage Access Patterns

### Read Operations (View Functions)

- `get_config()` → reads `Config` from instance storage
- `get_stream_state(stream_id)` → reads `Stream(stream_id)` from persistent storage
- `calculate_accrued(stream_id)` → reads `Stream(stream_id)` from persistent storage

### Write Operations (State Mutations)

- `init()` → writes `Config` and `NextStreamId` to instance storage
- `create_stream()` → reads/writes `NextStreamId`, writes `Stream(stream_id)`
- `pause_stream()` → reads/writes `Stream(stream_id)`
- `resume_stream()` → reads/writes `Stream(stream_id)`
- `cancel_stream()` → reads/writes `Stream(stream_id)`
- `withdraw()` → reads/writes `Stream(stream_id)`

## Storage Cost Considerations

### Instance Storage
- Fixed cost: 2 keys regardless of stream count
- Minimal storage footprint (~100 bytes total)
- Shared across all operations

### Persistent Storage
- Linear growth: 1 key per stream
- Per-stream footprint: ~200-300 bytes (depends on address sizes)
- Unbounded growth potential
- TTL maintenance required for long-lived streams

### Optimization Notes
- Stream IDs are sequential `u64` values (efficient key space)
- No secondary indexes (trade-off: no enumeration, but O(1) lookups)
- No stream deletion (terminal states remain in storage until TTL expiration)
- Consider archiving completed/cancelled streams off-chain for historical queries

## Security Considerations

- **Immutable config**: Token and admin addresses cannot be changed after `init()`
- **Atomic operations**: All state changes are transactional (no partial updates)
- **Key isolation**: Each stream has independent storage (no cross-stream interference)
- **TTL protection**: Active streams automatically maintain their TTL through normal usage

## Future Enhancements

Potential storage improvements for future versions:
- Stream enumeration support (e.g., by sender or recipient)
- Configurable TTL policies per stream
- Stream archival mechanism for completed streams
- Storage rent payment automation
