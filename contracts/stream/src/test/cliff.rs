//! Stage 2 — cliff semantics.
//!
//! The cliff **gates** the payout; it does not delay accrual. This surprises
//! people, so it gets its own file.

use soroban_sdk::testutils::Events as _;
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::{Paused, Resumed, Withdrawn};
use crate::{Error, StreamStatus};

/// Events the stream contract published during the last invocation.
///
/// `Events::all()` only reports the most recent contract invocation, so this
/// must be called immediately after the call under test.
fn published_by_stream(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

#[test]
fn nothing_is_withdrawable_one_second_before_the_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 90 * DAY;
    let id = h.create(
        1_200 * ONE,
        start,
        start + 360 * DAY,
        cliff,
        true,
        true,
        true,
    );

    h.warp_to(cliff - 1);
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);
}

/// At the cliff instant the recipient becomes entitled to everything accrued
/// since `start_time` — a quarter of a year-long stream, not zero.
#[test]
fn cliff_releases_all_accrual_since_start_not_since_the_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 90 * DAY;
    let id = h.create(
        1_200 * ONE,
        start,
        start + 360 * DAY,
        cliff,
        true,
        true,
        true,
    );

    h.warp_to(cliff);

    // 90 of 360 days elapsed => a quarter of the deposit, all at once.
    assert_eq!(h.client.vested_of(&id), 300 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    h.assert_pool_exact();
}

/// The transition must be a step at exactly `cliff_time`, with no off-by-one.
#[test]
fn the_cliff_step_lands_on_the_exact_second() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 100;
    let id = h.create(1_000, start, start + 1_000, cliff, true, true, true);

    h.warp_to(cliff - 1);
    assert_eq!(h.client.vested_of(&id), 0, "one second before");

    h.warp_to(cliff);
    assert_eq!(h.client.vested_of(&id), 100, "at the cliff");

    h.warp_to(cliff + 1);
    assert_eq!(h.client.vested_of(&id), 101, "one second after");
}

/// After the cliff opens, accrual continues linearly as if the cliff had never
/// existed.
#[test]
fn accrual_is_linear_after_the_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 90 * DAY;
    let id = h.create(
        1_200 * ONE,
        start,
        start + 360 * DAY,
        cliff,
        true,
        true,
        true,
    );

    for days in [90u64, 180, 270, 360] {
        h.warp_to(start + days * DAY);
        let expected = 1_200 * ONE * days as i128 / 360;
        assert_eq!(h.client.vested_of(&id), expected, "at day {days}");
    }
}

/// `cliff == end` is a legal degenerate case: a single lump sum at maturity,
/// which is how a straightforward vesting bonus is expressed.
#[test]
fn cliff_at_end_time_is_a_lump_sum() {
    let h = Harness::new();
    let start = h.now();
    let end = start + 365 * DAY;
    let id = h.create(1_000 * ONE, start, end, end, true, true, true);

    h.warp_to(end - 1);
    assert_eq!(h.client.vested_of(&id), 0);

    h.warp_to(end);
    assert_eq!(h.client.vested_of(&id), 1_000 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    h.assert_pool_exact();
}

#[test]
fn cliff_at_start_time_means_no_cliff() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        true,
    );

    h.advance(1);
    assert!(h.client.vested_of(&id) > 0, "accrual begins immediately");
}

/// Cancelling before the cliff refunds everything: pre-cliff the recipient's
/// entitlement is zero by definition, even though time has passed.
#[test]
fn cancelling_before_the_cliff_refunds_the_whole_deposit() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 90 * DAY;
    let id = h.create(
        1_200 * ONE,
        start,
        start + 360 * DAY,
        cliff,
        true,
        true,
        true,
    );
    let before = h.balance(&h.sender);

    h.warp_to(cliff - 1);
    h.client.cancel(&id);

    assert_eq!(h.balance(&h.sender), before + 1_200 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();

    // And the entitlement stays zero once the cliff time passes in wall-clock
    // terms — the cancel already settled it.
    h.warp_to(cliff + 10 * DAY);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Cliff boundary regression tests — exact ledger boundary behavior
// ---------------------------------------------------------------------------

/// **Boundary: read operations at cliff-1, cliff, cliff+1**
///
/// All read operations (`vested_of`, `withdrawable_of`, `refundable_of`) must
/// return consistent values exactly at boundaries without off-by-one errors.
#[test]
fn cliff_boundary_reads_are_exact() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    // Expected vested at cliff: 1000/10000 * deposit = 1000 * ONE
    let expected_at_cliff = 1000 * ONE;

    // Before cliff: nothing vested, everything refundable
    h.warp_to(cliff - 1);
    assert_eq!(h.client.vested_of(&id), 0, "vested before cliff");
    assert_eq!(
        h.client.withdrawable_of(&id),
        0,
        "withdrawable before cliff"
    );
    assert_eq!(
        h.client.refundable_of(&id),
        deposit,
        "refundable before cliff"
    );

    // Exactly at cliff: step function activates
    h.warp_to(cliff);
    assert_eq!(
        h.client.vested_of(&id),
        expected_at_cliff,
        "vested exactly at cliff"
    );
    assert_eq!(
        h.client.withdrawable_of(&id),
        expected_at_cliff,
        "withdrawable exactly at cliff"
    );
    assert_eq!(
        h.client.refundable_of(&id),
        deposit - expected_at_cliff,
        "refundable exactly at cliff"
    );

    // After cliff: linear accrual continues
    h.warp_to(cliff + 1);
    let expected_after = expected_at_cliff + ONE; // One more second elapsed
    assert_eq!(
        h.client.vested_of(&id),
        expected_after,
        "vested one second after cliff"
    );
    assert_eq!(
        h.client.withdrawable_of(&id),
        expected_after,
        "withdrawable one second after cliff"
    );
    assert_eq!(
        h.client.refundable_of(&id),
        deposit - expected_after,
        "refundable one second after cliff"
    );

    h.assert_pool_exact();
}

/// **Boundary: withdrawal exactly at cliff instant**
///
/// The recipient must be able to withdraw the exact vested amount at the cliff
/// ledger with no rejection or rounding error.
#[test]
fn withdrawal_succeeds_exactly_at_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    let expected_at_cliff = 1000 * ONE;
    let recipient_before = h.balance(&h.recipient);

    h.warp_to(cliff);

    // Full withdrawal of cliff amount
    let withdrawn = h.client.withdraw(&id, &None);
    assert_eq!(withdrawn, expected_at_cliff, "withdrawn amount at cliff");
    assert_eq!(
        h.balance(&h.recipient),
        recipient_before + expected_at_cliff,
        "recipient balance after withdrawal"
    );

    // Pool and stream state remain coherent
    h.assert_pool_exact();
    let stream = h.get(id);
    assert_eq!(stream.withdrawn, expected_at_cliff);
    assert_eq!(h.client.withdrawable_of(&id), 0);
}

/// **Boundary: partial withdrawal exactly at cliff instant**
///
/// Withdrawing less than the full cliff amount must work, and the residue must
/// remain available.
#[test]
fn partial_withdrawal_at_cliff_leaves_remainder_available() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    let expected_at_cliff = 1000 * ONE;
    let partial = 600 * ONE;

    h.warp_to(cliff);

    // Withdraw partial amount
    let withdrawn = h.client.withdraw(&id, &Some(partial));
    assert_eq!(withdrawn, partial);

    // Remainder is still withdrawable
    assert_eq!(
        h.client.withdrawable_of(&id),
        expected_at_cliff - partial,
        "remainder after partial withdrawal"
    );

    h.assert_pool_exact();
}

/// **Boundary: cancellation exactly at cliff instant**
///
/// Cancelling exactly at the cliff instant must vest the cliff amount to the
/// recipient and refund the remainder to the sender, with exact accounting.
#[test]
fn cancellation_exactly_at_cliff_vests_cliff_amount() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    let expected_at_cliff = 1000 * ONE;
    let sender_before = h.balance(&h.sender);

    h.warp_to(cliff);
    h.client.cancel(&id);

    // Sender gets back unvested portion
    let refunded = deposit - expected_at_cliff;
    assert_eq!(
        h.balance(&h.sender),
        sender_before + refunded,
        "sender refund at cliff cancel"
    );

    // Recipient can withdraw vested portion
    assert_eq!(
        h.client.withdrawable_of(&id),
        expected_at_cliff,
        "recipient withdrawable after cliff cancel"
    );

    let stream = h.get(id);
    assert_eq!(
        stream.deposited, expected_at_cliff,
        "deposited after cancel"
    );
    assert_eq!(stream.status, crate::StreamStatus::Cancelled);

    h.assert_pool_exact();
}

/// **Boundary: cancellation one second before cliff**
///
/// Cancelling at cliff-1 must refund the entire deposit because nothing has
/// vested yet.
#[test]
fn cancellation_before_cliff_refunds_everything() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    let sender_before = h.balance(&h.sender);

    h.warp_to(cliff - 1);
    h.client.cancel(&id);

    // Everything refunded
    assert_eq!(
        h.balance(&h.sender),
        sender_before + deposit,
        "sender gets full refund before cliff"
    );

    // Nothing withdrawable
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.pool(), 0);

    h.assert_pool_exact();
}

/// **Boundary: cancellation one second after cliff**
///
/// Cancelling at cliff+1 must vest the correct amount including the extra
/// second of accrual.
#[test]
fn cancellation_after_cliff_includes_additional_accrual() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    let expected_at_cliff_plus_one = 1001 * ONE;
    let sender_before = h.balance(&h.sender);

    h.warp_to(cliff + 1);
    h.client.cancel(&id);

    let refunded = deposit - expected_at_cliff_plus_one;
    assert_eq!(
        h.balance(&h.sender),
        sender_before + refunded,
        "sender refund one second after cliff"
    );

    assert_eq!(
        h.client.withdrawable_of(&id),
        expected_at_cliff_plus_one,
        "recipient withdrawable after cliff+1 cancel"
    );

    h.assert_pool_exact();
}

/// **Boundary: cliff equals start time**
///
/// When cliff == start, accrual must begin immediately from the first ledger
/// with no gate.
#[test]
fn cliff_equals_start_has_immediate_accrual() {
    let h = Harness::new();
    let start = h.now();
    let end = start + 1000;
    let deposit = 1000 * ONE;
    let id = h.create(deposit, start, end, start, true, true, true);

    // Exactly at start: cliff already passed, one stroop per ledger should vest
    h.warp_to(start);
    assert_eq!(h.client.vested_of(&id), 0, "at start, zero elapsed");

    h.warp_to(start + 1);
    assert_eq!(
        h.client.vested_of(&id),
        ONE,
        "one second after start with cliff=start"
    );

    h.assert_pool_exact();
}

/// **Boundary: cliff equals end time (lump sum at maturity)**
///
/// When cliff == end, nothing vests until the final instant, then the full
/// deposit becomes available.
#[test]
fn cliff_equals_end_is_lump_sum_at_final_instant() {
    let h = Harness::new();
    let start = h.now();
    let end = start + 1000;
    let deposit = 1000 * ONE;
    let id = h.create(deposit, start, end, end, true, true, true);

    // One second before end: nothing vested
    h.warp_to(end - 1);
    assert_eq!(h.client.vested_of(&id), 0, "before cliff=end");

    // Exactly at end: full deposit vests
    h.warp_to(end);
    assert_eq!(
        h.client.vested_of(&id),
        deposit,
        "at cliff=end, full deposit vests"
    );

    // Withdrawal must succeed
    let withdrawn = h.client.withdraw(&id, &None);
    assert_eq!(withdrawn, deposit);

    h.assert_pool_exact();
}

/// **Boundary: withdrawal attempt before cliff must fail with correct error**
///
/// Attempting to withdraw before the cliff, even with an explicit amount, must
/// return `Error::NothingToWithdraw`.
#[test]
fn withdrawal_before_cliff_fails_with_correct_error() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    h.warp_to(cliff - 1);

    // Try with None (full withdrawal)
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::NothingToWithdraw,
        "full withdrawal before cliff"
    );

    // Try with explicit amount
    let err = h
        .client
        .try_withdraw(&id, &Some(100 * ONE))
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::NothingToWithdraw,
        "explicit withdrawal before cliff"
    );
}

/// **Boundary: cliff boundaries across multiple streams**
///
/// The cliff gate is evaluated per stream: at the same instant, a stream
/// whose cliff is still ahead, one exactly at the cliff, and one already past
/// it must report distinct vested/withdrawable amounts.
#[test]
fn batch_reads_handle_cliff_boundaries_correctly() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;

    // Three streams: before cliff, at cliff, after cliff
    let id1 = h.create(deposit, start, end, cliff + 10, true, true, true);
    let id2 = h.create(deposit, start, end, cliff, true, true, true);
    let id3 = h.create(deposit, start, end, cliff - 10, true, true, true);

    h.warp_to(cliff);

    // id1: cliff not reached (cliff+10)
    assert_eq!(h.client.vested_of(&id1), 0, "id1 vested before cliff");
    assert_eq!(
        h.client.withdrawable_of(&id1),
        0,
        "id1 withdrawable before cliff"
    );

    // id2: exactly at cliff
    assert_eq!(h.client.vested_of(&id2), 1000 * ONE, "id2 vested at cliff");
    assert_eq!(
        h.client.withdrawable_of(&id2),
        1000 * ONE,
        "id2 withdrawable at cliff"
    );

    // id3: past cliff (cliff-10)
    assert_eq!(
        h.client.vested_of(&id3),
        1000 * ONE,
        "id3 vested after cliff"
    );
    assert_eq!(
        h.client.withdrawable_of(&id3),
        1000 * ONE,
        "id3 withdrawable after cliff"
    );

    h.assert_pool_exact();
}

/// **Boundary: pause and resume across cliff boundary**
///
/// Pausing before the cliff and resuming after should preserve the cliff gate:
/// the stream clock must pass through the cliff when unpaused for vesting to
/// activate.
#[test]
fn pause_across_cliff_preserves_cliff_gate() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    // Pause before cliff
    h.warp_to(cliff - 100);
    h.client.pause(&id);

    // Wall-clock advances past cliff while paused
    h.warp_to(cliff + 500);

    // Still nothing vested: stream clock is frozen
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "vested while paused across cliff"
    );

    // Resume
    h.client.resume(&id);

    // Stream clock needs to catch up to cliff
    h.advance(100); // Stream clock now at cliff
    assert_eq!(
        h.client.vested_of(&id),
        1000 * ONE,
        "vested after resume at cliff"
    );

    h.assert_pool_exact();
}

/// **Issue #1688: the cliff moves in wall-clock terms when a stream is paused.**
///
/// `cliff_reached` is evaluated against the stream clock, and `stream_time`
/// subtracts the cumulative `paused_total`. Pausing therefore freezes the cliff
/// gate as well as accrual, and a resume pushes the *wall-clock* instant at
/// which the gate opens forward by the whole paused interval:
/// `effective_cliff = cliff_time + paused_total`.
///
/// `docs/ABI.md` states this rule at the integration boundary; this test pins
/// it down at the exact second on either side of the moved instant.
#[test]
fn pause_across_cliff_delays_the_wall_clock_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    // Pause 100s before the cliff, then let the wall clock run 500s past it.
    h.warp_to(cliff - 100);
    h.client.pause(&id);
    h.warp_to(cliff + 500);

    // The gate is frozen: wall-clock time alone does not open it.
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "cliff must not open while the stream is paused"
    );

    h.client.resume(&id);
    let paused_total = h.get(id).paused_total;
    assert_eq!(paused_total, 600, "resume must absorb the whole interval");

    // The moved instant is cliff_time + paused_total, one full pause later.
    let effective_cliff = cliff + paused_total;
    assert_eq!(effective_cliff, cliff + 600);

    // One second before it the gate is still shut...
    h.warp_to(effective_cliff - 1);
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "cliff gate must still be shut one second before the moved instant"
    );

    // ...and at the moved instant the gate opens with everything accrued since
    // start_time, exactly as it would have at the un-paused cliff.
    h.warp_to(effective_cliff);
    assert_eq!(
        h.client.vested_of(&id),
        1000 * ONE,
        "vested at the moved cliff instant"
    );

    h.assert_pool_exact();
}

/// **Boundary: pause landing exactly on the cliff instant**
///
/// Issue #1830. The cliff gate is evaluated on the stream clock,
/// `stream_time(now) = (paused_at ?? now) - paused_total`. A `pause` call whose
/// ledger timestamp equals `cliff_time` is the single instant where the two
/// clocks meet while the stream is still gated, because the freeze point and
/// the gate satisfy `paused_at == cliff_time` with `paused_total == 0`.
/// Whether the gate is then open or shut is decided purely by the comparison
/// inside `cliff_reached` — `<` versus `<=` — and both wrong directions break
/// real value:
///
/// * `<` (gate treated as shut): the frozen clock sits exactly at `cliff_time`
///   and can never rise while paused, so the recipient can never withdraw —
///   not at the pause, not at the wall-clock cliff, and not after a resume,
///   which lands the stream clock back on `cliff_time` exactly.
/// * `>` (gate treated as open early): identical arithmetic opens the gate a
///   second before an un-paused stream would.
///
/// The contract documents `stream_time >= cliff_time` in docs/ABI.md's `resume`
/// section and pins the same rule for the in-progress pause in the `pause`
/// section ("the recipient can still withdraw value accrued before the
/// pause"). This test exercises the equality case end to end through the
/// public ABI — pause, withdraw while frozen, resume, continued accrual —
/// asserting accounting, emitted events and the final stream state.
#[test]
fn paused_exactly_at_the_cliff_instant_leaves_the_gate_open() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    // Land the pause exactly on the cliff instant, after 1000 seconds of
    // (gated) accrual. `paused_at` and `cliff_time` are now the same second.
    h.warp_to(cliff);
    h.client.pause(&id);

    // Exactly one `paused` event, carrying the freeze point and the cumulative
    // total *before* this in-progress pause (docs/ABI.md, pause events). The
    // event itself pins the freeze point to the cliff instant. Read it before
    // any other client call — the event log only holds the last invocation.
    assert_eq!(
        published_by_stream(&h),
        std::vec![Paused {
            stream_id: id,
            sender: h.sender.clone(),
            paused_at: cliff,
            paused_total: 0,
            recipient: h.recipient.clone(),
        }
        .to_xdr(&h.env, &h.contract_id)],
        "the paused event must pin the freeze point to the cliff instant"
    );

    let paused_at = h
        .get(id)
        .paused_at
        .expect("the stream must be frozen after pause");
    assert_eq!(
        paused_at, cliff,
        "precondition: the freeze point must be the cliff instant itself"
    );

    // While paused the equality must already open the gate: the frozen stream
    // clock is `cliff - 0`, which satisfies the documented
    // `stream_time >= cliff_time` rule. Everything accrued since `start_time`
    // is withdrawable — pausing freezes accrual, never access.
    assert_eq!(
        h.client.vested_of(&id),
        1000 * ONE,
        "vested while paused at the cliff instant"
    );
    assert_eq!(
        h.client.withdrawable_of(&id),
        1000 * ONE,
        "withdrawable while paused at the cliff instant"
    );
    assert_eq!(
        h.client.refundable_of(&id),
        deposit - 1000 * ONE,
        "refundable while paused at the cliff instant"
    );
    h.assert_invariants();

    // The recipient withdraws the whole cliff amount while the stream is
    // frozen. Return value, token ledger and event must all agree, and the
    // stream must stay paused with the payout booked.
    let recipient_before = h.balance(&h.recipient);
    let paid = h.client.withdraw(&id, &None);

    // Capture the invocation's events before any other client call — the log
    // only holds the most recent invocation, token reads included.
    let withdrawn_events = published_by_stream(&h);

    assert_eq!(
        paid,
        1000 * ONE,
        "withdrawal while paused at the cliff instant pays the cliff"
    );
    assert_eq!(
        h.balance(&h.recipient),
        recipient_before + 1000 * ONE,
        "the token ledger must credit the cliff amount"
    );
    let s = h.get(id);
    assert_eq!(s.withdrawn, 1000 * ONE);
    assert_eq!(
        s.status,
        StreamStatus::Paused,
        "a withdrawal must not unfreeze the stream"
    );
    assert_eq!(
        withdrawn_events,
        std::vec![Withdrawn {
            stream_id: id,
            recipient: h.recipient.clone(),
            amount: 1000 * ONE,
            withdrawn: s.withdrawn,
            deposited: s.deposited,
            status: s.status,
            sender: s.sender.clone(),
            paused_at: s.paused_at,
            paused_total: s.paused_total,
        }
        .to_xdr(&h.env, &h.contract_id)],
        "the withdrawn event must match post-call storage exactly"
    );

    // Wall-clock time passing while frozen changes nothing — and a fully drawn
    // frozen stream still reports the live-stream empty-balance error, not a
    // terminal one.
    h.warp_to(cliff + 500);
    assert_eq!(
        h.client.vested_of(&id),
        1000 * ONE,
        "vested must stay frozen while paused"
    );
    assert_eq!(
        h.client.withdrawable_of(&id),
        0,
        "the cliff amount was already drawn"
    );
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);

    // Resume: the elapsed pause is absorbed into `paused_total`, handing back
    // exactly the same stream time — `cliff` again. This is the second half of
    // the off-by-one exposure: a gate implemented with a strict `>` would
    // flicker shut here, after the recipient was already paid at the same
    // stream time.
    h.client.resume(&id);

    // Exactly one `resumed` event, enough for an indexer to recompute the
    // moved schedule from `stream_created` alone (docs/ABI.md, resume events).
    // Read before any other client call — the log holds one invocation only.
    assert_eq!(
        published_by_stream(&h),
        std::vec![Resumed {
            stream_id: id,
            sender: h.sender.clone(),
            paused_duration: 500,
            paused_total: 500,
            recipient: h.recipient.clone(),
        }
        .to_xdr(&h.env, &h.contract_id)],
        "the resumed event must publish the absorbed interval"
    );

    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Active);
    assert_eq!(s.paused_at, None, "resume clears the freeze point");
    assert_eq!(s.paused_total, 500, "the whole elapsed pause is absorbed");
    assert_eq!(
        h.client.vested_of(&id),
        1000 * ONE,
        "the gate must stay open across a resume at the cliff instant"
    );

    // Accrual continues linearly on the stream clock: one real second after
    // the resume puts the clock at cliff+1, not at the wall clock's cliff+501.
    h.advance(1);
    assert_eq!(
        h.client.vested_of(&id),
        1001 * ONE,
        "accrual continues from the frozen point, not the wall clock"
    );
    assert_eq!(h.get(id).paused_total, 500);

    // Funds conservation at the end of the scenario: vested (1001 * ONE) plus
    // refundable equals the deposit exactly, the pool backs the residual
    // liability exactly, and the recipient holds precisely what was paid out.
    assert_eq!(
        h.client.vested_of(&id) + h.client.refundable_of(&id),
        deposit,
        "I4: vested + refundable must equal deposited exactly"
    );
    assert_eq!(
        h.pool(),
        deposit - h.get(id).withdrawn,
        "the pool must hold exactly the outstanding liability"
    );
    assert_eq!(
        h.balance(&h.recipient) - recipient_before,
        h.get(id).withdrawn,
        "the recipient holds exactly what the stream says was withdrawn"
    );
    h.assert_pool_exact();
}

/// **Boundary: multiple withdrawals at cliff instant**
///
/// Making multiple partial withdrawals at the exact cliff instant must
/// correctly track the withdrawn amount without double-counting.
#[test]
fn multiple_partial_withdrawals_at_cliff_are_exact() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 1000;
    let end = start + 10000;
    let deposit = 10000 * ONE;
    let id = h.create(deposit, start, end, cliff, true, true, true);

    let expected_at_cliff = 1000 * ONE;

    h.warp_to(cliff);

    // Three partial withdrawals totaling the cliff amount
    let w1 = h.client.withdraw(&id, &Some(300 * ONE));
    assert_eq!(w1, 300 * ONE);

    let w2 = h.client.withdraw(&id, &Some(400 * ONE));
    assert_eq!(w2, 400 * ONE);

    let w3 = h.client.withdraw(&id, &Some(300 * ONE));
    assert_eq!(w3, 300 * ONE);

    // Total withdrawn exactly equals cliff amount
    let stream = h.get(id);
    assert_eq!(stream.withdrawn, expected_at_cliff);
    assert_eq!(h.client.withdrawable_of(&id), 0);

    h.assert_pool_exact();
}
