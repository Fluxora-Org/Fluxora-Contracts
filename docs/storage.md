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
| `Config` | `Config` struct | Contains `token` address and `admin` address | `init()` | `set_admin()` (admin key rotation) |
| `NextStreamId` | `u64` | Auto-incrementing counter for stream IDs | `init()` (set to 0) | `create_stream()` (incremented) |

**Characteristics:**
- Shared across all contract operations
- Low cardinality (only 2 keys)
- TTL extended on **every** read and write (see TTL Policy below)
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
- TTL extended on every **read and write** operation

## TTL (Time To Live) Policy

### Constants

All TTL values are defined as named constants for maintainability:

```rust
/// Minimum remaining TTL (in ledgers) before we bump.  ~1 day at 5 s/ledger.
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;
/// Extend to ~7 days of ledgers when bumping instance storage.
const INSTANCE_BUMP_AMOUNT: u32 = 120_960;
/// Minimum remaining TTL for persistent (stream) entries.
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 17_280;
/// Extend persistent entries to ~7 days of ledgers.
const PERSISTENT_BUMP_AMOUNT: u32 = 120_960;
```

### Instance Storage TTL

Instance TTL is extended via the `bump_instance_ttl()` helper, which is called on **every** entry-point that reads or writes instance storage:

```rust
fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}
```

**Extension points (exhaustive):**

| Function | Trigger |
|----------|---------|
| `init()` | After initial writes |
| `get_config()` | On every read of Config |
| `get_stream_count()` | On every read of NextStreamId |
| `set_stream_count()` | After writing NextStreamId |
| `set_admin()` | After updating Config with new admin |

- **Threshold**: 17,280 ledgers (~24 hours at 5s/ledger)
- **Max extension**: 120,960 ledgers (~7 days)
- **Rationale**: Any contract interaction — whether a read (e.g., `get_config`, `calculate_accrued`) or a write — refreshes the instance TTL.  This prevents the contract from becoming unusable due to storage expiration on testnet or mainnet, even during periods of low activity.

### Persistent Storage TTL

Extended on **every** stream load (`load_stream()`) and save (`save_stream()`):

```rust
// On read:
env.storage().persistent().extend_ttl(&key, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);

// On write:
env.storage().persistent().extend_ttl(&key, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
```

- **Threshold**: 17,280 ledgers (~24 hours)
- **Max extension**: 120,960 ledgers (~7 days)
- **Trigger**: Every read or write (create, pause, resume, cancel, withdraw, get_stream_state, calculate_accrued)
- **Rationale**: Both active and queried streams remain accessible. A UI polling `calculate_accrued` or `get_stream_state` will keep the stream alive.

### TTL Implications

- **Active streams**: TTL refreshed on any interaction (reads or writes)
- **Queried streams**: TTL refreshed when viewed via `get_stream_state` or `calculate_accrued`
- **Inactive streams**: May expire after ~7 days with **zero** interaction
- **Completed/Cancelled streams**: TTL still refreshed when queried; expire only if nobody reads them for 7 days
- **Recovery**: Expired entries cannot be recovered; data is permanently lost
- **Contract liveness**: Because instance TTL is bumped on every entry-point, the contract itself (Config + NextStreamId) stays alive as long as any function is called at least once per 7 days

## Storage Access Patterns

### Read Operations (View Functions)

- `get_config()` → reads `Config` from instance storage, **bumps instance TTL**
- `get_stream_state(stream_id)` → reads `Stream(stream_id)` from persistent storage, **bumps stream TTL**
- `calculate_accrued(stream_id)` → reads `Stream(stream_id)` from persistent storage, **bumps stream TTL**

### Write Operations (State Mutations)

- `init()` → writes `Config` and `NextStreamId` to instance storage, **bumps instance TTL**
- `create_stream()` → reads/writes `NextStreamId`, writes `Stream(stream_id)`, **bumps both TTLs**
- `pause_stream()` → reads/writes `Stream(stream_id)`, **bumps both stream and instance TTLs**
- `resume_stream()` → reads/writes `Stream(stream_id)`, **bumps both stream and instance TTLs**
- `cancel_stream()` → reads/writes `Stream(stream_id)`, **bumps both stream and instance TTLs**
- `withdraw()` → reads/writes `Stream(stream_id)`, **bumps both stream and instance TTLs**
- `set_admin()` → writes `Config`, **bumps instance TTL**

## Storage Cost Considerations

### Instance Storage
- Fixed cost: 2 keys regardless of stream count
- Minimal storage footprint (~100 bytes total)
- Shared across all operations

### Persistent Storage
- Linear growth: 1 key per stream
- Per-stream footprint: ~200-300 bytes (depends on address sizes)
- Unbounded growth potential
- TTL maintenance automatic through usage

### Optimization Notes
- Stream IDs are sequential `u64` values (efficient key space)
- No secondary indexes (trade-off: no enumeration, but O(1) lookups)
- No stream deletion (terminal states remain in storage until TTL expiration)
- Consider archiving completed/cancelled streams off-chain for historical queries

## Security Considerations

- **Admin rotation**: Admin can be changed via `set_admin()` with current-admin authorization
- **Atomic operations**: All state changes are transactional (no partial updates)
- **Key isolation**: Each stream has independent storage (no cross-stream interference)
- **TTL protection**: Both reads and writes keep storage alive, preventing accidental expiration
- **No stale state**: TTL bumps on reads mean even monitoring/UI queries keep data fresh

## Future Enhancements

Potential storage improvements for future versions:
- Stream enumeration support (e.g., by sender or recipient)
- Configurable TTL policies per stream
- Stream archival mechanism for completed streams
- Storage rent payment automation
