//! Missing stream ids have one contract across every public entry point:
//! record-dependent methods return `StreamNotFound`, while the existence
//! predicate returns `false`. A live stream with no accrued value is distinct
//! from a missing stream and returns a valid zero from its accrual views.

use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::Ledger as _;

use super::common::*;
use crate::{DataKey, Error};

const MISSING_ID: u64 = 999;

fn delete_stream(h: &Harness, stream_id: u64) {
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .remove(&DataKey::Stream(stream_id));
    });
}

#[test]
fn missing_id_is_false_only_for_the_existence_query() {
    let h = Harness::new();

    assert!(!h.client.stream_exists(&MISSING_ID));
    assert_eq!(h.client.stream_count(), 0);

    assert_eq!(
        h.client.try_get_stream(&MISSING_ID).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client
            .try_withdrawable_of(&MISSING_ID)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_vested_of(&MISSING_ID).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client
            .try_refundable_of(&MISSING_ID)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );
}

#[test]
fn zero_accrual_is_reserved_for_an_existing_stream() {
    let h = Harness::new();
    let start = h.now() + DAY;
    let id = h.create(100 * ONE, start, start + 100 * DAY, start, true, true, true);

    assert!(h.client.stream_exists(&id));
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), 100 * ONE);
}

#[test]
fn deleted_id_returns_stream_not_found_across_reads_and_mutations() {
    let h = Harness::new();
    let id = h.create_simple(100 * ONE, 100 * DAY);
    delete_stream(&h, id);

    assert!(!h.client.stream_exists(&id));
    assert_eq!(
        h.client.try_get_stream(&id).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_withdrawable_of(&id).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_vested_of(&id).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_refundable_of(&id).unwrap_err().unwrap(),
        Error::StreamNotFound
    );

    assert_eq!(
        h.client.try_top_up(&id, &(ONE)).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_withdraw(&id, &None).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_cancel(&id).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_pause(&id).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_resume(&id).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client
            .try_transfer_recipient(&id, &h.other)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_extend_stream_ttl(&id).unwrap_err().unwrap(),
        Error::StreamNotFound
    );

    assert_eq!(
        h.client
            .try_batch_withdraw(&h.recipient, &h.ids(&[id]))
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(h.client.batch_extend_ttl(&h.ids(&[id])), 0);
}

#[test]
fn expired_id_is_restored_and_keeps_typed_read_behavior() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(20_000);
    let id = h.create_simple(100 * ONE, 100 * DAY);

    h.env
        .ledger()
        .set_sequence_number(h.env.ledger().sequence() + 100_000);

    assert!(h.client.try_get_stream(&id).is_ok());
    assert!(h.client.stream_exists(&id));
    assert_eq!(h.client.vested_of(&id), 0);
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), 100 * ONE);
}
