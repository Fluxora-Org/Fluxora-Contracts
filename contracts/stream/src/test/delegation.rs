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
//!   • Revocation is **ordered, not retroactive**: within a single ledger a
//!     delegate call ordered before the revocation is honoured and one ordered
//!     after it is rejected. See the “Same-ledger revocation ordering” section
//!     below and `docs/delegation-revocation.md`.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error};

// ---------------------------------------------------------------------------
// Grant and basic use
// ---------------------------------------------------------------------------

#[test]
fn delegate_can_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 100 * ONE);
    h.assert_pool_exact();
}

#[test]
fn delegate_can_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::CANCEL, &None);

    h.client.delegate_cancel(&id, &agent);
    assert_eq!(
        h.client.get_stream(&id).status,
        crate::StreamStatus::Cancelled
    );
    h.assert_pool_exact();
}

#[test]
fn delegate_can_pause_and_resume() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));

    h.client
        .grant_delegate(&id, &h.sender, &agent, &(op::PAUSE | op::RESUME), &None);

    h.client.delegate_pause(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Paused);

    h.client.delegate_resume(&id, &agent);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);
}

#[test]
fn delegate_pause_requires_a_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    let err = h
        .client
        .try_delegate_pause(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);
}

#[test]
fn delegate_pause_rejects_a_grant_without_the_pause_bit() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::RESUME, &None);

    let err = h
        .client
        .try_delegate_pause(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);
}

#[test]
fn delegate_pause_rejects_an_expired_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let expires = h.now() + DAY;

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::PAUSE, &Some(expires));
    h.advance(2 * DAY);

    let err = h
        .client
        .try_delegate_pause(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateExpired);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);
}

#[test]
fn delegate_pause_rejects_a_grant_revoked_in_the_same_ledger() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::PAUSE, &None);
    h.client.revoke_delegate(&id, &h.sender, &agent);

    let err = h
        .client
        .try_delegate_pause(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(h.client.get_stream(&id).status, crate::StreamStatus::Active);
}

#[test]
fn delegate_can_top_up() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::TOP_UP, &None);

    // The delegate is permitted to initiate this operation; the harness's
    // mocked sender authorization covers the token spend required by top-up.
    h.client.delegate_top_up(&id, &agent, &(100 * ONE));
    assert_eq!(h.client.get_stream(&id).deposited, 1_100 * ONE);
}

#[test]
fn delegate_can_transfer_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    let new_recip = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);

    h.client
        .delegate_transfer_recipient(&id, &agent, &new_recip);
    assert_eq!(h.client.get_stream(&id).recipient, new_recip);
}

/// #1725 — the delegate-mediated transfer path enforces the self-stream rule.
///
/// `transfer_recipient` rejects a `new_recipient` equal to the sender, so a
/// `TRANSFER_RECIPIENT` grant must not become a way around that check: the
/// grant is authority to reassign the stream, not authority to collapse its
/// sender and recipient into the same address.
#[test]
fn delegate_cannot_transfer_recipient_to_the_sender() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);

    let before = h.client.get_stream(&id);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &h.sender)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::SelfStream);
    assert_eq!(
        h.client.get_stream(&id),
        before,
        "rejected self-stream transfer must not change the stream"
    );

    // The rejection is specific to `new_recipient == sender`: the grant is
    // still usable, so transferring to any other address succeeds.
    let new_recip = Address::generate(&h.env);
    h.client
        .delegate_transfer_recipient(&id, &agent, &new_recip);
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
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);
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
    h.token_admin.mint(&agent, &(1_000 * ONE));

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);
    h.client.revoke_delegate(&id, &h.recipient, &agent);
    // Second revoke should not panic or error.
    h.client.revoke_delegate(&id, &h.recipient, &agent);
}

#[test]
fn revoke_does_not_affect_already_moved_funds() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(20 * DAY);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

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
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Recipient issued the grant.
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

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
    h.token_admin.mint(&agent, &(1_000 * ONE));

    let expires = h.now() + 5 * DAY;
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &Some(expires));

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
    h.token_admin.mint(&agent, &(1_000 * ONE));

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

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
    h.token_admin.mint(&agent, &(1_000 * ONE));

    // Grant only WITHDRAW.
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

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
fn each_permission_bit_is_independent_and_requires_its_grantor() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = match op_bit {
            op::WITHDRAW => {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.advance(10 * DAY);
                h.client
                    .grant_delegate(&id, &h.recipient, &agent, &op_bit, &None);
                id
            }
            op::CANCEL => {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client
                    .grant_delegate(&id, &h.sender, &agent, &op_bit, &None);
                id
            }
            op::PAUSE => {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client
                    .grant_delegate(&id, &h.sender, &agent, &op_bit, &None);
                id
            }
            op::RESUME => {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client.pause(&id);
                h.client
                    .grant_delegate(&id, &h.sender, &agent, &op_bit, &None);
                id
            }
            op::TOP_UP => {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client
                    .grant_delegate(&id, &h.sender, &agent, &op_bit, &None);
                id
            }
            op::TRANSFER_RECIPIENT => {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client
                    .grant_delegate(&id, &h.recipient, &agent, &op_bit, &None);
                id
            }
            other => panic!("unhandled op bit {other}"),
        };

        let wrong_grantor = match op_bit {
            op::WITHDRAW | op::TRANSFER_RECIPIENT => &h.sender,
            _ => &h.recipient,
        };
        let err = h
            .client
            .try_grant_delegate(&id, wrong_grantor, &agent, &op_bit, &None)
            .unwrap_err()
            .unwrap();
        assert_eq!(
            err,
            Error::Unauthorized,
            "op bit {op_bit}: the grantor must own the delegated permission",
        );

        assert!(
            delegate_call_result(&h, id, &agent, op_bit).is_ok(),
            "op bit {op_bit}: the sole granted permission must succeed",
        );

        for other_bit in ALL_OPS {
            if other_bit == op_bit {
                continue;
            }
            assert!(
                delegate_call_result(&h, id, &agent, other_bit).is_err(),
                "op bit {op_bit}: unrelated permission {other_bit} must be rejected",
            );
        }
    }
}

#[test]
fn sender_delegate_cannot_call_recipient_ops() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Sender grants CANCEL to the agent.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::CANCEL, &None);

    // Agent tries delegate_withdraw — op::WITHDRAW not in their grant.
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
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // Grant WITHDRAW on stream A only.
    h.client
        .grant_delegate(&id_a, &h.recipient, &agent, &op::WITHDRAW, &None);

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
    h.token_admin.mint(&agent, &(1_000 * ONE));

    let expires = h.now() + DAY;
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::CANCEL, &Some(expires));

    // Let the grant expire.
    h.advance(2 * DAY);

    let before = h.client.get_stream(&id);
    let err = h
        .client
        .try_delegate_cancel(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateExpired);
    assert_eq!(
        h.client.get_stream(&id),
        before,
        "stream must not have changed"
    );
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
    h.token_admin.mint(&agent, &(1_000 * ONE));

    let err = h
        .client
        .try_grant_delegate(&id, &h.sender, &agent, &(op::CANCEL | op::WITHDRAW), &None)
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
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);
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
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 100 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Same-ledger revocation ordering (Issue #1730)
// ---------------------------------------------------------------------------
//
// `revoke_delegate` removes the grant from storage. Within a single ledger the
// host applies writes in call order, so a delegate call that runs *after* the
// revocation observes no grant and is rejected, while one that runs *before* it
// is honoured. The guarantee is **ordered, not retroactive**: revocation stops
// future calls, it does not unwind a call that already ran.
//
// The tests below pin both directions for every permission bit. No ledger
// advance happens between the calls, so grant, call and revoke all share one
// ledger — matching the issue's acceptance criteria exactly.

/// Every permission bit a [`crate::DelegateGrant`] can carry.
///
/// Kept exhaustive so a new op added to `types::op` must be threaded through
/// the same-ledger tests below, not silently skipped.
const ALL_OPS: [u32; 6] = [
    op::WITHDRAW,
    op::CANCEL,
    op::PAUSE,
    op::RESUME,
    op::TOP_UP,
    op::TRANSFER_RECIPIENT,
];

/// The party that owns `op` and may therefore grant (and revoke) it.
fn grantor_for<'a>(h: &'a Harness<'_>, op: u32) -> &'a Address {
    match op {
        op::WITHDRAW | op::TRANSFER_RECIPIENT => &h.recipient,
        _ => &h.sender,
    }
}

/// Create a stream on which `op` would succeed if `agent` held the grant.
///
/// The stream is funded, advanced past the cliff, and paused if `op` is
/// `RESUME`. No grant is issued; callers that want one use [`stream_with_grant`].
fn stream_ready_for(h: &Harness, agent: &Address, op_bit: u32) -> u64 {
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.token_admin.mint(agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // `resume` only makes sense on a paused stream; pause it first.
    if op_bit == op::RESUME {
        h.client.pause(&id);
    }
    id
}

/// Create a stream and give `agent` a grant covering exactly `op`.
///
/// The stream and the agent are left in a state where the op would succeed if
/// the grant were still live, so a later rejection can only be the revocation.
fn stream_with_grant(h: &Harness, agent: &Address, op_bit: u32) -> u64 {
    let id = stream_ready_for(h, agent, op_bit);
    h.client
        .grant_delegate(&id, grantor_for(h, op_bit), agent, &op_bit, &None);
    id
}

/// Invoke the delegate entry point gated on `op`, discarding its result.
///
/// Panics if the call errors, so callers must have arranged a state where the
/// op would succeed with a live grant.
fn delegate_call(h: &Harness, id: u64, agent: &Address, op_bit: u32) {
    delegate_call_result(h, id, agent, op_bit).expect("delegate call should succeed");
}

/// Invoke the delegate entry point gated on `op` and return its contract error.
fn delegate_call_error(h: &Harness, id: u64, agent: &Address, op_bit: u32) -> Error {
    delegate_call_result(h, id, agent, op_bit).expect_err("delegate call should be rejected")
}

/// Dispatch to the `delegate_*` entry point gated on `op_bit`, normalising the
/// heterogeneous success types to `()`.
///
/// `try_delegate_*` returns a contract error in the outer `Err(Ok(error))`.
/// Discard each successful return value and unwrap only the host error layer.
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
    outcome.map_err(|error| error.expect("host invocation trapped"))
}

/// A delegate revoked earlier in the same ledger cannot act afterwards.
///
/// The delegate call is ordered **after** the revocation, with no ledger
/// advance between them, and must be rejected for every permission bit.
#[test]
fn revoked_delegate_cannot_act_later_in_the_same_ledger() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = stream_with_grant(&h, &agent, op_bit);

        // Revoke, then invoke — both in the same ledger, revocation first.
        h.client
            .revoke_delegate(&id, grantor_for(&h, op_bit), &agent);
        let before = h.client.get_stream(&id);

        assert_eq!(
            delegate_call_error(&h, id, &agent, op_bit),
            Error::DelegateNotPermitted,
            "op bit {op_bit}: revoked delegate must be rejected",
        );

        // The rejection is a pure authorization failure: nothing mutated.
        assert_eq!(
            h.client.get_stream(&id),
            before,
            "op bit {op_bit}: rejected delegate call must not touch the stream",
        );
    }
}

/// A delegate call ordered **before** a same-ledger revocation is honoured.
///
/// Revocation is not retroactive: it removes the grant for subsequent calls but
/// does not unwind one that already ran. After the honored call, the next call
/// in the same ledger is rejected.
#[test]
fn delegate_call_ordered_before_revocation_in_the_same_ledger_is_honoured() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = stream_with_grant(&h, &agent, op_bit);

        // Grant, call and revoke all share one ledger — no `advance` here.
        delegate_call(&h, id, &agent, op_bit);
        // Cancellation terminates the stream, so there is no live grant left
        // to revoke. The revoke-before-cancel order is covered above.
        if op_bit == op::CANCEL {
            continue;
        }
        // A recipient transfer changes who can revoke the old recipient's
        // grant; the sender remains authorized after that transfer.
        let revoker = if op_bit == op::TRANSFER_RECIPIENT {
            &h.sender
        } else {
            grantor_for(&h, op_bit)
        };
        h.client.revoke_delegate(&id, revoker, &agent);

        assert_eq!(
            delegate_call_error(&h, id, &agent, op_bit),
            Error::DelegateNotPermitted,
            "op bit {op_bit}: call after revocation must be rejected",
        );
    }
}

/// A delegate call ordered **before** a same-ledger grant is not authorised
/// retroactively.
///
/// The grant only authorises calls ordered after it. The rejected call runs
/// with no grant present, so it also leaves the stream unchanged; the call
/// issued after the grant then succeeds in the same ledger. Together with
/// [`delegate_call_ordered_before_revocation_in_the_same_ledger_is_honoured`]
/// this pins both orderings of the `grant_delegate` / `delegate_*` pair.
#[test]
fn delegate_call_ordered_before_a_same_ledger_grant_is_rejected() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = stream_ready_for(&h, &agent, op_bit);

        // No grant yet: a call ordered before the grant is rejected.
        let before = h.client.get_stream(&id);
        assert_eq!(
            delegate_call_error(&h, id, &agent, op_bit),
            Error::DelegateNotPermitted,
            "op bit {op_bit}: call ordered before the grant must be rejected",
        );
        assert_eq!(
            h.client.get_stream(&id),
            before,
            "op bit {op_bit}: rejected call must not touch the stream",
        );

        // Grant, then call again — both in the same ledger, no `advance`.
        h.client
            .grant_delegate(&id, grantor_for(&h, op_bit), &agent, &op_bit, &None);
        delegate_call(&h, id, &agent, op_bit);
    }
}

/// The fixture is exhaustive: every permission bit is exercised by the
/// same-ledger tests above, so a newly added op cannot slip through untested.
#[test]
fn all_ops_fixture_covers_every_permission_bit() {
    let mut covered: u32 = 0;
    for op_bit in ALL_OPS {
        assert_eq!(
            op_bit.count_ones(),
            1,
            "ALL_OPS entries must be single bits"
        );
        assert_eq!(covered & op_bit, 0, "duplicate op bit {op_bit} in ALL_OPS");
        covered |= op_bit;
    }

    // The six bits used by `types::op` (1 << 0 .. 1 << 5). If a new bit is
    // added, extend ALL_OPS and this mask together.
    assert_eq!(
        covered, 0b11_1111,
        "ALL_OPS does not cover every permission bit"
    );
}

// ---------------------------------------------------------------------------
// Recipient transfer (Issue #1696)
// ---------------------------------------------------------------------------
//
// Documented rule — `docs/delegation-revocation.md`, section “Recipient
// transfer: grants survive”. A transfer reassigns who is paid; it does not
// touch `Delegate(stream_id, delegate)` entries. Recipient-issued grants
// therefore pass to the new holder of the recipient slot, who can revoke them,
// while the old recipient — no longer a party to the stream — can revoke
// nothing. Sender-issued grants are unaffected because the sender does not
// change with the transfer.
//
// Each test below loops over `ALL_OPS`, so the rule is asserted for every
// permission bit rather than for a representative sample.

/// The documented rule: a grant that was live before the transfer is still
/// live after it, for every permission bit.
#[test]
fn delegate_grants_survive_a_recipient_transfer_for_every_permission_bit() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        let agent = Address::generate(&h.env);
        let id = stream_with_grant(&h, &agent, op_bit);

        h.client.transfer_recipient(&id, &h.other);
        assert_eq!(
            h.client.get_stream(&id).recipient,
            h.other,
            "op bit {op_bit}: the transfer must have taken effect",
        );

        // Authorised only if `check_delegate` still found the grant — a
        // cleared grant would fail with `DelegateNotPermitted` instead.
        delegate_call(&h, id, &agent, op_bit);
    }
}

/// Grants survive, so authority over the recipient-issued ones follows the
/// recipient slot: the new recipient can revoke them the moment they take over.
#[test]
fn the_new_recipient_can_revoke_a_grant_that_survived_the_transfer() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        // Recipient-issued bits only — the sender's grants are the sender's
        // to revoke and are covered by `sender_can_revoke_recipient_issued_grant`.
        if *grantor_for(&h, op_bit) != h.recipient {
            continue;
        }
        let agent = Address::generate(&h.env);
        let id = stream_with_grant(&h, &agent, op_bit);

        h.client.transfer_recipient(&id, &h.other);
        h.client.revoke_delegate(&id, &h.other, &agent);

        let before = h.client.get_stream(&id);
        assert_eq!(
            delegate_call_error(&h, id, &agent, op_bit),
            Error::DelegateNotPermitted,
            "op bit {op_bit}: the new recipient's revocation must be effective",
        );
        assert_eq!(
            h.client.get_stream(&id),
            before,
            "op bit {op_bit}: a rejected delegate call must not touch the stream",
        );
    }
}

/// The previous recipient is no longer a party to the stream, so revocation is
/// theirs no longer — and the rejection must not have cleared the grant either.
#[test]
fn the_old_recipient_cannot_revoke_after_a_transfer() {
    for op_bit in ALL_OPS {
        let h = Harness::new();
        if *grantor_for(&h, op_bit) != h.recipient {
            continue;
        }
        let agent = Address::generate(&h.env);
        let id = stream_with_grant(&h, &agent, op_bit);

        h.client.transfer_recipient(&id, &h.other);

        let err = h
            .client
            .try_revoke_delegate(&id, &h.recipient, &agent)
            .unwrap_err()
            .unwrap();
        assert_eq!(
            err,
            Error::Unauthorized,
            "op bit {op_bit}: the old recipient is no longer a party",
        );

        // The grant is untouched by the rejected call: the delegate still acts.
        delegate_call(&h, id, &agent, op_bit);
    }
}
// ---------------------------------------------------------------------------
// Guard parity — the delegate entry points re-check the owner-path guards
//
// `check_delegate` only validates the grant (existence, expiry, op bit); it
// does not look at the stream. Each `delegate_*` entry point therefore repeats
// the terminal, amount-domain, maturity, and self-transfer guards of its owner
// counterpart. These tests pin those replicated rejections so the two paths
// cannot drift apart silently.
// ---------------------------------------------------------------------------

#[test]
fn delegate_withdraw_reports_nothing_to_withdraw_before_accrual() {
    let h = Harness::new();
    let start = h.now() + 10 * DAY;
    let id = h.create(100 * ONE, start, start + 100 * DAY, start, true, true, true);
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    // No accrual yet, but the stream is still live: that is `NothingToWithdraw`,
    // not `StreamTerminated`.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::NothingToWithdraw);
    h.assert_pool_exact();
}

#[test]
fn delegate_withdraw_on_a_settled_stream_is_terminated() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    h.advance(10 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, crate::StreamStatus::Depleted);

    // Nothing is left to withdraw and the stream is over: the delegate path
    // must report `StreamTerminated`, never pay twice.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);
    h.assert_pool_exact();
}

#[test]
fn delegate_withdraw_uses_the_same_amount_domain_as_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);
    h.advance(30 * DAY); // exactly 300 ONE accrued

    // An explicit partial amount is honoured.
    let paid = h.client.delegate_withdraw(&id, &agent, &Some(100 * ONE));
    assert_eq!(paid, 100 * ONE);

    for amount in [0i128, -1] {
        let err = h
            .client
            .try_delegate_withdraw(&id, &agent, &Some(amount))
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::InvalidAmount, "amount {amount}");
    }

    // 200 ONE remains accrued, so 300 ONE exceeds what is available.
    let err = h
        .client
        .try_delegate_withdraw(&id, &agent, &Some(300 * ONE))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InsufficientWithdrawable);
    h.assert_pool_exact();
}

#[test]
fn delegate_cancel_rejects_a_stream_that_is_already_terminal() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::CANCEL, &None);

    h.advance(10 * DAY);
    h.client.delegate_cancel(&id, &agent);

    let err = h
        .client
        .try_delegate_cancel(&id, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);
    h.assert_pool_exact();
}

#[test]
fn delegate_pause_rejects_already_paused_and_terminal_streams() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);

    let paused = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client
        .grant_delegate(&paused, &h.sender, &agent, &op::PAUSE, &None);
    h.client.delegate_pause(&paused, &agent);
    let err = h
        .client
        .try_delegate_pause(&paused, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamAlreadyPaused);

    let settled = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client
        .grant_delegate(&settled, &h.sender, &agent, &op::PAUSE, &None);
    h.client.cancel(&settled);
    let err = h
        .client
        .try_delegate_pause(&settled, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);
}

#[test]
fn delegate_resume_rejects_not_paused_and_terminal_streams() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);

    let active = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client
        .grant_delegate(&active, &h.sender, &agent, &op::RESUME, &None);
    let err = h
        .client
        .try_delegate_resume(&active, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamNotPaused);

    let settled = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client
        .grant_delegate(&settled, &h.sender, &agent, &op::RESUME, &None);
    h.client.cancel(&settled);
    let err = h
        .client
        .try_delegate_resume(&settled, &agent)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);
}

/// Positive: `delegate_resume` demands the delegate address that holds the grant.
#[test]
fn delegate_resume_requires_the_delegate() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::RESUME, &None);
    h.client.delegate_pause(&id, &agent);
    h.client.delegate_resume(&id, &agent);
    assert_eq!(required_auth(&h.env), agent, "delegate_resume");
}

/// Negative: no authorization → rejected.
#[test]
#[should_panic(expected = "Unauthorized")]
fn delegate_resume_fails_without_authorization() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::RESUME, &None);
    h.client.delegate_pause(&id, &agent);

    revoke_all_auths(&h.env);
    h.client.delegate_resume(&id, &agent);
}

/// Storage-unchanged guard: `paused_at`, `paused_total`, and `status` are
/// untouched after a rejected delegate_resume.
#[test]
fn rejected_delegate_resume_does_not_advance_paused_total_or_clear_paused_at() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::RESUME, &None);
    h.client.delegate_pause(&id, &agent);
    let paused_at_before = h.get(id).paused_at;

    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    let _ = h.client.try_delegate_resume(&id, &agent);
    h.env.mock_all_auths();

    let s = h.get(id);
    assert_eq!(s.status, crate::StreamStatus::Paused, "still paused");
    assert_eq!(s.paused_at, paused_at_before, "paused_at not cleared");
    assert_eq!(s.paused_total, 0, "paused_total not advanced");
    h.assert_pool_exact();
}

#[test]
fn delegate_top_up_guards_terminal_invalid_matured_and_sub_second() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);

    // Terminal stream.
    let settled = h.create_simple(1_000 * ONE, 10 * DAY);
    h.client
        .grant_delegate(&settled, &h.sender, &agent, &op::TOP_UP, &None);
    h.advance(10 * DAY);
    h.client.withdraw(&settled, &None);
    let err = h
        .client
        .try_delegate_top_up(&settled, &agent, &(100 * ONE))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);

    // Non-positive amount.
    let live = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client
        .grant_delegate(&live, &h.sender, &agent, &op::TOP_UP, &None);
    let err = h
        .client
        .try_delegate_top_up(&live, &agent, &0)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidAmount);

    // Matured stream.
    h.warp_to(h.get(live).end_time);
    let err = h
        .client
        .try_delegate_top_up(&live, &agent, &(100 * ONE))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamMatured);

    // 100 stroops/sec: one stroop buys no extra second, so it is rejected
    // rather than absorbed by raising the rate.
    let start = h.now();
    let dense = h.create(10_000, start, start + 100, start, true, true, true);
    h.client
        .grant_delegate(&dense, &h.sender, &agent, &op::TOP_UP, &None);
    let err = h
        .client
        .try_delegate_top_up(&dense, &agent, &1)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::TopUpTooSmall);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// `delegate_transfer_recipient` — parity with the direct `transfer_recipient`
// (Issue #1827)
//
// The delegated path carries an authorisation check the direct path does not,
// so it is the more dangerous of the two and must be tested at the same depth.
// `transfer_recipient`'s rejection paths are pinned in `test::transfer`,
// `test::capabilities` and `test::terminal_operations`; each test below mirrors
// one of them on the delegate path. Two are deliberately *not* duplicated here
// because they already exist and would only be re-stated:
//
//   * `NotTransferable` — `capabilities::not_transferable_rejects_delegate_transfer_recipient`
//   * `SelfStream` — `delegate_cannot_transfer_recipient_to_the_sender`
//
// The shared stream-level guards are exercised against a *valid* grant so a
// rejection can only come from the stream state, not from missing authority.
// The grant-specific rejections (no grant, wrong bit, expired, revoked) follow
// afterwards; they fail inside `check_delegate`, before any stream state is
// read, which is why each asserts a byte-for-byte unchanged stream.
// ---------------------------------------------------------------------------

/// Rejection parity for the stream-lookup case. `transfer_recipient` answers an
/// unknown id with `StreamNotFound`, but a grant can only be issued against an
/// existing stream — so on the delegated path `check_delegate` rejects first and
/// there is no `StreamNotFound` equivalent to pin. This test documents that
/// ordering so the asymmetry is intentional and visible, not an accident.
#[test]
fn delegate_transfer_recipient_rejects_an_unknown_stream_as_a_missing_grant() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);

    let err = h
        .client
        .try_delegate_transfer_recipient(&999, &agent, &new_recip)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::DelegateNotPermitted,
        "authorisation is checked before the stream lookup"
    );
}

/// No grant at all: rejected with `DelegateNotPermitted` and the stream is left
/// untouched. `transfer_recipient` has no equivalent — its gate is the sender's
/// signature — so this is the delegate-only half of the parity.
#[test]
fn delegate_transfer_recipient_requires_a_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);

    let before = h.get(id);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &new_recip)
        .unwrap_err()
        .unwrap();

    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(h.get(id), before, "rejected call must not touch the stream");
    assert_eq!(h.get(id).recipient, h.recipient);
    h.assert_pool_exact();
}

/// A grant that authorises another recipient-side op (`WITHDRAW`) but not
/// `TRANSFER_RECIPIENT` cannot reassign: permissions are orthogonal, so the
/// rejection is `DelegateNotPermitted` and the grant stays usable for the op it
/// does cover.
#[test]
fn delegate_transfer_recipient_requires_the_transfer_recipient_bit() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);

    // Correct grantor (the recipient owns `TRANSFER_RECIPIENT`) but the wrong bit.
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);

    let before = h.get(id);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &new_recip)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(h.get(id), before);

    // The grant is not consumed by the rejection: its own op still works.
    h.advance(10 * DAY);
    assert_eq!(h.client.delegate_withdraw(&id, &agent, &None), 100 * ONE);
    h.assert_pool_exact();
}

/// A grant whose `expires_at` has passed is rejected with `DelegateExpired` on
/// the transfer path — not only on withdraw. Mirrors `expired_grant_is_rejected`.
#[test]
fn delegate_transfer_recipient_rejects_an_expired_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    let expires = h.now() + 5 * DAY;
    h.client.grant_delegate(
        &id,
        &h.recipient,
        &agent,
        &op::TRANSFER_RECIPIENT,
        &Some(expires),
    );

    // Live just before expiry: the delegated transfer works.
    h.advance(4 * DAY);
    let first = Address::generate(&h.env);
    h.client.delegate_transfer_recipient(&id, &agent, &first);
    assert_eq!(h.get(id).recipient, first);

    // Past expiry: rejected, and the stream is not moved a second time.
    h.advance(2 * DAY);
    let before = h.get(id);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateExpired);
    assert_eq!(h.get(id), before);
    h.assert_pool_exact();
}

/// A grant revoked earlier in the **same ledger** is rejected, with no ledger
/// advance between the revocation and the call. The `ALL_OPS` loop in
/// `revoked_delegate_cannot_act_later_in_the_same_ledger` already covers this
/// bit; naming the transfer case here keeps the issue-to-test mapping explicit.
#[test]
fn delegate_transfer_recipient_rejects_a_grant_revoked_in_the_same_ledger() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);
    h.client.revoke_delegate(&id, &h.recipient, &agent);

    let before = h.get(id);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &new_recip)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
    assert_eq!(
        h.get(id).recipient,
        h.recipient,
        "revocation must leave the recipient in place"
    );
    assert_eq!(h.get(id), before);
    h.assert_pool_exact();
}

/// Parity with `transferring_to_the_current_recipient_is_an_error` and
/// `new_recipient_replay_fails_due_to_repeated_transfer`: the delegated path
/// reports `RepeatedTransfer` for a no-op reassignment instead of returning a
/// silent success. This is the guard #1827 added to close the drift left by
/// #1637, which introduced the error on the direct path only.
#[test]
fn delegate_transfer_recipient_rejects_a_repeated_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);

    // (1) Targeting the address that already holds the recipient slot.
    let before = h.get(id);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &h.recipient)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::RepeatedTransfer);
    assert_eq!(
        h.get(id),
        before,
        "no-op transfer must not change the stream"
    );

    // (2) Replaying a transfer that already landed: the same `new_recipient` is
    // now the current recipient, so the replay is rejected too.
    let moved = Address::generate(&h.env);
    h.client.delegate_transfer_recipient(&id, &agent, &moved);
    assert_eq!(h.get(id).recipient, moved);

    let after_move = h.get(id);
    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &moved)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::RepeatedTransfer);
    assert_eq!(h.get(id), after_move, "replay must not change the stream");
    h.assert_pool_exact();
}

/// A fully withdrawn stream has no claim left to reassign. Parity with
/// `depleted_stream_rejects_transfer_recipient_when_fully_drained` and
/// `transfer_after_depletion_is_rejected_and_retry_is_stable`.
#[test]
fn delegate_transfer_recipient_rejects_a_depleted_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);

    h.advance(10 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, crate::StreamStatus::Depleted);

    let before = h.get(id);
    for _ in 0..2 {
        let err = h
            .client
            .try_delegate_transfer_recipient(&id, &agent, &new_recip)
            .unwrap_err()
            .unwrap();
        assert_eq!(err, Error::StreamTerminated);
    }
    assert_eq!(h.get(id), before);
    h.assert_pool_exact();
}

/// A cancelled stream whose tail has been fully drawn is settled, so it is not
/// reassignable. Parity with
/// `cancelled_stream_with_settled_claim_rejects_transfer_recipient`. The grant
/// must be issued before the cancel, because `grant_delegate` refuses terminal
/// streams.
#[test]
fn delegate_transfer_recipient_rejects_a_cancelled_stream_with_a_settled_claim() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    h.client.withdraw(&id, &None); // draw the cancelled tail: claim settled
    let before = h.get(id);
    assert_eq!(
        before.withdrawn, before.deposited,
        "sanity: claim is settled"
    );
    assert_eq!(before.status, crate::StreamStatus::Cancelled);

    let err = h
        .client
        .try_delegate_transfer_recipient(&id, &agent, &new_recip)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);
    assert_eq!(h.get(id), before);
    h.assert_pool_exact();
}

/// Semantic parity with `transfer_before_accrual_moves_the_entire_claim`: the
/// delegated transfer re-points the whole outstanding claim, the new recipient
/// is the only one who can withdraw it, and the old recipient receives nothing.
#[test]
fn delegate_transfer_recipient_moves_the_entire_claim_to_the_new_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);
    let old_recipient_balance = h.balance(&h.recipient);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);

    let before = h.get(id);
    h.client
        .delegate_transfer_recipient(&id, &agent, &new_recip);

    // Exactly the recipient field moved; nothing else did.
    let mut expected = before.clone();
    expected.recipient = new_recip.clone();
    assert_eq!(h.get(id), expected);
    h.assert_pool_exact();

    // Accrual continues to the new recipient only.
    h.advance(100 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    assert_eq!(h.balance(&new_recip), 1_000 * ONE);
    assert_eq!(
        h.balance(&h.recipient),
        old_recipient_balance,
        "the old recipient must receive nothing"
    );
    h.assert_pool_exact();
}

/// A delegated transfer while paused must preserve the frozen claim, exactly
/// like `transfer_while_paused_preserves_the_frozen_claim`: the pause survives
/// the transfer and still gates accrual.
#[test]
fn delegate_transfer_recipient_while_paused_preserves_the_frozen_claim() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let new_recip = Address::generate(&h.env);

    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::TRANSFER_RECIPIENT, &None);

    h.advance(30 * DAY);
    h.client.pause(&id);
    let paused_at = h.now();
    h.advance(50 * DAY);

    let before = h.get(id);
    h.client
        .delegate_transfer_recipient(&id, &agent, &new_recip);
    let after = h.get(id);

    let mut expected = before.clone();
    expected.recipient = new_recip.clone();
    assert_eq!(after, expected);
    assert_eq!(after.status, crate::StreamStatus::Paused);
    assert_eq!(after.paused_at, Some(paused_at));

    // The pause still freezes accrual after the transfer.
    assert_eq!(h.client.withdrawable_of(&id), 300 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 300 * ONE);
    h.advance(20 * DAY);
    assert_eq!(
        h.client.withdrawable_of(&id),
        0,
        "pause still freezes accrual"
    );
    h.assert_pool_exact();
}

#[test]
fn grant_delegate_rejects_a_terminal_stream_and_zero_ops_grants_nothing() {
    let h = Harness::new();
    let agent = Address::generate(&h.env);

    let settled = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.cancel(&settled);
    let err = h
        .client
        .try_grant_delegate(&settled, &h.sender, &agent, &op::CANCEL, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::StreamTerminated);

    // `ops == 0` is a no-op that grants nothing.
    let live = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client
        .grant_delegate(&live, &h.recipient, &agent, &0, &None);
    let err = h
        .client
        .try_delegate_withdraw(&live, &agent, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::DelegateNotPermitted);
}
