#![cfg(test)]

use soroban_sdk::{testutils::Events, vec, Symbol, TryFromVal, TryIntoVal, Val};

use crate::test::common::{Harness, DAY, ONE, T0};

#[test]
fn test_golden_events() {
    let harness = Harness::new();
    let env = &harness.env;
    let _sender = &harness.sender;
    let _recipient = &harness.recipient;
    let _token = &harness.token;
    let other = &harness.other;

    let start = T0;
    let end = T0 + 10 * DAY;
    let cliff = T0;

    let mut raw_events = std::vec::Vec::new();
    let mut push_events = || {
        raw_events.extend(
            env.events()
                .all()
                .filter_by_contract(&harness.contract_id)
                .events()
                .to_vec(),
        );
    };

    // 1. Create Stream
    let stream_id = harness.create(100 * ONE, start, end, cliff, true, true, true);
    push_events();

    // 2. Pause
    harness.client.pause(&stream_id);
    push_events();

    harness.advance(DAY);

    // 3. Resume
    harness.client.resume(&stream_id);
    push_events();

    // 4. Top Up
    harness.client.top_up(&stream_id, &(10 * ONE));
    push_events();

    // 5. Transfer Recipient
    harness.client.transfer_recipient(&stream_id, other);
    push_events();

    harness.advance(2 * DAY);

    // 6. Withdraw
    let _withdrawn_amount = harness.client.withdraw(&stream_id, &Some(5 * ONE));
    push_events();

    // 7. Cancel
    harness.client.cancel(&stream_id);
    push_events();

    // 8. TTL Extended
    harness.client.extend_stream_ttl(&stream_id);
    push_events();
    let mut events = std::vec::Vec::new();
    for event in raw_events {
        let soroban_sdk::xdr::ContractEventBody::V0(body) = event.body;
        let mut topics = soroban_sdk::vec![&env];
        for t in body.topics.iter() {
            topics.push_back(soroban_sdk::Val::try_from_val(env, t).unwrap());
        }
        let data: Val = soroban_sdk::Val::try_from_val(env, &body.data).unwrap();
        events.push((topics, data));
    }

    // We expect exactly 8 events
    assert_eq!(
        events.len(),
        8,
        "Expected exactly 8 events for the golden path"
    );

    // Helper to check standard event structure
    let assert_event_topic = |event: &(soroban_sdk::Vec<Val>, Val), expected_name: &str| {
        let topics = &event.0;
        let topic_symbol: Symbol = topics.get(0).unwrap().try_into_val(env).unwrap();
        assert_eq!(topic_symbol, Symbol::new(env, expected_name));
    };

    assert_event_topic(events.first().unwrap(), "stream_created");
    assert_event_topic(events.get(1).unwrap(), "paused");
    assert_event_topic(events.get(2).unwrap(), "resumed");
    assert_event_topic(events.get(3).unwrap(), "topped_up");
    assert_event_topic(events.get(4).unwrap(), "recipient_transferred");
    assert_event_topic(events.get(5).unwrap(), "withdrawn");
    assert_event_topic(events.get(6).unwrap(), "cancelled");
    assert_event_topic(events.get(7).unwrap(), "ttl_extended");

    // Topic ABI tests
    assert_eq!(
        events.first().unwrap().0.len(),
        4,
        "StreamCreated has 3 dynamic topics"
    );
    assert_eq!(
        events.get(1).unwrap().0.len(),
        3,
        "Paused has 2 dynamic topics"
    );
    assert_eq!(
        events.get(2).unwrap().0.len(),
        3,
        "Resumed has 2 dynamic topics"
    );
    assert_eq!(
        events.get(3).unwrap().0.len(),
        3,
        "ToppedUp has 2 dynamic topics"
    );
    assert_eq!(
        events.get(4).unwrap().0.len(),
        4,
        "RecipientTransferred has 3 dynamic topics"
    );
    assert_eq!(
        events.get(5).unwrap().0.len(),
        3,
        "Withdrawn has 2 dynamic topics"
    );
    assert_eq!(
        events.get(6).unwrap().0.len(),
        4,
        "Cancelled has 3 dynamic topics"
    );
    assert_eq!(
        events.get(7).unwrap().0.len(),
        2,
        "TtlExtended has 1 dynamic topic"
    );

    // Detailed schema snapshots verified here via explicit field names and types.
    // By checking the exact map keys, any incompatible change (e.g. renaming a field or removing one)
    // will cause this test to fail, acting as a checked-in schema snapshot.
    let stream_created_payload = events.first().unwrap().1;
    // We expect the payload to be a map of the struct fields
    let map: soroban_sdk::Map<Symbol, Val> = stream_created_payload.try_into_val(env).unwrap();

    // Check all expected field names are present to freeze the ABI
    let expected_fields = vec![
        &env,
        Symbol::new(env, "cancellable"),
        Symbol::new(env, "cliff_time"),
        Symbol::new(env, "deposited"),
        Symbol::new(env, "end_time"),
        Symbol::new(env, "pausable"),
        Symbol::new(env, "start_time"),
        Symbol::new(env, "token"),
        Symbol::new(env, "transferable"),
    ];
    for field in expected_fields.iter() {
        assert!(
            map.contains_key(field.clone()),
            "Missing field in StreamCreated: {:?}",
            field
        );
    }

    let paused_payload: soroban_sdk::Map<Symbol, Val> =
        events.get(1).unwrap().1.try_into_val(env).unwrap();
    assert!(paused_payload.contains_key(Symbol::new(env, "paused_at")));
    assert!(paused_payload.contains_key(Symbol::new(env, "paused_total")));

    let resumed_payload: soroban_sdk::Map<Symbol, Val> =
        events.get(2).unwrap().1.try_into_val(env).unwrap();
    assert!(resumed_payload.contains_key(Symbol::new(env, "paused_duration")));
    assert!(resumed_payload.contains_key(Symbol::new(env, "paused_total")));

    let toppedup_payload: soroban_sdk::Map<Symbol, Val> =
        events.get(3).unwrap().1.try_into_val(env).unwrap();
    assert!(toppedup_payload.contains_key(Symbol::new(env, "amount")));
    assert!(toppedup_payload.contains_key(Symbol::new(env, "deposited")));
    assert!(toppedup_payload.contains_key(Symbol::new(env, "end_time")));

    // RecipientTransferred has no data fields (only topics), so its payload is likely an empty map or it has no fields to assert.
    // We can still verify it deserializes properly.
    let _rt_payload: soroban_sdk::Map<Symbol, Val> =
        events.get(4).unwrap().1.try_into_val(env).unwrap();

    let withdrawn_payload: soroban_sdk::Map<Symbol, Val> =
        events.get(5).unwrap().1.try_into_val(env).unwrap();
    assert!(withdrawn_payload.contains_key(Symbol::new(env, "amount")));
    assert!(withdrawn_payload.contains_key(Symbol::new(env, "deposited")));
    assert!(withdrawn_payload.contains_key(Symbol::new(env, "status")));
    assert!(withdrawn_payload.contains_key(Symbol::new(env, "withdrawn")));

    let cancelled_payload: soroban_sdk::Map<Symbol, Val> =
        events.get(6).unwrap().1.try_into_val(env).unwrap();
    assert!(cancelled_payload.contains_key(Symbol::new(env, "end_time")));
    assert!(cancelled_payload.contains_key(Symbol::new(env, "refunded")));
    assert!(cancelled_payload.contains_key(Symbol::new(env, "vested")));
    assert!(cancelled_payload.contains_key(Symbol::new(env, "withdrawn")));

    let ttl_extended_payload: soroban_sdk::Map<Symbol, Val> =
        events.get(7).unwrap().1.try_into_val(env).unwrap();
    assert!(ttl_extended_payload.contains_key(Symbol::new(env, "extended_to_ledgers")));
}

#[test]
fn test_no_event_on_failure() {
    let harness = Harness::new();
    let env = &harness.env;
    let start = T0;

    // Create an invalid stream (end < start) which should fail
    let res = harness.client.try_create_stream(
        &harness.sender,
        &harness.recipient,
        &harness.token,
        &(100 * ONE),
        &start,
        &(start - 100), // invalid end time
        &start,
        &true,
        &true,
        &true,
    );

    assert!(res.is_err(), "Stream creation should fail");

    let raw_events = env.events().all().events().to_vec();
    assert_eq!(
        raw_events.len(),
        0,
        "No events should be emitted on rejected calls"
    );
}
