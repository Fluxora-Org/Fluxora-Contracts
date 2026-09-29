//! A stream party acting without a delegate grant (issue #1882).
//!
//! The contract has two authorisation models living side by side:
//!
//! * the **owner** entry points (`withdraw`, `cancel`, `pause`, …) check party
//!   identity with `require_auth` on the stream's sender or recipient; nothing
//!   else is needed;
//! * the **delegate** entry points (`delegate_*`) check a stored grant in
//!   [`crate::storage::load_delegate`] *first*, then call `require_auth` on the
//!   delegate named in the call.
//!
//! The boundary between them is the question this module pins: does being a
//! party to the stream (sender or recipient) substitute for holding a grant?
//! It does not. The six `delegate_*` entry points are a single authorisation
//! model: they require a live grant covering the operation, whoever is asking.
//! A sender or recipient who wants to use them must be granted the permission
//! like anybody else; the owner paths are reached through the owner entry
//! points instead. Aggregating a party's identity into the delegate check would
//! silently widen the delegated surface to two addresses per stream.
//!
//! Both parties are covered for all six delegated operations below, and a
//! positive control shows the owner paths are *not* gated on a grant.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error};

/// Every permission bit a [`crate::DelegateGrant`] can carry.
///
/// Kept exhaustive so a new op added to `types::op` must be threaded through
/// these tests, not silently skipped.
const ALL_OPS: [u32; 6] = [
    op::WITHDRAW,
    op::CANCEL,
    op::PAUSE,
    op::RESUME,
    op::TOP_UP,
    op::TRANSFER_RECIPIENT,
];

/// Is there a stored grant for `(stream_id, who)`?
///
/// The storage layer is only reachable from inside a contract frame, so this
/// wraps the read the same way the contract does.
fn has_grant(h: &Harness, stream_id: u64, who: &Address) -> bool {
    h.env
        .as_contract(&h.contract_id, || {
            crate::storage::load_delegate(&h.env, stream_id, who)
        })
        .is_some()
}

/// Dispatch to the `delegate_*` entry point gated on `op_bit`, with `caller` as
/// the named delegate, and return the contract error.
///
/// No grant is ever issued to `caller` in these tests, so the first gate —
/// `check_delegate` — is what must reject every call.
fn delegate_call_error(h: &Harness, id: u64, caller: &Address, op_bit: u32) -> Error {
    let new_recip = Address::generate(&h.env);
    let outcome = match op_bit {
        op::WITHDRAW => h
            .client
            .try_delegate_withdraw(&id, caller, &None)
            .map(|_| ()),
        op::CANCEL => h.client.try_delegate_cancel(&id, caller).map(|_| ()),
        op::PAUSE => h.client.try_delegate_pause(&id, caller).map(|_| ()),
        op::RESUME => h.client.try_delegate_resume(&id, caller).map(|_| ()),
        op::TOP_UP => h
            .client
            .try_delegate_top_up(&id, caller, &(100 * ONE))
            .map(|_| ()),
        op::TRANSFER_RECIPIENT => h
            .client
            .try_delegate_transfer_recipient(&id, caller, &new_recip)
            .map(|_| ()),
        other => panic!("unhandled op bit {other}"),
    };
    outcome
        .expect_err("a delegate entry point must reject a caller with no grant")
        .expect("host invocation trapped")
}

/// The sender calling a `delegate_*` entry point for its own stream, without a
/// grant, is rejected — for every delegated operation.
#[test]
fn the_sender_cannot_use_a_delegate_entry_point_without_a_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    for op_bit in ALL_OPS {
        assert_eq!(
            delegate_call_error(&h, id, &h.sender, op_bit),
            Error::DelegateNotPermitted,
            "op bit {op_bit}: the sender holds no grant and must be rejected",
        );
    }

    assert!(!has_grant(&h, id, &h.sender));
    h.assert_pool_exact();
}

/// The same for the recipient. The recipient is the sole authority behind the
/// `WITHDRAW` and `TRANSFER_RECIPIENT` bits, yet the delegate path still gates
/// on a grant rather than on identity.
#[test]
fn the_recipient_cannot_use_a_delegate_entry_point_without_a_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    for op_bit in ALL_OPS {
        assert_eq!(
            delegate_call_error(&h, id, &h.recipient, op_bit),
            Error::DelegateNotPermitted,
            "op bit {op_bit}: the recipient holds no grant and must be rejected",
        );
    }

    assert!(!has_grant(&h, id, &h.recipient));
    h.assert_pool_exact();
}

/// A rejected party call is a pure authorization failure: the stream is exactly
/// as it was and no tokens move.
#[test]
fn a_party_call_without_a_grant_leaves_no_trace() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    let before = h.get(id);
    let recipient_before = h.balance(&h.recipient);

    delegate_call_error(&h, id, &h.recipient, op::WITHDRAW);
    delegate_call_error(&h, id, &h.sender, op::CANCEL);

    assert_eq!(
        h.get(id),
        before,
        "rejected calls must not change the stream"
    );
    assert_eq!(h.balance(&h.recipient), recipient_before);
    h.assert_pool_exact();
}

/// Positive control: the *owner* entry points are not gated on a grant. Only
/// the `delegate_*` surface requires one, so a party that never issued a grant
/// still controls its own stream through the owner paths.
#[test]
fn the_owner_paths_stay_ungated_on_a_grant() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(10 * DAY);
    let paid = h.client.withdraw(&id, &None);
    assert_eq!(paid, 100 * ONE);

    h.client.pause(&id);
    assert_eq!(h.get(id).status, crate::StreamStatus::Paused);
    h.client.resume(&id);
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, crate::StreamStatus::Cancelled);

    assert!(!has_grant(&h, id, &h.sender));
    assert!(!has_grant(&h, id, &h.recipient));
    h.assert_pool_exact();
}

/// After a genuine grant, the party that *issued* it still cannot use the
/// delegate path unless it is also the named delegate: the grant names one
/// address, not a role.
#[test]
fn a_grant_naming_someone_else_does_not_admit_the_grantor() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &None);
    h.advance(10 * DAY);

    assert_eq!(
        delegate_call_error(&h, id, &h.recipient, op::WITHDRAW),
        Error::DelegateNotPermitted,
    );
    let paid = h.client.delegate_withdraw(&id, &agent, &None);
    assert_eq!(paid, 100 * ONE);
    h.assert_pool_exact();
}
