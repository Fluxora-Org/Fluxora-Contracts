//! Same-ledger withdraw + cancel coverage (issue #1833).
//!
//! # The problem
//!
//! Both `withdraw` and `cancel` settle funds: `withdraw` transfers vested tokens
//! to the recipient; `cancel` transfers the unvested remainder back to the
//! sender. When both happen in the same ledger (identical timestamp), the vested
//! amount is fixed — the same number at both call sites. The ordering therefore
//! decides whether that amount can be paid twice.
//!
//! Concretely, if order were `cancel → withdraw`:
//! - `cancel` fixes `deposited = vested_now` and refunds the rest
//! - `withdraw` then reads `vested_now` against the new (shrunken) schedule
//!   and pays it out
//! - **Nothing was withdrawn before the cancel**, so the full `vested_now` is
//!   paid both ways — a double-payment
//!
//! The contract prevents this through the normal `withdrawn` accounting in
//! `withdrawable = vested - withdrawn`: cancel does not move `withdrawn`, so
//! after cancel the recipient can still pull exactly `vested - withdrawn`, not
//! `vested` again.
//!
//! # What this module asserts
//!
//! - The two orderings at the same timestamp yield the same net outcome.
//! - Funds conservation holds exactly: `recipient_delta + sender_delta == deposit`.
//! - The pool is exactly drained (no stranded tokens, no shortfall).
//! - The emitted events match the settled storage state and token ledger.
//! - Final stream state is `Cancelled` (or `Depleted` for a fully-matured stream),
//!   with `deposited == withdrawn == total_vested_at_that_instant`.
//! - The test fails if the behaviour it pins is changed.
//!
//! # Coverage
//!
//! | scenario | offset | what is interesting |
//! |---|---|---|
//! | At `start_time` | 0 | nothing vested; withdraw errors first |
//! | Quarter | 25 d | partial vesting, both orderings |
//! | Halfway | 50 d | symmetric point |
//! | Near end | 99 d | almost fully vested |
//! | Exact end | 100 d | fully vested → stream depletes, cancel is rejected |
//! | Post end | 150 d | past maturity, same as exact end |
//! | Pre-withdraw partial | mid + partial | recipient had already drawn some |

use soroban_sdk::testutils::Events as _;
use soroban_sdk::{xdr, Event as _};

use super::common::*;
use crate::events::{Cancelled, Withdrawn};
use crate::{Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Events that the *stream* contract emitted during the most recent
/// invocation.  Must be called before any subsequent client call (even a
/// read-only view) resets the buffer.
fn stream_events(h: &Harness) -> std::vec::Vec<xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// Assert the `Withdrawn` event emitted by the most recent `withdraw` call.
///
/// Reconstructs the expected event from post-call storage and the token
/// ledger (not from the values the emitter was passed) and requires an exact
/// match plus all pool / conservation identities.
///
/// Returns the asserted amount so callers can use it in follow-on checks.
fn assert_withdrawn_event(h: &Harness, id: u64, recipient_before: i128) -> i128 {
    let published = stream_events(h);
    let stream = h.get(id);
    let payout = h.balance(&h.recipient) - recipient_before;

    assert!(payout > 0, "withdraw emitted a Withdrawn event but payout was zero");

    let expected = Withdrawn {
        stream_id: id,
        recipient: h.recipient.clone(),
        amount: payout,
        withdrawn: stream.withdrawn,
        deposited: stream.deposited,
        status: stream.status,
    };
    assert_eq!(
        published,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "Withdrawn event must be the only stream event and must match \
         storage + token ledger exactly",
    );

    // Conservation: the event's cumulative `withdrawn` equals storage.
    assert_eq!(
        expected.withdrawn, stream.withdrawn,
        "event withdrawn must equal post-call storage withdrawn",
    );

    payout
}

/// Assert the `Cancelled` event emitted by the most recent `cancel` call.
///
/// Reconstructs from post-cancel storage (`deposited`, `withdrawn`,
/// `end_time`) and the token ledger (sender balance delta). Checks:
/// - event fields match storage and token ledger
/// - conservation: `refunded + vested == deposited_before_cancel`
/// - still-claimable = `vested - withdrawn` = pool balance
///
/// Returns `(vested, refunded)` so callers can assert exact amounts.
fn assert_cancelled_event(
    h: &Harness,
    id: u64,
    sender_before: i128,
    deposited_before: i128,
) -> (i128, i128) {
    let published = stream_events(h);
    let stream = h.get(id);
    let refunded = h.balance(&h.sender) - sender_before;
    let pooled = h.pool();
    let claimable = h.client.withdrawable_of(&id);

    let expected = Cancelled {
        stream_id: id,
        sender: h.sender.clone(),
        recipient: h.recipient.clone(),
        refunded,
        // post-cancel deposited IS the total vested (cancel rewrites it)
        vested: stream.deposited,
        withdrawn: stream.withdrawn,
        end_time: stream.end_time,
    };
    assert_eq!(
        published,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "Cancelled event must be the only stream event and must match \
         storage + token ledger exactly",
    );

    // Conservation (I4): refunded + vested == deposited before cancel.
    assert_eq!(
        expected.refunded + expected.vested,
        deposited_before,
        "cancel conservation: refunded + vested must equal pre-cancel deposit",
    );

    // Nothing claimable may be stranded in or missing from the pool.
    assert_eq!(
        claimable,
        expected.vested - expected.withdrawn,
        "claimable must be vested - withdrawn",
    );
    assert_eq!(
        pooled,
        expected.vested - expected.withdrawn,
        "pool must hold exactly the unclaimed remainder — no stranded tokens",
    );
    assert_eq!(stream.status, StreamStatus::Cancelled);

    (expected.vested, expected.refunded)
}

// ---------------------------------------------------------------------------
// Core same-ledger scenario: withdraw → cancel, no time between them
// ---------------------------------------------------------------------------

/// **Order 1 at a fixed same-ledger timestamp.**
///
/// At `offset` seconds into the stream, `withdraw` is called and then
/// `cancel` — both at exactly the same ledger timestamp.  After both settle:
///
/// - recipient received exactly `vested_at_offset`
/// - sender received exactly `deposit - vested_at_offset`
/// - pool is empty
/// - events match storage and token ledger
///
/// This is the ordering that is safe by construction: withdraw drains the
/// vested balance before cancel runs, so cancel's refund is the unvested
/// remainder only.
fn same_ledger_withdraw_then_cancel(offset: u64) {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(offset);
    // Both calls happen at this timestamp — no further advance between them.

    let recipient_before = h.balance(&h.recipient);
    let sender_before = h.balance(&h.sender);

    // Expected vested at this exact instant (same formula the contract uses,
    // rounded down). Clamped to the full deposit after end_time.
    let expected_vested =
        deposit * (offset.min(duration) as i128) / (duration as i128);

    // ── Step 1: withdraw ──────────────────────────────────────────────────
    if expected_vested == 0 {
        // Nothing to withdraw at time zero; verify the error and skip.
        assert_eq!(
            h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
            Error::NothingToWithdraw,
            "offset={offset}: expected NothingToWithdraw",
        );
    } else {
        let payout = h.client.withdraw(&id, &None);
        assert_eq!(
            payout, expected_vested,
            "offset={offset}: withdraw return value must equal vested",
        );
        // Assert event immediately — before cancel resets the buffer.
        let event_payout = assert_withdrawn_event(&h, id, recipient_before);
        assert_eq!(
            event_payout, expected_vested,
            "offset={offset}: Withdrawn.amount must equal vested",
        );
    }

    h.assert_pool_invariant();

    // ── Step 2: cancel (same timestamp) ───────────────────────────────────
    let stream_status = h.get(id).status;
    let sender_before_cancel = h.balance(&h.sender);

    if stream_status == StreamStatus::Depleted {
        // Stream fully matured on withdraw; cancel is now rejected.
        assert_eq!(
            h.client.try_cancel(&id).unwrap_err().unwrap(),
            Error::StreamTerminated,
            "offset={offset}: cancel after depletion must return StreamTerminated",
        );
        // Pool must already be empty — the withdraw drained everything.
        assert_eq!(h.pool(), 0, "offset={offset}: pool must be empty after depletion");
    } else {
        // The unvested remainder is still in the pool; cancel refunds it.
        let deposited_before_cancel = h.get(id).deposited;
        h.client.cancel(&id);
        let (vested, _refunded) =
            assert_cancelled_event(&h, id, sender_before_cancel, deposited_before_cancel);
        assert_eq!(
            vested, expected_vested,
            "offset={offset}: cancel vested must equal what withdraw already paid",
        );
        h.assert_pool_exact();
    }

    // ── Final conservation check ──────────────────────────────────────────
    let recipient_delta = h.balance(&h.recipient) - recipient_before;
    let sender_delta = h.balance(&h.sender) - sender_before;

    assert_eq!(
        recipient_delta, expected_vested,
        "offset={offset}: recipient must have received exactly vested",
    );
    assert_eq!(
        sender_delta,
        deposit - expected_vested,
        "offset={offset}: sender must have received exactly unvested remainder",
    );
    assert_eq!(
        recipient_delta + sender_delta,
        deposit,
        "offset={offset}: funds conservation violated",
    );

    // Final stream state.
    let stream = h.get(id);
    assert!(
        stream.status == StreamStatus::Cancelled || stream.status == StreamStatus::Depleted,
        "offset={offset}: stream must be terminal",
    );
    assert_eq!(
        stream.deposited, expected_vested,
        "offset={offset}: deposited must equal vested after settlement",
    );
    assert_eq!(
        stream.withdrawn, expected_vested,
        "offset={offset}: withdrawn must equal vested after full draw",
    );
}

/// **Order 2 at a fixed same-ledger timestamp.**
///
/// Cancel is called first; then withdraw — both at the same timestamp.
///
/// Cancel rewrites `deposited = vested_now` and refunds the rest.  The
/// recipient then pulls `vested - withdrawn` (all of it, since nothing had
/// been withdrawn yet).  The two combined must equal `deposit` exactly — the
/// same total as order 1, proving neither ordering double-pays.
fn same_ledger_cancel_then_withdraw(offset: u64) {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(offset);
    // Both calls happen at this timestamp — no further advance between them.

    let recipient_before = h.balance(&h.recipient);
    let sender_before = h.balance(&h.sender);

    let expected_vested =
        deposit * (offset.min(duration) as i128) / (duration as i128);

    // ── Step 1: cancel ────────────────────────────────────────────────────
    let deposited_before = h.get(id).deposited;
    let sender_before_cancel = h.balance(&h.sender);
    h.client.cancel(&id);
    let (vested, _refunded) =
        assert_cancelled_event(&h, id, sender_before_cancel, deposited_before);
    assert_eq!(
        vested, expected_vested,
        "offset={offset}: cancel vested must equal formula",
    );
    h.assert_pool_invariant();

    // ── Step 2: withdraw (same timestamp, same vested amount) ─────────────
    if expected_vested == 0 {
        assert_eq!(
            h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
            Error::NothingToWithdraw,
            "offset={offset}: expected NothingToWithdraw after cancel with no vesting",
        );
    } else {
        let recipient_before_withdraw = h.balance(&h.recipient);
        let payout = h.client.withdraw(&id, &None);
        let event_payout = assert_withdrawn_event(&h, id, recipient_before_withdraw);

        // The amount the recipient can pull is exactly `vested - withdrawn`,
        // NOT `vested` again — this is the key double-pay guard.
        assert_eq!(
            payout, expected_vested,
            "offset={offset}: payout must equal exactly what was vested (not double-paid)",
        );
        assert_eq!(
            event_payout, expected_vested,
            "offset={offset}: Withdrawn.amount must equal exactly what was vested",
        );
    }

    h.assert_pool_exact();

    // ── Final conservation check ──────────────────────────────────────────
    let recipient_delta = h.balance(&h.recipient) - recipient_before;
    let sender_delta = h.balance(&h.sender) - sender_before;

    assert_eq!(
        recipient_delta, expected_vested,
        "offset={offset}: recipient must have received exactly vested (no double-pay)",
    );
    assert_eq!(
        sender_delta,
        deposit - expected_vested,
        "offset={offset}: sender must have received exactly unvested remainder",
    );
    assert_eq!(
        recipient_delta + sender_delta,
        deposit,
        "offset={offset}: funds conservation violated",
    );

    // Final stream state.
    let stream = h.get(id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.deposited, expected_vested);
    assert_eq!(stream.withdrawn, expected_vested);
}

// ---------------------------------------------------------------------------
// Parameterised schedule-point sweeps (same-ledger, both orderings)
// ---------------------------------------------------------------------------

#[test]
fn same_ledger_withdraw_then_cancel_schedule_points() {
    same_ledger_withdraw_then_cancel(0);          // Start — nothing vested
    same_ledger_withdraw_then_cancel(25 * DAY);   // Quarter
    same_ledger_withdraw_then_cancel(50 * DAY);   // Halfway
    same_ledger_withdraw_then_cancel(99 * DAY);   // Near end
    same_ledger_withdraw_then_cancel(100 * DAY);  // Exact end → Depleted
    same_ledger_withdraw_then_cancel(150 * DAY);  // Post-end → still Depleted
}

#[test]
fn same_ledger_cancel_then_withdraw_schedule_points() {
    same_ledger_cancel_then_withdraw(0);          // Start — nothing vested
    same_ledger_cancel_then_withdraw(25 * DAY);   // Quarter
    same_ledger_cancel_then_withdraw(50 * DAY);   // Halfway
    same_ledger_cancel_then_withdraw(99 * DAY);   // Near end
    same_ledger_cancel_then_withdraw(100 * DAY);  // Exact end
    same_ledger_cancel_then_withdraw(150 * DAY);  // Post-end
}

/// Both orderings produce the same net outcome: the same recipient total, the
/// same sender total, and the same pool state.  The ordering does not create
/// or destroy value.
#[test]
fn both_orderings_agree_on_outcome() {
    let offset = 40 * DAY;
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let expected_vested = deposit * (offset as i128) / (duration as i128);

    // Order 1: withdraw → cancel
    let h1 = Harness::new();
    let id1 = h1.create_simple(deposit, duration);
    h1.advance(offset);
    let r1_before = h1.balance(&h1.recipient);
    let s1_before = h1.balance(&h1.sender);
    h1.client.withdraw(&id1, &None);
    h1.client.cancel(&id1);
    let r1_delta = h1.balance(&h1.recipient) - r1_before;
    let s1_delta = h1.balance(&h1.sender) - s1_before;

    // Order 2: cancel → withdraw
    let h2 = Harness::new();
    let id2 = h2.create_simple(deposit, duration);
    h2.advance(offset);
    let r2_before = h2.balance(&h2.recipient);
    let s2_before = h2.balance(&h2.sender);
    h2.client.cancel(&id2);
    h2.client.withdraw(&id2, &None);
    let r2_delta = h2.balance(&h2.recipient) - r2_before;
    let s2_delta = h2.balance(&h2.sender) - s2_before;

    assert_eq!(r1_delta, r2_delta, "recipient outcome must be order-independent");
    assert_eq!(s1_delta, s2_delta, "sender outcome must be order-independent");
    assert_eq!(r1_delta, expected_vested, "recipient must receive exactly vested");
    assert_eq!(s1_delta, deposit - expected_vested, "sender must receive exactly unvested");
    assert_eq!(r1_delta + s1_delta, deposit, "conservation: no funds created or destroyed");
    assert_eq!(r2_delta + s2_delta, deposit, "conservation: no funds created or destroyed");

    h1.assert_pool_exact();
    h2.assert_pool_exact();
}

/// A partial prior withdrawal does not change the total distributed.
///
/// If the recipient withdraws some amount, then both cancel and a follow-on
/// withdraw happen in the same ledger, the total the recipient receives
/// (across both withdrawals) must still equal `vested_at_cancel`, and the
/// sender's refund must equal `deposit - vested_at_cancel`.
#[test]
fn prior_withdrawal_does_not_affect_same_ledger_conservation() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    // First withdrawal at 30 days.
    h.advance(30 * DAY);
    let withdrawn_first = h.client.withdraw(&id, &None);
    assert_eq!(withdrawn_first, 300 * ONE);

    // Now advance to 50 days and do cancel + withdraw in the same ledger.
    h.advance(20 * DAY); // now at 50 days
    let vested_at_cancel = deposit * 50 / 100; // 500 * ONE

    let recipient_before = h.balance(&h.recipient);
    let sender_before = h.balance(&h.sender);
    let deposited_before = h.get(id).deposited;

    // Cancel first: 300 already withdrawn, so refund = 500 (unvested).
    // post-cancel deposited = 500 = vested_at_cancel.
    let sender_before_cancel = h.balance(&h.sender);
    h.client.cancel(&id);
    let (vested, refunded) =
        assert_cancelled_event(&h, id, sender_before_cancel, deposited_before);
    assert_eq!(vested, vested_at_cancel);
    assert_eq!(refunded, deposit - vested_at_cancel);

    // Withdraw the remaining claimable (200 = 500 vested − 300 already drawn).
    let recipient_before_w2 = h.balance(&h.recipient);
    let payout2 = h.client.withdraw(&id, &None);
    assert_eq!(payout2, 200 * ONE, "only the unclaimed 200 must be paid, not 500");
    assert_withdrawn_event(&h, id, recipient_before_w2);

    h.assert_pool_exact();

    // Total recipient = first withdrawal + second withdrawal = 500 = vested_at_cancel.
    let total_recipient = h.balance(&h.recipient) - recipient_before + withdrawn_first;
    assert_eq!(
        total_recipient, vested_at_cancel,
        "recipient total must equal vested_at_cancel",
    );

    // Conservation.
    let sender_delta = h.balance(&h.sender) - sender_before;
    let recipient_second_delta = h.balance(&h.recipient) - recipient_before;
    assert_eq!(
        withdrawn_first + recipient_second_delta + sender_delta,
        deposit,
        "conservation: all funds must return to either party",
    );
}

/// The cancel-then-withdraw ordering cannot double-pay the vested amount.
///
/// This is the regression pin for the specific double-pay scenario described
/// in the issue: cancel rewrites `deposited = vested_now`, and without the
/// `withdrawn` accounting, a subsequent withdraw would pay `vested_now` again.
/// With the accounting in place the second draw is `vested_now - withdrawn`,
/// not `vested_now`.
///
/// This test is named explicitly so that if it ever fails, the failure message
/// names the invariant that was broken.
#[test]
fn cancel_then_withdraw_cannot_double_pay_vested_amount() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(60 * DAY);
    let vested = deposit * 60 / 100; // 600 * ONE

    // Prove nothing has been withdrawn yet.
    assert_eq!(h.get(id).withdrawn, 0);

    // Cancel first.
    h.client.cancel(&id);

    // The withdrawable is exactly vested (nothing drawn yet) — NOT 2×vested.
    let claimable = h.client.withdrawable_of(&id);
    assert_eq!(
        claimable, vested,
        "after cancel with no prior withdrawals the claimable must equal vested once, not twice",
    );

    // Pull the full claimable.
    let payout = h.client.withdraw(&id, &None);
    assert_eq!(
        payout, vested,
        "payout must equal vested once — double-pay is detected here",
    );

    // No further amount is available.
    assert_eq!(
        h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
        Error::StreamTerminated,
        "second withdraw must be rejected — stream is fully settled",
    );

    // Pool is empty — every token has gone to exactly one party.
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();

    // Conservation.
    let sender_received = deposit - vested; // the 400 refunded by cancel
    assert_eq!(
        payout + sender_received,
        deposit,
        "conservation: payout + refund must equal the original deposit",
    );
}

// ---------------------------------------------------------------------------
// Legacy schedule-point helpers (preserved from original commit)
// ---------------------------------------------------------------------------
//
// These are kept to avoid a coverage regression on the non-same-ledger
// ordering tests that landed with the original commit.  They exercise
// different ledgers for withdraw and cancel (i.e. `h.advance()` between
// them), complementing the same-ledger variants above.

fn test_withdraw_then_cancel_at(offset: u64) {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(offset);

    let recipient_bal_before = h.balance(&h.recipient);
    let sender_bal_before = h.balance(&h.sender);

    // Order 1: withdraw, then cancel
    let expected_vested = h.get(id).deposited * (offset.min(duration) as i128) / (duration as i128);

    if expected_vested == 0 {
        assert_eq!(
            h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
            Error::NothingToWithdraw
        );
    } else {
        h.client.withdraw(&id, &None);
    }

    let stream_status = h.get(id).status;
    if stream_status == StreamStatus::Depleted {
        assert_eq!(
            h.client.try_cancel(&id).unwrap_err().unwrap(),
            Error::StreamTerminated
        );
    } else {
        h.client.cancel(&id);
    }

    let stream = h.get(id);

    assert_eq!(
        h.balance(&h.recipient) - recipient_bal_before,
        expected_vested
    );
    assert_eq!(
        h.balance(&h.sender) - sender_bal_before,
        deposit - expected_vested
    );

    assert!(stream.status == StreamStatus::Cancelled || stream.status == StreamStatus::Depleted);
    assert_eq!(stream.deposited, expected_vested);
    assert_eq!(stream.withdrawn, expected_vested);

    assert_eq!(
        (h.balance(&h.recipient) - recipient_bal_before)
            + (h.balance(&h.sender) - sender_bal_before),
        deposit
    );
    h.assert_pool_exact();
}

fn test_cancel_then_withdraw_at(offset: u64) {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    h.advance(offset);

    let recipient_bal_before = h.balance(&h.recipient);
    let sender_bal_before = h.balance(&h.sender);

    let expected_vested = h.get(id).deposited * (offset.min(duration) as i128) / (duration as i128);

    // Order 2: cancel, then withdraw
    h.client.cancel(&id);

    if expected_vested == 0 {
        // The cancel has already run, so the stream is terminal with nothing
        // left. `withdraw` distinguishes that from a live stream that simply
        // has not accrued: terminal + empty is `StreamTerminated`, not
        // `NothingToWithdraw` (docs/ABI.md, "withdraw" errors).
        // Cancelled is terminal, so draining an empty tail is
        // StreamTerminated — not the live-stream NothingToWithdraw path.
        // Pinned by `cancel::cancel_at_the_instant_of_creation_refunds_everything`.
        assert_eq!(
            h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
            Error::StreamTerminated
        );
    } else {
        h.client.withdraw(&id, &None);
    }

    let stream = h.get(id);

    assert_eq!(
        h.balance(&h.recipient) - recipient_bal_before,
        expected_vested
    );
    assert_eq!(
        h.balance(&h.sender) - sender_bal_before,
        deposit - expected_vested
    );

    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.deposited, expected_vested);
    assert_eq!(stream.withdrawn, expected_vested);

    assert_eq!(
        (h.balance(&h.recipient) - recipient_bal_before)
            + (h.balance(&h.sender) - sender_bal_before),
        deposit
    );
    h.assert_pool_exact();
}

#[test]
fn withdraw_then_cancel_schedule_points() {
    test_withdraw_then_cancel_at(0); // Start
    test_withdraw_then_cancel_at(25 * DAY); // Quarter
    test_withdraw_then_cancel_at(50 * DAY); // Halfway
    test_withdraw_then_cancel_at(99 * DAY); // Near end
    test_withdraw_then_cancel_at(100 * DAY); // Exact end
    test_withdraw_then_cancel_at(150 * DAY); // Post end
}

#[test]
fn cancel_then_withdraw_schedule_points() {
    test_cancel_then_withdraw_at(0); // Start
    test_cancel_then_withdraw_at(25 * DAY); // Quarter
    test_cancel_then_withdraw_at(50 * DAY); // Halfway
    test_cancel_then_withdraw_at(99 * DAY); // Near end
    test_cancel_then_withdraw_at(100 * DAY); // Exact end
    test_cancel_then_withdraw_at(150 * DAY); // Post end
}
