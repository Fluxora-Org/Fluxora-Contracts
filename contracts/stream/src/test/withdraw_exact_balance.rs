//! Issue #1839 — withdrawal of *exactly* the full withdrawable amount.
//!
//! `accrual::withdrawable` computes `vested(now) - withdrawn` and then
//! **saturates at zero** (`available < 0 => 0`). The clamp is deliberate
//! defence in depth, but it also means an off-by-one at the boundary — a
//! `vested` one stroop short of `withdrawn`, or a payout that leaves a stroop
//! stranded — is invisible to any assertion that only checks "withdrawable is
//! zero afterwards". The only way to pin the boundary is to drain it exactly:
//! pay `Some(withdrawable)` and require the resulting state, emitted event and
//! token balances to be exact, then require the *next* call to be rejected with
//! the error the balance state actually calls for.
//!
//! The scenarios, all driven through the public ABI:
//!
//! | scenario | expected after the exact draw |
//! |---|---|
//! | mid-schedule, `withdrawn < deposited` | `withdrawable == 0`, status stays `Active`, next call `NothingToWithdraw` (17) |
//! | final draw, `withdrawn == deposited` | `withdrawable == 0`, status flips to `Depleted`, next call `StreamTerminated` (14) |
//! | truncating schedule | exact draw pays the floored `vested`, residue stays pooled as the sender's refundable |
//!
//! Everything asserted here is the documented contract in `docs/ABI.md`
//! §`withdraw` and the "empty balance" distinction at lines 249–259: on a live
//! stream a zero available balance is `NothingToWithdraw` regardless of the
//! requested amount (the zero check runs first, so `InsufficientWithdrawable` is
//! *not* reachable there); a `Cancelled`/`Depleted` stream with nothing left is
//! `StreamTerminated`; `Some(available)` succeeds; and draining to `deposited`
//! flips a non-`Cancelled` stream to `Depleted`. No documentation change is
//! required — these tests confirm the document rather than correcting it.

use soroban_sdk::testutils::Events as _;
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::Withdrawn;
use crate::{Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The events the *stream* contract published during the most recent
/// invocation.
///
/// `Events::all()` only reports the last invocation, so this must be called
/// immediately after the withdrawal — before any other client call (including
/// the read-only `get_stream` view) replaces the snapshot. Same constraint as
/// `test::withdraw_events::published_by_stream`.
fn stream_events(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

// ---------------------------------------------------------------------------
// Mid-schedule: the exact draw empties the balance but is not terminal
// ---------------------------------------------------------------------------

/// Drawing exactly `withdrawable_of(id)` part-way through the schedule leaves
/// `withdrawable == 0` and the stream `Active`, and the next attempt is the
/// *live-but-empty* `NothingToWithdraw`, never `InsufficientWithdrawable`.
#[test]
fn exact_withdrawable_midstream_clears_to_zero_and_stays_active() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    // Read the boundary value through the public ABI before touching state.
    let available = h.client.withdrawable_of(&id);
    assert_eq!(available, 300 * ONE, "30 of 100 days on 1000 ONE");

    let recipient_before = h.balance(&h.recipient);

    // Draw exactly the whole withdrawable amount, explicitly.
    let returned = h.client.withdraw(&id, &Some(available));
    let published = stream_events(&h);

    // Return value and token delta are both the exact boundary value.
    assert_eq!(returned, available);
    assert_eq!(h.balance(&h.recipient) - recipient_before, available);

    let stream = h.get(id);
    assert_eq!(stream.withdrawn, available, "withdrawn == the exact draw");
    assert_eq!(
        stream.status,
        StreamStatus::Active,
        "a mid-schedule drain is not terminal: withdrawn < deposited",
    );

    // The single emitted event is built from independent ground truth (the
    // post-call stream state and the token delta) and must match exactly.
    assert_eq!(
        published,
        std::vec![Withdrawn {
            stream_id: id,
            recipient: h.recipient.clone(),
            amount: available,
            withdrawn: available,
            deposited: 1_000 * ONE,
            status: StreamStatus::Active,
        }
        .to_xdr(&h.env, &h.contract_id)],
        "the Withdrawn event must be the only stream event and match storage",
    );

    // The boundary itself: withdrawable is exactly zero — not one stroop, not
    // negative. This is the value the saturating subtraction would mask.
    assert_eq!(h.client.withdrawable_of(&id), 0);

    // A second attempt — implicit or explicit — sees a live stream with a zero
    // balance. The zero check precedes the amount comparison, so every request
    // reports `NothingToWithdraw` (17); the stream is not terminal, so
    // `StreamTerminated` (14) is not reachable and `InsufficientWithdrawable`
    // (16) is not reachable either.
    for amount in [None, Some(1i128), Some(available)] {
        let err = h.client.try_withdraw(&id, &amount).unwrap_err().unwrap();
        assert_eq!(err, Error::NothingToWithdraw, "amount {amount:?}");
        assert!(
            stream_events(&h).is_empty(),
            "a rejected draw emits no event"
        );
    }

    assert_eq!(h.get(id).withdrawn, available, "rejections changed nothing");
    assert_eq!(h.balance(&h.recipient) - recipient_before, available);

    h.assert_pool_invariant();
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Terminal: the exact draw reaches `deposited` and flips to `Depleted`
// ---------------------------------------------------------------------------

/// Draining the last of a stream with an **explicit** amount — not `None` —
/// must still reach `withdrawn == deposited`, flip the status to `Depleted`,
/// empty the pool for that stream, and make the next call `StreamTerminated`.
#[test]
fn exact_remaining_balance_at_end_time_flips_to_depleted() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Take a partial draw first so the final exact draw is a remainder rather
    // than the whole deposit.
    h.advance(40 * DAY);
    assert_eq!(h.client.withdraw(&id, &Some(100 * ONE)), 100 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);

    h.warp_to(T0 + 100 * DAY);
    let remaining = h.client.withdrawable_of(&id);
    assert_eq!(remaining, 900 * ONE, "deposited - withdrawn at end_time");
    assert_eq!(remaining, h.get(id).deposited - h.get(id).withdrawn);

    let recipient_before = h.balance(&h.recipient);
    let returned = h.client.withdraw(&id, &Some(remaining));
    let published = stream_events(&h);

    assert_eq!(returned, remaining);
    assert_eq!(h.balance(&h.recipient) - recipient_before, remaining);
    assert_eq!(
        h.balance(&h.recipient),
        1_000 * ONE,
        "full deposit paid out"
    );

    let stream = h.get(id);
    assert_eq!(stream.withdrawn, 1_000 * ONE);
    assert_eq!(
        stream.status,
        StreamStatus::Depleted,
        "withdrawn reached deposited on a non-Cancelled stream",
    );
    assert_eq!(
        published,
        std::vec![Withdrawn {
            stream_id: id,
            recipient: h.recipient.clone(),
            amount: remaining,
            withdrawn: 1_000 * ONE,
            deposited: 1_000 * ONE,
            status: StreamStatus::Depleted,
        }
        .to_xdr(&h.env, &h.contract_id)],
    );

    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.pool(), 0, "the deposit left the pool in full");

    // The exact draw closed the stream, so the follow-up error is the terminal
    // one (14), not the live-but-empty `NothingToWithdraw` (17).
    for amount in [None, Some(1i128)] {
        let err = h.client.try_withdraw(&id, &amount).unwrap_err().unwrap();
        assert_eq!(err, Error::StreamTerminated, "amount {amount:?}");
        assert!(
            stream_events(&h).is_empty(),
            "a rejected draw emits no event"
        );
    }
    assert_eq!(
        h.get(id).withdrawn,
        1_000 * ONE,
        "rejections changed nothing"
    );

    h.assert_pool_invariant();
    h.assert_pool_exact();
}

/// The literal boundary from the issue title: one explicit draw equal to the
/// entire deposit, at exactly `end_time`, in a single call.
#[test]
fn exact_withdrawable_equal_to_the_full_deposit_depletes_in_one_call() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);

    let available = h.client.withdrawable_of(&id);
    assert_eq!(available, 1_000 * ONE, "fully vested at end_time");
    assert_eq!(available, h.get(id).deposited, "available == deposited");

    let recipient_before = h.balance(&h.recipient);
    let returned = h.client.withdraw(&id, &Some(available));
    let published = stream_events(&h);

    assert_eq!(returned, available);
    assert_eq!(h.balance(&h.recipient) - recipient_before, available);

    let stream = h.get(id);
    assert_eq!(stream.withdrawn, stream.deposited);
    assert_eq!(stream.status, StreamStatus::Depleted);
    assert_eq!(
        published,
        std::vec![Withdrawn {
            stream_id: id,
            recipient: h.recipient.clone(),
            amount: available,
            withdrawn: available,
            deposited: available,
            status: StreamStatus::Depleted,
        }
        .to_xdr(&h.env, &h.contract_id)],
    );

    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.pool(), 0);
    assert_eq!(
        h.client
            .try_withdraw(&id, &Some(available))
            .unwrap_err()
            .unwrap(),
        Error::StreamTerminated,
    );

    h.assert_pool_invariant();
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// The stroop either side of the boundary
// ---------------------------------------------------------------------------

/// One stroop *over* the exact drawable is `InsufficientWithdrawable` and moves
/// nothing; exactly at the boundary succeeds; after it the balance reads zero.
/// This is the off-by-one the saturating subtraction can hide.
#[test]
fn one_stroop_over_the_exact_withdrawable_is_rejected_without_side_effects() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let available = h.client.withdrawable_of(&id);
    assert_eq!(available, 300 * ONE);

    // A positive balance is present, so an over-request is
    // `InsufficientWithdrawable` (16) — *not* the zero-balance
    // `NothingToWithdraw` (17) — and must not touch stream or token state.
    let err = h
        .client
        .try_withdraw(&id, &Some(available + 1))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable);
    assert!(
        stream_events(&h).is_empty(),
        "a rejected draw emits no event"
    );

    let stream = h.get(id);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(h.balance(&h.recipient), 0);
    assert_eq!(
        h.client.withdrawable_of(&id),
        available,
        "balance unchanged"
    );

    // Exactly at the boundary succeeds, and only now does the balance read 0.
    assert_eq!(h.client.withdraw(&id, &Some(available)), available);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(
        h.client.try_withdraw(&id, &Some(1)).unwrap_err().unwrap(),
        Error::NothingToWithdraw,
    );

    h.assert_pool_invariant();
    h.assert_pool_exact();
}

/// When `deposited * elapsed / duration` truncates, `vested` rounds down and
/// leaves a residue. Drawing exactly `withdrawable` must pay the floored amount
/// and leave the residue pooled as the sender's refundable claim — never a
/// stroop past the boundary, and never a stranded stroop with no accounting.
#[test]
fn exact_withdrawable_of_a_truncating_schedule_leaves_the_residue_pooled() {
    let h = Harness::new();

    // One stroop above 100 ONE over 100 days: 30/100 of it does not divide
    // evenly, so `vested` must floor and hold a residue in the pool.
    let deposit = 100 * ONE + 1;
    let id = h.create_simple(deposit, 100 * DAY);
    h.advance(30 * DAY);

    let vested = h.client.vested_of(&id);
    let available = h.client.withdrawable_of(&id);
    let refundable = h.client.refundable_of(&id);

    // Ground truth by hand: floor(deposit * 30 / 100) == 30 * ONE exactly.
    assert_eq!(vested, 30 * ONE, "vested rounds down");
    assert_eq!(available, vested, "nothing withdrawn yet");
    assert!(available < deposit, "truncation leaves a residue");
    assert_eq!(vested + refundable, deposit, "conservation before the draw");

    let recipient_before = h.balance(&h.recipient);
    assert_eq!(h.client.withdraw(&id, &Some(available)), available);
    assert_eq!(h.balance(&h.recipient) - recipient_before, available);

    // The exact draw lands on zero, not on a one-stroop overshoot.
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.get(id).withdrawn, vested);
    assert_eq!(
        h.get(id).status,
        StreamStatus::Active,
        "a partial drain does not terminate the stream",
    );

    // The floored residue is still the sender's and still backs the pool.
    assert_eq!(h.client.refundable_of(&id), refundable);
    assert_eq!(
        h.client.vested_of(&id) + h.client.refundable_of(&id),
        deposit
    );
    assert_eq!(h.pool(), refundable, "residue stays pooled for the sender");

    // Draining the balance did not make the boundary terminal.
    assert_eq!(
        h.client.try_withdraw(&id, &Some(1)).unwrap_err().unwrap(),
        Error::NothingToWithdraw,
    );

    h.assert_pool_invariant();
    h.assert_pool_exact();
}
