extern crate std;

    ContractError, DataKey, FluxoraStream, FluxoraStreamClient, StreamScheduleTemplate,
    MAX_GLOBAL_TEMPLATES, MAX_TEMPLATES_PER_OWNER, StreamKind,
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
        .create_stream_from_template(&sender, &tid, &recipient, &3600_i128, &1_i128, &0, &None, &None, &StreamKind::Linear);
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

/// Registering a 65th template for the same owner returns TemplateLimitExceeded.
///
/// Exercises the `ids.len() >= MAX_TEMPLATES_PER_OWNER` branch in
/// `register_stream_template` (lib.rs owner-cap guard).
#[test]
fn test_owner_template_cap_exceeded() {
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
    env.ledger().set_timestamp(1_000_000);

    // Register exactly MAX_TEMPLATES_PER_OWNER (64) templates.
    for i in 0..MAX_TEMPLATES_PER_OWNER {
        client.register_stream_template(&owner, &0u64, &0u64, &(3600u64 + u64::from(i)));
    }

    // The 65th registration must fail.
    let err = client.try_register_stream_template(&owner, &0u64, &0u64, &9999u64);
    assert_eq!(err, Err(Ok(ContractError::TemplateLimitExceeded)));

    // After deleting one template the owner can register again.
    let first_tid = client.get_stream_template(&0u64).template_id;
    client.delete_stream_template(&owner, &first_tid);
    let new_tid = client.register_stream_template(&owner, &0u64, &0u64, &9999u64);
    assert!(client.try_get_stream_template(&new_tid).is_ok());
}

/// Filling the global 10 000-template cap returns TemplateLimitExceeded on the next call.
///
/// Exercises the `active >= MAX_GLOBAL_TEMPLATES` branch in `register_stream_template`
/// (lib.rs global-cap guard).  Rather than creating 10 000 templates (which would exhaust
/// the Soroban test-environment budget), we seed `ActiveTemplateCount` directly in instance
/// storage to `MAX_GLOBAL_TEMPLATES - 1`, register one more to reach the cap, then assert
/// the next registration fails.  After deleting the last template the global slot is freed
/// and registration succeeds again.
#[test]
fn test_global_template_cap_exceeded() {
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

    // Seed the active template count to MAX_GLOBAL_TEMPLATES - 1 so the next
    // registration fills the cap without requiring 9 999 actual contract calls.
    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&DataKey::ActiveTemplateCount, &(MAX_GLOBAL_TEMPLATES - 1));
    });

    // Register the final allowed template (fills the cap).
    let last_tid = client.register_stream_template(&owner, &0u64, &0u64, &3600u64);

    // Global cap is now full — next registration must fail.
    let new_owner = Address::generate(&env);
    let err = client.try_register_stream_template(&new_owner, &0u64, &0u64, &9999u64);
    assert_eq!(err, Err(Ok(ContractError::TemplateLimitExceeded)));

    // Deleting the last template frees a global slot; registration succeeds again.
    client.delete_stream_template(&owner, &last_tid);
    let recovered_tid = client.register_stream_template(&new_owner, &0u64, &0u64, &9999u64);
    assert!(client.try_get_stream_template(&recovered_tid).is_ok());
}
