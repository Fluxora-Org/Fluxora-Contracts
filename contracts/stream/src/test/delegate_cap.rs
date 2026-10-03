//! Issue #1729 — per-stream delegate grants have a bounded storage footprint.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error, MAX_DELEGATES_PER_STREAM};

#[test]
fn delegate_cap_allows_the_boundary_and_only_replacements_after_it() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let mut delegates = std::vec::Vec::new();

    for _ in 0..MAX_DELEGATES_PER_STREAM {
        let delegate = Address::generate(&h.env);
        h.client
            .grant_delegate(&id, &h.recipient, &delegate, &op::WITHDRAW, &None);
        delegates.push(delegate);
    }

    let extra = Address::generate(&h.env);
    assert_eq!(
        h.client
            .try_grant_delegate(&id, &h.recipient, &extra, &op::WITHDRAW, &None)
            .unwrap_err()
            .unwrap(),
        Error::TooManyDelegates,
    );

    // Replacing a grant does not allocate another storage entry or slot.
    h.client.grant_delegate(
        &id,
        &h.recipient,
        &delegates[0],
        &op::TRANSFER_RECIPIENT,
        &None,
    );

    // Removing one grant frees exactly one slot for a new delegate.
    h.client
        .revoke_delegate(&id, &h.recipient, &delegates[0]);
    h.client
        .grant_delegate(&id, &h.recipient, &extra, &op::WITHDRAW, &None);
}
