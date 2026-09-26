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

/// Create a stream and give `agent` a grant covering exactly `op`.
///
/// The stream and the agent are left in a state where the op would succeed if
/// the grant were still live, so a later rejection can only be the revocation.
fn stream_with_grant(h: &Harness, agent: &Address, op_bit: u32) -> u64 {
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.token_admin.mint(agent, &(1_000 * ONE));
    h.advance(10 * DAY);

    // `resume` only makes sense on a paused stream; pause it first.
    if op_bit == op::RESUME {
        h.client.pause(&id);
    }

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

/// Peel the host layer off a `try_delegate_*` result and normalise it to
/// `Result<(), Error>`.
///
/// The four delegate entry points do not share one generated client type
/// (`delegate_withdraw` returns `i128`, the others `()`, and the SDK's
/// return-value conversion error differs between them), so each arm is
/// normalised here instead of in the `match` above.
fn peel<T, X: std::fmt::Debug>(
    outcome: Result<Result<T, X>, Result<Error, soroban_sdk::InvokeError>>,
) -> Result<(), Error> {
    match outcome {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(x)) => panic!("delegate return value failed to convert: {x:?}"),
        Err(Ok(e)) => Err(e),
        Err(Err(e)) => panic!("delegate call trapped in the host: {e:?}"),
    }
}

/// Dispatch to the `delegate_*` entry point gated on `op_bit`, normalising the
/// heterogeneous success types to `()`.
///
/// `try_delegate_*` returns a contract error in the outer `Err(Ok(error))`.
/// Discard each successful return value and unwrap only the host error layer.
fn delegate_call_result(
    h: &Harness,
    id: u64,
    agent: &Address,
    op_bit: u32,
) -> Result<(), Error> {
    let new_recip = Address::generate(&h.env);
    let outcome = match op_bit {
        op::WITHDRAW => h.client.try_delegate_withdraw(&id, agent, &None).map(|_| ()),
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
