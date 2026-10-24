//! Issue #1822 — settlement conserves uneven deposits and leaves at most one
//! smallest token unit of sender-owned dust.

use super::common::*;
use crate::StreamStatus;

#[test]
fn uneven_deposits_settle_without_touching_recipient_funds() {
    // Sweep uneven deposit/duration combinations. At the terminal timestamp
    // the recipient is owed the full deposit, so settlement leaves no residue;
    // the bound remains valid for every stream in the generated domain.
    for deposit in 3i128..24 {
        for duration in 2u64..=(deposit as u64) {
            if deposit % duration as i128 == 0 {
                continue;
            }
            let h = Harness::new();
            let id = h.create_simple(deposit, duration);
            h.advance(duration);
            let paid = h.client.withdraw(&id, &None);
            assert_eq!(paid, deposit);
            assert_eq!(h.get(id).status, StreamStatus::Depleted);
            assert_eq!(h.balance(&h.recipient), paid);

            let sender_before = h.balance(&h.sender);
            let dust = h.client.reclaim_dust(&id);
            assert!(
                (0..=1).contains(&dust),
                "deposit={deposit}, duration={duration}"
            );
            assert_eq!(dust, 0, "no sender residue after exact settlement");
            assert_eq!(h.balance(&h.sender), sender_before);
            assert_eq!(h.pool(), 0, "all deposited funds were accounted for");
        }
    }
}

#[test]
fn reclaim_dust_does_nothing_for_a_live_stream() {
    let h = Harness::new();
    let id = h.create_simple(100, 10);
    assert_eq!(h.client.reclaim_dust(&id), 0);
}

/// Calling on a Cancelled stream returns the dust and transfers it to the sender.
#[test]
fn reclaim_dust_returns_dust_on_cancelled_stream() {
    let h = Harness::new();
    // Use an uneven deposit/duration pair so settlement can leave dust.
    let id = h.create_simple(100, 3);
    h.advance(1);
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);

    // Withdraw what the recipient is owed, leaving any residue in the pool.
    let paid = h.client.withdraw(&id, &None);
    let pool_before = h.pool();
    let sender_before = h.balance(&h.sender);

    let dust = h.client.reclaim_dust(&id);
    assert_eq!(dust, pool_before, "dust must equal the remaining pool");
    assert_eq!(h.balance(&h.sender), sender_before + dust);
    assert_eq!(h.pool(), 0);
    assert_eq!(h.balance(&h.recipient), paid);
}

/// Calling on a Matured, fully-withdrawn stream returns Ok(0).
#[test]
fn reclaim_dust_returns_zero_on_matured_fully_withdrawn() {
    let h = Harness::new();
    let id = h.create_simple(100, 10);
    h.advance(10);
    let paid = h.client.withdraw(&id, &None);
    assert_eq!(paid, 100);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);

    let sender_before = h.balance(&h.sender);
    assert_eq!(h.client.reclaim_dust(&id), 0);
    assert_eq!(h.balance(&h.sender), sender_before);
    assert_eq!(h.pool(), 0);
}

/// A caller who is not the sender is rejected by require_auth.
#[test]
#[should_panic(expected = "Unauthorized")]
fn reclaim_dust_requires_sender_auth() {
    let h = Harness::new();
    let id = h.create_simple(100, 3);
    h.advance(1);
    h.client.cancel(&id);
    h.client.withdraw(&id, &None);

    // Drop every mocked authorization, the same discipline as `test::auth`.
    h.env.mock_auths(&[]);
    h.client.reclaim_dust(&id);
}

/// Calling twice on the same terminal stream returns 0(the second time).
#[test]
fn reclaim_dust_twice_returns_zero_second_time() {
    let h = Harness::new();
    let id = h.create_simple(100, 3);
    h.advance(1);
    h.client.cancel(&id);
    h.client.withdraw(&id, &None);

    let first = h.client.reclaim_dust(&id);
    let sender_after_first = h.balance(&h.sender);
    let second = h.client.reclaim_dust(&id);
    assert_eq!(second, 0);
    assert_eq!(h.balance(&h.sender), sender_after_first);
    assert!(first >= 0);
}
