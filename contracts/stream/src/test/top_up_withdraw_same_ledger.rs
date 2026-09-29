//! Same-ledger top_up + withdraw coverage.
//!
//! # The scenario
//!
//! `top_up` changes `deposited` and `end_time` in storage.  `withdraw` reads
//! `vested = deposited * elapsed / duration` from storage.  When both land in
//! the same ledger at the same timestamp the value `withdraw` sees depends on
//! whether `top_up` has already written its changes.
//!
//! # Why the ordering does not change the withdrawable amount
//!
//! The top-up formula is rate-preserving by design:
//!
//! ```text
//! before:  deposited     , duration     = elapsed / duration * deposited
//! top_up:  deposited + Δ , duration + δ   where δ = Δ * duration / deposited  (rounded down)
//! after:   vested'       = (deposited + Δ) * elapsed / (duration + δ)
//! ```
//!
//! Because `δ = Δ * duration / deposited` (floored), the new rate
//! `(deposited + Δ) / (duration + δ)` is **at least** the old rate.  Combined
//! with the post-top-up `VestedDecreased` guard inside `top_up`, the vested
//! amount at a fixed instant is guaranteed to be **unchanged or at most one
//! stroop higher** after the call.  It can never decrease (invariant I3).
//!
//! Consequently:
//!
//! * `top_up → withdraw`:  withdraw sees the updated `deposited` and `duration`;
//!   the vested amount is the same as it was before the top-up (I3 guarantee),
//!   so the payout is unchanged.
//! * `withdraw → top_up`:  withdraw reads the original values, pays out the
//!   correct vested amount, then top_up extends the schedule.
//!
//! Either way the recipient receives exactly `floor(deposited * elapsed /
//! duration)` from before the top-up, the pool stays balanced, and the
//! combined post-call state satisfies every invariant.
//!
//! # What these tests assert
//!
//! * Both orderings yield the same net outcome across a sweep of schedule
//!   points (quarter, halfway, near-end).
//! * Funds conservation holds exactly after both calls complete:
//!   `pool == original_deposited + top_up_amount - withdrawn_amount`.
//! * The emitted `ToppedUp` and `Withdrawn` events match storage and the
//!   token ledger exactly.
//! * The final stream state (deposited, withdrawn, end_time) is coherent.
//! * The test fails if the rate-preserving invariant (I3) is violated.
//!
//! # Scope
//!
//! This module does **not** advance the ledger clock between the two calls in
//! the same-ledger pair.  That is the technique from `withdraw_cancel_same_ledger`
//! and is what pins the same-ledger case rather than two-ledger sequencing.
//!
//! docs/same-ledger-ordering.md does not list top_up + withdraw as an
//! ordering-sensitive pair (the four listed pairs are auth- and
//! terminal-state-sensitive).  These tests confirm that the pair is in fact
//! *not* ordering-sensitive from a conservation and vested-amount standpoint,
//! and pin that property so a future change cannot quietly break it.

use soroban_sdk::testutils::Events as _;
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::{ToppedUp, Withdrawn};
use crate::StreamStatus;

// ---------------------------------------------------------------------------
// Event helpers
// ---------------------------------------------------------------------------

/// Events emitted by the stream contract during the most recent invocation.
///
/// Must be called before the next client call — `Events::all()` only retains
/// the snapshot from the last invocation.
fn stream_events(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// Assert that exactly one `ToppedUp` event was emitted and that every field
/// matches post-call storage and the declared amount.  Returns the payout
/// (same as `amount`) for downstream conservation checks.
fn assert_topped_up_event(h: &Harness, id: u64, amount: i128) {
    let published = stream_events(h);
    let stream = h.get(id);

    let expected = ToppedUp {
        stream_id: id,
        sender: h.sender.clone(),
        amount,
        deposited: stream.deposited,
        end_time: stream.end_time,
        recipient: h.recipient.clone(),
    };

    assert_eq!(
        published,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "ToppedUp event must be the only stream event and must match \
         storage exactly after the top_up call",
    );
}

/// Assert that exactly one `Withdrawn` event was emitted and that every field
/// matches post-call storage and the token ledger delta.  Returns the payout.
fn assert_withdrawn_event(h: &Harness, id: u64, recipient_before: i128) -> i128 {
    let published = stream_events(h);
    let stream = h.get(id);
    let payout = h.balance(&h.recipient) - recipient_before;

    assert!(
        payout > 0,
        "expected a positive payout but recipient balance did not change"
    );

    let expected = Withdrawn {
        stream_id: id,
        recipient: h.recipient.clone(),
        amount: payout,
        withdrawn: stream.withdrawn,
        deposited: stream.deposited,
        status: stream.status,
        sender: stream.sender.clone(),
        paused_at: stream.paused_at,
        paused_total: stream.paused_total,
    };

    assert_eq!(
        published,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "Withdrawn event must be the only stream event and must match \
         storage + token ledger exactly after the withdraw call",
    );

    payout
}

// ---------------------------------------------------------------------------
// Core same-ledger scenario helpers
// ---------------------------------------------------------------------------

/// `top_up → withdraw` at a fixed timestamp.
///
/// Both calls happen at the same ledger instant (no `h.advance()` between
/// them).  After both settle:
///
/// * recipient received exactly the vested amount at that timestamp
/// * pool == original_deposit + top_up_amount − payout
/// * events match storage and token ledger
/// * stream.deposited == original_deposit + top_up_amount − (payout if depleted)
fn same_ledger_top_up_then_withdraw(offset: u64, top_up_amount: i128) {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(offset);
    // ── Both calls are issued at this timestamp ──────────────────────────

    let original_end = h.get(id).end_time;
    let recipient_before_all = h.balance(&h.recipient);
    let sender_before_all = h.balance(&h.sender);

    // Step 1: top_up — extend the schedule, keep the rate.
    h.client.top_up(&id, &top_up_amount);
    assert_topped_up_event(&h, id, top_up_amount);

    let s_after_topup = h.get(id);
    let new_deposited = s_after_topup.deposited;
    let new_end = s_after_topup.end_time;

    assert_eq!(new_deposited, deposit + top_up_amount);
    assert!(
        new_end > original_end,
        "top_up must extend end_time; got end={new_end} original={original_end}",
    );

    // Vested at this fixed instant: the top-up must not have reduced it (I3).
    // Because the rate is preserved, the vested value is the same as before.
    let expected_vested_before_topup =
        deposit * (offset.min(duration) as i128) / (duration as i128);
    let vested_after_topup = h.client.vested_of(&id);
    assert!(
        vested_after_topup >= expected_vested_before_topup,
        "I3 violated: top_up reduced vested at fixed timestamp; \
         before={expected_vested_before_topup} after={vested_after_topup}",
    );

    // Step 2: withdraw — same timestamp, no clock advance.
    let recipient_before_withdraw = h.balance(&h.recipient);
    let payout = h.client.withdraw(&id, &None);
    let event_payout = assert_withdrawn_event(&h, id, recipient_before_withdraw);

    assert_eq!(
        event_payout, payout,
        "Withdrawn.amount must equal withdraw() return value",
    );

    // The amount the recipient receives must equal vested at this timestamp.
    // (After top_up: the rate is preserved, so vested is unchanged or off by
    // at most 1 stroop due to integer rounding — both are ≥ expected.)
    assert!(
        payout >= expected_vested_before_topup,
        "offset={offset}: payout {payout} < expected vested {expected_vested_before_topup}",
    );
    // And it must not exceed the deposited amount.
    assert!(
        payout <= new_deposited,
        "offset={offset}: payout {payout} > new deposited {new_deposited}",
    );

    // ── Pool and conservation ─────────────────────────────────────────────
    //
    // Pool must be: original_deposit + top_up_amount − payout
    // (top_up brought tokens in; withdraw sent tokens out)
    let expected_pool = deposit + top_up_amount - payout;
    assert_eq!(
        h.pool(),
        expected_pool,
        "offset={offset}: pool mismatch after top_up then withdraw",
    );
    h.assert_pool_exact();

    // Sender paid the top_up_amount; never directly paid or received anything
    // from the stream in this test (they created it and topped it up).
    let sender_delta = h.balance(&h.sender) - sender_before_all;
    assert_eq!(
        sender_delta, -top_up_amount,
        "offset={offset}: sender balance must decrease by exactly the top_up amount",
    );

    // Recipient received exactly `payout`.
    let recipient_delta = h.balance(&h.recipient) - recipient_before_all;
    assert_eq!(
        recipient_delta, payout,
        "offset={offset}: recipient balance must increase by exactly the payout",
    );

    // ── Final stream state ────────────────────────────────────────────────
    let s_final = h.get(id);
    assert_eq!(s_final.deposited, new_deposited);
    assert_eq!(s_final.withdrawn, payout);
    assert!(
        s_final.status == StreamStatus::Active
            || s_final.status == StreamStatus::Depleted,
        "offset={offset}: unexpected stream status {:?}",
        s_final.status,
    );
    // If depleted, the pool must be empty.
    if s_final.status == StreamStatus::Depleted {
        assert_eq!(h.pool(), 0, "offset={offset}: depleted stream must leave pool empty");
    }
}

/// `withdraw → top_up` at a fixed timestamp.
///
/// The ordering is reversed: recipient draws down the accrued amount first,
/// then the sender tops the stream up.  Net outcome must match the other
/// ordering (same payout, same pool balance, same event semantics).
fn same_ledger_withdraw_then_top_up(offset: u64, top_up_amount: i128) {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(offset);
    // ── Both calls are issued at this timestamp ──────────────────────────

    let original_end = h.get(id).end_time;
    let recipient_before_all = h.balance(&h.recipient);
    let sender_before_all = h.balance(&h.sender);

    let expected_vested = deposit * (offset.min(duration) as i128) / (duration as i128);

    // Step 1: withdraw — reads original deposited and duration.
    let recipient_before_withdraw = h.balance(&h.recipient);
    let payout = h.client.withdraw(&id, &None);
    let event_payout = assert_withdrawn_event(&h, id, recipient_before_withdraw);

    assert_eq!(
        event_payout, payout,
        "Withdrawn.amount must equal withdraw() return value",
    );
    assert_eq!(
        payout, expected_vested,
        "offset={offset}: withdraw must pay exactly the vested amount",
    );

    let s_after_withdraw = h.get(id);
    assert_eq!(s_after_withdraw.withdrawn, payout);

    // Step 2: top_up — same timestamp, no clock advance.
    h.client.top_up(&id, &top_up_amount);
    assert_topped_up_event(&h, id, top_up_amount);

    let s_after_topup = h.get(id);
    let new_deposited = s_after_topup.deposited;
    let new_end = s_after_topup.end_time;

    assert_eq!(new_deposited, deposit + top_up_amount);
    assert!(
        new_end > original_end,
        "top_up must extend end_time; got end={new_end} original={original_end}",
    );

    // ── Pool and conservation ─────────────────────────────────────────────
    //
    // Same identity as the other ordering:
    // pool == original_deposit + top_up_amount − payout
    let expected_pool = deposit + top_up_amount - payout;
    assert_eq!(
        h.pool(),
        expected_pool,
        "offset={offset}: pool mismatch after withdraw then top_up",
    );
    h.assert_pool_exact();

    // Sender paid the top_up_amount.
    let sender_delta = h.balance(&h.sender) - sender_before_all;
    assert_eq!(
        sender_delta, -top_up_amount,
        "offset={offset}: sender balance must decrease by exactly the top_up amount",
    );

    // Recipient received exactly `payout`.
    let recipient_delta = h.balance(&h.recipient) - recipient_before_all;
    assert_eq!(
        recipient_delta, payout,
        "offset={offset}: recipient balance must increase by exactly the payout",
    );

    // ── Final stream state ────────────────────────────────────────────────
    let s_final = h.get(id);
    assert_eq!(s_final.deposited, new_deposited);
    assert_eq!(s_final.withdrawn, payout);
    // The stream may not be Depleted here even at offset == duration,
    // because top_up extended end_time after the withdraw.
    assert!(
        s_final.status == StreamStatus::Active
            || s_final.status == StreamStatus::Depleted,
        "offset={offset}: unexpected stream status {:?}",
        s_final.status,
    );
}

// ---------------------------------------------------------------------------
// Schedule-point sweeps — both orderings
// ---------------------------------------------------------------------------

/// `top_up → withdraw` across representative schedule points.
#[test]
fn same_ledger_top_up_then_withdraw_schedule_points() {
    let top_up_amount = 500 * ONE; // 50% of original deposit

    same_ledger_top_up_then_withdraw(25 * DAY, top_up_amount); // Quarter
    same_ledger_top_up_then_withdraw(50 * DAY, top_up_amount); // Halfway
    same_ledger_top_up_then_withdraw(75 * DAY, top_up_amount); // Three-quarters
    same_ledger_top_up_then_withdraw(99 * DAY, top_up_amount); // Near-end
}

/// `withdraw → top_up` across representative schedule points.
#[test]
fn same_ledger_withdraw_then_top_up_schedule_points() {
    let top_up_amount = 500 * ONE;

    same_ledger_withdraw_then_top_up(25 * DAY, top_up_amount);
    same_ledger_withdraw_then_top_up(50 * DAY, top_up_amount);
    same_ledger_withdraw_then_top_up(75 * DAY, top_up_amount);
    same_ledger_withdraw_then_top_up(99 * DAY, top_up_amount);
}

// ---------------------------------------------------------------------------
// Ordering symmetry: both orderings yield the same net outcome
// ---------------------------------------------------------------------------

/// Both orderings must produce the same recipient payout, the same pool
/// balance, and the same final `deposited` / `withdrawn` on the stream.
///
/// This is the key correctness property: a top_up that races a withdraw in the
/// same ledger cannot create or destroy value regardless of how the network
/// sequences the two calls.
#[test]
fn both_orderings_agree_on_outcome() {
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let offset = 40 * DAY;
    let top_up_amount = 400 * ONE;

    // Order 1: top_up → withdraw
    let h1 = Harness::new();
    let id1 = h1.create_simple(deposit, duration);
    h1.advance(offset);
    let r1_before = h1.balance(&h1.recipient);
    let s1_before = h1.balance(&h1.sender);
    h1.client.top_up(&id1, &top_up_amount);
    let payout1 = h1.client.withdraw(&id1, &None);
    let r1_delta = h1.balance(&h1.recipient) - r1_before;
    let s1_delta = h1.balance(&h1.sender) - s1_before;
    let pool1 = h1.pool();
    let s1_final = h1.get(id1);
    h1.assert_pool_exact();

    // Order 2: withdraw → top_up
    let h2 = Harness::new();
    let id2 = h2.create_simple(deposit, duration);
    h2.advance(offset);
    let r2_before = h2.balance(&h2.recipient);
    let s2_before = h2.balance(&h2.sender);
    let payout2 = h2.client.withdraw(&id2, &None);
    h2.client.top_up(&id2, &top_up_amount);
    let r2_delta = h2.balance(&h2.recipient) - r2_before;
    let s2_delta = h2.balance(&h2.sender) - s2_before;
    let pool2 = h2.pool();
    let s2_final = h2.get(id2);
    h2.assert_pool_exact();

    // The recipient receives the same amount in both orderings.
    assert_eq!(
        payout1, payout2,
        "payout must be order-independent: top_up-first={payout1} withdraw-first={payout2}",
    );
    assert_eq!(
        r1_delta, r2_delta,
        "recipient balance delta must be order-independent",
    );

    // The sender's net cost is the same: they paid the top_up amount either way.
    assert_eq!(
        s1_delta, s2_delta,
        "sender balance delta must be order-independent",
    );

    // Pool must match in both orderings.
    assert_eq!(
        pool1, pool2,
        "pool balance must be the same regardless of ordering",
    );

    // Final stream deposited and withdrawn must agree.
    assert_eq!(s1_final.deposited, s2_final.deposited);
    assert_eq!(s1_final.withdrawn, s2_final.withdrawn);

    // Conservation: pool == deposited - withdrawn == original + topup - payout
    assert_eq!(
        pool1,
        deposit + top_up_amount - payout1,
        "conservation: pool must equal original_deposit + top_up − payout",
    );
}

// ---------------------------------------------------------------------------
// Funds conservation across a sweep of top_up amounts
// ---------------------------------------------------------------------------

/// The total of what the recipient received plus what remains in the pool
/// always equals the post-top_up deposited amount.  Verified across a range
/// of top_up amounts with a fractional rate, which is the arithmetic stress
/// case for the rate-preserving formula.
///
/// Uses `top_up → withdraw` ordering for all cases.
#[test]
fn conservation_holds_across_top_up_amounts() {
    // Fractional rate: 1_000 stroops over 300 seconds = 3.33/sec.
    let start = T0;
    let h = Harness::new();

    for top_up_amount in [7i128, 13, 101, 1_000, 7_777, 100_000] {
        let id = h.create(1_000, start, start + 300, start, true, true, true);

        h.advance(150);

        h.client.top_up(&id, &top_up_amount);
        let s_after_topup = h.get(id);
        let new_deposited = s_after_topup.deposited;
        let new_end = s_after_topup.end_time;

        let payout = h.client.withdraw(&id, &None);

        // Pool == deposited (after top_up) - payout (what left via withdraw)
        let pool = h.pool();
        assert_eq!(
            pool,
            new_deposited - payout,
            "top_up_amount={top_up_amount}: pool must equal new_deposited - payout",
        );

        // I3: vested did not decrease across the top_up.  We can verify this
        // post-hoc: if vested had decreased, payout would have been less than
        // what the original rate implies; and conservation would break.
        let original_vested = 1_000i128 * 150 / 300; // 500 stroops
        assert!(
            payout >= original_vested,
            "top_up_amount={top_up_amount}: payout {payout} < original_vested {original_vested} \
             (I3 violated: top_up reduced withdrawable)",
        );

        h.assert_pool_exact();

        // Wind the stream to its extended end and verify full delivery.
        h.warp_to(new_end);
        let remaining = h.client.withdraw(&id, &None);
        let final_pool = h.pool();
        assert_eq!(final_pool, 0, "pool must be zero after full delivery");
        assert_eq!(
            payout + remaining,
            new_deposited,
            "top_up_amount={top_up_amount}: total withdrawn must equal new deposited",
        );

        h.assert_pool_exact();
    }
}

// ---------------------------------------------------------------------------
// Same-ledger top_up + withdraw after a prior partial withdrawal
// ---------------------------------------------------------------------------

/// A prior withdrawal does not break same-ledger conservation.
///
/// The recipient draws some funds at time A.  Then, at time B (a different
/// ledger), both top_up and withdraw happen at the same timestamp.  The
/// combined total across both withdrawals must equal exactly the vested
/// amount at time B, and the pool must remain balanced.
#[test]
fn prior_withdrawal_does_not_affect_same_ledger_conservation() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    // First withdrawal at 30 days.
    h.advance(30 * DAY);
    let first_payout = h.client.withdraw(&id, &None);
    assert_eq!(first_payout, 300 * ONE);

    let recipient_after_first = h.balance(&h.recipient);
    let sender_before_topup = h.balance(&h.sender);

    // Now at 50 days: top_up then withdraw in the same ledger.
    h.advance(20 * DAY); // now at 50 days total
    let top_up_amount = 200 * ONE;

    let vested_at_50 = deposit * 50 / 100;   // 500 * ONE
    let already_withdrawn = h.get(id).withdrawn; // 300 * ONE
    let expected_second_payout = vested_at_50 - already_withdrawn; // 200 * ONE

    h.client.top_up(&id, &top_up_amount);
    assert_topped_up_event(&h, id, top_up_amount);

    let recipient_before_second = h.balance(&h.recipient);
    let second_payout = h.client.withdraw(&id, &None);
    assert_withdrawn_event(&h, id, recipient_before_second);

    assert_eq!(
        second_payout, expected_second_payout,
        "second withdrawal must equal vested_at_50 - already_withdrawn",
    );

    // Total received by recipient across both withdrawals.
    let total_received = first_payout + second_payout;
    assert_eq!(
        total_received, vested_at_50,
        "total received must equal vested at time of same-ledger pair",
    );

    // Sender paid the top_up_amount.
    let sender_delta = h.balance(&h.sender) - sender_before_topup;
    assert_eq!(sender_delta, -top_up_amount);

    // Pool: original_deposit + top_up_amount − total_withdrawn_so_far
    let new_deposited = deposit + top_up_amount;
    let total_withdrawn = h.get(id).withdrawn;
    assert_eq!(total_withdrawn, first_payout + second_payout);
    assert_eq!(h.pool(), new_deposited - total_withdrawn);
    h.assert_pool_exact();

    // Balance check from the recipient's perspective.
    let recipient_delta = h.balance(&h.recipient) - recipient_after_first;
    assert_eq!(recipient_delta, second_payout);
}

// ---------------------------------------------------------------------------
// Invariant I3 pin: top_up never reduces withdrawable at the same instant
// ---------------------------------------------------------------------------

/// At a fixed timestamp, `vested_of` must not decrease after `top_up`.
///
/// This is invariant I3, applied specifically to the top_up operation.  The
/// same invariant is asserted exhaustively in `test::monotonicity` and over
/// random schedules in `test::props`, but this test names it explicitly in
/// the context of the same-ledger pair so a regression is immediately
/// attributable.
///
/// Uses awkward (fractional) rates that maximise rounding divergence.
#[test]
fn top_up_does_not_reduce_withdrawable_at_same_instant() {
    let h = Harness::new();
    let start = h.now();
    // Fractional rate: 1_000 stroops over 300 seconds.
    let id = h.create(1_000, start, start + 300, start, true, true, true);

    h.advance(137);

    // Snapshot vested at this exact instant before top_up.
    let vested_before = h.client.vested_of(&id);
    let withdrawable_before = h.client.withdrawable_of(&id);

    // Top up with amounts that produce awkward remainders.
    for amount in [7i128, 13, 101, 1_000] {
        h.client.top_up(&id, &amount);
        // No clock advance between top_ups — all at t=137.

        let vested_after = h.client.vested_of(&id);
        let withdrawable_after = h.client.withdrawable_of(&id);

        assert!(
            vested_after >= vested_before,
            "I3 violated at fixed timestamp: top_up({amount}) reduced vested \
             from {vested_before} to {vested_after}",
        );
        assert!(
            withdrawable_after >= withdrawable_before,
            "I3 violated at fixed timestamp: top_up({amount}) reduced withdrawable \
             from {withdrawable_before} to {withdrawable_after}",
        );
    }

    // Now withdraw — must receive at least the original vested amount.
    let payout = h.client.withdraw(&id, &None);
    assert!(
        payout >= vested_before,
        "payout {payout} < original vested {vested_before} — I3 violation propagated to payment",
    );

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// End-to-end: full stream lifecycle with a same-ledger top_up + withdraw
// ---------------------------------------------------------------------------

/// A stream created, partially drawn, topped up and drawn in the same ledger,
/// then run to full delivery.  Verifies that the extended schedule delivers
/// all funds and the pool is exactly zero at the end.
#[test]
fn full_lifecycle_with_same_ledger_top_up_and_withdraw() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    // Advance to halfway.
    h.advance(50 * DAY);
    let top_up_amount = 500 * ONE;

    // top_up → withdraw in the same ledger.
    h.client.top_up(&id, &top_up_amount);
    assert_topped_up_event(&h, id, top_up_amount);

    let s_after_topup = h.get(id);
    let new_deposited = s_after_topup.deposited;  // 1_500 * ONE
    let new_end = s_after_topup.end_time;

    let recipient_before = h.balance(&h.recipient);
    let payout = h.client.withdraw(&id, &None);
    assert_withdrawn_event(&h, id, recipient_before);

    // Conservation after the same-ledger pair.
    assert_eq!(h.pool(), new_deposited - payout);
    h.assert_pool_exact();

    // Run to the extended end and drain remaining funds.
    h.warp_to(new_end);
    let final_payout = h.client.withdraw(&id, &None);
    assert_eq!(h.pool(), 0, "pool must be empty after full delivery");
    assert_eq!(
        payout + final_payout,
        new_deposited,
        "total withdrawn across both calls must equal the final deposited",
    );

    let s_final = h.get(id);
    assert_eq!(s_final.status, StreamStatus::Depleted);
    assert_eq!(s_final.withdrawn, new_deposited);
    h.assert_pool_exact();
}
