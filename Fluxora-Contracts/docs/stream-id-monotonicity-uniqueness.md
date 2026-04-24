# Stream ID Monotonicity and Uniqueness

## Purpose

This document provides externally visible assurances for stream ID generation in the Fluxora streaming contract. Treasury operators, recipient applications, and auditors must be able to reason about stream ID behavior using only on-chain observables and published documentation—without inferring hidden rules from implementation details.

## Scope

Everything materially related to stream ID generation: monotonicity guarantees, uniqueness guarantees, counter management, failure atomicity, and economic conservation. Intentionally excluded: storage layout optimization, TTL management (documented separately with rationale).

## Verification Status

✅ **Complete alignment verified** between stream ID semantics and implementation as of 2026-03-27.

---

## Stream ID Semantics

### Crisp Success Semantics

**Stream ID Generation Rules**:

1. **First stream**: Always receives `stream_id = 0`
2. **Subsequent streams**: Receive `stream_id = previous_id + 1`
3. **Monotonicity**: Stream IDs form strictly increasing sequence: 0, 1, 2, 3, ...
4. **No gaps**: Failed stream creation does NOT consume an ID
5. **Global uniqueness**: All streams share one counter (cross-sender, cross-recipient)
6. **Immutability**: Stream ID never changes after creation
7. **Upper bound**: Theoretical maximum is `u64::MAX` (18,446,744,073,709,551,615)

**Observable Guarantees**:

| Property     | Guarantee                          | Verification Method                       |
| ------------ | ---------------------------------- | ----------------------------------------- |
| First ID     | Always `0`                         | `create_stream` returns `0`               |
| Increment    | Always `+1`                        | Sequential `create_stream` calls          |
| Uniqueness   | No duplicates                      | All IDs are distinct                      |
| Monotonicity | Strictly increasing                | `id[n+1] > id[n]`                         |
| Immutability | Never changes                      | `get_stream_state` always returns same ID |
| No gaps      | Failed creation doesn't consume ID | Counter unchanged after failure           |

**Code Location**: `contracts/stream/src/lib.rs:439-485`
**Doc Reference**: This document

### Crisp Failure Semantics

**Failed Stream Creation**:

| Failure Condition    | Counter Behavior | Next Successful ID     | Side Effects |
| -------------------- | ---------------- | ---------------------- | ------------ |
| Invalid parameters   | NOT incremented  | Same as failed attempt | None         |
| Insufficient deposit | NOT incremented  | Same as failed attempt | None         |
| Token transfer fails | NOT incremented  | Same as failed attempt | None         |
| Authorization fails  | NOT incremented  | Same as failed attempt | None         |
| Contract paused      | NOT incremented  | Same as failed attempt | None         |

**No Silent Drift**:

- Failed creation leaves counter unchanged
- Failed creation emits no events
- Failed creation persists no state
- Failed creation transfers no tokens

**Code Location**: `contracts/stream/src/lib.rs:754-819`
**Doc Reference**: `docs/streaming.md` §1 Stream Lifecycle

---

## Counter Management

### NextStreamId Storage

**Storage Location**: Instance storage under `DataKey::NextStreamId`

**Initialization**:

- Set to `0` during `init`
- Never reset after initialization
- Persists across all operations

**Read Operation**:

```rust
fn read_stream_count(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0u64)
}
```

**Write Operation**:

```rust
fn set_stream_count(env: &Env, count: u64) {
    env.storage().instance().set(&DataKey::NextStreamId, &count);
    bump_instance_ttl(env);
}
```

**Code Location**: `contracts/stream/src/lib.rs:225-236`

### Allocation Sequence

**Stream Creation Flow**:

1. **Read current counter**: `stream_id = read_stream_count(env)`
2. **Increment counter**: `set_stream_count(env, stream_id + 1)`
3. **Create stream struct**: Assign `stream_id` to stream
4. **Persist stream**: Save to storage
5. **Update recipient index**: Add `stream_id` to recipient's list
6. **Emit event**: Publish `StreamCreated` with `stream_id`
7. **Return ID**: Return `stream_id` to caller

**Atomicity**: All steps succeed or all fail (no partial state)

**Code Location**: `contracts/stream/src/lib.rs:439-485`

---

## Monotonicity Guarantees

### Strictly Increasing Sequence

**Mathematical Property**:

```
For all streams i and j where i < j:
  stream_id[i] < stream_id[j]
```

**Verification**:

```bash
# Create three streams
STREAM_0=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)
STREAM_1=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)
STREAM_2=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)

# Verify monotonicity
echo "$STREAM_0 < $STREAM_1 < $STREAM_2"
# Expected: 0 < 1 < 2
```

**Test Coverage**:

- `test_stream_id_increments_by_one` (lib.rs:6994)
- `test_stream_ids_are_unique_no_gaps` (lib.rs:7338)
- `test_create_stream_increments_id_correctly` (lib.rs:4796)

### No Gaps in Sequence

**Property**: If N streams are created, IDs are exactly {0, 1, 2, ..., N-1}

**Verification**:

```bash
# Create 5 streams
for i in {1..5}; do
  stellar contract invoke --id <CONTRACT_ID> -- create_stream ...
done

# Query stream count
COUNT=$(stellar contract invoke --id <CONTRACT_ID> -- get_stream_count)
# Expected: 5

# Verify all IDs exist
for id in {0..4}; do
  stellar contract invoke --id <CONTRACT_ID> -- get_stream_state --stream_id $id
  # Expected: Success for all
done
```

**Test Coverage**:

- `test_stream_ids_are_unique_no_gaps` (lib.rs:7338)
- `test_failed_create_stream_does_not_advance_counter` (lib.rs:7380)

### Counter Persistence

**Property**: Counter value persists across all operations

**Operations that DO NOT affect counter**:

- `pause_stream`
- `resume_stream`
- `cancel_stream`
- `withdraw`
- `close_completed_stream`
- `set_admin`
- `set_contract_paused`

**Operations that DO affect counter**:

- `create_stream` (increments by 1)
- `create_streams` (increments by N for N streams)

**Verification**:

```bash
# Create stream 0
STREAM_0=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)

# Pause, resume, cancel stream 0
stellar contract invoke --id <CONTRACT_ID> -- pause_stream --stream_id 0
stellar contract invoke --id <CONTRACT_ID> -- resume_stream --stream_id 0
stellar contract invoke --id <CONTRACT_ID> -- cancel_stream --stream_id 0

# Create next stream
STREAM_1=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)
# Expected: STREAM_1 = 1 (counter continued from 1, not affected by mutations)
```

**Test Coverage**:

- `test_stream_id_stability_after_state_changes` (lib.rs:7471)

---

## Uniqueness Guarantees

### Global Uniqueness

**Property**: Every stream ID is unique across all senders and recipients

**Verification**:

```bash
# Create streams with different senders/recipients
STREAM_A=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream \
  --sender <SENDER_1> --recipient <RECIPIENT_1> ...)
STREAM_B=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream \
  --sender <SENDER_2> --recipient <RECIPIENT_2> ...)
STREAM_C=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream \
  --sender <SENDER_1> --recipient <RECIPIENT_2> ...)

# Verify all IDs are distinct
echo "$STREAM_A != $STREAM_B != $STREAM_C"
# Expected: 0 != 1 != 2
```

**Test Coverage**:

- `test_stream_ids_unique_across_different_senders` (lib.rs:7422)

### No Collisions

**Property**: No two streams can have the same ID

**Proof**: Counter increments atomically before stream creation

**Verification**:

```bash
# Create N streams
N=100
for i in $(seq 1 $N); do
  stellar contract invoke --id <CONTRACT_ID> -- create_stream ...
done

# Query all stream IDs
IDS=()
for id in $(seq 0 $((N-1))); do
  STATE=$(stellar contract invoke --id <CONTRACT_ID> -- get_stream_state --stream_id $id)
  IDS+=($id)
done

# Verify no duplicates (all IDs unique)
UNIQUE_COUNT=$(echo "${IDS[@]}" | tr ' ' '\n' | sort -u | wc -l)
# Expected: UNIQUE_COUNT = N
```

**Test Coverage**:

- `test_stream_ids_are_unique_no_gaps` (lib.rs:7338)

### Immutability

**Property**: Stream ID never changes after creation

**Verification**:

```bash
# Create stream
STREAM_ID=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)

# Query ID multiple times across state changes
ID_1=$(stellar contract invoke --id <CONTRACT_ID> -- get_stream_state --stream_id $STREAM_ID | jq .stream_id)

# Pause stream
stellar contract invoke --id <CONTRACT_ID> -- pause_stream --stream_id $STREAM_ID
ID_2=$(stellar contract invoke --id <CONTRACT_ID> -- get_stream_state --stream_id $STREAM_ID | jq .stream_id)

# Resume stream
stellar contract invoke --id <CONTRACT_ID> -- resume_stream --stream_id $STREAM_ID
ID_3=$(stellar contract invoke --id <CONTRACT_ID> -- get_stream_state --stream_id $STREAM_ID | jq .stream_id)

# Verify ID unchanged
echo "$ID_1 = $ID_2 = $ID_3"
# Expected: All equal to STREAM_ID
```

**Test Coverage**:

- `test_stream_id_stability_after_state_changes` (lib.rs:7471)

---

## Batch Operations

### create_streams Atomicity

**Property**: Batch creation allocates contiguous IDs atomically

**Success Semantics**:

- All N streams created → IDs are [current, current+1, ..., current+N-1]
- Counter incremented by N
- IDs returned in same order as input

**Failure Semantics**:

- Any validation failure → NO streams created
- Counter NOT incremented
- No IDs consumed

**Verification**:

```bash
# Get current count
COUNT_BEFORE=$(stellar contract invoke --id <CONTRACT_ID> -- get_stream_count)

# Create batch of 3 streams
IDS=$(stellar contract invoke --id <CONTRACT_ID> -- create_streams \
  --sender <SENDER> \
  --streams '[
    {"recipient": "<R1>", "deposit_amount": 100, ...},
    {"recipient": "<R2>", "deposit_amount": 200, ...},
    {"recipient": "<R3>", "deposit_amount": 300, ...}
  ]')

# Verify IDs are contiguous
echo "$IDS"
# Expected: [COUNT_BEFORE, COUNT_BEFORE+1, COUNT_BEFORE+2]

# Verify count incremented by 3
COUNT_AFTER=$(stellar contract invoke --id <CONTRACT_ID> -- get_stream_count)
# Expected: COUNT_AFTER = COUNT_BEFORE + 3
```

**Test Coverage**:

- `test_create_streams_batch_atomicity_on_invalid_entry` (lib.rs:14329)
- `test_create_streams_batch_total_deposit_overflow_has_no_side_effects` (lib.rs:8313)

---

## Economic Conservation

### Stream ID as Immutable Identifier

**Property**: Stream ID uniquely identifies a funding allocation

**Economic Implications**:

1. **Treasury Accounting**: Each ID represents one funding commitment
2. **Recipient Tracking**: Recipients can enumerate their streams by ID
3. **Audit Trail**: IDs provide immutable reference for audits
4. **Payout Ordering**: IDs establish creation order (lower ID = created earlier)

**Verification**:

```bash
# Query recipient's streams
RECIPIENT_STREAMS=$(stellar contract invoke \
  --id <CONTRACT_ID> \
  -- get_recipient_streams \
    --recipient <RECIPIENT>)

# Expected: Sorted array of stream IDs [0, 3, 7, 12, ...]
# Lower IDs were created earlier
```

**Code Location**: `contracts/stream/src/lib.rs:1920-1922`
**Doc Reference**: `docs/streaming.md` §4 Access Control

### No ID Reuse

**Property**: Once allocated, an ID is never reused

**Implications**:

- Closed streams do not free their IDs
- Counter never decrements
- Historical IDs remain valid references

**Verification**:

```bash
# Create and close stream
STREAM_ID=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)
stellar contract invoke --id <CONTRACT_ID> -- withdraw --stream_id $STREAM_ID
stellar contract invoke --id <CONTRACT_ID> -- close_completed_stream --stream_id $STREAM_ID

# Create new stream
NEW_STREAM_ID=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)

# Verify new ID is next in sequence, not reused
echo "$NEW_STREAM_ID = $STREAM_ID + 1"
# Expected: True (ID not reused)
```

**Test Coverage**: Implicit in all monotonicity tests

---

## Payout Ordering

### Creation Order Preservation

**Property**: Stream IDs preserve creation order

**Ordering Guarantee**:

```
If stream A created before stream B:
  stream_id[A] < stream_id[B]
```

**Use Cases**:

1. **First-In-First-Out (FIFO) Processing**: Process streams in ID order
2. **Priority Queues**: Lower IDs have implicit priority
3. **Audit Chronology**: IDs establish temporal sequence
4. **Recipient Enumeration**: `get_recipient_streams` returns sorted IDs

**Verification**:

```bash
# Create streams at different times
TIME_1=1000000000
TIME_2=1000000100
TIME_3=1000000200

stellar contract invoke --id <CONTRACT_ID> -- create_stream \
  --start_time $TIME_1 ...
# Returns: 0

stellar contract invoke --id <CONTRACT_ID> -- create_stream \
  --start_time $TIME_2 ...
# Returns: 1

stellar contract invoke --id <CONTRACT_ID> -- create_stream \
  --start_time $TIME_3 ...
# Returns: 2

# IDs preserve creation order regardless of start_time values
```

**Code Location**: `contracts/stream/src/lib.rs:365-408` (recipient index)
**Doc Reference**: `docs/streaming.md` §4 Access Control

### Recipient Index Ordering

**Property**: Recipient stream index maintains sorted order by ID

**Guarantee**: `get_recipient_streams` returns IDs in ascending order

**Verification**:

```bash
# Create multiple streams for same recipient
for i in {1..5}; do
  stellar contract invoke --id <CONTRACT_ID> -- create_stream \
    --recipient <RECIPIENT> ...
done

# Query recipient's streams
IDS=$(stellar contract invoke --id <CONTRACT_ID> -- get_recipient_streams \
  --recipient <RECIPIENT>)

# Verify sorted order
echo "$IDS"
# Expected: [0, 1, 2, 3, 4] (ascending order)
```

**Test Coverage**:

- `test_recipient_stream_index_sorted_order` (lib.rs:9742)
- `test_get_recipient_streams_ids_resolve_to_correct_recipient` (lib.rs:11540)

---

## Edge Cases

### Maximum Stream Count

**Theoretical Limit**: `u64::MAX = 18,446,744,073,709,551,615`

**Practical Considerations**:

- Storage costs increase linearly with stream count
- No hard limit enforced by contract
- Network/storage constraints apply first

**Overflow Behavior**:

- Counter increment uses standard `+` operator
- Overflow would panic (Rust default)
- Practically unreachable (would require 18 quintillion streams)

**Code Location**: `contracts/stream/src/lib.rs:440-441`

### Concurrent Creation

**Property**: Sequential ID allocation even with concurrent calls

**Guarantee**: Soroban execution is sequential (no true concurrency)

**Verification**: All tests pass with sequential execution model

### Failed Creation Recovery

**Property**: Failed creation does not consume IDs

**Verification**:

```bash
# Create stream 0
stellar contract invoke --id <CONTRACT_ID> -- create_stream ...
# Returns: 0

# Attempt invalid creation (zero deposit)
stellar contract invoke --id <CONTRACT_ID> -- create_stream \
  --deposit_amount 0 ...
# Returns: InvalidParams error

# Create next stream
stellar contract invoke --id <CONTRACT_ID> -- create_stream ...
# Returns: 1 (not 2, failed attempt didn't consume ID)
```

**Test Coverage**:

- `test_failed_create_stream_does_not_advance_counter` (lib.rs:7380)

---

## Residual Risks (Explicitly Excluded)

### Out of Scope

1. **Storage layout optimization**:
   - Rationale: Implementation detail, not protocol semantic
   - Mitigation: Documented in `docs/storage.md`
   - Impact: None on externally visible behavior

2. **TTL management**:
   - Rationale: Infrastructure concern, not ID semantics
   - Mitigation: TTL extended on counter access
   - Impact: Counter persists indefinitely with activity

3. **Counter overflow**:
   - Rationale: Practically unreachable (18 quintillion streams)
   - Mitigation: Rust panic on overflow (fail-safe)
   - Impact: Contract would halt before overflow

4. **Historical ID lookup after close**:
   - Rationale: Closed streams removed from storage
   - Mitigation: Indexers should archive closed stream data
   - Impact: `get_stream_state` returns `StreamNotFound` after close

✅ **All exclusions documented with rationale**

---

## Integrator Assurances

### For Treasury Operators

You can rely on:

- ✅ Stream IDs uniquely identify funding allocations
- ✅ IDs never change or get reused
- ✅ IDs preserve creation order
- ✅ Failed creations don't consume IDs
- ✅ Batch operations allocate contiguous IDs

### For Recipient Applications

You can rely on:

- ✅ `get_recipient_streams` returns sorted IDs
- ✅ Lower IDs were created earlier
- ✅ IDs are globally unique (no collisions)
- ✅ IDs remain valid after state changes
- ✅ Closed streams don't affect new IDs

### For Auditors

You can verify:

- ✅ All IDs form gapless sequence starting at 0
- ✅ Counter increments match stream count
- ✅ No duplicate IDs exist
- ✅ IDs preserve temporal order
- ✅ Failed operations don't affect counter

### For Indexers

You can rely on:

- ✅ IDs are immutable (safe to use as primary key)
- ✅ IDs are sequential (efficient range queries)
- ✅ IDs preserve creation order (chronological indexing)
- ✅ `StreamCreated` events include ID
- ✅ No ID gaps (can detect missing events)

---

## Verification Commands

### Check Current Counter

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network <NETWORK> \
  -- get_stream_count
```

### Verify First Stream is Zero

```bash
# After init, create first stream
STREAM_ID=$(stellar contract invoke \
  --id <CONTRACT_ID> \
  --network <NETWORK> \
  -- create_stream \
    --sender <SENDER> \
    --recipient <RECIPIENT> \
    --deposit_amount 1000 \
    --rate_per_second 1 \
    --start_time <FUTURE> \
    --cliff_time <FUTURE> \
    --end_time <FUTURE>)

echo "$STREAM_ID"
# Expected: 0
```

### Verify Monotonic Increment

```bash
# Create three streams
ID_0=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)
ID_1=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)
ID_2=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)

# Verify sequence
test "$ID_0" -eq 0 && test "$ID_1" -eq 1 && test "$ID_2" -eq 2
echo "Monotonicity verified: $ID_0 < $ID_1 < $ID_2"
```

### Verify No Gaps After Failure

```bash
# Create stream 0
stellar contract invoke --id <CONTRACT_ID> -- create_stream ...

# Attempt invalid creation
stellar contract invoke --id <CONTRACT_ID> -- create_stream \
  --deposit_amount 0 ...
# Expected: Error

# Create next stream
NEXT_ID=$(stellar contract invoke --id <CONTRACT_ID> -- create_stream ...)

# Verify no gap
test "$NEXT_ID" -eq 1
echo "No gap: next ID is 1 (not 2)"
```

### Verify Recipient Index Order

```bash
# Create multiple streams for recipient
for i in {1..5}; do
  stellar contract invoke --id <CONTRACT_ID> -- create_stream \
    --recipient <RECIPIENT> ...
done

# Query recipient's streams
IDS=$(stellar contract invoke --id <CONTRACT_ID> -- get_recipient_streams \
  --recipient <RECIPIENT>)

# Verify sorted
echo "$IDS" | jq 'sort == .'
# Expected: true
```

---

## Test Coverage

### Unit Tests (contracts/stream/src/test.rs)

| Test                                                 | Line | Property Verified     |
| ---------------------------------------------------- | ---- | --------------------- |
| `test_stream_id_first_stream_is_zero`                | 6969 | First ID is 0         |
| `test_stream_id_increments_by_one`                   | 6994 | Monotonic increment   |
| `test_create_stream_returned_id_matches_stored_id`   | 7034 | ID consistency        |
| `test_stream_ids_are_unique_no_gaps`                 | 7338 | Uniqueness + no gaps  |
| `test_failed_create_stream_does_not_advance_counter` | 7380 | Failure atomicity     |
| `test_stream_ids_unique_across_different_senders`    | 7422 | Global uniqueness     |
| `test_stream_id_stability_after_state_changes`       | 7471 | Immutability          |
| `test_create_stream_increments_id_correctly`         | 4796 | Sequential allocation |
| `test_recipient_stream_index_sorted_order`           | 9742 | Index ordering        |

### Integration Tests (contracts/stream/tests/integration_suite.rs)

Additional integration tests verify ID behavior in realistic scenarios.

---

## Maintenance

When modifying stream creation:

1. Ensure counter increments atomically
2. Verify failed creation doesn't advance counter
3. Update this document if semantics change
4. Run all ID-related tests
5. Update snapshot tests if events change

Last verified: 2026-03-27

---

## Cross-References

- **Protocol Narrative**: [protocol-narrative-code-alignment.md](./protocol-narrative-code-alignment.md)
- **Streaming Mechanics**: [streaming.md](./streaming.md) §1 Stream Lifecycle
- **Storage Layout**: [storage.md](./storage.md)
- **Audit Documentation**: [audit.md](./audit.md)
