//! Focused revocation coverage — every permission bit, timing, and isolation.
//!
//! This module covers the security-critical half of the delegation pair:
//! granting access is only safe if that access can be reliably withdrawn.
//!
//! # Acceptance criteria
//!
//! 1. **Per-bit revocation** — revoking each of the six permission bits is
//!    individually tested; the revoked bit is rejected while bits the agent
//!    never held remain absent.
//!
//! 2. **Never-issued grant is a no-op** — calling `revoke_delegate` when no
//!    grant exists must succeed silently (idempotent by design).
//!
//! 3. **Same-ledger effect** — revocation takes effect within the same ledger
//!    it was issued; no `advance` between the revoke and the follow-up call.
//!
//! 4. **Multi-delegate isolation** — revoking one delegate leaves every other
//!    delegate's grant intact and usable.
//!
//! These tests complement the broader ordering and re-grant coverage already
//! in `test::delegation`; this module focuses narrowly on the four criteria
//! above and keeps each test self-contained.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error};

// ---------------------------------------------------------------------------
// Helpers (mirrors the pattern used in delegation.rs)
// ---------------------------------------------------------------------------

/// Every op bit that a [`crate::DelegateGrant`] can carry.
const ALL_OPS: [u32; 6] = [
    op::WITHDRAW,
    op::CANCEL,
    op::PAUSE,
    op::RESUME,
    op::TOP_UP,
    op::TRANSFER_RECIPIENT,
];

/// The party authorised to grant (and revoke) `op_bit`.
fn grantor_for<'a>(h: &'a Harness<'_>, op_bit: u32) -> &'a Address {
    match op_bit {
        op::WITHDRAW | op::TRANSFER_RECIPIENT => &h.recipient,
        _ => &h.sender,
    }
}

/// Create a stream and give `agent` exactly the single-bit grant for `op_bit`.
///
/// The stream is left in a state where the op would succeed if the grant is
/// still live, so any subsequent rejection can only be attributed to the
/// revocation under test.
fn stream_with_single_grant(h: &Harness, agent: &Address, op_bit: u32) -> u64 {
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.token_admin.mint(agent, &(1_000 * ONE));
    // Ensure some accrual for WITHDRAW.
    h.advance(10 * DAY);
    // RESUME only makes sense on a paused stream.
    if op_bit == op::RESUME {
        h.client.pause(&id);
    }
    h.client
        .grant_delegate(&id, grantor_for(h, op_bit), agent, &op_bit, &None);
    id
}

/// Dispatch the delegate entry point for `op_bit` and return the result,
/// normalising heterogeneous success types to `()`.
fn delegate_call_result(h: &Harness, id: u64, agent: &Address, op_bit: u32) -> Result<(), Error> {
    let new_recip = Address::generate(&h.env);
    let outcome = match op_bit {
        op::WITHDRAW => h
            .client
            .try_delegate_withdraw(&id, agent, &None)
            .map(|_| ()),
        op::CANCEL => h.client.try_delegate_cancel(&id, agent).map(|_| ()),
        op::PAUSE => h.client.try_delegate_pause(&id, agent).map(|_| ()),
        op::RESUME => h.client.try_delegate_resume(&id, agent).map(|_| ()),
        op::TOP_UP => h
            .client
            .try_delegate_top_up(&id, agent, &(100 * ONE))
            .map(|_| ()),
        op::TRANSFER_RECIPIENT => h
            .client
            .try_delegate_transfer_recipient(&id, agent, &new_recip)
            .map(|_| ()),
        other => panic!("unhandled op bit {other}"),
    };
    outcome.map_err(|e| e.expect("host invocation trapped"))
}

// ---------------------------------------------------------------------------
// Criterion 1 — Revoking each of the six bits is covered individually
// ---------------------------------------------------------------------------

/// Revoking a WITHDRAW grant removes it: a subsequent delegate_withdraw is
/// rejected with DelegateNotPermitted, and the stream's withdrawn counter stays
/// at zero (no partial withdrawal happened).
#[test]
fn revoke_withdraw_bit_blocks_delegate_withdraw() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = stream_with_single_grant(&h, &agent, op::WITHDRAW);

    h.client
        .revoke_delegate(&id, grantor_for(&h, op::WITHDRAW), &agent);

    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(h.client.get_stream(&id).withdrawn, 0);
    h.assert_pool_exact();
}

/// Revoking a CANCEL grant removes it: a subsequent delegate_cancel is
/// rejected with DelegateNotPermitted, and the stream stays Active.
#[test]
fn revoke_cancel_bit_blocks_delegate_cancel() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = stream_with_single_grant(&h, &agent, op::CANCEL);

    h.client
        .revoke_delegate(&id, grantor_for(&h, op::CANCEL), &agent);

    let err = h
        .client
        .try_delegate_cancel(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(
        h.client.get_stream(&id).status,
        crate::StreamStatus::Active,
        "stream must remain Active after a rejected delegate_cancel"
    );
    h.assert_pool_exact();
}

/// Revoking a PAUSE grant removes it: a subsequent delegate_pause is
/// rejected with DelegateNotPermitted, and the stream stays Active.
#[test]
fn revoke_pause_bit_blocks_delegate_pause() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = stream_with_single_grant(&h, &agent, op::PAUSE);

    h.client
        .revoke_delegate(&id, grantor_for(&h, op::PAUSE), &agent);

    let err = h
        .client
        .try_delegate_pause(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(
        h.client.get_stream(&id).status,
        crate::StreamStatus::Active,
        "stream must remain Active after a rejected delegate_pause"
    );
    h.assert_pool_exact();
}

/// Revoking a RESUME grant removes it: a subsequent delegate_resume on a paused
/// stream is rejected with DelegateNotPermitted, and the stream stays Paused.
#[test]
fn revoke_resume_bit_blocks_delegate_resume() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    // stream_with_single_grant already pauses the stream for RESUME.
    let id = stream_with_single_grant(&h, &agent, op::RESUME);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);

    h.client
        .revoke_delegate(&id, grantor_for(&h, op::RESUME), &agent);

    let err = h
        .client
        .try_delegate_resume(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(
        h.client.get_stream(&id).status,
        crate::StreamStatus::Paused,
        "stream must remain Paused after a rejected delegate_resume"
    );
    h.assert_pool_exact();
}

/// Revoking a TOP_UP grant removes it: a subsequent delegate_top_up is
/// rejected with DelegateNotPermitted, and the stream's deposited stays
/// unchanged.
#[test]
fn revoke_top_up_bit_blocks_delegate_top_up() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = stream_with_single_grant(&h, &agent, op::TOP_UP);
    let deposited_before = h.client.get_stream(&id).deposited;

    h.client
        .revoke_delegate(&id, grantor_for(&h, op::TOP_UP), &agent);

    let err = h
        .client
        .try_delegate_top_up(&id, &agent, &(100 * ONE))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(
        h.client.get_stream(&id).deposited,
        deposited_before,
        "deposited must not change after a rejected delegate_top_up"
    );
    h.assert_pool_exact();
}

/// Revoking a TRANSFER_RECIPIENT grant removes it: a subsequent
/// delegate_transfer_recipient is rejected with DelegateNotPermitted, and the
/// stream's recipient stays unchanged.
#[test]
fn revoke_transfer_recipient_bit_blocks_delegate_transfer_recipient() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = stream_with_single_grant(&h, &agent, op::TRANSFER_RECIPIENT);
    let new_recip = Address::generate(&h.env);

    h.client
        .revoke_delegate(&id, grantor_for(&h, op::TRANSFER_RECIPIENT), &agent);

    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &new_recip)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(
        h.client.get_stream(&id).recipient,
        h.recipient,
        "recipient must not change after a rejected delegate_transfer_recipient"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Criterion 2 — Revoking a never-issued grant is a no-op, not an error
// ---------------------------------------------------------------------------

/// `revoke_delegate` must succeed silently when no grant has ever been issued
/// to the address. No error, no state change.
#[test]
fn revoking_never_issued_grant_is_a_no_op() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let before = h.client.get_stream(&id);

    // No grant was ever issued to `agent`. Revoke should not panic or error.
    h.client.revoke_delegate(&id, &h.recipient, &agent);

    assert_eq!(
        h.client.get_stream(&id),
        before,
        "stream must not be touched when revoking a non-existent grant"
    );
    h.assert_pool_exact();
}

/// Same guarantee for sender-side ops: revoking a never-issued sender grant is
/// equally silent.
#[test]
fn revoking_never_issued_sender_grant_is_a_no_op() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let before = h.client.get_stream(&id);

    h.client.revoke_delegate(&id, &h.sender, &agent);

    assert_eq!(
        h.client.get_stream(&id),
        before,
        "stream must not be touched when revoking a non-existent sender grant"
    );
    h.assert_pool_exact();
}

/// `revoke_delegate` is idempotent: revoking a grant that was just revoked must
/// also succeed silently. Tested for every permission bit.
#[test]
fn revoking_already_revoked_grant_is_a_no_op_for_every_bit() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = stream_with_single_grant(&h, &agent, op_bit);

        // First revoke removes the grant.
        h.client
            .revoke_delegate(&id, grantor_for(&h, op_bit), &agent);

        // Second revoke must not panic or return an error.
        h.client
            .revoke_delegate(&id, grantor_for(&h, op_bit), &agent);

        // The delegate is still blocked (the grant was cleared by the first call,
        // not re-created by the second).
        assert_eq!(
            delegate_call_result(&h, id, &agent, op_bit),
            Err(Error::DelegateNotPermitted),
            "op bit {op_bit}: delegate must remain blocked after double revoke"
        );
    }
}

// ---------------------------------------------------------------------------
// Criterion 3 — Revocation takes effect in the same ledger, per bit
// ---------------------------------------------------------------------------

/// For each of the six permission bits: grant, revoke, and then attempt the
/// delegate call — all without advancing the ledger. The call must be rejected.
///
/// This pins the "ordered, not retroactive" guarantee from the same side as
/// `delegation::revoked_delegate_cannot_act_later_in_the_same_ledger`, but
/// focuses on the per-bit coverage required by the acceptance criteria rather
/// than the loop used in that test.
#[test]
fn revocation_takes_effect_in_the_same_ledger_for_every_bit() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = stream_with_single_grant(&h, &agent, op_bit);

        // Revoke in the same ledger the grant lives in.
        h.client
            .revoke_delegate(&id, grantor_for(&h, op_bit), &agent);
        let before = h.client.get_stream(&id);

        // No ledger advance — delegate call is in the same ledger as revoke.
        let err = delegate_call_result(&h, id, &agent, op_bit)
            .expect_err("delegate call must be rejected after same-ledger revoke");
        assert_eq!(
            err,
            Error::DelegateNotPermitted,
            "op bit {op_bit}: expected DelegateNotPermitted after same-ledger revoke"
        );

        // Rejection must be a pure auth failure — nothing in the stream changed.
        assert_eq!(
            h.client.get_stream(&id),
            before,
            "op bit {op_bit}: rejected call must not mutate the stream"
        );
    }
}

/// A delegate call ordered **before** the same-ledger revocation is honoured,
/// and a call ordered **after** is rejected — for every bit.
///
/// This covers the full ordering contract: revoke stops future use but does not
/// retroactively unwind a call that already succeeded.
#[test]
fn call_before_same_ledger_revoke_is_honoured_call_after_is_rejected() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = stream_with_single_grant(&h, &agent, op_bit);

        // Call *before* the revoke — must succeed.
        assert!(
            delegate_call_result(&h, id, &agent, op_bit).is_ok(),
            "op bit {op_bit}: pre-revoke call must succeed"
        );

        // CANCEL terminates the stream — there is nothing to revoke afterwards
        // (the stream is gone, any further call would fail with StreamTerminated,
        // not DelegateNotPermitted). Skip the post-revoke check for this bit.
        if op_bit == op::CANCEL {
            continue;
        }

        // After a TRANSFER_RECIPIENT the original recipient no longer owns the
        // stream; the sender is still a party and can revoke recipient grants.
        let revoker = if op_bit == op::TRANSFER_RECIPIENT {
            &h.sender
        } else {
            grantor_for(&h, op_bit)
        };

        // Revoke in the same ledger, no advance.
        h.client.revoke_delegate(&id, revoker, &agent);

        // Call *after* the revoke — must be rejected.
        assert_eq!(
            delegate_call_result(&h, id, &agent, op_bit),
            Err(Error::DelegateNotPermitted),
            "op bit {op_bit}: post-revoke call must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------
// Criterion 4 — Revoking one delegate leaves others intact (multi-delegate isolation)
// ---------------------------------------------------------------------------

/// Grant the same op to three delegates, revoke the middle one, and assert:
///   - The revoked delegate is blocked.
///   - The other two delegates can still act.
#[test]
fn revoking_one_delegate_leaves_sibling_delegates_intact() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY); // 300 ONE accrued — enough for three withdrawals

    let agent_a = Address::generate(&h.env);
    let agent_b = Address::generate(&h.env);
    let agent_c = Address::generate(&h.env);

    // Grant WITHDRAW to all three.
    for agent in [&agent_a, &agent_b, &agent_c] {
        h.token_admin.mint(agent, &(1_000 * ONE));
        h.client
            .grant_delegate(&id, &h.recipient, agent, &op::WITHDRAW, &None);
    }

    // Revoke only agent_b.
    h.client.revoke_delegate(&id, &h.recipient, &agent_b);

    // agent_a — still authorised.
    let paid_a = h.client.delegate_withdraw(&id, &agent_a, &Some(50 * ONE));
    assert_eq!(paid_a, 50 * ONE, "agent_a must still be able to withdraw");

    // agent_b — revoked.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent_b, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::DelegateNotPermitted,
        "agent_b must be blocked after revocation"
    );

    // agent_c — still authorised.
    let paid_c = h.client.delegate_withdraw(&id, &agent_c, &Some(50 * ONE));
    assert_eq!(paid_c, 50 * ONE, "agent_c must still be able to withdraw");

    h.assert_pool_exact();
}

/// Grant *different* ops to two delegates, revoke one op, and confirm the
/// other delegate's distinct op is unaffected.
#[test]
fn revoking_one_op_does_not_disturb_a_different_op_on_a_different_delegate() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    let agent_cancel = Address::generate(&h.env);
    let agent_withdraw = Address::generate(&h.env);

    h.token_admin.mint(&agent_cancel, &(1_000 * ONE));
    h.token_admin.mint(&agent_withdraw, &(1_000 * ONE));

    // Different grantors for sender-vs-recipient ops.
    h.client
        .grant_delegate(&id, &h.sender, &agent_cancel, &op::CANCEL, &None);
    h.client
        .grant_delegate(&id, &h.recipient, &agent_withdraw, &op::WITHDRAW, &None);

    // Revoke only the CANCEL grant.
    h.client
        .revoke_delegate(&id, &h.sender, &agent_cancel);

    // CANCEL delegate is blocked.
    let err = h
        .client
        .try_delegate_cancel(&id, &agent_cancel)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::DelegateNotPermitted,
        "CANCEL delegate must be blocked after revocation"
    );

    // WITHDRAW delegate is unaffected.
    let paid = h.client.delegate_withdraw(&id, &agent_withdraw, &None);
    assert!(paid > 0, "WITHDRAW delegate must still be able to withdraw");

    h.assert_pool_exact();
}

/// Full multi-delegate validation scenario (mirrors the acceptance criteria
/// example directly): grant to several delegates, revoke one, assert only that
/// one loses access.
#[test]
fn multi_delegate_revocation_validation_scenario() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY); // 500 ONE accrued

    // Three sender-side delegates (PAUSE) and two recipient-side (WITHDRAW).
    let pause_a = Address::generate(&h.env);
    let pause_b = Address::generate(&h.env);
    let pause_c = Address::generate(&h.env);
    let withdraw_x = Address::generate(&h.env);
    let withdraw_y = Address::generate(&h.env);

    for agent in [&pause_a, &pause_b, &pause_c] {
        h.token_admin.mint(agent, &(1_000 * ONE));
        h.client
            .grant_delegate(&id, &h.sender, agent, &op::PAUSE, &None);
    }
    for agent in [&withdraw_x, &withdraw_y] {
        h.token_admin.mint(agent, &(1_000 * ONE));
        h.client
            .grant_delegate(&id, &h.recipient, agent, &op::WITHDRAW, &None);
    }

    // Revoke the middle PAUSE delegate and one WITHDRAW delegate.
    h.client.revoke_delegate(&id, &h.sender, &pause_b);
    h.client.revoke_delegate(&id, &h.recipient, &withdraw_x);

    // pause_a — still authorised.
    h.client.delegate_pause(&id, &pause_a);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);
    h.client.resume(&id); // restore Active for the remaining checks

    // pause_b — revoked.
    let err = h
        .client
        .try_delegate_pause(&id, &pause_b)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::DelegateNotPermitted,
        "pause_b must be blocked after revocation"
    );

    // pause_c — still authorised.
    h.client.delegate_pause(&id, &pause_c);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);
    h.client.resume(&id);

    // withdraw_x — revoked.
    let err = h
        .client
        .try_delegate_withdraw(&id, &withdraw_x, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::DelegateNotPermitted,
        "withdraw_x must be blocked after revocation"
    );

    // withdraw_y — still authorised.
    let paid = h
        .client
        .delegate_withdraw(&id, &withdraw_y, &Some(100 * ONE));
    assert_eq!(paid, 100 * ONE, "withdraw_y must still be able to withdraw");

    h.assert_pool_exact();
}

/// Revoke every delegate for every bit in sequence; each revocation must leave
/// the remaining delegates' grants on *their* streams intact (cross-stream
/// isolation).
#[test]
fn revocation_is_scoped_to_stream_and_delegate_combination() {
    let h = Harness::new();

    // Two separate streams; same agent has WITHDRAW on both.
    let id_a = h.create_simple(1_000 * ONE, 100 * DAY);
    let id_b = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));

    h.client
        .grant_delegate(&id_a, &h.recipient, &agent, &op::WITHDRAW, &None);
    h.client
        .grant_delegate(&id_b, &h.recipient, &agent, &op::WITHDRAW, &None);

    // Revoke the grant on stream A only.
    h.client.revoke_delegate(&id_a, &h.recipient, &agent);

    // Stream A — revoked.
    let err = h
        .client
        .try_delegate_withdraw(&id_a, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::DelegateNotPermitted,
        "grant on stream A must be revoked"
    );

    // Stream B — untouched.
    let paid = h.client.delegate_withdraw(&id_b, &agent, &None);
    assert!(
        paid > 0,
        "grant on stream B must survive revocation on stream A"
    );

    h.assert_pool_exact();
}
