#![cfg(test)]

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient, StreamConfig};
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
    // register_stellar_asset_contract_v2 returns a StellarAssetContract helper struct
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_address = sac.address(); // Extract the Address for use in clients

    let token_client = TokenClient::new(&env, &token_address);
    let stellar_asset_client = StellarAssetClient::new(&env, &token_address);
    stellar_asset_client.mint(&sender, &100_000);

    // Deploy Stream Contract
    let stream_id = env.register_contract(None, fluxora_stream::FluxoraStream {});
    let stream_client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_id);

    // Initialize stream contract with proper Address types
    stream_client.init(&token_address, &admin);

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

    let created_id = factory.create_stream(&StreamConfig {
        sender: sender.clone(),
        recipient: recipient.clone(),
        deposit_amount,
        rate_per_second,
        start_time,
        cliff_time,
        end_time,
    });

    assert_eq!(created_id, 0);
    assert_eq!(stream_client.get_stream_count(), 1);

    // The stream contract should have the tokens now
    assert_eq!(token_client.balance(&stream_id), 1_000);
}

#[test]
fn test_factory_enforces_allowlist() {
    let (_env, factory, _admin, sender, _recipient, unauthorized, _, _) = setup_env();

    let res = factory.try_create_stream(&StreamConfig {
        sender: sender.clone(),
        recipient: unauthorized.clone(),
        deposit_amount: 1_000,
        rate_per_second: 1,
        start_time: 100,
        cliff_time: 100,
        end_time: 1100,
    });
    assert_eq!(res, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

#[test]
fn test_factory_enforces_max_deposit_cap() {
    let (_env, factory, _admin, sender, recipient, _, _, _) = setup_env();

    let res = factory.try_create_stream(&StreamConfig {
        sender: sender.clone(),
        recipient: recipient.clone(),
        deposit_amount: 20_000, // max is 10_000
        rate_per_second: 20,
        start_time: 100,
        cliff_time: 100,
        end_time: 1100,
    });
    assert_eq!(res, Err(Ok(FactoryError::DepositExceedsCap)));
}

#[test]
fn test_factory_enforces_min_duration() {
    let (_env, factory, _admin, sender, recipient, _, _, _) = setup_env();

    let res = factory.try_create_stream(&StreamConfig {
        sender: sender.clone(),
        recipient: recipient.clone(),
        deposit_amount: 1_000,
        rate_per_second: 10,
        start_time: 100,
        cliff_time: 100,
        end_time: 200, // duration 100 < min 500
    });
    assert_eq!(res, Err(Ok(FactoryError::DurationTooShort)));
}

#[test]
fn test_factory_admin_updates() {
    let (env, factory, _admin, sender, recipient, unauthorized, _, _) = setup_env();

    // Update allowlist
    factory.set_allowlist(&unauthorized, &true);
    let id1 = factory.create_stream(&StreamConfig {
        sender: sender.clone(),
        recipient: unauthorized.clone(),
        deposit_amount: 1_000,
        rate_per_second: 1,
        start_time: 100,
        cliff_time: 100,
        end_time: 1100,
    });
    assert_eq!(id1, 0);

    // Update Cap
    factory.set_cap(&500);
    let res1 = factory.try_create_stream(&StreamConfig {
        sender: sender.clone(),
        recipient: recipient.clone(),
        deposit_amount: 1_000,
        rate_per_second: 1,
        start_time: 100,
        cliff_time: 100,
        end_time: 1100,
    });
    assert_eq!(res1, Err(Ok(FactoryError::DepositExceedsCap)));

    // Update Min Duration
    factory.set_min_duration(&2000);
    let res2 = factory.try_create_stream(&StreamConfig {
        sender: sender.clone(),
        recipient: recipient.clone(),
        deposit_amount: 500,
        rate_per_second: 1,
        start_time: 100,
        cliff_time: 100,
        end_time: 1100,
    });
    assert_eq!(res2, Err(Ok(FactoryError::DurationTooShort)));

    // Update Stream Contract
    let dummy = Address::generate(&env);
    factory.set_stream_contract(&dummy);

    // Update Admin
    let new_admin = Address::generate(&env);
    factory.set_admin(&new_admin);
}
