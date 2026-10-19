//! `delegate_pause` repeat-submission coverage.
//!
//! A client retrying after a timeout may call `delegate_pause` a second time
//! on a stream that the first call already paused. This module pins the
//! contract's chosen behaviour:
//!
//! * The second call is **refused** with [`Error::StreamAlreadyPaused`].
//! * The refusal leaves every field of the stream exactly as the first call
//!   left it — in particular `paused_at` is not overwritten, so no paused
//!   time is lost.
//! * Token balances are unchanged by the refusal: `delegate_pause` never
//!   moves funds, so the pool and all party balances stay constant across both
//!   calls.
//! * The first call emits exactly one `paused` event; the second call emits
//!   nothing (it is refused before reaching the emit path).
//!
//! See the doc comment on
//! [`crate::FluxoraStream::delegate_pause`] for the authoritative statement.

extern crate std;

use soroban_sdk::testutils::Events as _;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{xdr, Address, Event as _};

use super::common::*;
use crate::events::Paused;
use crate::{op, Error, StreamStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Events emitted by the stream contract in the most recent invocation.
///
/// Must be called immediately after the entry point under test — `events().all()`
/// captures only the last invocation, so any subsequent call (even a read)
/// would replace the buffer.
fn stream_events(h: &Harness) -> std::vec::Vec<xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

// ---------------------------------------------------------------------------
// Core: second call is refused and first-call state is preserved
// ---------------------------------------------------------------------------

/// A delegate calling `delegate_pause` twice receives `StreamAlreadyPaused`
/// on the second attempt, and the stream is left in exactly the state the
/// first call produced.
///
/// This is the direct test for the repeat-submission scenario described in the
/// doc comment: a client retries after a timeout and the first call had already
/// landed.
#[test]
fn delegate_pause_twice_in_a_row_is_refused_with_stream_already_paused() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::PAUSE, &None);

    // First call succeeds.
    h.client.delegate_pause(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, StreamStatus::Paused);

    // Second call is refused.
    let err = h
        .client
        .try_delegate_pause(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamAlreadyPaused);
}

/// The refused second call must not touch `paused_at`.
///
/// If the second call overwrote `paused_at` with the later timestamp it would
/// silently erase the paused duration accumulated since the first call and
/// reduce the recipient's final payout. The refusal must leave `paused_at`
/// identical to what the first call set.
#[test]
fn refused_second_delegate_pause_does_not_overwrite_paused_at() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::PAUSE, &None);

    // First call: freeze the clock.
    h.advance(10 * DAY);
    h.client.delegate_pause(&id, &agent);
    let freeze_point = h.get(id).paused_at;
    assert!(freeze_point.is_some(), "paused_at must be set after the first call");

    // Advance time — the second call would move `paused_at` if it were allowed.
    h.advance(5 * DAY);
    let _ = h.client.try_delegate_pause(&id, &agent);

    assert_eq!(
        h.get(id).paused_at,
        freeze_point,
        "paused_at must not be overwritten by a refused second call",
    );
}

/// After the second (refused) call the accumulated pause time is exactly what
/// the first call captured.
///
/// Advances time, resumes, and checks `paused_total` equals the interval
/// between the first `delegate_pause` and `delegate_resume`. If the freeze
/// point had been corrupted by the second call the interval would be shorter
/// and the recipient would receive less.
#[test]
fn refused_second_delegate_pause_does_not_lose_paused_time() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client.grant_delegate(
        &id,
        &h.sender,
        &agent,
        &(op::PAUSE | op::RESUME),
        &None,
    );

    // First pause at T0+10d.
    h.advance(10 * DAY);
    h.client.delegate_pause(&id, &agent);

    // Advance 20 days while paused, then attempt a second pause (refused).
    h.advance(20 * DAY);
    let _ = h.client.try_delegate_pause(&id, &agent);

    // Resume. The paused interval must count from the first call, not the second.
    h.client.delegate_resume(&id, &agent);
    assert_eq!(
        h.get(id).paused_total,
        20 * DAY,
        "paused_total must cover the full interval from the first pause call",
    );
    assert_eq!(h.get(id).paused_at, None, "paused_at must be cleared on resume");
    assert_eq!(h.get(id).status, StreamStatus::Active);
}

// ---------------------------------------------------------------------------
// Token balance assertions
// ---------------------------------------------------------------------------

/// `delegate_pause` never moves tokens, so no balance changes across either
/// call. Asserted for sender, recipient, and the contract pool.
#[test]
fn delegate_pause_twice_does_not_change_any_token_balance() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let id = h.create_simple(deposit, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::PAUSE, &None);

    // Capture balances before any pause.
    let sender_before = h.balance(&h.sender);
    let recipient_before = h.balance(&h.recipient);
    let pool_before = h.pool();

    h.client.delegate_pause(&id, &agent);

    // Balances unchanged after first call.
    assert_eq!(
        h.balance(&h.sender),
        sender_before,
        "sender balance unchanged after first delegate_pause",
    );
    assert_eq!(
        h.balance(&h.recipient),
        recipient_before,
        "recipient balance unchanged after first delegate_pause",
    );
    assert_eq!(
        h.pool(),
        pool_before,
        "pool unchanged after first delegate_pause",
    );

    // Second call (refused).
    let _ = h.client.try_delegate_pause(&id, &agent);

    // Balances still unchanged after second (refused) call.
    assert_eq!(
        h.balance(&h.sender),
        sender_before,
        "sender balance unchanged after refused second delegate_pause",
    );
    assert_eq!(
        h.balance(&h.recipient),
        recipient_before,
        "recipient balance unchanged after refused second delegate_pause",
    );
    assert_eq!(
        h.pool(),
        pool_before,
        "pool unchanged after refused second delegate_pause",
    );

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Event assertions
// ---------------------------------------------------------------------------

/// The first `delegate_pause` call emits exactly one `paused` event whose
/// payload matches the post-pause storage state. The second (refused) call
/// emits no events.
#[test]
fn first_delegate_pause_emits_paused_event_second_does_not() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::PAUSE, &None);

    h.advance(10 * DAY);
    let pause_instant = h.now();
    h.client.delegate_pause(&id, &agent);

    // Capture the event immediately, before any other call.
    let first_events = stream_events(&h);

    // Read back the settled stream for ground truth.
    let stream = h.get(id);
    assert_eq!(stream.status, StreamStatus::Paused);
    assert_eq!(stream.paused_at, Some(pause_instant));

    // Build the expected event from storage — not from the values the emitter
    // saw, so we detect any disagreement between storage and the published payload.
    let expected = Paused {
        stream_id: id,
        sender: h.sender.clone(),
        paused_at: pause_instant,
        paused_total: stream.paused_total,
        recipient: h.recipient.clone(),
    };

    assert_eq!(
        first_events,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "first delegate_pause must emit exactly one `paused` event matching storage",
    );

    // Second call (refused) must emit nothing.
    h.advance(5 * DAY);
    let _ = h.client.try_delegate_pause(&id, &agent);
    let second_events = stream_events(&h);

    assert!(
        second_events.is_empty(),
        "refused second delegate_pause must emit no events, got: {:?}",
        second_events,
    );
}

// ---------------------------------------------------------------------------
// Full state snapshot: both calls leave the same final state
// ---------------------------------------------------------------------------

/// The complete stream state after two `delegate_pause` calls — one successful,
/// one refused — is identical to the state after a single `delegate_pause`.
///
/// This is the acceptance criterion stated in the problem statement: "calling
/// twice leaves the same state as calling once".  The assertion compares every
/// field of the `Stream` struct by value.
#[test]
fn stream_state_after_two_delegate_pause_calls_equals_state_after_one() {
    // ── Baseline: single call ──────────────────────────────────────────────
    let h1 = Harness::new();
    let id1 = h1.create_simple(1_000 * ONE, 100 * DAY);
    let agent1 = Address::generate(&h1.env);
    h1.client
        .grant_delegate(&id1, &h1.sender, &agent1, &op::PAUSE, &None);
    h1.advance(10 * DAY);
    h1.client.delegate_pause(&id1, &agent1);
    let state_after_one = h1.get(id1);

    // ── Two calls: first succeeds, second is refused ───────────────────────
    let h2 = Harness::new();
    let id2 = h2.create_simple(1_000 * ONE, 100 * DAY);
    let agent2 = Address::generate(&h2.env);
    h2.client
        .grant_delegate(&id2, &h2.sender, &agent2, &op::PAUSE, &None);
    h2.advance(10 * DAY);
    h2.client.delegate_pause(&id2, &agent2);
    let _ = h2.client.try_delegate_pause(&id2, &agent2);
    let state_after_two = h2.get(id2);

    // Compare the fields that reflect the pause state — the addresses differ
    // between the two harnesses, so we compare by value rather than the whole struct.
    assert_eq!(
        state_after_one.status,
        state_after_two.status,
        "status must match",
    );
    assert_eq!(
        state_after_one.paused_at.map(|_| true),
        state_after_two.paused_at.map(|_| true),
        "paused_at presence must match",
    );
    assert_eq!(
        state_after_one.paused_total,
        state_after_two.paused_total,
        "paused_total must match",
    );
    assert_eq!(
        state_after_one.deposited,
        state_after_two.deposited,
        "deposited must match",
    );
    assert_eq!(
        state_after_one.withdrawn,
        state_after_two.withdrawn,
        "withdrawn must match",
    );
    assert_eq!(
        state_after_one.start_time,
        state_after_two.start_time,
        "start_time must match",
    );
    assert_eq!(
        state_after_one.end_time,
        state_after_two.end_time,
        "end_time must match",
    );
}
