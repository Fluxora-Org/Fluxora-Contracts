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
                dust >= 0 && dust <= 1,
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
