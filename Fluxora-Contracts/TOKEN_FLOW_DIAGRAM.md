# Token Flow Architecture Diagram

**Visual reference for understanding token movements in the Fluxora streaming contract**

---

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    FLUXORA STREAMING CONTRACT                        │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────┐   │
│  │              TOKEN TRANSFER CENTRALIZATION                  │   │
│  │                                                              │   │
│  │   ┌──────────────────┐         ┌──────────────────┐       │   │
│  │   │   pull_token()   │         │   push_token()   │       │   │
│  │   │                  │         │                  │       │   │
│  │   │  External → Contract       │  Contract → External     │   │
│  │   │                  │         │                  │       │   │
│  │   │  • create_stream │         │  • withdraw      │       │   │
│  │   │  • create_streams│         │  • withdraw_to   │       │   │
│  │   │  • top_up_stream │         │  • batch_withdraw│       │   │
│  │   │                  │         │  • cancel_stream │       │   │
│  │   │                  │         │  • shorten_end   │       │   │
│  │   └──────────────────┘         └──────────────────┘       │   │
│  │                                                              │   │
│  └────────────────────────────────────────────────────────────┘   │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Detailed Token Flow Map

### INBOUND FLOWS (pull_token)

```
┌─────────────────────────────────────────────────────────────────────┐
│                         INBOUND TOKEN FLOWS                          │
└─────────────────────────────────────────────────────────────────────┘

1. CREATE STREAM
   ┌──────────┐                                    ┌──────────────┐
   │  Sender  │──────────────────────────────────>│   Contract   │
   └──────────┘                                    └──────────────┘
        │                                                  │
        │ 1. sender.require_auth()                        │
        │ 2. validate_stream_params()                     │
        │ 3. pull_token(&sender, deposit_amount)          │
        │ 4. persist_new_stream()                         │
        │ 5. emit StreamCreated event                     │
        │                                                  │
        └──────────────────────────────────────────────────┘

   Amount: deposit_amount (full stream funding)
   Authorization: sender.require_auth()
   State Change: New stream created with status Active
   Event: StreamCreated(stream_id, deposit_amount, ...)

2. CREATE STREAMS (BATCH)
   ┌──────────┐                                    ┌──────────────┐
   │  Sender  │──────────────────────────────────>│   Contract   │
   └──────────┘                                    └──────────────┘
        │                                                  │
        │ 1. sender.require_auth() (once)                 │
        │ 2. validate all streams                         │
        │ 3. calculate total_deposit                      │
        │ 4. pull_token(&sender, total_deposit)           │
        │ 5. persist all streams                          │
        │ 6. emit StreamCreated for each                  │
        │                                                  │
        └──────────────────────────────────────────────────┘

   Amount: sum(deposit_amount) for all streams
   Authorization: sender.require_auth() (single auth for batch)
   State Change: Multiple streams created
   Event: StreamCreated for each stream

3. TOP UP STREAM
   ┌──────────┐                                    ┌──────────────┐
   │  Funder  │──────────────────────────────────>│   Contract   │
   └──────────┘                                    └──────────────┘
        │                                                  │
        │ 1. funder.require_auth()                        │
        │ 2. validate stream is Active/Paused             │
        │ 3. update deposit_amount (CEI!)                 │
        │ 4. save_stream()                                │
        │ 5. pull_token(&funder, amount)                  │
        │ 6. emit StreamToppedUp event                    │
        │                                                  │
        └──────────────────────────────────────────────────┘

   Amount: top_up_amount (additional funding)
   Authorization: funder.require_auth() (any address can fund)
   State Change: deposit_amount increased
   Event: StreamToppedUp(stream_id, amount, new_deposit)
   Note: CEI pattern - state updated BEFORE pull_token
```

---

### OUTBOUND FLOWS (push_token)

```
┌─────────────────────────────────────────────────────────────────────┐
│                        OUTBOUND TOKEN FLOWS                          │
└─────────────────────────────────────────────────────────────────────┘

1. WITHDRAW
   ┌──────────────┐                                ┌───────────┐
   │   Contract   │──────────────────────────────>│ Recipient │
   └──────────────┘                                └───────────┘
        │                                                  │
        │ 1. recipient.require_auth()                     │
        │ 2. validate stream is Active/Cancelled          │
        │ 3. calculate withdrawable amount                │
        │ 4. update withdrawn_amount (CEI!)               │
        │ 5. save_stream()                                │
        │ 6. push_token(&recipient, withdrawable)         │
        │ 7. emit Withdrawal event                        │
        │ 8. emit StreamCompleted if fully withdrawn      │
        │                                                  │
        └──────────────────────────────────────────────────┘

   Amount: accrued - withdrawn_amount
   Authorization: recipient.require_auth()
   State Change: withdrawn_amount increased, possibly status → Completed
   Event: Withdrawal(stream_id, amount) + StreamCompleted (if done)
   Note: CEI pattern - state updated BEFORE push_token

2. WITHDRAW TO DESTINATION
   ┌──────────────┐                                ┌─────────────┐
   │   Contract   │──────────────────────────────>│ Destination │
   └──────────────┘                                └─────────────┘
        │                                                  │
        │ 1. recipient.require_auth() (not destination!)  │
        │ 2. validate destination != contract             │
        │ 3. validate stream is Active/Cancelled          │
        │ 4. calculate withdrawable amount                │
        │ 5. update withdrawn_amount (CEI!)               │
        │ 6. save_stream()                                │
        │ 7. push_token(&destination, withdrawable)       │
        │ 8. emit WithdrawalTo event                      │
        │                                                  │
        └──────────────────────────────────────────────────┘

   Amount: accrued - withdrawn_amount
   Authorization: recipient.require_auth() (recipient authorizes, not destination)
   State Change: withdrawn_amount increased, possibly status → Completed
   Event: WithdrawalTo(stream_id, recipient, destination, amount)
   Use Case: Wallet migration, custody workflows

3. BATCH WITHDRAW
   ┌──────────────┐                                ┌───────────┐
   │   Contract   │──────────────────────────────>│ Recipient │
   └──────────────┘                                └───────────┘
        │                                                  │
        │ 1. recipient.require_auth() (once)              │
        │ 2. for each stream_id:                          │
        │    a. validate recipient owns stream            │
        │    b. calculate withdrawable                    │
        │    c. update withdrawn_amount (CEI!)            │
        │    d. save_stream()                             │
        │    e. push_token(&recipient, withdrawable)      │
        │    f. emit Withdrawal event                     │
        │                                                  │
        └──────────────────────────────────────────────────┘

   Amount: sum of withdrawable amounts from all streams
   Authorization: recipient.require_auth() (single auth for batch)
   State Change: Multiple streams updated
   Event: Withdrawal for each stream with amount > 0

4. CANCEL STREAM (REFUND)
   ┌──────────────┐                                ┌──────────┐
   │   Contract   │──────────────────────────────>│  Sender  │
   └──────────────┘                                └──────────┘
        │                                                  │
        │ 1. sender.require_auth() OR admin.require_auth()│
        │ 2. validate stream is Active/Paused             │
        │ 3. calculate accrued_at_cancel                  │
        │ 4. calculate refund = deposit - accrued         │
        │ 5. update status = Cancelled (CEI!)             │
        │ 6. save_stream()                                │
        │ 7. push_token(&sender, refund_amount)           │
        │ 8. emit StreamCancelled event                   │
        │                                                  │
        └──────────────────────────────────────────────────┘

   Amount: deposit_amount - accrued_at_cancel
   Authorization: sender.require_auth() OR admin.require_auth()
   State Change: status → Cancelled, cancelled_at set
   Event: StreamCancelled(stream_id)
   Note: Accrued amount stays in contract for recipient to withdraw

5. SHORTEN STREAM END TIME (PARTIAL REFUND)
   ┌──────────────┐                                ┌──────────┐
   │   Contract   │──────────────────────────────>│  Sender  │
   └──────────────┘                                └──────────┘
        │                                                  │
        │ 1. sender.require_auth()                        │
        │ 2. validate new_end_time constraints            │
        │ 3. calculate new_max_streamable                 │
        │ 4. calculate refund = old_deposit - new_max     │
        │ 5. update end_time and deposit_amount (CEI!)    │
        │ 6. save_stream()                                │
        │ 7. push_token(&sender, refund_amount)           │
        │ 8. emit StreamEndShortened event                │
        │                                                  │
        └──────────────────────────────────────────────────┘

   Amount: old_deposit - new_max_streamable
   Authorization: sender.require_auth()
   State Change: end_time shortened, deposit_amount reduced
   Event: StreamEndShortened(stream_id, old_end, new_end, refund)
```

---

## CEI Pattern Visualization

```
┌─────────────────────────────────────────────────────────────────────┐
│              CHECKS-EFFECTS-INTERACTIONS (CEI) PATTERN               │
└─────────────────────────────────────────────────────────────────────┘

CORRECT ORDER (✅):

┌─────────────┐
│   CHECKS    │  1. Authorization (require_auth)
│             │  2. Validation (status, amounts, time)
│             │  3. Calculations (accrued, withdrawable, refund)
└─────────────┘
      │
      ▼
┌─────────────┐
│  EFFECTS    │  4. Update state variables
│             │  5. Save to storage (save_stream)
│             │  6. Update counters/metrics
└─────────────┘
      │
      ▼
┌─────────────┐
│INTERACTIONS │  7. Token transfer (pull_token/push_token)
│             │  8. Emit events
│             │  9. External calls
└─────────────┘

WHY THIS ORDER MATTERS:

1. If CHECKS fail → No state change, no token transfer (clean revert)
2. If EFFECTS fail → No token transfer (clean revert)
3. If INTERACTIONS fail → State already saved, transaction reverts atomically

WRONG ORDER (❌):

┌─────────────┐
│   CHECKS    │
└─────────────┘
      │
      ▼
┌─────────────┐
│INTERACTIONS │  ❌ Token transfer BEFORE state update
└─────────────┘  ❌ Reentrancy risk!
      │           ❌ Inconsistent state on failure
      ▼
┌─────────────┐
│  EFFECTS    │  ❌ State update AFTER external call
└─────────────┘
```

---

## Authorization Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                      AUTHORIZATION MATRIX                            │
└─────────────────────────────────────────────────────────────────────┘

OPERATION              │ HELPER      │ AUTHORIZATION
───────────────────────┼─────────────┼──────────────────────────────
create_stream          │ pull_token  │ sender.require_auth()
create_streams         │ pull_token  │ sender.require_auth()
top_up_stream          │ pull_token  │ funder.require_auth()
───────────────────────┼─────────────┼──────────────────────────────
withdraw               │ push_token  │ recipient.require_auth()
withdraw_to            │ push_token  │ recipient.require_auth()
batch_withdraw         │ push_token  │ recipient.require_auth()
───────────────────────┼─────────────┼──────────────────────────────
cancel_stream          │ push_token  │ sender.require_auth()
cancel_stream_as_admin │ push_token  │ admin.require_auth()
───────────────────────┼─────────────┼──────────────────────────────
shorten_stream_end     │ push_token  │ sender.require_auth()
───────────────────────┴─────────────┴──────────────────────────────

KEY PRINCIPLES:

1. Authorization ALWAYS checked before helper call
2. Authorization matches the address providing/receiving tokens
3. Admin can override sender operations (cancel, pause, resume)
4. Admin CANNOT override recipient operations (withdraw)
5. Funder can be any address (flexible top-up model)
```

---

## State Transition Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                    STREAM STATUS TRANSITIONS                         │
└─────────────────────────────────────────────────────────────────────┘

                    ┌──────────────────┐
                    │  create_stream   │
                    │  (pull_token)    │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌─────────────────┐
              ┌────>│     ACTIVE      │<────┐
              │     └─────────────────┘     │
              │              │               │
              │              │               │
    resume_stream    pause_stream    resume_stream
    (no token)       (no token)      (no token)
              │              │               │
              │              ▼               │
              │     ┌─────────────────┐     │
              └─────│     PAUSED      │─────┘
                    └─────────────────┘
                             │
                             │
              ┌──────────────┼──────────────┐
              │              │               │
              │              │               │
    cancel_stream    withdraw (full)   withdraw (full)
    (push_token)     (push_token)      (push_token)
              │              │               │
              ▼              ▼               ▼
    ┌─────────────┐  ┌─────────────┐  ┌─────────────┐
    │  CANCELLED  │  │  COMPLETED  │  │  COMPLETED  │
    └─────────────┘  └─────────────┘  └─────────────┘
         │                  │
         │                  │
    withdraw          close_completed_stream
    (push_token)      (remove from storage)
         │                  │
         ▼                  ▼
    ┌─────────────┐  ┌─────────────┐
    │  CANCELLED  │  │   DELETED   │
    │ (no more    │  │             │
    │  withdraws) │  │             │
    └─────────────┘  └─────────────┘

TOKEN MOVEMENTS BY TRANSITION:

Active → Paused:        No token movement
Paused → Active:        No token movement
Active → Cancelled:     push_token (refund to sender)
Active → Completed:     push_token (final withdrawal to recipient)
Paused → Cancelled:     push_token (refund to sender)
Cancelled → Cancelled:  push_token (recipient withdraws accrued)
Completed → Deleted:    No token movement (already fully withdrawn)
```

---

## Balance Tracking

```
┌─────────────────────────────────────────────────────────────────────┐
│                      CONTRACT BALANCE TRACKING                       │
└─────────────────────────────────────────────────────────────────────┘

INVARIANT: Contract balance = Sum of all (deposit_amount - withdrawn_amount)

EXAMPLE SCENARIO:

Time  │ Operation              │ Amount │ Contract Balance │ Notes
──────┼────────────────────────┼────────┼──────────────────┼──────────
T0    │ Initial                │   0    │        0         │ Empty
T1    │ create_stream (S1)     │ +1000  │     1000         │ pull_token
T2    │ create_stream (S2)     │ +2000  │     3000         │ pull_token
T3    │ withdraw S1 (partial)  │  -300  │     2700         │ push_token
T4    │ top_up S1              │  +500  │     3200         │ pull_token
T5    │ cancel S2              │ -1500  │     1700         │ push_token (refund)
T6    │ withdraw S1 (full)     │  -700  │     1000         │ push_token
T7    │ withdraw S2 (accrued)  │  -500  │      500         │ push_token
T8    │ withdraw S1 (top-up)   │  -500  │        0         │ push_token

BALANCE VERIFICATION:

After T8:
- S1: deposit=1500, withdrawn=1500 → 0 remaining
- S2: deposit=2000, accrued=500, withdrawn=500 → 0 remaining
- Contract balance: 0 ✅

AUDIT TRAIL:

Total pulled:  1000 + 2000 + 500 = 3500
Total pushed:  300 + 1500 + 700 + 500 + 500 = 3500
Net balance:   3500 - 3500 = 0 ✅
```

---

## Error Handling Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        ERROR HANDLING FLOW                           │
└─────────────────────────────────────────────────────────────────────┘

SCENARIO 1: Insufficient Balance (pull_token fails)

┌──────────┐                                    ┌──────────────┐
│  Sender  │─────── create_stream ────────────>│   Contract   │
└──────────┘                                    └──────────────┘
     │                                                  │
     │ 1. sender.require_auth() ✅                     │
     │ 2. validate_params() ✅                         │
     │ 3. pull_token() ❌ FAILS                        │
     │    (insufficient balance)                       │
     │                                                  │
     │ <────── Transaction Reverts ──────────          │
     │                                                  │
     │ Result:                                          │
     │ - No stream created                             │
     │ - No state change                               │
     │ - No events emitted                             │
     │ - Sender balance unchanged                      │
     └──────────────────────────────────────────────────┘

SCENARIO 2: Insufficient Contract Balance (push_token fails)

┌──────────────┐                                ┌───────────┐
│   Contract   │─────── withdraw ──────────────>│ Recipient │
└──────────────┘                                └───────────┘
     │                                                  │
     │ 1. recipient.require_auth() ✅                  │
     │ 2. validate status ✅                           │
     │ 3. calculate withdrawable ✅                    │
     │ 4. update withdrawn_amount ✅                   │
     │ 5. save_stream() ✅                             │
     │ 6. push_token() ❌ FAILS                        │
     │    (insufficient contract balance - should not happen!)
     │                                                  │
     │ <────── Transaction Reverts ──────────          │
     │                                                  │
     │ Result:                                          │
     │ - State changes rolled back                     │
     │ - withdrawn_amount reverted                     │
     │ - No tokens transferred                         │
     │ - No events emitted                             │
     │                                                  │
     │ Note: This should never happen if contract      │
     │       logic is correct (invariant violation)    │
     └──────────────────────────────────────────────────┘

SCENARIO 3: Authorization Failure

┌──────────┐                                    ┌──────────────┐
│ Attacker │─────── withdraw ──────────────────>│   Contract   │
└──────────┘                                    └──────────────┘
     │                                                  │
     │ 1. recipient.require_auth() ❌ FAILS            │
     │    (attacker is not recipient)                  │
     │                                                  │
     │ <────── Transaction Reverts ──────────          │
     │                                                  │
     │ Result:                                          │
     │ - No state change                               │
     │ - No token transfer                             │
     │ - No events emitted                             │
     │ - Authorization error returned                  │
     └──────────────────────────────────────────────────┘
```

---

## Summary

This diagram package provides visual references for:

1. **High-Level Architecture** - Overall token transfer centralization
2. **Detailed Token Flows** - All 8 token transfer operations
3. **CEI Pattern** - Correct ordering for security
4. **Authorization Matrix** - Who can trigger what
5. **State Transitions** - Stream lifecycle with token movements
6. **Balance Tracking** - Invariant verification
7. **Error Handling** - Failure scenarios and rollback

**Key Takeaways:**

✅ All token transfers centralized through 2 helpers  
✅ CEI pattern consistently applied  
✅ Authorization checked before token operations  
✅ Atomic transactions ensure consistency  
✅ Balance invariants maintained

---

**Last Updated:** 2026-03-26  
**Version:** 1.0
