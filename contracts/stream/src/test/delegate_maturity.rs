//! A delegate acting on a stream that reaches maturity mid-call (Issue #1924).
//!
//! # The gap this module closes
//!
//! `test::delegation` covers the *grant* check — who may act, and whether the
//! grant is live. `test::terminal_operations` covers *terminal transitions* —
//! what happens to a stream that is already `Cancelled` or `Depleted`. Neither
//! names the instant in between: the moment the accrual clock reaches
//! `end_time`, the stream becomes **matured**. Maturity is a property of the
//! clock, not of `status`, and it is exactly where those two modules meet.
//!
//! # Maturity is not a status
//!
//! The distinction that makes these cases non-obvious, and that every test
//! below pins:
//!
//! * At `end_time` the whole deposit has vested, but `status` is still
//!   `Active`. Maturity alone is **not** terminal.
//! * `Depleted` is reached only when `withdrawn == deposited` — that is, when
//!   the matured claim is actually drawn.
//!
//! So a *matured* stream still accepts `pause` (it has a live schedule and a
//! recipient who has not been paid), and only a matured **and drained** stream
//! rejects it. `test::pause::pausing_a_matured_stream_changes_nothing` already
//! pins that for the direct path; the delegated path is pinned here so the two
//! cannot drift.
//!
//! # Coverage map
//!
//! | case | outcome | test |
//! |---|---|---|
//! | withdrawal at the maturity instant settles the exact remainder | pays `deposited - withdrawn` | `delegate_withdraw_at_maturity_settles_the_exact_remainder` |
//! | — with an awkward partial draw first | 370 + 630 = 1000, no dust | `delegate_withdraw_at_maturity_settles_an_inexact_remainder_exactly` |
//! | — never pays twice | `StreamTerminated` | `a_second_delegated_withdraw_at_maturity_pays_nothing` |
//! | pause on a matured stream | allowed, matches direct path | `delegate_pause_on_a_matured_but_undrained_stream_is_allowed` |
//! | pause on a matured **and drained** stream | `StreamTerminated` | `delegate_pause_on_a_matured_and_drained_stream_is_rejected` |
//! | cancel at maturity refunds nothing | refund `0` | `delegate_cancel_at_maturity_refunds_nothing` |
//! | — after a partial draw | refund still `0` | `delegate_cancel_at_maturity_refunds_nothing_after_a_partial_draw` |
//!
//! Every case asserts conservation (`h.assert_pool_exact()`, which also runs
//! invariants I1/I4 and the pool invariant) so the maturity instant cannot
//! quietly strand or over-pay tokens.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A 1000-token / 100-day stream with a delegate holding exactly `op_bit`,
/// granted by the party that owns it.
///
/// The delegate is funded on purpose: no test below may settle a figure out of
/// the delegate's own balance rather than the stream's.
fn stream_with_grant(h: &Harness, op_bit: u32) -> (u64, Address) {
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    let grantor = match op_bit {
        op::WITHDRAW | op::TRANSFER_RECIPIENT => &h.recipient,
        _ => &h.sender,
    };
    h.client
        .grant_delegate(&id, grantor, &agent, &op_bit, &None);
    (id, agent)
}

/// Position the ledger at exactly the stream's maturity instant.
///
/// `end_time` is read from the stored record rather than recomputed, so the
/// instant is the contract's own, not the test's arithmetic.
fn warp_to_maturity(h: &Harness, id: u64) -> u64 {
    let end = h.get(id).end_time;
    h.warp_to(end);
    end
}

// ---------------------------------------------------------------------------
// 1. Withdrawal at the maturity instant
// ---------------------------------------------------------------------------

/// A delegated withdrawal taken at `end_time` settles exactly the remainder.
///
/// Maturity means the whole deposit has vested, so the withdrawable balance is
/// `deposited - withdrawn` — the exact remainder after the partial draw — and
/// the withdrawal must deliver precisely that, no more and no less. Drawing it
/// exhausts the claim, so the stream becomes `Depleted` and the pool returns to
/// zero: every deposited token has moved to the recipient and none is stranded.
#[test]
fn delegate_withdraw_at_maturity_settles_the_exact_remainder() {
    let h = Harness::new();
    let (id, agent) = stream_with_grant(&h, op::WITHDRAW);

    h.advance(30 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE, "partial draw");

    let end = warp_to_maturity(&h, id);
    assert_eq!(
        h.client.vested_of(&id),
        1_000 * ONE,
        "the whole deposit vests at the maturity instant (end_time={end})",
    );
    assert_eq!(
        h.get(id).status,
        StreamStatus::Active,
        "maturity alone is not a status: the stream is not terminal yet",
    );

    let paid = h.client.delegate_withdraw(&id, &agent, &None);

    assert_eq!(
        paid,
        700 * ONE,
        "the remainder is exactly deposited - withdrawn",
    );
    let after = h.get(id);
    assert_eq!(after.withdrawn, 1_000 * ONE, "the claim is settled in full");
    assert_eq!(
        after.status,
        StreamStatus::Depleted,
        "settling a matured claim exhausts the stream",
    );
    assert_eq!(h.pool(), 0, "nothing is left pooled");
    assert_eq!(h.balance(&h.recipient), 1_000 * ONE);
    h.assert_pool_exact();
}

/// The remainder need not divide evenly. Drawing 37/100 of a 1000-token stream
/// first leaves 630 to collect, and the maturity withdrawal must move exactly
/// that — the boundary is the clock, not the arithmetic, so no rounding dust
/// may appear here.
#[test]
fn delegate_withdraw_at_maturity_settles_an_inexact_remainder_exactly() {
    let h = Harness::new();
    let (id, agent) = stream_with_grant(&h, op::WITHDRAW);

    h.advance(37 * DAY);
    assert_eq!(h.client.delegate_withdraw(&id, &agent, &None), 370 * ONE);
    assert_eq!(h.get(id).withdrawn, 370 * ONE);

    warp_to_maturity(&h, id);
    let paid = h.client.delegate_withdraw(&id, &agent, &None);

    assert_eq!(paid, 630 * ONE, "1000 - 370, with no dust either way");
    assert_eq!(h.get(id).withdrawn, 1_000 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    h.assert_pool_exact();
}

/// The maturity withdrawal is the last one. A second delegated attempt in the
/// same matured state is refused with `StreamTerminated` — the stream is over,
/// not merely empty for now — and the recipient's balance does not move, so the
/// claim cannot be collected twice.
#[test]
fn a_second_delegated_withdraw_at_maturity_pays_nothing() {
    let h = Harness::new();
    let (id, agent) = stream_with_grant(&h, op::WITHDRAW);

    h.advance(30 * DAY);
    h.client.withdraw(&id, &None);
    warp_to_maturity(&h, id);
    h.client.delegate_withdraw(&id, &agent, &None);

    let balance_after_first = h.balance(&h.recipient);
    let before = h.get(id);

    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::StreamTerminated,
        "a settled matured stream is over, not merely empty for now",
    );
    assert_eq!(h.get(id), before, "the refusal must not touch the stream");
    assert_eq!(
        h.balance(&h.recipient),
        balance_after_first,
        "the recipient must not be paid twice",
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 2. Pause across the maturity boundary
// ---------------------------------------------------------------------------

/// A matured stream that has **not** been drained is still `Active`, so a
/// delegated pause is accepted — the same as the direct `pause`, which
/// `test::pause::pausing_a_matured_stream_changes_nothing` pins. Pinning it here
/// is what makes the rejection in the next test meaningful: the two differ only
/// in whether the claim has been drawn, so maturity cannot be the reason a
/// pause is refused.
#[test]
fn delegate_pause_on_a_matured_but_undrained_stream_is_allowed() {
    let h = Harness::new();
    let (id, agent) = stream_with_grant(&h, op::PAUSE);

    warp_to_maturity(&h, id);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Active);

    h.client.delegate_pause(&id, &agent);

    assert_eq!(
        h.get(id).status,
        StreamStatus::Paused,
        "an undrained matured stream keeps a live schedule to freeze",
    );
    // Freezing a matured stream must not disturb the claim: the recipient is
    // still owed everything.
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    h.assert_pool_exact();
}

/// A matured stream whose claim has been **drawn** is `Depleted`, and
/// `Depleted` is terminal. The delegated pause is then rejected with
/// `StreamTerminated` — the documented terminal error — and changes nothing.
///
/// This is the case the issue names: the delegate's grant is still perfectly
/// live, and the stream matured *and* completed between their grant check and
/// their call. The grant is not what refuses them; the stream's state is.
#[test]
fn delegate_pause_on_a_matured_and_drained_stream_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let w_agent = Address::generate(&h.env);
    let p_agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &w_agent, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id, &h.sender, &p_agent, &op::PAUSE, &None);

    // The stream matures, then the delegate draws it down to nothing.
    warp_to_maturity(&h, id);
    h.client.delegate_withdraw(&id, &w_agent, &None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);

    let before = h.get(id);
    let pool_before = h.pool();

    let err = h
        .client
        .try_delegate_pause(&id, &p_agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);
    assert_eq!(
        h.get(id),
        before,
        "a refused pause must not freeze, resume or otherwise touch the stream",
    );
    assert_eq!(h.get(id).paused_at, None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    assert_eq!(h.pool(), pool_before, "no token may move");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 3. Cancellation at the maturity instant
// ---------------------------------------------------------------------------

/// Cancelling at `end_time` refunds nothing, because nothing is unvested any
/// more. The whole deposit belongs to the recipient, so the sender gets back
/// zero and the entire deposit stays in the pool to be withdrawn by them.
///
/// Note the shape this produces: the stream is `Cancelled` (the sender's
/// choice) yet its liability is the *full* deposit, because cancelling after
/// maturity cannot claw anything back. Conservation therefore holds with
/// `refundable == 0`, and the pool must still cover the whole claim.
#[test]
fn delegate_cancel_at_maturity_refunds_nothing() {
    let h = Harness::new();
    let (id, agent) = stream_with_grant(&h, op::CANCEL);

    let sender_before = h.balance(&h.sender);
    warp_to_maturity(&h, id);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);

    h.client.delegate_cancel(&id, &agent);

    assert_eq!(
        h.balance(&h.sender),
        sender_before,
        "nothing is unvested at maturity, so nothing is refunded",
    );
    let after = h.get(id);
    assert_eq!(
        after.status,
        StreamStatus::Cancelled,
        "the stream is still cancelled by the sender's choice",
    );
    assert_eq!(
        after.deposited,
        1_000 * ONE,
        "a cancel after maturity cannot reduce the deposit",
    );
    assert_eq!(
        h.pool(),
        1_000 * ONE,
        "the whole claim remains owed to the recipient",
    );

    // And it is still collectable: the recipient can draw the full remainder.
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

/// The same holds after a partial draw: the sender has already received their
/// 30 days, and cancelling at maturity refunds the remaining zero rather than
/// topping the sender up or clawing back what the recipient earned.
#[test]
fn delegate_cancel_at_maturity_refunds_nothing_after_a_partial_draw() {
    let h = Harness::new();
    let (id, agent) = stream_with_grant(&h, op::CANCEL);

    h.advance(30 * DAY);
    h.client.withdraw(&id, &None);
    let sender_before = h.balance(&h.sender);

    warp_to_maturity(&h, id);
    h.client.delegate_cancel(&id, &agent);

    assert_eq!(
        h.balance(&h.sender),
        sender_before,
        "no refund is due once the schedule has run to completion",
    );
    let after = h.get(id);
    assert_eq!(after.status, StreamStatus::Cancelled);
    assert_eq!(after.deposited, 1_000 * ONE, "deposit is untouched");
    assert_eq!(after.withdrawn, 300 * ONE, "the drawn part stays drawn");
    assert_eq!(
        h.pool(),
        700 * ONE,
        "the undrawn remainder is still the recipient's",
    );

    assert_eq!(h.client.withdraw(&id, &None), 700 * ONE);
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();
}
