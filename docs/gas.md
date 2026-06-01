# Gas / Budget Review: Hot Paths and Batching

This document characterises the Soroban CPU-instruction and memory-byte cost
profile for the three hot paths in the Fluxora streaming contract, explains the
batching design decisions, records the observable guarantees that integrators
and auditors can rely on, and documents the recommended safe batch-size limits.

---

## Gas Profiling Harness

A dedicated profiling test suite lives at `contracts/stream/tests/gas_profile.rs`.
It measures CPU instructions and memory bytes for `create_streams` and
`batch_withdraw` across batch sizes 1, 5, 10, 20, and 50.

Run with:

```bash
cargo test -p fluxora_stream --test gas_profile -- --nocapture
```

Each test resets the Soroban budget to unlimited before the measured call and
asserts against the documented guardrails below.

---

## Recommended Safe Batch Limits

These limits are derived from the profiling harness and leave headroom below
the Soroban network's per-transaction budget.

### `create_streams`

| Batch size | CPU guardrail | Memory guardrail | Notes |
|---|---|---|---|
| 1 | ≤ 2 000 000 | ≤ 1 000 000 | Baseline |
| 5 | ≤ 4 000 000 | ≤ 2 000 000 | |
| 10 | ≤ 6 000 000 | ≤ 3 000 000 | **Recommended default** |
| 20 | ≤ 12 000 000 | ≤ 5 000 000 | |
| 50 | ≤ 30 000 000 | ≤ 12 000 000 | Practical upper bound |

**Recommendation:** Use batches of ≤ 10 streams for routine treasury operations.
Batches of 20–50 are safe in isolation but leave less headroom for other
operations in the same transaction.

**Same-recipient penalty:** When all streams in a batch share the same recipient,
the `RecipientStreams` index is updated N times.
- **Legacy Flat List**: O(N) persistent I/O per update (reads/writes full list).
- **Paged Index**: O(1) persistent I/O per update (touches last page only).

For 10 streams to the same recipient, allow up to 8 000 000 CPU / 4 000 000 bytes if using flat list; paged index significantly reduces this overhead.

### `batch_withdraw`

| Batch size | CPU guardrail | Memory guardrail | Notes |
|---|---|---|---|
| 1 | ≤ 1 500 000 | ≤ 600 000 | Baseline |
| 5 | ≤ 4 000 000 | ≤ 2 000 000 | |
| 10 | ≤ 6 000 000 | ≤ 3 000 000 | |
| 20 | ≤ 10 000 000 | ≤ 4 000 000 | **Recommended default** |
| 50 | ≤ 25 000 000 | ≤ 10 000 000 | Practical upper bound |

**Recommendation:** Use batches of ≤ 20 streams for routine recipient withdrawals.

### `withdraw` (single stream)

| Metric | Guardrail |
|---|---|
| CPU instructions | ≤ 1 000 000 |
| Memory bytes | ≤ 500 000 |

---

## Hot paths

### 1. `withdraw` (single stream)

**Call pattern:** recipient calls once per stream per claim cycle.

**Work done per call:**
- 1× persistent storage read (`load_stream`)
- 1× `calculate_accrued` (pure arithmetic, no storage)
- 1× persistent storage write + TTL bump (`save_stream`) — only when `withdrawable > 0`
- 1× token `transfer` call (contract → recipient) — only when `withdrawable > 0`
- 1–2× event publishes (`withdrew`, optionally `completed`)

**Zero-withdrawable short-circuit:** when `accrued == withdrawn_amount` (before
cliff, or already fully withdrawn) the function returns `0` immediately — no
storage write, no token transfer, no event. This is the cheapest possible
outcome and is safe to call speculatively.

**Guardrail (unit + integration tests):** ≤ 1 000 000 CPU instructions,
≤ 500 000 memory bytes per single call.

---

### 2. `batch_withdraw` (N streams, one auth)

**Call pattern:** recipient calls once to drain multiple streams in one
transaction.

**Gas-saving vs N individual `withdraw` calls:**
- Authorization is paid **once** for the entire batch instead of once per stream.
- The Soroban auth overhead (signature verification, sub-invocation tree) is the
  dominant fixed cost per transaction; batching amortises it across all streams.
- `env.ledger().timestamp()` is called **once** before the loop and cached in
  `let now = env.ledger().timestamp()`. The cached value is threaded into
  `calculate_accrued_amount_checkpointed` for every stream, eliminating one
  host-function call per stream iteration (Issue #515).

**Work done per stream entry:**
- 1× persistent storage read (`load_stream`)
- 1× `calculate_accrued` (pure arithmetic)
- 1× persistent storage write + TTL bump — only when `withdrawable > 0`
- 1× token `transfer` — only when `withdrawable > 0`
- 1–2× event publishes — only when `withdrawable > 0`

**Status semantics in batch:**

| Stream status | Behaviour | Panics? |
|---|---|---|
| `Active` | Normal accrual + transfer | No |
| `Cancelled` | Accrual frozen at `cancelled_at`; transfers remaining accrued − withdrawn | No |
| `Completed` | Returns `amount = 0`; no transfer, no event | No |
| `Paused` | Returns `ContractError::InvalidState`; entire batch reverts | Yes |
| Wrong recipient | Returns `ContractError::Unauthorized`; entire batch reverts | Yes |

**Atomicity:** the batch is all-or-nothing. Any error (stream not found, wrong
recipient, paused stream) reverts all state changes and token transfers for the
entire call.

**Guardrail (integration tests):** ≤ 10 000 000 CPU instructions, ≤ 4 000 000
memory bytes for a 20-stream batch.

---

### 3. `create_streams` (N streams, single bulk token pull)

**Call pattern:** treasury operator creates multiple streams in one transaction.

**Gas-saving vs N individual `create_stream` calls:**
- Authorization is paid **once**.
- Token transfer is a **single** `transfer(sender → contract, total_deposit)`
  instead of one transfer per stream. This is the primary gas saving: each
  token transfer invokes the SAC contract and incurs its own CPU budget.

**Work done:**
- First pass: validate all entries (pure arithmetic, no storage, no token calls)
- 1× token `transfer` for the sum of all deposits
- Second pass: for each entry — 1× stream ID allocation, 1× persistent write,
  1× recipient-index update, 1× `created` event

**Atomicity:** validation failures, arithmetic overflow in total deposit, or
token transfer failure abort the entire call. No streams are created, no tokens
move, and no events are emitted.

**Guardrail (integration tests):** ≤ 6 000 000 CPU instructions, ≤ 3 000 000
memory bytes for a 10-stream batch.

---

## Invariants

1. **Accrual is pure.** `calculate_accrued` reads no storage and performs no
   token calls. It is safe to call from any context without budget concern.

2. **CEI ordering.** All three hot paths write state before making external
   token calls. This prevents reentrancy from observing stale state.

3. **Zero-amount paths skip I/O.** When `withdrawable == 0`, no storage write
   and no token transfer occur. Callers may speculatively invoke `withdraw` or
   include already-completed streams in `batch_withdraw` without wasting budget
   on I/O.

4. **Single token pull in `create_streams`.** The total deposit is computed with
   `checked_add` across all entries before any token interaction. Overflow in
   the sum returns `ContractError::ArithmeticOverflow` and is atomic.

5. **TTL bumps are bounded.** Every `load_stream` and `save_stream` call bumps
   the persistent entry TTL by at most `PERSISTENT_BUMP_AMOUNT` (120 960
   ledgers ≈ 7 days). Instance storage is bumped on every entry-point that
   touches it. These bumps are included in the guardrail measurements above.

6. **Scaling is sub-linear.** The profiling harness verifies that CPU cost for
   both `create_streams` and `batch_withdraw` scales at most 5× when batch size
   grows 4× (from 5 to 20 streams). This confirms no hidden quadratic behaviour.

---

## Residual risks and audit notes

- **Recipient-index updates in `create_streams`:** each stream creation calls
  `add_stream_to_recipient_index`, which reads and writes a persistent
  `Vec<u64>` per recipient. For a batch where all streams share the same
  recipient, this is O(N) reads and writes on the same key. The index is
  maintained in sorted order via binary search (O(log N) per insert), but the
  persistent I/O cost is O(N). Operators creating large batches to a single
  recipient should be aware of this.

- **No hard batch size limit.** The contract does not enforce a maximum number
  of entries in `create_streams` or `batch_withdraw`. The Soroban network
  enforces a per-transaction CPU and memory budget; calls that exceed it will
  fail at the network level. The guardrails in the test suite are conservative
  upper bounds, not protocol limits.

- **Token contract trust.** All token transfers use the SAC interface. The
  contract assumes the token contract does not reenter the streaming contract.
  CEI ordering mitigates this risk but does not eliminate it for non-SAC tokens
  if the contract is ever re-initialised with a custom token address.


---

## Recipient Stream Index Performance

The recipient stream index has significant performance implications for high-volume recipients. See [recipient-stream-index.md](./recipient-stream-index.md) for detailed analysis.

### Key Metrics

**Paged Index (CONTRACT_VERSION 6+):**
- **Page size**: 100 stream IDs per page (MAX_RECIPIENT_PAGE_SIZE)
- **Add stream**: ~2,500 CPU instructions (O(1), touches last page only)
- **Remove stream**: ~3,400 CPU instructions (O(1) amortized, touches ≤2 pages)
- **Query 100 streams**: ~5,000 CPU instructions (loads 1 page)
- **Query 1,000 streams**: ~50,000 CPU instructions (loads 10 pages)

**Flat List (Legacy):**
- **Add stream**: ~N × 100 CPU instructions (O(N), loads/saves entire list)
- **Remove stream**: ~N × 100 CPU instructions (O(N), loads/saves entire list)
- **Query all streams**: ~N × 100 CPU instructions (loads entire list)

### Batch Creation: Same-Recipient Penalty

When creating multiple streams for the same recipient in a single `create_streams` call:

**Flat List:**
- Each stream creation reads and writes the entire recipient index
- For 10 streams to the same recipient: 10 × (read + write) of growing Vec
- **Total overhead**: ~50,000 CPU instructions (quadratic growth)

**Paged Index:**
- Each stream creation appends to the last page (≤100 IDs)
- For 10 streams to the same recipient: 10 × (read + write) of last page
- **Total overhead**: ~25,000 CPU instructions (linear growth)

**Recommendation**: For batches with same-recipient streams, use paged index or limit batch size to ≤10 streams.

### Index Cleanup Impact

Calling `close_completed_stream` regularly reduces future query costs:

**Without Cleanup (1,000 streams, 900 completed):**
- Query all: ~100,000 CPU instructions (loads all 1,000)
- Mutation: ~2,500 CPU instructions (O(1) with paged index)

**With Cleanup (100 active streams):**
- Query all: ~10,000 CPU instructions (loads only 100)
- Mutation: ~2,500 CPU instructions (O(1) with paged index)

**Savings**: 90% reduction in query costs after cleanup.

### Migration Cost

Migrating from flat list to paged index via `migrate_recipient_index`:

| Streams | CPU Instructions | Memory Bytes | Fee (XLM) |
|---------|------------------|--------------|-----------|
| 100 | ~10,000 | ~5,000 | ~0.0001 |
| 1,000 | ~50,000 | ~25,000 | ~0.0005 |
| 10,000 | ~500,000 | ~250,000 | ~0.005 |

**Recommendation**: Migrate recipients with >200 streams to paged index for optimal performance.

### DoS Protection

The paged index provides DoS protection by bounding per-operation costs:

- **MAX_RECIPIENT_PAGE_SIZE = 100**: Limits single-page I/O to ~850 bytes
- **MAX_PAGE_SIZE = 100**: Limits query results to 100 streams per call
- **Cursor-based pagination**: Prevents unbounded list traversal

See [recipient-stream-index.md](./recipient-stream-index.md) for detailed performance characteristics, worked examples with soroban-cli, and indexer integration guidance.
