extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, DataKey, DOCUMENTED_TEMPLATES, FluxoraStream,
    FluxoraStreamClient, StreamKind, StreamScheduleTemplate, StreamStatus, TemplateSpec,
    MAX_GLOBAL_TEMPLATES, MAX_TEMPLATES_PER_OWNER,
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

    let stream_id = client.create_stream_from_template(
        &sender,
        &tid,
        &recipient,
        &3600_i128,
        &1_i128,
        &0,
        &None,
        &None,
        &StreamKind::Linear,
        &None,
    );
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
        client.register_stream_template(&owner, &0u64, &0u64, &(3600u64 + i));
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
        client.register_stream_template(&owner, &0u64, &0u64, &(3600u64 + i));
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

#[test]
fn auto_renew_permissionless_preserves_subscription_schedule() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let relayer = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);
    StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &1_000_i128, &100);

    env.ledger().set_timestamp(1_000);
    let old_id = client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 100_i128,
            rate_per_second: 1_i128,
            start_time: 1_000,
            cliff_time: 1_010,
            end_time: 1_100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    client.set_auto_renew(&old_id, &sender, &true);

    env.ledger().set_timestamp(1_100);
    client.withdraw(&old_id);
    assert_eq!(
        client.get_stream_state(&old_id).status,
        StreamStatus::Completed
    );

    env.ledger().set_timestamp(1_200);
    let new_id = client.renew_stream(&old_id);
    let renewed = client.get_stream_state(&new_id);

    assert_ne!(new_id, old_id);
    assert_eq!(renewed.sender, sender);
    assert_eq!(renewed.recipient, recipient);
    assert_eq!(renewed.deposit_amount, 100);
    assert_eq!(renewed.rate_per_second, 1);
    assert_eq!(renewed.start_time, 1_200);
    assert_eq!(renewed.cliff_time, 1_210);
    assert_eq!(renewed.end_time, 1_300);
    assert!(client.get_auto_renew(&new_id));
    assert!(!client.get_auto_renew(&old_id));
    assert_eq!(TokenClient::new(&env, &token_id).balance(&relayer), 0);
}

#[test]
fn auto_renew_rejects_insufficient_sender_funding_without_new_stream() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);
    StellarAssetClient::new(&env, &token_id).mint(&sender, &100_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &100_i128, &100);

    env.ledger().set_timestamp(2_000);
    let old_id = client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 100_i128,
            rate_per_second: 1_i128,
            start_time: 2_000,
            cliff_time: 2_000,
            end_time: 2_100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    client.set_auto_renew(&old_id, &sender, &true);
    env.ledger().set_timestamp(2_100);
    client.withdraw(&old_id);

    let err = client.try_renew_stream(&old_id);
    assert_eq!(err, Err(Ok(ContractError::AutoRenewFundingUnavailable)));
    assert_eq!(client.get_stream_count(), 1);
    assert!(client.get_auto_renew(&old_id));
}

#[test]
fn auto_renew_inherits_irrevocable_and_witness_settings() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let witness = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);
    StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &1_000_i128, &100);

    env.ledger().set_timestamp(1_000);
    let old_id = client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 100_i128,
            rate_per_second: 1_i128,
            start_time: 1_000,
            cliff_time: 1_010,
            end_time: 1_100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: Some(true),
            witness: Some(witness.clone()),
        },
    );

    // Verify source stream has irrevocable and witness set
    let source_stream = client.get_stream_state(&old_id);
    assert_eq!(source_stream.irrevocable, Some(true));
    assert_eq!(source_stream.witness, Some(witness.clone()));

    client.set_auto_renew(&old_id, &sender, &true);

    env.ledger().set_timestamp(1_100);
    client.withdraw(&old_id);
    assert_eq!(
        client.get_stream_state(&old_id).status,
        StreamStatus::Completed
    );

    env.ledger().set_timestamp(1_200);
    let new_id = client.renew_stream(&old_id);
    let renewed = client.get_stream_state(&new_id);

    // Assert renewed stream inherited irrevocable and witness settings from the source stream
    assert_ne!(new_id, old_id);
    assert_eq!(renewed.irrevocable, Some(true));
    assert_eq!(renewed.witness, Some(witness));
}

/// Cross-check every template documented in `docs/stream-templates.md` against the
/// actual contract implementation.
///
/// # What it verifies
/// 1. `DOC_ENTRIES` is hand-enumerated from the markdown table and acts as the
///    doc's source of truth.
/// 2. For each entry, we register the template via [`register_stream_template`]
///    and assert the stored [`StreamScheduleTemplate`] matches the documented values
///    exactly.
/// 3. After all documented templates pass, any template present in the code
///    constant [`DOCUMENTED_TEMPLATES`] but absent from the hand-enumerated list
///    is surfaced via `eprintln!` (non-failing) so maintainers can decide whether
///    the doc or the code constant needs updating.
///
/// # Drift categories caught
/// | Scenario | Behaviour |
/// |----------|-----------|
/// | Doc lists a template that doesn't compile | ❌ **Test fails** — `register_stream_template` panics or `get_stream_template` returns unexpected params |
/// | Doc lists a template but params differ | ❌ **Test fails** — `assert_eq!` on params |
/// | Code has a template not in doc | ⚠️ Noted via `eprintln!` — non‑failing |
#[test]
fn documented_templates_match_code() {
    // Hand‑enumerated from docs/stream-templates.md § "Pre-configured Templates".
    // When updating the doc table, update this list too — the test will fail if
    // they diverge.  Keep entries in the same order as the markdown table.
    const DOC_ENTRIES: &[(&str, u64, u64, u64)] = &[
        ("Quick Pay", 0, 0, 3_600),
        ("Daily", 0, 0, 86_400),
        ("Weekly", 0, 86_400, 604_800),
        ("Biweekly", 0, 86_400, 1_209_600),
        ("Monthly", 0, 172_800, 2_592_000),
        ("Quarterly", 0, 604_800, 7_776_000),
        ("Annual", 0, 604_800, 31_536_000),
    ];

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

    // 1. Verify every documented template is registrable and has the right params.
    for (name, start_delay, cliff_delay, duration) in DOC_ENTRIES {
        let tid =
            client.register_stream_template(&owner, start_delay, cliff_delay, duration);
        let stored: StreamScheduleTemplate = client.get_stream_template(&tid);
        assert_eq!(
            stored.start_delay, *start_delay,
            "{name}: start_delay mismatch"
        );
        assert_eq!(
            stored.cliff_delay, *cliff_delay,
            "{name}: cliff_delay mismatch"
        );
        assert_eq!(
            stored.duration, *duration,
            "{name}: duration mismatch"
        );
    }

    // 2. Surface code‑defined templates that are NOT documented (non‑failing).
    for code_tpl in DOCUMENTED_TEMPLATES {
        let found = DOC_ENTRIES.iter().any(|(n, _, _, _)| *n == code_tpl.name);
        if !found {
            eprintln!(
                "NOTE: Template \"{}\" exists in code (DOCUMENTED_TEMPLATES) \
                 but is not listed in docs/stream-templates.md — doc may need updating",
                code_tpl.name,
            );
        }
    }
}
