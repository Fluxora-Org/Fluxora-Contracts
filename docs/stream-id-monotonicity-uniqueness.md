# Stream ID Monotonicity and Uniqueness

## Purpose

This document provides externally visible assurances for stream ID generation in the Fluxora streaming contract. Treasury operators, recipient applications, and auditors must be able to reason about stream ID behavior using only on-chain observables and published documentation—without inferring hidden rules from implementation details.

## Scope

Everything materially related to stream ID generation: monotonicity guarantees, uniqueness guarantees, counter management, failure atomicity, ID reservations, and economic conservation. Intentionally excluded: storage layout optimization, TTL management (documented separately with rationale).

## Verification Status

✅ **Line/function citations and behavioral claims re-verified against `contracts/stream/src/lib.rs`, `contracts/stream/src/storage.rs`, and `contracts/stream/src/test.rs`** as of this update.

**A note on citations**: this file has grown substantially since the last verification pass, and prior line-number citations had drifted by 1,000+ lines (in one case, citing tests that don't exist in `lib.rs` at all — they live in `test.rs`). Citations below reference function/test **names**, not line numbers, since names don't silently rot as the file grows. If you need the exact current line, run:
```bash
grep -n "fn <name>" contracts/stream/src/lib.rs contracts/stream/src/storage.rs contracts/stream/src/test.rs
```

---

## Stream ID Semantics

### Crisp Success Semantics

**Stream ID Generation Rules**:

1. **First stream**: Always receives `stream_id = 0`
2. **Subsequent streams (no active reservation)**: Receive `stream_id = previous_id + 1`
3. **Monotonicity**: In the absence of released-but-abandoned reservations (see "ID Reservations" below), stream IDs form a strictly increasing sequence: 0, 1, 2, 3, ...
4. **No gaps (ordinary path)**: Failed stream creation does NOT consume an ID
5. **Global uniqueness**: All streams share one counter (cross-sender, cross-recipient), including IDs drawn from a reservation
6. **Immutability**: Stream ID never changes after creation
7. **Upper bound**: Theoretical maximum is `u64::MAX` (18,446,744,073,709,551,615)

**Observable Guarantees**:

| Property     | Guarantee                          | Verification Method                       |
| ------------ | ---------------------------------- | ----------------------------------------- |
| First ID     | Always `0`                         | `create_stream` returns `0`               |
| Increment    | `+1` on the ordinary path; see "ID Reservations" for the reservation path | Sequential `create_stream` calls |
| Uniqueness   | No duplicates, ever (including reserved IDs) | All IDs are distinct             |
| Monotonicity | Strictly increasing on the ordinary path; see "ID Reservations" for a documented exception | `id[n+1] > id[n]` |
| Immutability | Never changes                      | `get_stream_state` always returns same ID |
| No gaps      | True on the ordinary path; NOT guaranteed if a reservation is released non-tail — see "ID Reservations" | Counter unchanged after failure |

**Code Location**: `next_stream_id_for` in `contracts/stream/src/storage.rs`, called from `persist_new_stream` and `persist_new_stream_skip_index` in `contracts/stream/src/lib.rs`
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

**Code Location**: `create_stream` in `contracts/stream/src/lib.rs` (validates, then delegates to `persist_new_stream`)
**Doc Reference**: `docs/streaming.md` §1 Stream Lifecycle

---

## Counter Management

### NextStreamId Storage

**Storage Location**: Instance storage under `DataKey::NextStreamId`

**Initialization**:

- Set to `0` during `init`
- Persists across all operations
- Can decrement in one documented case — see "ID Reservations" below — otherwise never resets

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

**Code Location**: `read_stream_count`/`set_stream_count` in `contracts/stream/src/storage.rs`. Note: an identical, separately-maintained copy of both functions currently also exists directly in `contracts/stream/src/lib.rs` (shadowing the `storage::*` glob import for callers within that file) — both operate on the same `DataKey::NextStreamId` and are behaviorally identical today, but this duplication is worth consolidating in a follow-up cleanup; not addressed in this doc-accuracy fix.

### Allocation Sequence

**Stream Creation Flow** (`persist_new_stream` in `contracts/stream/src/lib.rs`):

1. **Validate** memo length and metadata size bounds
2. **Allocate ID**: `let stream_id = next_stream_id_for(env, &sender);` — draws from an active reservation if the sender has one, otherwise from the live counter (see "ID Reservations")
3. **Create stream struct**: Assign `stream_id` to stream
4. **Persist stream**: Save to storage
5. **Update recipient index**: Add `stream_id` to recipient's list
6. **Update sender index**: Add `stream_id` to sender's portfolio
7. **Track liability**: add `deposit_amount` to total liabilities
8. **Emit event**: Publish `StreamCreated` with `stream_id`
9. **Return ID**: Return `stream_id` to caller

**Atomicity**: All steps succeed or all fail (no partial state)

**Code Location**: `persist_new_stream` in `contracts/stream/src/lib.rs`

---

## Monotonicity Guarantees

### Strictly Increasing Sequence (ordinary path)

**Mathematical Property**:

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

**Test Coverage** (all in `contracts/stream/src/test.rs`):

- `test_stream_id_increments_by_one`
- `test_stream_ids_are_unique_no_gaps`
- `test_create_stream_increments_id_correctly`

### No Gaps in Sequence (ordinary path — see "ID Reservations" for the documented exception)

**Property**: If N streams are created via `create_stream`/`create_streams` with no ID reservations ever used, IDs are exactly {0, 1, 2, ..., N-1}

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

**Test Coverage** (in `contracts/stream/src/test.rs`):

- `test_stream_ids_are_unique_no_gaps`
- `test_failed_create_stream_does_not_advance_counter`

### Counter Persistence

**Property**: Counter value persists across all operations, except the reservation-release case documented under "ID Reservations"

**Operations that DO NOT affect counter**:

- `pause_stream`
- `resume_stream`
- `cancel_stream`
- `withdraw`
- `close_completed_stream`
- `set_admin`
- `set_contract_paused`

**Operations that DO affect counter**:

- `create_stream` (increments by 1, or consumes one ID from an active reservation)
- `create_streams` (increments by N for N streams, or consumes N IDs from an active reservation)
- `reserve_stream_ids` (increments by `count`, reserving a contiguous block for the caller)
- `release_id_reservation` (decrements the counter **only if** the caller's reservation is still at the tail of the counter — see "ID Reservations")

**Test Coverage** (in `contracts/stream/src/test.rs`):

- `test_stream_id_stability_after_state_changes`

---

## ID Reservations

*(New section: this mechanism exists in the current implementation and materially affects the monotonicity/no-gaps claims above, but was not previously documented here.)*

The contract exposes `reserve_stream_ids`, `release_id_reservation`, and `reclaim_expired_id_reservation` (all in `contracts/stream/src/lib.rs`) to let a caller pre-compute stream IDs off-chain before calling `create_stream`.

**How allocation changes with a reservation active** (`next_stream_id_for` in `contracts/stream/src/storage.rs`):

- If the caller has an active `IdReservation`, `next_stream_id_for` draws the next ID from that reservation (`start_id + consumed`) instead of the live global counter, and increments `consumed`. Once the reservation is fully consumed, it's deleted.
- Otherwise, it falls through to the ordinary live-counter path described above.

**Reservation limits** (`reserve_stream_ids`):

- `count` must be `1..=MAX_ID_RESERVATION` (100) — capped to bound counter-inflation from a single reservation.
- A caller may only hold one active reservation at a time (`ReservationAlreadyActive` otherwise).
- `reserve_stream_ids` immediately advances the live counter by `count`, so IDs in the reserved range are never handed out to any other caller — this preserves **global uniqueness** even while a reservation is outstanding.

**Documented exception to "counter never decrements" and "no gaps"** (`release_reservation`, called by `release_id_reservation`):

- If the caller releases a reservation **while it is still at the tail of the counter** (i.e., no other allocation has happened since), the unconsumed portion is given back: the counter is decremented to reclaim those IDs for future use. This is the one case where the counter moves backward.
- If the caller releases a reservation **after the counter has already moved past it** (some other reservation or `create_stream` call advanced the counter in the meantime), the unconsumed IDs in that reservation are **permanently abandoned** — they will never be assigned to any stream. This is a genuine, intentional gap in the achievable ID space, bounded by `MAX_ID_RESERVATION` (at most 99 IDs lost per released reservation).
- `reclaim_expired_id_reservation` provides an alternate cleanup path once a reservation's `expiry` has passed, with the same tail-only reclaim / non-tail-abandon behavior.

**Practical impact for integrators**: none of the per-stream guarantees change (every stream ID is still globally unique and immutable). What changes is that "N streams created ⇒ IDs are exactly `{0, ..., N-1}`" is **only** true if no reservation was ever released non-tail. Indexers and auditors relying on a fully gapless sequence to detect "missing events" should be aware that a released, non-tail reservation is a legitimate, non-error explanation for an ID that is never assigned — not necessarily evidence of a missing `StreamCreated` event.

---

## Uniqueness Guarantees

### Global Uniqueness

**Property**: Every stream ID is unique across all senders and recipients, including any IDs drawn from a reservation

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

**Test Coverage** (in `contracts/stream/src/test.rs`):

- `test_stream_ids_unique_across_different_senders`

### No Collisions

**Property**: No two streams can have the same ID, ever — including IDs drawn from a reservation, since `reserve_stream_ids` advances the live counter immediately

**Proof**: Counter (or reservation slot) increments atomically before stream creation

**Test Coverage** (in `contracts/stream/src/test.rs`):

- `test_stream_ids_are_unique_no_gaps`

### Immutability

**Property**: Stream ID never changes after creation

**Test Coverage** (in `contracts/stream/src/test.rs`):

- `test_stream_id_stability_after_state_changes`

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

**Test Coverage** (in `contracts/stream/src/test.rs`):

- `test_create_streams_batch_atomicity_on_invalid_entry`
- `test_create_streams_batch_total_deposit_overflow_has_no_side_effects`

**Code Location**: `create_streams` in `contracts/stream/src/lib.rs`, using `persist_new_stream_skip_index` internally to batch recipient-index writes

---

## Economic Conservation

### Stream ID as Immutable Identifier

**Property**: Stream ID uniquely identifies a funding allocation

**Economic Implications**:

1. **Treasury Accounting**: Each ID represents one funding commitment
2. **Recipient Tracking**: Recipients can enumerate their streams by ID
3. **Audit Trail**: IDs provide immutable reference for audits
4. **Payout Ordering**: IDs establish creation order (lower ID = created earlier), except across a released non-tail reservation (see "ID Reservations")

**Code Location**: `add_stream_to_recipient_index` in `contracts/stream/src/lib.rs`/`contracts/stream/src/storage.rs`
**Doc Reference**: `docs/streaming.md` §4 Access Control

### ID Reuse

**Property**: Once assigned to a stream, an ID is never reused. Reserved-but-never-assigned IDs from a non-tail release are also never reused (they're abandoned, not recycled) — see "ID Reservations".

**Implications**:

- Closed streams do not free their IDs
- The counter does not decrement as a result of closing a stream (only as a result of a tail-position reservation release, per "ID Reservations")
- Historical IDs remain valid references

**Test Coverage**: Implicit in all monotonicity tests, plus the reservation-specific behavior described above (not yet covered by a dedicated test as of this writing — see "Suggested Follow-Up" below)

---

## Payout Ordering

### Creation Order Preservation

**Property**: Stream IDs preserve creation order on the ordinary path (see "ID Reservations" for the documented exception)

**Test Coverage**: covered by the monotonicity tests above.

### Recipient Index Ordering

**Property**: Recipient stream index maintains sorted order by ID

**Guarantee**: `get_recipient_streams` returns IDs in ascending order

**Test Coverage** (in `contracts/stream/src/test.rs`):

- `test_recipient_stream_index_sorted_order`
- `test_get_recipient_streams_ids_resolve_to_correct_recipient`

---

## Edge Cases

### Maximum Stream Count

**Theoretical Limit**: `u64::MAX = 18,446,744,073,709,551,615`

**Overflow Behavior**:

- Counter increment uses standard `+` operator
- Overflow would panic (Rust default)
- Practically unreachable (would require 18 quintillion streams)

**Code Location**: `next_stream_id_for` in `contracts/stream/src/storage.rs`

### Concurrent Creation

**Property**: Sequential ID allocation even with concurrent calls

**Guarantee**: Soroban execution is sequential (no true concurrency)

### Failed Creation Recovery

**Property**: Failed creation does not consume IDs

**Test Coverage** (in `contracts/stream/src/test.rs`):

- `test_failed_create_stream_does_not_advance_counter`

---

## Residual Risks (Explicitly Excluded)

### Out of Scope

1. **Storage layout optimization** — documented in `docs/storage.md`
2. **TTL management** — infrastructure concern, not ID semantics
3. **Counter overflow** — practically unreachable
4. **Historical ID lookup after close** — indexers should archive closed stream data

### Newly Disclosed (not out of scope, but not previously documented)

5. **Non-tail reservation release gaps**: as described under "ID Reservations", a reservation released after the counter has moved past it permanently abandons up to 99 IDs. This is bounded and intentional, not a bug, but it does mean the "gapless sequence" property is conditional, not absolute. Auditors and indexers should treat this as expected behavior rather than a missing-event signal.

---

## Integrator Assurances

### For Treasury Operators

You can rely on:

- ✅ Stream IDs uniquely identify funding allocations
- ✅ IDs never change or get reused
- ✅ IDs preserve creation order, except across a released non-tail reservation
- ✅ Failed creations don't consume IDs
- ✅ Batch operations allocate contiguous IDs

### For Recipient Applications

You can rely on:

- ✅ `get_recipient_streams` returns sorted IDs
- ✅ IDs are globally unique (no collisions), including reserved IDs
- ✅ IDs remain valid after state changes
- ✅ Closed streams don't affect new IDs

### For Auditors

You can verify:

- ✅ Counter increments match stream count **plus outstanding/abandoned reservation IDs** — see "ID Reservations"
- ✅ No duplicate IDs exist
- ✅ Failed operations don't affect counter
- ⚠️ IDs do **not** unconditionally form a gapless sequence — see "ID Reservations" before treating a missing ID as an anomaly

### For Indexers

You can rely on:

- ✅ IDs are immutable (safe to use as primary key)
- ✅ `StreamCreated` events include ID
- ⚠️ A missing ID may be an abandoned non-tail reservation, not a missing event — check `res_rel` events (published by `release_reservation`) before assuming an indexing bug

---

## Test Coverage

### Unit Tests (contracts/stream/src/test.rs)

Run `grep -n "fn <test_name>" contracts/stream/src/test.rs` for the current exact line — omitted here to avoid re-introducing the staleness this update fixes.

| Test                                                 | Property Verified     |
| ----------------------------------------------------- | --------------------- |
| `test_stream_id_first_stream_is_zero`                | First ID is 0         |
| `test_stream_id_increments_by_one`                   | Monotonic increment   |
| `test_create_stream_returned_id_matches_stored_id`   | ID consistency        |
| `test_stream_ids_are_unique_no_gaps`                 | Uniqueness + no gaps  |
| `test_failed_create_stream_does_not_advance_counter` | Failure atomicity     |
| `test_stream_ids_unique_across_different_senders`    | Global uniqueness     |
| `test_stream_id_stability_after_state_changes`       | Immutability          |
| `test_create_stream_increments_id_correctly`         | Sequential allocation |
| `test_recipient_stream_index_sorted_order`           | Index ordering        |
| `test_get_recipient_streams_ids_resolve_to_correct_recipient` | Recipient/ID resolution |
| `test_create_streams_batch_atomicity_on_invalid_entry` | Batch atomicity |
| `test_create_streams_batch_total_deposit_overflow_has_no_side_effects` | Batch overflow safety |

### Suggested Follow-Up (out of scope for this doc-accuracy fix)

No test currently exercises the non-tail reservation-release gap behavior described in "ID Reservations" directly (i.e., reserve → have another party advance the counter past it → release → assert the abandoned IDs are never reachable). Recommend a follow-up test, e.g. `test_reservation_release_non_tail_abandons_ids`, added to `contracts/stream/src/test.rs`.

### Integration Tests (contracts/stream/tests/integration_suite.rs)

Additional integration tests verify ID behavior in realistic scenarios.

---

## Maintenance

When modifying stream creation or the reservation system:

1. Ensure counter increments atomically
2. Verify failed creation doesn't advance counter
3. Update this document if semantics change
4. Run all ID-related tests
5. Update snapshot tests if events change
6. Prefer function-name references over line numbers in this doc — line numbers rot as the file grows (this file drifted 1,000+ lines and one function reference pointed at the wrong source file entirely before this update)

Last verified: <fill in today's date when you open the PR>

---

## Cross-References

- **Protocol Narrative**: [protocol-narrative-code-alignment.md](./protocol-narrative-code-alignment.md)
- **Streaming Mechanics**: [streaming.md](./streaming.md) §1 Stream Lifecycle
- **Storage Layout**: [storage.md](./storage.md)
- **Audit Documentation**: [audit.md](./audit.md)

