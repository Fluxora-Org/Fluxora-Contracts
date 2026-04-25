extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, StreamScheduleTemplate,
    MAX_TEMPLATES_PER_OWNER,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

#[test]
fn template_register_create_delete_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let admin = Address::generate(&env);
    let owner = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &10_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    env.ledger().set_timestamp(1_000_000);

    let tid = client.register_stream_template(&owner, &0u64, &0u64, &3600u64);

    let stored: StreamScheduleTemplate = client.get_stream_template(&tid);
    assert_eq!(stored.template_id, tid);
    assert_eq!(stored.owner, owner);
    assert_eq!(stored.start_delay, 0);
    assert_eq!(stored.cliff_delay, 0);
    assert_eq!(stored.duration, 3600);

    let stream_id = client
        .create_stream_from_template(&sender, &tid, &recipient, &3600_i128, &1_i128, &0, &None);
    assert_eq!(stream_id, 0u64);

    client.delete_stream_template(&owner, &tid);
    let err = client.try_get_stream_template(&tid);
    assert_eq!(err, Err(Ok(ContractError::TemplateNotFound)));
}

#[test]
fn delete_template_rejects_wrong_owner() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let admin = Address::generate(&env);
    let owner = Address::generate(&env);
    let other = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    env.ledger().set_timestamp(1_000_000);
    let tid = client.register_stream_template(&owner, &0u64, &60u64, &3600u64);

    let err = client.try_delete_stream_template(&other, &tid);
    assert_eq!(err, Err(Ok(ContractError::TemplateUnauthorized)));
}

#[test]
fn per_owner_template_cap_enforced() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let admin = Address::generate(&env);
    let owner = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);
    env.ledger().set_timestamp(2_000_000);

    for i in 0..MAX_TEMPLATES_PER_OWNER {
        client.register_stream_template(&owner, &0u64, &0u64, &(3600u64 + u64::from(i)));
    }

    let err = client.try_register_stream_template(&owner, &0u64, &0u64, &9999u64);
    assert_eq!(err, Err(Ok(ContractError::TemplateLimitExceeded)));
}

#[test]
fn template_id_monotonic_distinct() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let admin = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);
    env.ledger().set_timestamp(3_000_000);

    let t0 = client.register_stream_template(&a, &0u64, &0u64, &100u64);
    let t1 = client.register_stream_template(&b, &0u64, &0u64, &200u64);
    assert_ne!(t0, t1);
}
