use soroban_sdk::{testutils::Address as _, Address, Env, String};
use fluxora_stream::{ContractError, FluxoraStream, StreamStatus};

fn setup() -> (Env, FluxoraStreamClient, Address, Address, Address) {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    // Mock token client for balance/allowance checks
    let token_client = MockTokenClient::new(&env, &token);
    token_client.mint(&sender, &1_000_000_000_000i128);
    token_client.approve(&sender, &contract_id, &i128::MAX, &999999999);

    client.init(&token, &admin);

    (env, client, sender, recipient, token)
}

/// Helper to create a valid stream with a given rate_per_second.
fn create_stream_with_rate(
    env: &Env,
    client: &FluxoraStreamClient,
    sender: &Address,
    recipient: &Address,
    rate_per_second: i128,
) -> u64 {
    let start_time = env.ledger().timestamp() + 10;
    let cliff_time = start_time;
    let end_time = start_time + 1000;
    let deposit = rate_per_second * 1000; // exactly covers the stream

    client
        .create_stream(
            sender,
            recipient,
            &deposit,
            &rate_per_second,
            &start_time,
            &cliff_time,
            &end_time,
            &0i128, // no dust threshold
            &None,  // no memo
        )
        .unwrap()
}


// Happy path: rate at or above MIN_RATE_PER_SECOND


#[test]
fn test_create_stream_at_min_rate_succeeds() {
    let (env, client, sender, recipient, _token) = setup();
    let rate = 100i128; // exactly MIN_RATE_PER_SECOND
    let stream_id = create_stream_with_rate(&env, &client, &sender, &recipient, rate);
    let stream = client.get_stream_state(&stream_id).unwrap();
    assert_eq!(stream.rate_per_second, rate);
    assert_eq!(stream.status, StreamStatus::Active);
}

#[test]
fn test_create_stream_above_min_rate_succeeds() {
    let (env, client, sender, recipient, _token) = setup();
    let rate = 1_000i128;
    let stream_id = create_stream_with_rate(&env, &client, &sender, &recipient, rate);
    let stream = client.get_stream_state(&stream_id).unwrap();
    assert_eq!(stream.rate_per_second, rate);
}

#[test]
fn test_create_stream_at_large_rate_succeeds() {
    let (env, client, sender, recipient, _token) = setup();
    let rate = 1_000_000i128;
    let stream_id = create_stream_with_rate(&env, &client, &sender, &recipient, rate);
    let stream = client.get_stream_state(&stream_id).unwrap();
    assert_eq!(stream.rate_per_second, rate);
}


// Failure path: rate below MIN_RATE_PER_SECOND


#[test]
fn test_create_stream_at_zero_rate_fails() {
    let (env, client, sender, recipient, _token) = setup();
    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    let result = client.try_create_stream(
        &sender,
        &recipient,
        &1000i128,
        &0i128, // rate = 0
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::RateTooLow)));
}

#[test]
fn test_create_stream_at_one_stroop_fails() {
    let (env, client, sender, recipient, _token) = setup();
    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    let result = client.try_create_stream(
        &sender,
        &recipient,
        &1000i128,
        &1i128, // 1 stroop — below MIN_RATE_PER_SECOND
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::RateTooLow)));
}

#[test]
fn test_create_stream_at_ninety_nine_stroops_fails() {
    let (env, client, sender, recipient, _token) = setup();
    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    let result = client.try_create_stream(
        &sender,
        &recipient,
        &99000i128,
        &99i128, // 99 stroops — just below MIN_RATE_PER_SECOND
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::RateTooLow)));
}

#[test]
fn test_create_stream_at_min_rate_boundary_succeeds() {
    let (env, client, sender, recipient, _token) = setup();
    let rate = 100i128; // exactly at boundary
    let stream_id = create_stream_with_rate(&env, &client, &sender, &recipient, rate);
    assert_eq!(stream_id, 0); // first stream
}


// Batch creation: rate bounds enforced per entry


#[test]
fn test_create_streams_with_mixed_rates_fails_atomically() {
    let (env, client, sender, recipient, _token) = setup();
    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    let streams = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000i128 * 100,
            rate_per_second: 100i128, // valid
            start_time,
            cliff_time: start_time,
            end_time,
            withdraw_dust_threshold: None,
            memo: None,
        },
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000i128,
            rate_per_second: 1i128, // invalid — below MIN_RATE_PER_SECOND
            start_time,
            cliff_time: start_time,
            end_time,
            withdraw_dust_threshold: None,
            memo: None,
        },
    ];

    let result = client.try_create_streams(&sender, &streams);
    assert_eq!(result, Err(Ok(ContractError::RateTooLow)));

    // Verify no streams were created (atomic failure)
    assert_eq!(client.get_stream_count(), 0);
}

#[test]
fn test_create_streams_all_valid_rates_succeeds() {
    let (env, client, sender, recipient, _token) = setup();
    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    let streams = soroban_sdk::vec![
        &env,
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000i128 * 100,
            rate_per_second: 100i128,
            start_time,
            cliff_time: start_time,
            end_time,
            withdraw_dust_threshold: None,
            memo: None,
        },
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000i128 * 200,
            rate_per_second: 200i128,
            start_time,
            cliff_time: start_time,
            end_time,
            withdraw_dust_threshold: None,
            memo: None,
        },
    ];

    let ids = client.create_streams(&sender, &streams);
    assert_eq!(ids.len(), 2);
    assert_eq!(client.get_stream_count(), 2);
}


// Relative time creation: rate bounds enforced


#[test]
fn test_create_stream_relative_below_min_rate_fails() {
    let (env, client, sender, recipient, _token) = setup();

    let params = CreateStreamRelativeParams {
        recipient: recipient.clone(),
        deposit_amount: 1000i128,
        rate_per_second: 50i128, // below MIN_RATE_PER_SECOND
        start_delay: 10,
        cliff_delay: 10,
        duration: 1000,
        withdraw_dust_threshold: None,
        memo: None,
    };

    let result = client.try_create_stream_relative(&sender, &params);
    assert_eq!(result, Err(Ok(ContractError::RateTooLow)));
}

#[test]
fn test_create_stream_relative_at_min_rate_succeeds() {
    let (env, client, sender, recipient, _token) = setup();

    let params = CreateStreamRelativeParams {
        recipient: recipient.clone(),
        deposit_amount: 1000i128 * 100,
        rate_per_second: 100i128, // exactly MIN_RATE_PER_SECOND
        start_delay: 10,
        cliff_delay: 10,
        duration: 1000,
        withdraw_dust_threshold: None,
        memo: None,
    };

    let stream_id = client.create_stream_relative(&sender, &params).unwrap();
    let stream = client.get_stream_state(&stream_id).unwrap();
    assert_eq!(stream.rate_per_second, 100i128);
}


// Template-based creation: rate bounds enforced

#[test]
fn test_create_stream_from_template_below_min_rate_fails() {
    let (env, client, sender, recipient, _token) = setup();

    // Register a template first
    let template_id = client
        .register_stream_template(&sender, &10, &10, &1000)
        .unwrap();

    let result = client.try_create_stream_from_template(
        &sender,
        &template_id,
        &recipient,
        &1000i128,
        &50i128, // below MIN_RATE_PER_SECOND
        &0i128,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::RateTooLow)));
}

// Edge cases

#[test]
fn test_negative_rate_fails_with_rate_too_low() {
    let (env, client, sender, recipient, _token) = setup();
    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    let result = client.try_create_stream(
        &sender,
        &recipient,
        &1000i128,
        &-1i128, // negative rate
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
    );
    // Negative rates are caught by the MIN_RATE_PER_SECOND check
    assert_eq!(result, Err(Ok(ContractError::RateTooLow)));
}

#[test]
fn test_rate_at_i128_max_fails_with_invalid_params() {
    let (env, client, sender, recipient, _token) = setup();
    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    let result = client.try_create_stream(
        &sender,
        &recipient,
        &i128::MAX,
        &i128::MAX, // exceeds any reasonable max_rate
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
    );
    // Should fail with InvalidParams (max rate cap) or ArithmeticOverflow
    assert!(result.is_err());
}

#[test]
fn test_min_rate_with_long_duration_succeeds() {
    let (env, client, sender, recipient, _token) = setup();
    let start_time = env.ledger().timestamp() + 10;
    let duration = 31_536_000u64; // 1 year in seconds
    let end_time = start_time + duration;
    let rate = 100i128; // MIN_RATE_PER_SECOND
    let deposit = rate * (duration as i128);

    let stream_id = client
        .create_stream(
            &sender,
            &recipient,
            &deposit,
            &rate,
            &start_time,
            &start_time,
            &end_time,
            &0i128,
            &None,
        )
        .unwrap();

    let stream = client.get_stream_state(&stream_id).unwrap();
    assert_eq!(stream.rate_per_second, rate);
}

#[test]
fn test_min_rate_preserves_existing_max_rate_cap() {
    let (env, client, sender, recipient, _token) = setup();
    let admin = Address::generate(&env);

    // Set a max rate cap lower than the default
    client.set_max_rate_per_second(&admin, &500i128);

    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    // Rate below MIN_RATE_PER_SECOND should fail with RateTooLow
    let result = client.try_create_stream(
        &sender,
        &recipient,
        &50000i128,
        &50i128,
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::RateTooLow)));

    // Rate above max cap but above min should fail with InvalidParams
    let result = client.try_create_stream(
        &sender,
        &recipient,
        &600000i128,
        &600i128,
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Rate within [MIN, MAX] should succeed
    let result = client.try_create_stream(
        &sender,
        &recipient,
        &300000i128,
        &300i128,
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
    );
    assert!(result.is_ok());
}