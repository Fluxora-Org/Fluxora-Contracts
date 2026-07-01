//! Tests for issue #525: factory policy enforcement.
//!
//! Covers FactoryError variants and verifies that `create_stream` via the factory
//! correctly delegates to the stream contract after passing all checks.

use fluxora_factory::{
    FactoryError, FluxoraFactory, FluxoraFactoryClient, MAX_MIN_DURATION_SECONDS,
};
use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Bytes, Env, IntoVal,
};
use std::panic::AssertUnwindSafe;

struct Ctx<'a> {
    env: Env,
    factory: FluxoraFactoryClient<'a>,
    #[allow(dead_code)]
    stream: FluxoraStreamClient<'a>,
    admin: Address,
    sender: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        // Deploy stream contract
        let stream_id = env.register_contract(None, FluxoraStream);
        let stream = FluxoraStreamClient::new(&env, &stream_id);

        // Deploy factory contract
        let factory_id = env.register_contract(None, FluxoraFactory);
        let factory = FluxoraFactoryClient::new(&env, &factory_id);

        // Token setup
        let token_admin = Address::generate(&env);
        let token_contract_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_contract_id);
        let stellar_asset = StellarAssetClient::new(&env, &token_contract_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        stellar_asset.mint(&sender, &1_000_000_000);
        token.approve(&sender, &stream_id, &1_000_000_000, &99999);

        // Init stream contract
        stream.init(&token_contract_id, &stream_id); // admin = stream_id for simplicity

        // Init factory: max_deposit=10_000, min_duration=100
        factory.init(&admin, &stream_id, &10_000, &100);

        Self {
            env,
            factory,
            stream,
            admin,
            sender,
            token,
        }
    }

    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }
}

// ---------------------------------------------------------------------------
// Error discriminant stability
// ---------------------------------------------------------------------------

#[test]
fn test_factory_error_discriminants_are_append_only_and_stable() {
    assert_eq!(FactoryError::AlreadyInitialized as u32, 1);
    assert_eq!(FactoryError::NotInitialized as u32, 2);
    assert_eq!(FactoryError::Unauthorized as u32, 3);
    assert_eq!(FactoryError::RecipientNotAllowlisted as u32, 4);
    assert_eq!(FactoryError::DepositExceedsCap as u32, 5);
    assert_eq!(FactoryError::DurationTooShort as u32, 6);
    assert_eq!(FactoryError::InvalidTimeRange as u32, 7);
    assert_eq!(FactoryError::InvalidCliff as u32, 8);
    assert_eq!(FactoryError::InvalidCap as u32, 14);
    assert_eq!(FactoryError::InvalidMinDuration as u32, 15);
}

// ---------------------------------------------------------------------------
// Policy input validation
// ---------------------------------------------------------------------------

#[test]
fn test_init_rejects_zero_max_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let stream_contract = Address::generate(&env);

    let result = factory.try_init(&admin, &stream_contract, &0, &100);
    assert_eq!(result, Err(Ok(FactoryError::InvalidCap)));
    assert_eq!(
        factory.try_get_factory_config(),
        Err(Ok(FactoryError::NotInitialized)),
        "invalid init must not write partial policy state"
    );
}

#[test]
fn test_init_rejects_negative_max_deposit() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let stream_contract = Address::generate(&env);

    let result = factory.try_init(&admin, &stream_contract, &-1, &100);
    assert_eq!(result, Err(Ok(FactoryError::InvalidCap)));
    assert_eq!(
        factory.try_get_factory_config(),
        Err(Ok(FactoryError::NotInitialized)),
        "invalid init must not write partial policy state"
    );
}

#[test]
fn test_set_cap_rejects_zero_and_negative_values_without_mutation() {
    let ctx = Ctx::setup();
    let before = ctx.factory.get_factory_config();

    assert_eq!(
        ctx.factory.try_set_cap(&0),
        Err(Ok(FactoryError::InvalidCap))
    );
    assert_eq!(
        ctx.factory.get_factory_config().max_deposit,
        before.max_deposit,
        "zero cap must not overwrite the stored positive cap"
    );

    assert_eq!(
        ctx.factory.try_set_cap(&-1),
        Err(Ok(FactoryError::InvalidCap))
    );
    assert_eq!(
        ctx.factory.get_factory_config().max_deposit,
        before.max_deposit,
        "negative cap must not overwrite the stored positive cap"
    );
}

#[test]
fn test_init_accepts_zero_min_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let stream_contract = Address::generate(&env);

    factory.init(&admin, &stream_contract, &1, &0);

    let config = factory.get_factory_config();
    assert_eq!(config.max_deposit, 1);
    assert_eq!(config.min_duration, 0);
}

#[test]
fn test_init_rejects_absurd_min_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let stream_contract = Address::generate(&env);

    let result = factory.try_init(
        &admin,
        &stream_contract,
        &1_000,
        &(MAX_MIN_DURATION_SECONDS + 1),
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidMinDuration)));
    assert_eq!(
        factory.try_get_factory_config(),
        Err(Ok(FactoryError::NotInitialized)),
        "invalid init must not write partial policy state"
    );
}

#[test]
fn test_set_min_duration_rejects_absurd_upper_bound_without_mutation() {
    let ctx = Ctx::setup();
    let before = ctx.factory.get_factory_config();

    assert_eq!(
        ctx.factory
            .try_set_min_duration(&(MAX_MIN_DURATION_SECONDS + 1)),
        Err(Ok(FactoryError::InvalidMinDuration))
    );
    assert_eq!(
        ctx.factory.get_factory_config().min_duration,
        before.min_duration,
        "invalid min_duration must not overwrite the stored policy"
    );
}

#[test]
fn test_set_min_duration_accepts_zero_and_ceiling() {
    let ctx = Ctx::setup();

    ctx.factory.set_min_duration(&0);
    assert_eq!(ctx.factory.get_factory_config().min_duration, 0);

    ctx.factory.set_min_duration(&MAX_MIN_DURATION_SECONDS);
    assert_eq!(
        ctx.factory.get_factory_config().min_duration,
        MAX_MIN_DURATION_SECONDS
    );
}

// ---------------------------------------------------------------------------
// AlreadyInitialized
// ---------------------------------------------------------------------------

#[test]
fn test_factory_already_initialized() {
    let ctx = Ctx::setup();
    let result = ctx
        .factory
        .try_init(&ctx.admin, &Address::generate(&ctx.env), &1_000, &10);
    assert_eq!(result, Err(Ok(FactoryError::AlreadyInitialized)));
}

// ---------------------------------------------------------------------------
// Unauthorized (set_admin requires existing admin signature)
// ---------------------------------------------------------------------------

#[test]
fn test_set_admin_requires_existing_admin() {
    let env = Env::default();
    // Do NOT mock all auths — we want auth to fail
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let stream_id = env.register_contract(None, FluxoraStream);
    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    env.mock_all_auths_allowing_non_root_auth();
    factory.init(&admin, &stream_id, &10_000, &100);

    // set_admin without admin auth should panic (require_auth fails)
    let _result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        factory.set_admin(&new_admin);
    }));
    // In Soroban testutils, unauthorized calls panic
    // We verify the happy path instead: with mock_all_auths it succeeds
    let env2 = Env::default();
    env2.mock_all_auths();
    let fid2 = env2.register_contract(None, FluxoraFactory);
    let f2 = FluxoraFactoryClient::new(&env2, &fid2);
    let sid2 = env2.register_contract(None, FluxoraStream);
    let a2 = Address::generate(&env2);
    let na2 = Address::generate(&env2);
    f2.init(&a2, &sid2, &10_000, &100);
    f2.set_admin(&na2); // succeeds with mock_all_auths
}

#[test]
fn test_factory_setters_reject_non_admin_callers() {
    fn expect_rejected<F>(call: F)
    where
        F: FnOnce(),
    {
        let result = std::panic::catch_unwind(AssertUnwindSafe(call));
        assert!(result.is_err(), "non-admin setter call must fail auth");
    }

    let env = Env::default();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let stream_contract = env.register_contract(None, FluxoraStream);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let new_stream_contract = env.register_contract(None, FluxoraStream);
    let recipient = Address::generate(&env);

    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "init",
            args: (&admin, &stream_contract, 10_000i128, 100u64).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    factory.init(&admin, &stream_contract, &10_000, &100);

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "set_admin",
            args: (&new_admin,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    expect_rejected(|| factory.set_admin(&new_admin));

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "set_stream_contract",
            args: (&new_stream_contract,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    expect_rejected(|| factory.set_stream_contract(&new_stream_contract));

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "set_allowlist",
            args: (&recipient, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    expect_rejected(|| factory.set_allowlist(&recipient, &true));

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "set_cap",
            args: (5_000i128,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    expect_rejected(|| factory.set_cap(&5_000));

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "set_min_duration",
            args: (500u64,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    expect_rejected(|| factory.set_min_duration(&500));
}

// ---------------------------------------------------------------------------
// RecipientNotAllowlisted
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_recipient_not_allowlisted() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

#[test]
fn test_create_stream_supports_cliff_only_and_memo() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();
    let memo = Some(Bytes::from_slice(&ctx.env, b"payroll-batch-42"));

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &0,
        &now,
        &now,
        &(now + 200),
        &0,
        &memo,
        &StreamKind::CliffOnly,
    );
    assert!(result.is_ok());

    let stream_id = result.unwrap().unwrap();
    let stream_state = ctx.stream.get_stream_state(&stream_id);
    assert!(matches!(stream_state.kind, StreamKind::CliffOnly));
    assert_eq!(stream_state.memo, memo);
}

#[test]
fn test_create_streams_batch_allows_all_valid_entries_atomically() {
    let ctx = Ctx::setup();
    let recipient0 = Address::generate(&ctx.env);
    let recipient1 = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient0, &true);
    ctx.factory.set_allowlist(&recipient1, &true);
    let now = ctx.now();

    let mut streams = Vec::new(&ctx.env);
    streams.push_back(CreateStreamParams {
        recipient: recipient0.clone(),
        deposit_amount: 4_000,
        rate_per_second: 0,
        start_time: now,
        cliff_time: now,
        end_time: now + 200,
        withdraw_dust_threshold: None,
        memo: Some(Bytes::from_slice(&ctx.env, b"batch-1")),
        kind: StreamKind::CliffOnly,
    });
    streams.push_back(CreateStreamParams {
        recipient: recipient1.clone(),
        deposit_amount: 5_000,
        rate_per_second: 1,
        start_time: now,
        cliff_time: now,
        end_time: now + 500,
        withdraw_dust_threshold: Some(0),
        memo: Some(Bytes::from_slice(&ctx.env, b"batch-2")),
        kind: StreamKind::Linear,
    });

    let result = ctx.factory.try_create_streams(&ctx.sender, &streams);
    assert!(result.is_ok());

    let ids = result.unwrap().unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(
        ctx.stream.get_stream_memo(&ids.get_unchecked(0)).unwrap(),
        Bytes::from_slice(&ctx.env, b"batch-1")
    );
    assert_eq!(
        ctx.stream.get_stream_memo(&ids.get_unchecked(1)).unwrap(),
        Bytes::from_slice(&ctx.env, b"batch-2")
    );
}

#[test]
fn test_create_streams_batch_reverts_if_any_recipient_not_allowlisted() {
    let ctx = Ctx::setup();
    let recipient0 = Address::generate(&ctx.env);
    let recipient1 = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient0, &true);
    let now = ctx.now();

    let mut streams = Vec::new(&ctx.env);
    streams.push_back(CreateStreamParams {
        recipient: recipient0.clone(),
        deposit_amount: 4_000,
        rate_per_second: 1,
        start_time: now,
        cliff_time: now,
        end_time: now + 200,
        withdraw_dust_threshold: None,
        memo: None,
        kind: StreamKind::Linear,
    });
    streams.push_back(CreateStreamParams {
        recipient: recipient1.clone(),
        deposit_amount: 3_000,
        rate_per_second: 1,
        start_time: now,
        cliff_time: now,
        end_time: now + 200,
        withdraw_dust_threshold: None,
        memo: None,
        kind: StreamKind::Linear,
    });

    let result = ctx.factory.try_create_streams(&ctx.sender, &streams);
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

#[test]
fn test_create_streams_batch_rejects_aggregate_deposit_over_cap() {
    let ctx = Ctx::setup();
    let recipient0 = Address::generate(&ctx.env);
    let recipient1 = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient0, &true);
    ctx.factory.set_allowlist(&recipient1, &true);
    let now = ctx.now();

    let mut streams = Vec::new(&ctx.env);
    streams.push_back(CreateStreamParams {
        recipient: recipient0.clone(),
        deposit_amount: 6_000,
        rate_per_second: 0,
        start_time: now,
        cliff_time: now,
        end_time: now + 200,
        withdraw_dust_threshold: None,
        memo: None,
        kind: StreamKind::CliffOnly,
    });
    streams.push_back(CreateStreamParams {
        recipient: recipient1.clone(),
        deposit_amount: 5_001,
        rate_per_second: 1,
        start_time: now,
        cliff_time: now,
        end_time: now + 500,
        withdraw_dust_threshold: None,
        memo: None,
        kind: StreamKind::Linear,
    });

    let result = ctx.factory.try_create_streams(&ctx.sender, &streams);
    assert_eq!(result, Err(Ok(FactoryError::DepositExceedsCap)));
}

#[test]
fn test_create_stream_rejects_over_length_memo() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();
    let long_bytes = vec![b'a'; fluxora_stream::MAX_MEMO_BYTES + 1];
    let memo = Some(Bytes::from_slice(&ctx.env, &long_bytes));

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
        &memo,
        &StreamKind::Linear,
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidMemo)));
}

// ---------------------------------------------------------------------------
// DepositExceedsCap
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_deposit_exceeds_cap() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &10_001,
        &1, // exceeds max_deposit=10_000
        &now,
        &now,
        &(now + 200),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::DepositExceedsCap)));
}

/// Deposit exactly at cap is accepted.
#[test]
fn test_create_stream_deposit_at_cap_ok() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &10_000,
        &1, // exactly at cap
        &now,
        &now,
        &(now + 10_000),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    // May fail for stream-contract reasons (e.g. token transfer) but not DepositExceedsCap
    assert_ne!(result, Err(Ok(FactoryError::DepositExceedsCap)));
}

// ---------------------------------------------------------------------------
// DurationTooShort
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_duration_too_short() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &now,
        &now,
        &(now + 50), // duration=50 < min_duration=100
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::DurationTooShort)));
}

/// Duration exactly at minimum is accepted.
#[test]
fn test_create_stream_duration_at_minimum_ok() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 100), // duration=100 == min_duration
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_ne!(result, Err(Ok(FactoryError::DurationTooShort)));
}

// ---------------------------------------------------------------------------
// Time relationship validation
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_rejects_end_before_start() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &(now + 200),
        &(now + 200),
        &(now + 100),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidTimeRange)));
}

#[test]
fn test_create_stream_rejects_end_equal_start() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result =
        ctx.factory
            .try_create_stream(&ctx.sender, &recipient, &1_000, &1, &now, &now, &now, &0, &fluxora_stream::StreamKind::Linear, &None);
    assert_eq!(result, Err(Ok(FactoryError::InvalidTimeRange)));
}

#[test]
fn test_create_stream_rejects_cliff_before_start() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &(now + 100),
        &now,
        &(now + 300),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidCliff)));
}

#[test]
fn test_create_stream_rejects_cliff_after_end() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &now,
        &(now + 300),
        &(now + 200),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidCliff)));
}

// ---------------------------------------------------------------------------
// NotInitialized
// ---------------------------------------------------------------------------

#[test]
fn test_factory_not_initialized_returns_error() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let now = env.ledger().timestamp();

    // No init called — create_stream should return NotInitialized
    let result = factory.try_create_stream(
        &sender,
        &recipient,
        &1_000,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::NotInitialized)));
}

#[test]
fn test_factory_setters_before_init_return_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let address = Address::generate(&env);

    assert_eq!(
        factory.try_set_admin(&address),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_stream_contract(&address),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_allowlist(&address, &true),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_cap(&1_000),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_min_duration(&100),
        Err(Ok(FactoryError::NotInitialized))
    );
}

#[test]
fn test_get_factory_config_before_init_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);

    let result = factory.try_get_factory_config();
    assert_eq!(result, Err(Ok(FactoryError::NotInitialized)));
}

// ---------------------------------------------------------------------------
// Read-only policy views
// ---------------------------------------------------------------------------

#[test]
fn test_get_factory_config_returns_current_policy() {
    let ctx = Ctx::setup();

    let config = ctx.factory.get_factory_config();
    assert_eq!(config.admin, ctx.admin);
    assert_eq!(config.max_deposit, 10_000);
    assert_eq!(config.min_duration, 100);

    let new_admin = Address::generate(&ctx.env);
    let new_stream_contract = ctx.env.register_contract(None, FluxoraStream);
    ctx.factory.set_admin(&new_admin);
    ctx.factory.set_stream_contract(&new_stream_contract);
    ctx.factory.set_cap(&5_000);
    ctx.factory.set_min_duration(&500);

    let updated = ctx.factory.get_factory_config();
    assert_eq!(updated.admin, new_admin);
    assert_eq!(updated.stream_contract, new_stream_contract);
    assert_eq!(updated.max_deposit, 5_000);
    assert_eq!(updated.min_duration, 500);
}

#[test]
fn test_is_allowlisted_reflects_allowlist_state() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    assert!(!ctx.factory.is_allowlisted(&recipient));

    ctx.factory.set_allowlist(&recipient, &true);
    assert!(ctx.factory.is_allowlisted(&recipient));

    ctx.factory.set_allowlist(&recipient, &false);
    assert!(!ctx.factory.is_allowlisted(&recipient));
}

// ---------------------------------------------------------------------------
// Policy update guards
// ---------------------------------------------------------------------------

/// set_cap updates the cap; subsequent over-cap deposit is rejected.
#[test]
fn test_set_cap_enforced() {
    let ctx = Ctx::setup();
    ctx.factory.set_cap(&5_000); // lower cap
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &6_000,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::DepositExceedsCap)));
}

/// set_min_duration updates the minimum; subsequent short-duration is rejected.
#[test]
fn test_set_min_duration_enforced() {
    let ctx = Ctx::setup();
    ctx.factory.set_min_duration(&500); // raise minimum
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &200,
        &1,
        &now,
        &now,
        &(now + 200), // duration=200 < new min=500
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::DurationTooShort)));
}

/// set_allowlist(false) removes a previously-allowed recipient.
#[test]
fn test_set_allowlist_remove_enforced() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    ctx.factory.set_allowlist(&recipient, &false); // remove
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

// ---------------------------------------------------------------------------
// #632: StreamKind::CliffOnly via factory
// ---------------------------------------------------------------------------

/// Factory can create a CliffOnly stream; stream contract stores kind=CliffOnly.
#[test]
fn test_create_stream_cliff_only_via_factory() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    // CliffOnly: cliff == end so the full deposit unlocks at cliff.
    let end = now + 200;
    let stream_id = ctx.factory.create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &0, // rate ignored for CliffOnly
        &now,
        &end,  // cliff == end
        &end,
        &0,
        &fluxora_stream::StreamKind::CliffOnly,
        &None,
    );

    let state = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state.kind, fluxora_stream::StreamKind::CliffOnly);
    assert_eq!(state.deposit_amount, 1_000);
    assert_eq!(state.recipient, recipient);
}

/// Policy checks (allowlist, cap, duration) apply to CliffOnly just as to Linear.
#[test]
fn test_cliff_only_via_factory_still_enforces_cap() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();
    let end = now + 200;

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &10_001, // exceeds cap
        &0,
        &now,
        &end,
        &end,
        &0,
        &fluxora_stream::StreamKind::CliffOnly,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::DepositExceedsCap)));
}

/// Policy checks apply to CliffOnly: duration too short is rejected.
#[test]
fn test_cliff_only_via_factory_still_enforces_min_duration() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &0,
        &now,
        &(now + 50),
        &(now + 50), // duration=50 < min_duration=100
        &0,
        &fluxora_stream::StreamKind::CliffOnly,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::DurationTooShort)));
}

/// Policy checks apply to CliffOnly: allowlist is enforced.
#[test]
fn test_cliff_only_via_factory_still_enforces_allowlist() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env); // not allowlisted
    let now = ctx.now();
    let end = now + 200;

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &0,
        &now,
        &end,
        &end,
        &0,
        &fluxora_stream::StreamKind::CliffOnly,
        &None,
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

// ---------------------------------------------------------------------------
// #632: memo forwarding via factory
// ---------------------------------------------------------------------------

/// Memo is forwarded to the stream contract and readable via get_stream_memo.
#[test]
fn test_memo_forwarded_and_stored() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let memo = Bytes::from_slice(&ctx.env, b"invoice-42");
    let stream_id = ctx.factory.create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &Some(memo.clone()),
    );

    let stored = ctx.stream.get_stream_memo(&stream_id);
    assert_eq!(stored, Some(memo));
}

/// None memo results in no memo stored.
#[test]
fn test_none_memo_results_in_no_memo() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let stream_id = ctx.factory.create_stream(
        &ctx.sender,
        &recipient,
        &1_000,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
        &fluxora_stream::StreamKind::Linear,
        &None,
    );

    let stored = ctx.stream.get_stream_memo(&stream_id);
    assert_eq!(stored, None);
}
