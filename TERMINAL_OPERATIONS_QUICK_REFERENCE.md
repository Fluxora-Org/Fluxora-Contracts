# Terminal Operations - Quick Reference Guide

## What Are Terminal Operations?

Terminal states (`Cancelled` and `Depleted`) represent streams that have reached their end state. Once terminal, a stream rejects all mutating lifecycle operations.

## Terminal States

```rust
pub enum StreamStatus {
    Active,      // ← Normal operating state
    Paused,      // ← Temporarily frozen
    Cancelled,   // ← Terminal: sender clawed back unvested
    Depleted,    // ← Terminal: ran to completion and fully paid
}

impl StreamStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, StreamStatus::Cancelled | StreamStatus::Depleted)
    }
}
```

## Rejection Matrix

| Operation | Active | Paused | Cancelled | Depleted |
|-----------|--------|--------|-----------|----------|
| **pause** | ✓ | ✗¹ | ✗² | ✗² |
| **resume** | ✗³ | ✓ | ✗² | ✗² |
| **top_up** | ✓ | ✓ | ✗² | ✗² |
| **withdraw** | ✓ | ✓ | ✓⁴ | ✗² |
| **cancel** | ✓ | ✓ | ✗² | ✗² |
| **transfer_recipient** | ✓ | ✓ | ✗² | ✓⁵ |

**Legend:**
- ✓ = Allowed
- ✗ = Rejected with error

**Error codes:**
1. `StreamAlreadyPaused` - cannot pause when already paused
2. `StreamTerminated` - cannot mutate terminal state
3. `StreamNotPaused` - cannot resume when not paused
4. Works if `withdrawable > 0`, else `StreamTerminated`
5. Works if `withdrawn < deposited`, else `StreamTerminated`

## Error: StreamTerminated

**Discriminant**: 14

**When returned:**
- Any mutating operation on `Cancelled` stream
- Any mutating operation on `Depleted` stream  
- Exceptions noted in matrix above

**Guarantees when this error is returned:**
- ✓ Stream state unchanged
- ✓ Token balances unchanged
- ✓ No events emitted
- ✓ TTL not extended
- ✓ Idempotent (retry produces same result)

## State Transitions to Terminal

### Path to Cancelled

```
Active → cancel() → Cancelled (terminal)
Paused → cancel() → Cancelled (terminal)
```

**Characteristics:**
- Sender clawed back unvested remainder
- `deposited` reduced to what vested at cancel time
- Schedule collapsed onto cancel instant
- Sticky: stays `Cancelled` even after full withdrawal
- Distinguishes "sender took it back" from "ran to completion"

### Path to Depleted

```
Active → withdraw(all) → Depleted (terminal)
Paused → withdraw(all) → Depleted (terminal)
```

**Characteristics:**
- Stream ran its course and recipient withdrew everything
- `withdrawn == deposited`
- Clean completion, no clawback
- Status honors the original schedule completion

## Common Scenarios

### Scenario 1: Cancel and Withdraw

```rust
// Stream at day 50 of 100
h.client.cancel(&id);
// Status: Cancelled, deposited reduced to 50% vested

// Recipient can still withdraw
h.client.withdraw(&id, &None);  // ✓ Works
// Status: Still Cancelled (not Depleted)

// But cannot do anything else
h.client.pause(&id);   // ✗ StreamTerminated
h.client.top_up(&id);  // ✗ StreamTerminated
```

### Scenario 2: Full Completion

```rust
// Stream reaches end naturally
h.warp_to(end_time);
h.client.withdraw(&id, &None);
// Status: Depleted

// Try to cancel after depletion
h.client.cancel(&id);  // ✗ StreamTerminated
```

### Scenario 3: Cancel Before Cliff

```rust
// Stream with cliff at day 50, cancel at day 30
h.client.cancel(&id);
// Status: Cancelled
// deposited: 0 (nothing vested before cliff)
// refund: 100% (everything returned to sender)

h.client.withdraw(&id, &None);  // ✗ StreamTerminated (nothing to withdraw)
```

### Scenario 4: Pause Then Cancel

```rust
h.client.pause(&id);
// Status: Paused, paused_at: Some(timestamp)

h.client.cancel(&id);
// Status: Cancelled, paused_at: None (cleared)

h.client.resume(&id);  // ✗ StreamTerminated (not StreamNotPaused)
```

## Testing Terminal Operations

### Basic Pattern

```rust
let id = h.create_simple(1_000 * ONE, 100 * DAY);
h.advance(30 * DAY);
h.client.cancel(&id);

// Capture state before operation
let before = h.get(id);
let pool_before = h.pool();

// Attempt rejected operation
let err = h.client.try_pause(&id).unwrap_err().unwrap();
assert_eq!(err, Error::StreamTerminated);

// Verify state unchanged
let after = h.get(id);
assert_eq!(before, after);
assert_eq!(h.pool(), pool_before);
h.assert_pool_exact();  // Checks all invariants
```

### Boundary Testing Pattern

```rust
// Test with withdrawable balance remaining
h.client.cancel(&id);
assert!(h.client.withdrawable_of(&id) > 0);

// Still rejected
assert_eq!(
    h.client.try_pause(&id).unwrap_err().unwrap(),
    Error::StreamTerminated
);

// But withdraw still works
assert!(h.client.withdraw(&id, &None) > 0);
```

## Integration Guidance

### For Frontend Developers

**Check status before showing actions:**

```typescript
const stream = await contract.get_stream({ stream_id });

if (stream.status === "Cancelled" || stream.status === "Depleted") {
  // Hide: pause, resume, cancel, top_up buttons
  // Show: terminal state badge
  
  if (stream.status === "Cancelled") {
    // Show: "Sender cancelled this stream"
    // Enable: withdraw button if withdrawable > 0
  } else {
    // Show: "Stream completed"
    // Disable: all mutation buttons
  }
}
```

**Handle errors gracefully:**

```typescript
try {
  await contract.pause({ stream_id });
} catch (e) {
  if (e.code === 14) {  // StreamTerminated
    // Refresh stream state - it became terminal
    // Update UI to show terminal status
  }
}
```

### For Backend/Indexers

**Track terminal transitions:**

```sql
-- Cancelled is sticky
UPDATE streams 
SET status = 'Cancelled'
WHERE stream_id = ? 
  AND status != 'Cancelled';  -- Don't overwrite

-- Depleted only when fully paid
UPDATE streams
SET status = 'Depleted'
WHERE stream_id = ?
  AND withdrawn >= deposited
  AND status != 'Cancelled';  -- Cancelled takes precedence
```

**Index final withdrawals:**

```typescript
// On withdrawal event
if (stream.withdrawn >= stream.deposited) {
  if (stream.status === "Cancelled") {
    // Final withdrawal from cancelled stream
    metrics.record("cancelled_stream_final_withdrawal");
  } else {
    // Stream naturally completed
    stream.status = "Depleted";
    metrics.record("stream_natural_completion");
  }
}
```

### For Keepers

**Skip terminal streams in extend sweeps:**

```typescript
async function extendStreams(streams: u64[]) {
  const active = [];
  
  for (const id of streams) {
    const stream = await contract.get_stream({ stream_id: id });
    
    // Skip terminal streams - they decay to floor naturally
    if (stream.status !== "Cancelled" && stream.status !== "Depleted") {
      active.push(id);
    }
  }
  
  await contract.batch_extend_ttl({ stream_ids: active });
}
```

## Quick Diagnostics

### "Why is my operation being rejected?"

1. **Check stream status:**
   ```rust
   let stream = contract.get_stream(&id);
   println!("Status: {:?}", stream.status);
   ```

2. **If Cancelled or Depleted:**
   - This is expected behavior
   - Terminal states are permanent
   - Only withdrawal may work (if balance remains)

3. **Check withdrawable balance:**
   ```rust
   let available = contract.withdrawable_of(&id);
   println!("Withdrawable: {}", available);
   ```

### "Stream shows as terminal but I see a balance"

**For Cancelled streams:**
- This is normal
- Recipient keeps what vested before cancellation
- Call `withdraw` to claim it
- Status stays `Cancelled` after withdrawal

**For Depleted streams:**
- Check if `withdrawn < deposited`
- Small rounding residue may remain
- Try withdrawing the remainder

### "Can I 'undo' a terminal state?"

**No.** Terminal states are permanent by design:

- Cannot cancel a cancellation
- Cannot reactivate a depleted stream
- Cannot pause/resume terminal streams
- Cannot top-up terminal streams

**Instead:** Create a new stream if needed

## Performance Notes

- **Terminal check is O(1)**: Single enum comparison
- **No storage reads**: Status is in the loaded stream struct
- **Fail-fast**: Checked before auth, transfers, or writes
- **Zero cost**: Failed operations write nothing

## Testing Checklist

When testing terminal operations:

- [ ] Test both `Cancelled` and `Depleted` states
- [ ] Verify `StreamTerminated` error returned
- [ ] Check stream state unchanged (before/after)
- [ ] Verify token balances unchanged
- [ ] Confirm TTL not extended
- [ ] Test with withdrawable balance > 0
- [ ] Test with withdrawable balance == 0
- [ ] Verify idempotency (retry same operation)
- [ ] Check pause state cleared on termination

## Related Documentation

- `test/terminal_operations.rs` - Full test suite
- `TERMINAL_OPERATIONS_TEST_SUMMARY.md` - Detailed documentation
- `test/cancel.rs` - Cancellation behavior
- `test/pause.rs` - Pause state management
- `test/withdraw.rs` - Withdrawal and depletion
- `docs/ABI.md` - Public interface specification

## Error Reference

| Code | Error | Meaning | Action |
|------|-------|---------|--------|
| 14 | `StreamTerminated` | Stream is Cancelled or Depleted | Check status, withdraw if balance, or create new stream |
| 12 | `StreamNotPaused` | Cannot resume non-paused stream | Check current status |
| 13 | `StreamAlreadyPaused` | Cannot pause paused stream | Check current status |
| 17 | `NothingToWithdraw` | Zero withdrawable (live stream) | Wait for accrual or check cliff |

## Summary

**Terminal operations are REJECTED** to maintain state consistency:
- ✓ Predictable behavior (no surprise mutations)
- ✓ Clear error messages
- ✓ State always consistent
- ✓ Pool invariant preserved
- ✓ No silent failures

**This is a feature, not a bug.** Terminal states represent completion and should not be mutated.
