#![cfg(test)]

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

fn setup_env<'a>() -> (
    Env,
    FluxoraFactoryClient<'a>,
    Address,
    Address,
    Address,
    Address,
    Address,
    TokenClient<'a>,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 100);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let unauthorized_recipient = Address::generate(&env);

    // Deploy native token
    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(token_admin.clone());
    let token_client = TokenClient::new(&env, &token_id);
    let stellar_asset_client = StellarAssetClient::new(&env, &token_id);
    stellar_asset_client.mint(&sender, &100_000);

    // Deploy Stream Contract
    let stream_id = env.register_contract(None, fluxora_stream::FluxoraStream {});
    let stream_client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_id);
    stream_client.init(&token_id, &admin);

    // Deploy Factory
    let factory_id = env.register_contract(None, FluxoraFactory {});
    let factory_client = FluxoraFactoryClient::new(&env, &factory_id);

    // Initialize Factory
    let max_deposit: i128 = 10_000;
    let min_duration: u64 = 500;
    factory_client.init(&admin, &stream_id, &max_deposit, &min_duration);

    // Add valid recipient to allowlist
    factory_client.set_allowlist(&recipient, &true);

    (
        env,
        factory_client,
        admin,
        sender,
        recipient,
        unauthorized_recipient,
        stream_id,
        token_client,
    )
}

#[test]
fn test_factory_create_stream_success() {
    let (env, factory, _admin, sender, recipient, _, stream_id, token_client) = setup_env();

    let deposit_amount: i128 = 1_000;
    let rate_per_second: i128 = 1;
    let start_time: u64 = 100;
    let cliff_time: u64 = 100;
    let end_time: u64 = 1100; // Duration 1000 >= min_duration(500)

    let stream_client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_id);
    assert_eq!(stream_client.get_stream_count(), 0);

    let created_id = factory.create_stream(
        &sender,
        &recipient,
        &deposit_amount,
        &rate_per_second,
        &start_time,
        &cliff_time,
        &end_time,
        &0,
    );

    assert_eq!(created_id, 0);
    assert_eq!(stream_client.get_stream_count(), 1);

    // The stream contract should have the tokens now
    assert_eq!(token_client.balance(&stream_id), 1_000);
}

#[test]
fn test_factory_enforces_allowlist() {
    let (_env, factory, _admin, sender, recipient, unauthorized, _, _) = setup_env();

    let res =
        factory.try_create_stream(&sender, &recipient, &1_000, &1, &100, &100, &1100, &0_i128);
    assert_eq!(res, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

#[test]
fn test_factory_enforces_max_deposit_cap() {
    let (_env, factory, _admin, sender, recipient, _, _, _) = setup_env();

    let res = factory.try_create_stream(
        &sender, &recipient, &20_000, // max is 10_000
        &20, &100, &100, &1100, &0,
    );
    assert_eq!(res, Err(Ok(FactoryError::DepositExceedsCap)));
}

#[test]
fn test_factory_enforces_min_duration() {
    let (_env, factory, _admin, sender, recipient, _, _, _) = setup_env();

    let res = factory.try_create_stream(
        &sender, &recipient, &1_000, &10, &100, &100, &200, &0, // duration 100 < min 500
    );
    assert_eq!(res, Err(Ok(FactoryError::DurationTooShort)));
}

#[test]
fn test_factory_admin_updates() {
    let (env, factory, _admin, sender, recipient, unauthorized, _, _) = setup_env();

    // Update allowlist
    factory.set_allowlist(&unauthorized, &true);
    let id1 = factory.create_stream(
        &sender,
        &unauthorized,
        &1_000,
        &1,
        &100,
        &100,
        &1100,
        &0_i128,
    );
    assert_eq!(id1, 0);

    // Update Cap
    factory.set_cap(&500);
    let res1 =
        factory.try_create_stream(&sender, &recipient, &1_000, &1, &100, &100, &1100, &0_i128);
    assert_eq!(res1, Err(Ok(FactoryError::DepositExceedsCap)));

    // Update Min Duration
    factory.set_min_duration(&2000);
    let res2 = factory.try_create_stream(&sender, &recipient, &500, &1, &100, &100, &1100, &0_i128);
    assert_eq!(res2, Err(Ok(FactoryError::DurationTooShort)));

    // Update Stream Contract
    let dummy = Address::generate(&env);
    factory.set_stream_contract(&dummy);
    // Note: Calling create_stream now would fail because `dummy` is not a stream contract.

    // Update Admin
    let new_admin = Address::generate(&env);
    factory.set_admin(&new_admin);
}
