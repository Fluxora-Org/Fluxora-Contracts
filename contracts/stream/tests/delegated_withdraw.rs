#![cfg(test)]

use fluxora_stream::{ContractError, CreateStreamParams, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, BytesN, Env,
};

fn setup() -> (Env, FluxoraStreamClient<'static>, u64, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, fluxora_stream::FluxoraStream {});
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let relayer = Address::generate(&env);

    client.init(&token_id, &admin);

    let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &10_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    env.ledger().set_timestamp(0);
    let stream_id = client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    (env, client, stream_id, recipient, relayer)
}

#[test]
fn test_delegated_withdraw_with_relayer_fee_success() {
    let (env, client, stream_id, recipient, relayer) = setup();
    env.ledger().set_timestamp(500);

    // Get recipient's public key
    let pubkey = recipient.public_key();

    // Build signature payload
    let nonce = client.get_delegated_nonce(&recipient);
    let deadline = 1000u64;
    let expected_minimum_amount = 400i128;
    let relayer_fee = 50i128;

    // Build message
    let mut msg = soroban_sdk::Bytes::new(&env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());

    // Sign with recipient's private key
    let signature = env
        .crypto()
        .ed25519_sign(&pubkey, &msg, &recipient.private_key());

    // Get initial balances
    let token_id = client.get_config().unwrap().token;
    let token = TokenClient::new(&env, &token_id);
    let initial_recipient_balance = token.balance(&recipient);
    let initial_relayer_balance = token.balance(&relayer);

    // Execute delegated withdraw
    let amount = client.delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    // Verify amounts
    assert_eq!(amount, 450); // 500 - 50
    assert_eq!(
        token.balance(&recipient),
        initial_recipient_balance + 450
    );
    assert_eq!(token.balance(&relayer), initial_relayer_balance + 50);
}

#[test]
fn test_delegated_withdraw_fee_too_high_fails() {
    let (env, client, stream_id, recipient, relayer) = setup();
    env.ledger().set_timestamp(500);

    let pubkey = recipient.public_key();
    let nonce = client.get_delegated_nonce(&recipient);
    let deadline = 1000u64;
    let expected_minimum_amount = 0i128;
    let relayer_fee = 600i128; // More than withdrawable (500)

    let mut msg = soroban_sdk::Bytes::new(&env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());

    let signature = env
        .crypto()
        .ed25519_sign(&pubkey, &msg, &recipient.private_key());

    let result = client.try_delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    assert_eq!(result.unwrap_err(), ContractError::InsufficientBalance);
}

#[test]
fn test_delegated_withdraw_net_below_minimum_fails() {
    let (env, client, stream_id, recipient, relayer) = setup();
    env.ledger().set_timestamp(500);

    let pubkey = recipient.public_key();
    let nonce = client.get_delegated_nonce(&recipient);
    let deadline = 1000u64;
    let expected_minimum_amount = 500i128; // Expecting 500 but net is 450
    let relayer_fee = 50i128;

    let mut msg = soroban_sdk::Bytes::new(&env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());

    let signature = env
        .crypto()
        .ed25519_sign(&pubkey, &msg, &recipient.private_key());

    let result = client.try_delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    assert_eq!(result.unwrap_err(), ContractError::BelowMinimumAmount);
}

#[test]
fn test_delegated_withdraw_zero_fee_works() {
    let (env, client, stream_id, recipient, relayer) = setup();
    env.ledger().set_timestamp(500);

    let pubkey = recipient.public_key();
    let nonce = client.get_delegated_nonce(&recipient);
    let deadline = 1000u64;
    let expected_minimum_amount = 0i128;
    let relayer_fee = 0i128;

    let mut msg = soroban_sdk::Bytes::new(&env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());

    let signature = env
        .crypto()
        .ed25519_sign(&pubkey, &msg, &recipient.private_key());

    let token_id = client.get_config().unwrap().token;
    let token = TokenClient::new(&env, &token_id);
    let initial_recipient_balance = token.balance(&recipient);
    let initial_relayer_balance = token.balance(&relayer);

    let amount = client.delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    assert_eq!(amount, 500);
    assert_eq!(
        token.balance(&recipient),
        initial_recipient_balance + 500
    );
    assert_eq!(token.balance(&relayer), initial_relayer_balance);
}

#[test]
fn test_delegated_withdraw_negative_fee_fails() {
    let (env, client, stream_id, recipient, relayer) = setup();
    env.ledger().set_timestamp(500);

    let pubkey = recipient.public_key();
    let nonce = client.get_delegated_nonce(&recipient);
    let deadline = 1000u64;
    let expected_minimum_amount = 0i128;
    let relayer_fee = -50i128;

    let mut msg = soroban_sdk::Bytes::new(&env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());

    let signature = env
        .crypto()
        .ed25519_sign(&pubkey, &msg, &recipient.private_key());

    let result = client.try_delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    assert_eq!(result.unwrap_err(), ContractError::InvalidParams);
}

#[test]
fn test_delegated_withdraw_nonce_replay_protection() {
    let (env, client, stream_id, recipient, relayer) = setup();
    env.ledger().set_timestamp(500);

    let pubkey = recipient.public_key();
    let nonce = client.get_delegated_nonce(&recipient);
    let deadline = 1000u64;
    let expected_minimum_amount = 0i128;
    let relayer_fee = 10i128;

    let mut msg = soroban_sdk::Bytes::new(&env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());

    let signature = env
        .crypto()
        .ed25519_sign(&pubkey, &msg, &recipient.private_key());

    // First withdrawal succeeds
    client.delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    // Second withdrawal with same nonce fails
    env.ledger().set_timestamp(600);
    let result = client.try_delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    assert_eq!(result.unwrap_err(), ContractError::InvalidSignature);
}

#[test]
fn test_delegated_withdraw_deadline_expired_fails() {
    let (env, client, stream_id, recipient, relayer) = setup();
    env.ledger().set_timestamp(500);

    let pubkey = recipient.public_key();
    let nonce = client.get_delegated_nonce(&recipient);
    let deadline = 400u64; // Expired
    let expected_minimum_amount = 0i128;
    let relayer_fee = 10i128;

    let mut msg = soroban_sdk::Bytes::new(&env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());

    let signature = env
        .crypto()
        .ed25519_sign(&pubkey, &msg, &recipient.private_key());

    let result = client.try_delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    assert_eq!(result.unwrap_err(), ContractError::SignatureDeadlineExpired);
}

#[test]
fn test_delegated_withdraw_wrong_public_key_fails() {
    let (env, client, stream_id, recipient, relayer) = setup();
    env.ledger().set_timestamp(500);

    // Use a different public key
    let wrong_recipient = Address::generate(&env);
    let pubkey = wrong_recipient.public_key();
    let nonce = client.get_delegated_nonce(&recipient);
    let deadline = 1000u64;
    let expected_minimum_amount = 0i128;
    let relayer_fee = 10i128;

    let mut msg = soroban_sdk::Bytes::new(&env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg.extend_from_array(&relayer_fee.to_be_bytes());

    // Sign with wrong private key
    let signature = env
        .crypto()
        .ed25519_sign(&pubkey, &msg, &wrong_recipient.private_key());

    let result = client.try_delegated_withdraw(
        &stream_id,
        &relayer,
        &BytesN::from_array(&env, &pubkey),
        &nonce,
        &deadline,
        &expected_minimum_amount,
        &relayer_fee,
        &BytesN::from_array(&env, &signature),
    );

    assert_eq!(result.unwrap_err(), ContractError::InvalidSignature);
}
