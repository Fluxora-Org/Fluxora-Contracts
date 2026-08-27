//! Delegation scope and revocation — per-stream, per-operation.
//!
//! Design:
//!   • Grants are scoped to a single stream and a bitmask of operations.
//!   • Sender-side ops (CANCEL, PAUSE, RESUME, TOP_UP) are granted by the sender.
//!   • Recipient-side ops (WITHDRAW, TRANSFER_RECIPIENT) are granted by the recipient.
//!   • Grants may carry an expiry; they may be revoked at any time.
//!   • Revocation takes effect immediately and does not touch already-moved funds.
//!   • Delegate entry points (`delegate_withdraw`, `delegate_cancel`, …) take the
//!     delegate address explicitly; existing entry points are unchanged.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{Error, Op};

// ---------------------------------------------------------------------------
// Grant and basic use
// ---------------------------------------------------------------------------

#[test]
fn delegate_can_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);

    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 100 * ONE);
    h.assert_pool_exact();
}

#[test]
fn delegate_can_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &Op::CANCEL, &None);

    h.client.delegate_cancel(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Cancelled);
    h.assert_pool_exact();
}

#[test]
fn delegate_can_pause_and_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client.grant_delegate(
        &id,
        &h.sender,
        &agent,
        &(Op::PAUSE | Op::RESUME),
        &None,
    );

    h.client.delegate_pause(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);

    h.client.delegate_resume(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);
}

#[test]
fn delegate_can_top_up() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &Op::TOP_UP, &None);

    h.client.delegate_top_up(&id, &agent, &(100 * ONE));
    assert_eq!(h.client.get_stream(&id).deposited, 1_100 * ONE);
}

#[test]
fn delegate_can_transfer_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);

    h.client.grant_delegate(
        &id,
        &h.recipient,
        &agent,
        &Op::TRANSFER_RECIPIENT,
        &None,
    );

    h.client.delegate_transfer_recipient(&id, &agent, &new_recip);
    assert_eq!(h.client.get_stream(&id).recipient, new_recip);
}

// ---------------------------------------------------------------------------
// Revocation
// ---------------------------------------------------------------------------

#[test]
fn revoke_takes_effect_immediately() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);
    h.client.revoke_delegate(&id, &h.recipient, &agent);

    // Grant is gone — delegate_withdraw must fail.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);

    // The stream is untouched — no withdrawal happened.
    assert_eq!(h.client.get_stream(&id).withdrawn, 0);
}

#[test]
fn revoke_is_idempotent() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);
    h.client.revoke_delegate(&id, &h.recipient, &agent);
    // Second revoke should not panic or error.
    h.client.revoke_delegate(&id, &h.recipient, &agent);
}

#[test]
fn revoke_does_not_affect_already_moved_funds() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(20 * DAY);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);

    // Delegate withdraws while the grant is live.
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 200 * ONE);

    // Revoke.
    h.client.revoke_delegate(&id, &h.recipient, &agent);

    // Withdrawn balance in the stream reflects the completed payout.
    assert_eq!(h.client.get_stream(&id).withdrawn, 200 * ONE);
    h.assert_pool_exact();
}

#[test]
fn sender_can_revoke_recipient_issued_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);

    // Recipient issued the grant.
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);

    // Sender revokes it — allowed because the sender is a party to the stream.
    h.client.revoke_delegate(&id, &h.sender, &agent);

    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
}

// ---------------------------------------------------------------------------
// Expiry
// ---------------------------------------------------------------------------

#[test]
fn expired_grant_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    let expires = h.now() + 5 * DAY;
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &Some(expires));

    // Valid just before expiry.
    h.advance(4 * DAY);
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert!(paid > 0, "should succeed before expiry");

    // Advance past expiry.
    h.advance(2 * DAY);
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateExpired);
}

#[test]
fn grant_with_no_expiry_does_not_expire() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);

    // Jump well past the stream's end — grant is still valid.
    h.advance(200 * DAY);
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert!(paid > 0);
}

// ---------------------------------------------------------------------------
// Wrong operation
// ---------------------------------------------------------------------------

#[test]
fn delegate_cannot_call_an_op_not_in_their_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    // Grant only WITHDRAW.
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);

    // Agent tries TRANSFER_RECIPIENT — not in the grant.
    let new_recip = Address::generate(&h.env);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &new_recip)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);

    // Stream is unchanged.
    assert_eq!(h.client.get_stream(&id).recipient, h.recipient);
}

#[test]
fn sender_delegate_cannot_call_recipient_ops() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);

    // Sender grants CANCEL to the agent.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &Op::CANCEL, &None);

    // Agent tries delegate_withdraw — Op::WITHDRAW not in their grant.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);

    assert_eq!(h.client.get_stream(&id).withdrawn, 0);
}

// ---------------------------------------------------------------------------
// Wrong stream
// ---------------------------------------------------------------------------

#[test]
fn grant_on_stream_a_does_not_work_on_stream_b() {
    let h = Harness::new();
    let id_a = h.create_simple(1_000 * ONE, 100 * DAY);
    let id_b = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);

    // Grant WITHDRAW on stream A only.
    h.client
        .grant_delegate(&id_a, &h.recipient, &agent, &Op::WITHDRAW, &None);

    // No grant on stream B — must be rejected.
    let err = h
        .client
        .try_delegate_withdraw(&id_b, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);

    // Stream B is unchanged.
    assert_eq!(h.client.get_stream(&id_b).withdrawn, 0);
}

// ---------------------------------------------------------------------------
// Failed calls do not mutate state
// ---------------------------------------------------------------------------

#[test]
fn failed_delegate_call_leaves_stream_unchanged() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    let expires = h.now() + DAY;
    h.client
        .grant_delegate(&id, &h.sender, &agent, &Op::CANCEL, &Some(expires));

    // Let the grant expire.
    h.advance(2 * DAY);

    let before = h.client.get_stream(&id);
    let err = h
        .client
        .try_delegate_cancel(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateExpired);
    assert_eq!(h.client.get_stream(&id), before, "stream must not have changed");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Mixed grant is rejected
// ---------------------------------------------------------------------------

#[test]
fn granting_mixed_sender_and_recipient_ops_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    let err = h
        .client
        .try_grant_delegate(
            &id,
            &h.sender,
            &agent,
            &(Op::CANCEL | Op::WITHDRAW),
            &None,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Unauthorized);
}

// ---------------------------------------------------------------------------
// Replay: re-grant after revocation restores access
// ---------------------------------------------------------------------------

#[test]
fn regranting_after_revocation_restores_access() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.advance(10 * DAY);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);
    h.client.revoke_delegate(&id, &h.recipient, &agent);

    // Attempt after revocation fails.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);

    // Re-grant restores access.
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &Op::WITHDRAW, &None);

    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 100 * ONE);
    h.assert_pool_exact();
}
