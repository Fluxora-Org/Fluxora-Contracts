Dynamic Pricing Edge Cases
Status: Normative — describes deployed behavior for update_rate_per_second and decrease_rate_per_second.
Issue references: Dynamic pricing review (current issue).
Contract file: contracts/stream/src/lib.rs
Regression tests: contracts/stream/tests/dynamic_pricing_edge_cases.rs

1. Overview
The streaming contract provides two sender-controlled rate-mutation entrypoints for Linear streams:

Entry point	Direction	Checkpoint?	Refund?	Authorization
update_rate_per_second	Increase only (new > old)	Yes (locks accrued-to-date)	No (deposit preserved)	Stream sender
decrease_rate_per_second	Decrease only (0 < new < old)	Yes (locks accrued-to-date)	Yes (old_deposit − new_max_payable)	Stream sender
Both entrypoints are gated by:

require_not_globally_paused (global emergency pause blocks mutations)
stream.kind == StreamKind::Linear (UnsupportedStreamKind otherwise)
stream.decommissioned.unwrap_or(false) == false (InvalidState otherwise)
MIN_RATE_INTERVAL_LEDGERS = 17 cooldown (RateCooldownActive otherwise)
stream.status != Completed && stream.status != Cancelled (StreamTerminalState)
For increases: now < stream.end_time (InvalidState if expired)
For decreases: now < stream.end_time (InvalidState if expired)
2. update_rate_per_second — Observable Semantics
text

update_rate_per_second(stream_id, new_rate_per_second)
2.1 Pre-conditions (checked in order)
stream_id exists — StreamNotFound (propagated by load_stream)
stream.kind == Linear — else UnsupportedStreamKind (28)
stream.decommissioned.unwrap_or(false) == false — else InvalidState (2)
env.ledger().timestamp() < stream.end_time — else InvalidState (2)
stream.status is Active or Paused — StreamTerminalState (13) for terminal; InvalidState (2) for other non-terminal states
Cooldown: current_ledger − last_rate_change_ledger >= 17 — else RateCooldownActive (36)
new_rate_per_second > 0 — else InvalidParams (3)
new_rate_per_second > old_rate — else InvalidParams (3) (direction guard: must be strictly increasing)
new_rate_per_second <= max_rate_per_second (governance cap) — else RateCapExceeded (29) with RateCapEnforced event
new_rate_per_second × (end_time − start_time) <= deposit_amount — else InsufficientDeposit (10)
2.2 Mutations (applied atomically, only after all checks pass)
stream.checkpointed_amount ← calculate_accrued_amount_checkpointed(..., old_rate, now)
stream.checkpointed_at ← now
stream.rate_per_second ← new_rate_per_second
stream.last_rate_change_ledger ← current_ledger (already bumped by check_and_bump_rate_cooldown)
save_stream(&env, &stream) — CEI: state persisted before any external interaction (no token transfer on increase)
2.3 Events
RateUpdated { stream_id, old_rate_per_second, new_rate_per_second, effective_time: now } ("rate_upd")
RateCapEnforced { stream_id, attempted_rate, max_rate_per_second } ("rate_cap") — only when the cap is exceeded (failure path). The event is emitted before returning the error.
2.4 Boundary conditions
Condition	Expected result	Error / Note
new_rate == old_rate	Rejected	InvalidParams (3)
new_rate < old_rate	Rejected	InvalidParams (3) — use decrease_rate_per_second
new_rate == 0	Rejected	InvalidParams (3)
new_rate < 0	Rejected	InvalidParams (3)
new_rate == max_rate	Allowed	Cap is inclusive (> not >=)
new_rate == max_rate + 1 (with cap set)	Rejected	RateCapExceeded (29) + event
current_time == end_time (exactly at end)	Rejected	InvalidState (2)
current_time > end_time (past end)	Rejected	InvalidState (2)
Stream is Paused	Allowed	Rate updates work on paused streams
Stream is Completed / Cancelled	Rejected	StreamTerminalState (13)
deposit_amount < new_rate × duration	Rejected	InsufficientDeposit (10)
deposit_amount == new_rate × duration	Allowed	Exact coverage
First rate change (last_rate_change_ledger == 0)	Allowed	Exempt from cooldown
Second change within 16 ledgers	Rejected	RateCooldownActive (36)
3. decrease_rate_per_second — Observable Semantics
text

decrease_rate_per_second(stream_id, new_rate_per_second)
3.1 Pre-conditions (checked in order)
stream_id exists — StreamNotFound
stream.kind == Linear — else UnsupportedStreamKind (28)
stream.decommissioned.unwrap_or(false) == false — else InvalidState (2)
env.ledger().timestamp() < stream.end_time — else InvalidState (2)
stream.status is Active or Paused — StreamTerminalState (13) for terminal states
Cooldown: current_ledger − last_rate_change_ledger >= 17 — else RateCooldownActive (36)
new_rate_per_second > 0 — else InvalidParams (3)
new_rate_per_second < old_rate — else InvalidParams (3) (direction guard: must be strictly decreasing)
new_deposit = checkpointed_amount + new_rate × max(0, end_time − now) must not exceed old_deposit — else ArithmeticOverflow (6) (deterministic rejection if state is inconsistent; on normal state this is impossible because lower rate × same remaining time ≤ old rate × remaining time + checkpoint ≤ old deposit)
3.2 Checkpoint computation (before mutation)
text

accrued_now = calculate_accrued_amount_checkpointed(
    CheckpointState {
        checkpointed_amount: stream.checkpointed_amount,
        checkpointed_at: stream.checkpointed_at,
        cliff_time: stream.cliff_time,
        end_time: stream.end_time,
        deposit_amount: stream.deposit_amount,
        kind: stream.kind,
    },
    old_rate,
    now,
)
This locks the mathematical entitlement earned under the old rate up to now.

3.3 New deposit ceiling and refund
text

remaining_seconds = (end_time − now) as i128
future_accrual = new_rate_per_second × remaining_seconds (checked_mul)
new_deposit = accrued_now + future_accrual (checked_add)
refund_amount = old_deposit − new_deposit (checked_sub)
If new_deposit > old_deposit: ArithmeticOverflow (6) — deterministic failure, no state persisted.
refund_amount is refunded to stream.sender via push_token if > 0.
TotalLiabilities is decremented by refund_amount (if > 0) via checked_sub + unwrap_or(0).
3.4 Mutations (CEI: state persisted before token transfer)
stream.checkpointed_amount ← accrued_now
stream.checkpointed_at ← now
stream.rate_per_second ← new_rate_per_second
stream.deposit_amount ← new_deposit
stream.last_rate_change_ledger ← current_ledger
save_stream(&env, &stream)
Then (if refund_amount > 0): reduce liabilities + push_token(&env, &stream.sender, refund_amount)
3.5 Events
RateDecreased { stream_id, old_rate_per_second, new_rate_per_second, effective_time: now, checkpointed_amount: accrued_now, refund_amount } ("rate_dec")
No event is emitted on the failure paths (InvalidState, InvalidParams, RateCooldownActive, ArithmeticOverflow).

3.6 Boundary conditions
Condition	Expected result	Error / Note
new_rate == old_rate	Rejected	InvalidParams (3)
new_rate > old_rate	Rejected	InvalidParams (3) — use update_rate_per_second
new_rate == 0	Rejected	InvalidParams (3)
new_rate < 0	Rejected	InvalidParams (3)
current_time == end_time (exactly at end)	Rejected	InvalidState (2)
current_time > end_time (past end)	Rejected	InvalidState (2)
Stream is Paused	Allowed	Decrease works on paused streams
Stream is Completed / Cancelled	Rejected	StreamTerminalState (13)
new_rate == 1, old_rate == 2, t == 500, end == 1000, deposit == 2000	refund_amount == 500	Verified by regression test
new_rate == 1, old_rate == 3, t == 500, end == 1000, deposit == 3000	refund_amount == 1000 (approx)	Verified by regression test
4. Combined dynamic pricing + schedule interaction
Both entrypoints interact with top_up_stream, extend_stream_end_time, shorten_stream_end_time, and withdraw through the same persistent Stream state. Key interaction rules:

Top-up before rate change: Increases deposit_amount; does not alter rate. Rate change then validates the new rate against the (now larger) deposit.
Rate increase before top-up: Increases rate_per_second; does not alter deposit. Subsequent top-up increases deposit_amount independently.
Rate decrease before extend: Decreases rate_per_second and deposit_amount (refund). extend_stream_end_time then validates the new duration against the reduced deposit_amount.
Extend before rate increase: Increases end_time. Rate increase validates new_rate × new_duration <= deposit_amount.
Withdraw before rate change: Updates stream.withdrawn_amount. Rate change calculates accrued_now using the checkpoint and old_rate; the withdrawn amount is not directly involved in the new deposit ceiling (only in future get_withdrawable).
5. Invariants (must hold after every mutation)
Invariant	Enforcement mechanism
checkpointed_amount + new_rate × max(0, end_time − checkpointed_at) <= deposit_amount	InsufficientDeposit (increase) / new_deposit calculation + ArithmeticOverflow guard (decrease)
new_rate > 0	InvalidParams (3)
new_rate direction matches entrypoint (increase vs. decrease)	InvalidParams (3)
Rate change applies only from now forward (checkpointed_at = now)	stream.checkpointed_at assignment
Previous accrued amount preserved (checkpointed_amount)	calculate_accrued_amount_checkpointed result assigned before mutation
deposit_amount never silently grows via decrease_rate_per_second	new_deposit > old_deposit → ArithmeticOverflow (6)
Refund equals old_deposit − new_deposit exactly (integer arithmetic)	checked_sub + checked_add in both directions
TotalLiabilities reflects deposit changes (increase → no change; decrease → −refund)	read_total_liabilities / write_total_liabilities
last_rate_change_ledger updated after every successful change	check_and_bump_rate_cooldown
rate_per_second unchanged for non-Linear streams	UnsupportedStreamKind (28) guard
6. Clock regression
Both entrypoints call current_accrual_timestamp(&env)?, which detects if the ledger timestamp regressed compared to the previous accrual timestamp (LastAccrualLedgerTimestamp). If regression is detected (current_timestamp < last_observed), the call returns ClockRegression (27) and no mutation occurs.

7. Stream kind restrictions
update_rate_per_second and decrease_rate_per_second are only allowed for StreamKind::Linear.
CliffOnly (kind == CliffOnly) and CliffSlope (kind == CliffSlope) streams return UnsupportedStreamKind (28) for both rate-change entrypoints.
This is consistent with the creation-time validation (rate_per_second == 0 is required for CliffOnly), making rate mutation meaningless for non-linear kinds.
8. Legacy update_rate guards
update_rate(stream_id, new_rate, caller) (legacy entrypoint) has the same authorization rules (sender or admin) but does not adjust deposit_amount, checkpointed_amount, or emit RateUpdated in the same structured way. It is covered by dedicated regression tests (tests/dynamic_pricing_edge_cases.rs — section 4) to prevent accidental removal.

9. Regression surface
The behaviors above are pinned by tests/dynamic_pricing_edge_cases.rs. Changing any branch condition, error code mapping, or invariant without updating both this document and the corresponding regression test violates the protocol's regression policy.

9.1 Backward compatibility
No DataKey variant or Stream field is added or removed.
update_rate_per_second and decrease_rate_per_second signatures are unchanged.
RateUpdated, RateCapEnforced, and RateDecreased event payload shapes are unchanged.
CONTRACT_VERSION does not require a bump for documentation-only changes.
The change is 100% backward-compatible with the current release.
10. References
Implementation: contracts/stream/src/lib.rs (update_rate_per_second ~line 4904; decrease_rate_per_second ~line 5118)
Regression tests: contracts/stream/tests/dynamic_pricing_edge_cases.rs
Related docs: docs/streaming.md (§3, §14, §15 — update/decrease observable semantics; rate-cooldown policy; liability invariants)
Related docs: docs/ABI_STABILITY.md (entrypoint signatures; event payload schemas)
Related docs: docs/storage.md (DataKey::Stream layout; last_rate_change_ledger field)
Related docs: docs/liability-invariants.md (rate-decrease liability reduction; rate-increase liability preservation)