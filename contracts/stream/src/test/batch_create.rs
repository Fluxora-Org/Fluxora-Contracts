use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Vec};

use super::common::*;
use crate::{BatchCreateRequest, Error, MAX_BATCH_SIZE};

fn request(h: &Harness, recipient: Address) -> BatchCreateRequest {
    let start = h.now();
    BatchCreateRequest {
        recipient,
        token: h.token.clone(),
        deposit: 100 * ONE,
        start_time: start,
        end_time: start + DAY,
        cliff_time: start,
        cancellable: true,
        pausable: true,
        transferable: true,
    }
}

#[test]
fn batch_create_returns_ordered_ids_and_creates_every_stream() {
    let h = Harness::new();
    let first = Address::generate(&h.env);
    let second = Address::generate(&h.env);
    let mut requests = Vec::new(&h.env);
    requests.push_back(request(&h, first.clone()));
    requests.push_back(request(&h, second.clone()));

    let ids = h.client.batch_create(&h.sender, &requests);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids.get_unchecked(0), 0);
    assert_eq!(ids.get_unchecked(1), 1);
    assert_eq!(h.client.stream_count(), 2);
}

#[test]
fn invalid_element_does_not_create_any_stream() {
    let h = Harness::new();
    let mut requests = Vec::new(&h.env);
    requests.push_back(request(&h, Address::generate(&h.env)));
    let mut invalid = request(&h, Address::generate(&h.env));
    invalid.end_time = invalid.start_time;
    requests.push_back(invalid);

    assert_eq!(
        h.client.try_batch_create(&h.sender, &requests),
        Err(Ok(Error::InvalidTimeRange))
    );
    assert_eq!(h.client.stream_count(), 0);
}

#[test]
fn batch_over_maximum_is_rejected() {
    let h = Harness::new();
    let mut requests = Vec::new(&h.env);
    for _ in 0..=MAX_BATCH_SIZE {
        requests.push_back(request(&h, Address::generate(&h.env)));
    }

    assert_eq!(
        h.client.try_batch_create(&h.sender, &requests),
        Err(Ok(Error::BatchTooLarge))
    );
    assert_eq!(h.client.stream_count(), 0);
}
