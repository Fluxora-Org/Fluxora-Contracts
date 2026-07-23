use fluxora_stream::{
    ContractError, CreateStreamParams, CreateStreamRelativeParams, FluxoraStream,
    FluxoraStreamClient, StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    StellarAssetClient::new(&env, &token).mint(&sender, &1_000_000_000_000i128);
    TokenClient::new(&env, &token).approve(&sender, &contract_id, &i128::MAX, &1_000_000u32);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token, &admin);

    (env, contract_id, sender, recipient, token)
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
            &StreamKind::Linear,
        )
}

// Happy path: rate above 0

#[test]
fn test_create_stream_at_one_stroop_succeeds() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let rate = 1i128;
    let stream_id = create_stream_with_rate(&env, &client, &sender, &recipient, rate);
    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.rate_per_second, rate);
    assert_eq!(stream.status, StreamStatus::Active);
}

#[test]
fn test_create_stream_above_min_rate_succeeds() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let rate = 1_000i128;
    let stream_id = create_stream_with_rate(&env, &client, &sender, &recipient, rate);
    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.rate_per_second, rate);
}

#[test]
fn test_create_stream_at_large_rate_succeeds() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let rate = 1_000_000i128;
    let stream_id = create_stream_with_rate(&env, &client, &sender, &recipient, rate);
    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.rate_per_second, rate);
}

// Failure path: rate at or below 0

#[test]
fn test_create_stream_at_zero_rate_fails() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
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
        &StreamKind::Linear,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

// Batch creation: rate bounds enforced per entry

#[test]
fn test_create_streams_with_mixed_rates_fails_atomically() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
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
            withdraw_dust_threshold: Some(0i128),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
        },
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000i128,
            rate_per_second: 0i128, // invalid — rate <= 0
            start_time,
            cliff_time: start_time,
            end_time,
            withdraw_dust_threshold: Some(0i128),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
        },
    ];

    let result = client.try_create_streams(&sender, &streams);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Verify no streams were created (atomic failure)
    assert_eq!(client.get_stream_count(), 0);
}

#[test]
fn test_create_streams_all_valid_rates_succeeds() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
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
            withdraw_dust_threshold: Some(0i128),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
        },
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000i128 * 200,
            rate_per_second: 200i128,
            start_time,
            cliff_time: start_time,
            end_time,
            withdraw_dust_threshold: Some(0i128),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
        },
    ];

    let ids = client.create_streams(&sender, &streams);
    assert_eq!(ids.len(), 2);
    assert_eq!(client.get_stream_count(), 2);
}

// Relative time creation: rate bounds enforced

#[test]
fn test_create_stream_relative_below_min_rate_fails() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let params = CreateStreamRelativeParams {
        recipient: recipient.clone(),
        deposit_amount: 1000i128,
        rate_per_second: 0i128, // invalid
        start_delay: 10,
        cliff_delay: 10,
        duration: 1000,
        withdraw_dust_threshold: Some(0i128),
        memo: None,
        kind: StreamKind::Linear,
        metadata: None,
    };

    let result = client.try_create_stream_relative(&sender, &params);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_create_stream_relative_at_min_rate_succeeds() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let params = CreateStreamRelativeParams {
        recipient: recipient.clone(),
        deposit_amount: 1000i128 * 100,
        rate_per_second: 100i128,
        start_delay: 10,
        cliff_delay: 10,
        duration: 1000,
        withdraw_dust_threshold: Some(0i128),
        memo: None,
        kind: StreamKind::Linear,
        metadata: None,
    };

    let stream_id = client.create_stream_relative(&sender, &params);
    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.rate_per_second, 100i128);
}

// Template-based creation: rate bounds enforced

#[test]
fn test_create_stream_from_template_below_min_rate_fails() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);

    // Register a template first
    let template_id = client
        .register_stream_template(&sender, &10, &10, &1000);

    let result = client.try_create_stream_from_template(
        &sender,
        &template_id,
        &recipient,
        &1000i128,
        &0i128, // invalid rate
        &0i128,
        &None,
        &None,
        &StreamKind::Linear,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

// Edge cases

#[test]
fn test_negative_rate_fails_with_invalid_params() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
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
        &StreamKind::Linear,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_rate_at_i128_max_fails_with_invalid_params() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
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
        &StreamKind::Linear,
    );
    assert!(result.is_err());
}

#[test]
fn test_min_rate_with_long_duration_succeeds() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let start_time = env.ledger().timestamp() + 10;
    let duration = 31_536_000u64; // 1 year in seconds
    let end_time = start_time + duration;
    let rate = 100i128;
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
            &StreamKind::Linear,
        );

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.rate_per_second, rate);
}

#[test]
fn test_min_rate_preserves_existing_max_rate_cap() {
    let (env, contract_id, sender, recipient, _token) = setup();
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    // Set a max rate cap lower than the default
    client.set_max_rate_per_second(&500i128);

    let start_time = env.ledger().timestamp() + 10;
    let end_time = start_time + 1000;

    // Rate below 0 should fail with InvalidParams
    let result = client.try_create_stream(
        &sender,
        &recipient,
        &50000i128,
        &0i128,
        &start_time,
        &start_time,
        &end_time,
        &0i128,
        &None,
        &StreamKind::Linear,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Rate above max cap should fail with InvalidParams
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
        &StreamKind::Linear,
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
        &StreamKind::Linear,
    );
    assert!(result.is_ok());
}
