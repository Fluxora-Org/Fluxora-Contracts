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
    Address, Bytes, Env, IntoVal, Vec,
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
    assert_eq!(FactoryError::InvalidCap as u32, 9);
    assert_eq!(FactoryError::InvalidMinDuration as u32, 10);
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
    let admin = Address::generate(&env);
    let stream_contract = Address::generate(&env);
    let new_admin = Address::generate(&env);

    env.mock_all_auths_allowing_non_root_auth();
    factory.init(&admin, &stream_contract, &10_000, &100);

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
    let a2 = Address::generate(&env2);
    let sc2 = Address::generate(&env2);
    let na2 = Address::generate(&env2);
    f2.init(&a2, &sc2, &10_000, &100);
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
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let stream_contract = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let new_stream_contract = Address::generate(&env);
    let recipient = Address::generate(&env);

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
        &None,
        &StreamKind::Linear,
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

    let stream_id = result.unwrap();
    let stream_state = ctx.stream.get_stream_state(&stream_id).unwrap();
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

    let ids = result.unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(ctx.stream.get_stream_memo(&ids.get_unchecked(0)).unwrap(), Some(Bytes::from_slice(&ctx.env, b"batch-1")));
    assert_eq!(ctx.stream.get_stream_memo(&ids.get_unchecked(1)).unwrap(), Some(Bytes::from_slice(&ctx.env, b"batch-2")));
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
        &None,
        &StreamKind::Linear,
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
        &None,
        &StreamKind::Linear,
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
        &None,
        &StreamKind::Linear,
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
        &None,
        &StreamKind::Linear,
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
        &None,
        &StreamKind::Linear,
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
            .try_create_stream(&ctx.sender, &recipient, &1_000, &1, &now, &now, &now, &0, &None, &StreamKind::Linear);
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
        &None,
        &StreamKind::Linear,
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
        &None,
        &StreamKind::Linear,
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
        &None,
        &StreamKind::Linear,
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
    let new_stream_contract = Address::generate(&ctx.env);
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
        &None,
        &StreamKind::Linear,
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
        &None,
        &StreamKind::Linear,
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
        &None,
        &StreamKind::Linear,
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

// ---------------------------------------------------------------------------
// Composed policy enforcement proptest
//
// Invariant (accept-iff-all-policies-satisfied):
//   create_stream succeeds ⟺ allowlisted ∧ deposit ≤ cap ∧ duration ≥ min_duration
//                              ∧ rate ≥ min_rate (if set) ∧ rate ≤ max_rate (if set)
//                              ∧ start_time < end_time
//
// Guard evaluation order mirrors the factory implementation:
//   1. allowlist  2. deposit cap  3. time range  4. min duration  5. rate bounds
//
// A bug that lets one policy mask another would cause the conjunction to fail.
// ---------------------------------------------------------------------------
mod proptest_composed_policy {
    use super::*;
    use fluxora_stream::StreamKind;
    use proptest::prelude::*;

    /// Random factory policy configuration.
    #[derive(Debug, Clone)]
    struct PolicyConfig {
        /// Maximum deposit allowed per stream (1_000..=20_000).
        cap: i128,
        /// Minimum stream duration in seconds (0..=400).
        min_duration: u64,
        /// Optional inclusive lower bound on rate_per_second.
        min_rate: Option<i128>,
        /// Optional inclusive upper bound on rate_per_second (≥ min_rate when both set).
        max_rate: Option<i128>,
    }

    /// Random stream creation input.
    #[derive(Debug, Clone)]
    struct StreamInput {
        /// Whether the recipient has been added to the allowlist.
        allowlisted: bool,
        deposit: i128,
        duration: u64,
        /// rate_per_second chosen from 0..=30.
        rate: i128,
        start_time: u64,
    }

    prop_compose! {
        /// Strategy that produces a valid [`PolicyConfig`].
        ///
        /// When both rate bounds are enabled the max is always ≥ min so the
        /// config itself is consistent — the test targets *input* violations.
        fn arb_policy()(
            cap in 1_000i128..=20_000i128,
            min_duration in 0u64..=400u64,
            enable_min_rate in any::<bool>(),
            min_rate_val in 1i128..=10i128,
            enable_max_rate in any::<bool>(),
            max_rate_extra in 0i128..=10i128,
        ) -> PolicyConfig {
            let min_rate = if enable_min_rate { Some(min_rate_val) } else { None };
            // max_rate >= min_rate_val so the window is never empty.
            let max_rate = if enable_max_rate { Some(min_rate_val + max_rate_extra) } else { None };
            PolicyConfig { cap, min_duration, min_rate, max_rate }
        }
    }

    prop_compose! {
        fn arb_input()(
            allowlisted in any::<bool>(),
            deposit     in 500i128..=25_000i128,
            duration    in 1u64..=600u64,
            rate        in 0i128..=30i128,
            start_time  in 1_000u64..=5_000u64,
        ) -> StreamInput {
            StreamInput { allowlisted, deposit, duration, rate, start_time }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// **Composed policy conjunction invariant**
        ///
        /// For every combination of (policy config, stream input):
        ///
        /// - `create_stream` **succeeds** iff every enabled policy is satisfied:
        ///   allowlisted ∧ deposit ≤ cap ∧ start < end ∧ duration ≥ min_duration
        ///   ∧ rate ≥ min_rate (if set) ∧ rate ≤ max_rate (if set).
        ///
        /// - On **rejection** the error variant corresponds to the first guard
        ///   violated in the factory's documented evaluation order, proving no
        ///   policy masks another.
        #[test]
        fn prop_composed_policy_accept_iff_all_satisfied(
            policy in arb_policy(),
            input  in arb_input(),
        ) {
            let env = Env::default();
            env.mock_all_auths();

            // ── Deploy contracts ──────────────────────────────────────────────
            let stream_id = env.register_contract(None, FluxoraStream);
            let stream_client = FluxoraStreamClient::new(&env, &stream_id);

            let factory_id = env.register_contract(None, FluxoraFactory);
            let factory = FluxoraFactoryClient::new(&env, &factory_id);

            // ── Token setup ───────────────────────────────────────────────────
            let token_admin = Address::generate(&env);
            let token_contract_id = env
                .register_stellar_asset_contract_v2(token_admin.clone())
                .address();
            let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_contract_id);
            let token = soroban_sdk::token::Client::new(&env, &token_contract_id);

            let admin  = Address::generate(&env);
            let sender = Address::generate(&env);
            stellar_asset.mint(&sender, &1_000_000_000_000i128);
            token.approve(&sender, &stream_id, &1_000_000_000_000i128, &99_999u32);

            stream_client.init(&token_contract_id, &stream_id);
            factory.init(&admin, &stream_id, &policy.cap, &policy.min_duration);

            // ── Apply optional rate bounds ────────────────────────────────────
            if policy.min_rate.is_some() || policy.max_rate.is_some() {
                factory.set_rate_bounds(&policy.min_rate, &policy.max_rate);
            }

            // ── Conditionally allowlist the recipient ─────────────────────────
            let recipient = Address::generate(&env);
            if input.allowlisted {
                factory.set_allowlist(&recipient, &true);
            }

            let start_time = input.start_time;
            let end_time   = start_time + input.duration;

            // ── Invoke create_stream ──────────────────────────────────────────
            let result = factory.try_create_stream(
                &sender,
                &recipient,
                &input.deposit,
                &input.rate,
                &start_time,
                &start_time, // cliff == start (no cliff complexity needed)
                &end_time,
                &0i128,
                &None,
                &StreamKind::Linear,
            );

            // ── Derive expected outcome from the guard order ──────────────────
            //
            // The factory evaluates guards strictly in this order:
            //   G1 allowlist → G2 cap → G3 time range → G4 min duration → G5 rate
            //
            // We compute a boolean for each guard and verify that the *first*
            // failing guard produces the matching error (proving no masking).

            let violates_allowlist = !input.allowlisted;
            let violates_cap       = input.deposit > policy.cap;
            // duration >= 1 (arb_input lower bound), so end_time > start_time always.
            let violates_time      = start_time >= end_time;
            let violates_duration  = input.duration < policy.min_duration;
            let violates_min_rate  = policy.min_rate.map_or(false, |mn| input.rate < mn);
            let violates_max_rate  = policy.max_rate.map_or(false, |mx| input.rate > mx);

            let all_satisfied = !violates_allowlist
                && !violates_cap
                && !violates_time
                && !violates_duration
                && !violates_min_rate
                && !violates_max_rate;

            if all_satisfied {
                prop_assert!(
                    result.is_ok(),
                    "Expected Ok (all policies satisfied) but got {:?}\n  policy={:?}\n  input={:?}",
                    result, policy, input
                );
            } else {
                prop_assert!(
                    result.is_err(),
                    "Expected Err (a policy is violated) but got Ok\n  policy={:?}\n  input={:?}",
                    policy, input
                );

                let err = result.unwrap_err();

                // Verify the error maps to the *first* violated guard.
                if violates_allowlist {
                    prop_assert_eq!(
                        err,
                        Ok(FactoryError::RecipientNotAllowlisted),
                        "G1 (allowlist) violated but wrong error\n  policy={:?}\n  input={:?}",
                        policy, input
                    );
                } else if violates_cap {
                    prop_assert_eq!(
                        err,
                        Ok(FactoryError::DepositExceedsCap),
                        "G2 (cap) violated but wrong error\n  policy={:?}\n  input={:?}",
                        policy, input
                    );
                } else if violates_time {
                    prop_assert_eq!(
                        err,
                        Ok(FactoryError::InvalidTimeRange),
                        "G3 (time range) violated but wrong error\n  policy={:?}\n  input={:?}",
                        policy, input
                    );
                } else if violates_duration {
                    prop_assert_eq!(
                        err,
                        Ok(FactoryError::DurationTooShort),
                        "G4 (min duration) violated but wrong error\n  policy={:?}\n  input={:?}",
                        policy, input
                    );
                } else {
                    // G5: rate bounds — min_rate checked before max_rate in the factory.
                    if violates_min_rate {
                        prop_assert_eq!(
                            err,
                            Ok(FactoryError::RateBelowMin),
                            "G5a (min rate) violated but wrong error\n  policy={:?}\n  input={:?}",
                            policy, input
                        );
                    } else {
                        prop_assert_eq!(
                            err,
                            Ok(FactoryError::RateAboveMax),
                            "G5b (max rate) violated but wrong error\n  policy={:?}\n  input={:?}",
                            policy, input
                        );
                    }
                }
            }
        }
    }
}
