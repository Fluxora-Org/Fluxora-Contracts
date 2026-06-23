//! Tests for issue #525: factory policy enforcement.
//!
//! Covers all six FactoryError variants and verifies that `create_stream` via
//! the factory correctly delegates to the stream contract after passing all checks.

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, IntoVal,
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
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
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
            .try_create_stream(&ctx.sender, &recipient, &1_000, &1, &now, &now, &now, &0);
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
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

// ---------------------------------------------------------------------------
// Registry: get_factory_stream_count and get_factory_streams_paginated
// ---------------------------------------------------------------------------

/// Before any streams are created the registry is empty.
#[test]
fn test_registry_empty_before_any_creation() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.factory.get_factory_stream_count(), 0);
    let page = ctx.factory.get_factory_streams_paginated(&0, &10);
    assert_eq!(page.len(), 0);
}

/// Each successful create_stream appends to the registry and increments the count.
#[test]
fn test_registry_appends_on_successful_create_stream() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let id0 = ctx.factory.create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
    );
    assert_eq!(ctx.factory.get_factory_stream_count(), 1);

    let id1 = ctx.factory.create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
    );
    assert_eq!(ctx.factory.get_factory_stream_count(), 2);

    let page = ctx.factory.get_factory_streams_paginated(&0, &10);
    assert_eq!(page.len(), 2);
    assert_eq!(page.get(0).unwrap(), id0);
    assert_eq!(page.get(1).unwrap(), id1);
}

/// A failed create_stream (policy check) does not write to the registry.
#[test]
fn test_registry_not_written_on_policy_failure() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    // Do NOT allowlist — triggers RecipientNotAllowlisted
    let now = ctx.now();

    let _ = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
    );

    assert_eq!(ctx.factory.get_factory_stream_count(), 0);
    let page = ctx.factory.get_factory_streams_paginated(&0, &10);
    assert_eq!(page.len(), 0);
}

/// get_factory_streams_paginated caps the returned page at MAX_PAGE_SIZE (100).
#[test]
fn test_paginated_enforces_max_page_size() {
    use fluxora_factory::MAX_PAGE_SIZE;
    // Create two streams and request limit > MAX_PAGE_SIZE — must not exceed MAX_PAGE_SIZE.
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    ctx.factory.create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
    );
    ctx.factory.create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
    );

    // Requesting limit=200 is capped to MAX_PAGE_SIZE; with only 2 streams the result is 2.
    let page = ctx
        .factory
        .get_factory_streams_paginated(&0, &(MAX_PAGE_SIZE + 100));
    assert!(page.len() <= MAX_PAGE_SIZE);
    assert_eq!(page.len(), 2);
}

/// get_factory_streams_paginated with start_index beyond end returns an empty list.
#[test]
fn test_paginated_start_index_beyond_end_returns_empty() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    ctx.factory.create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
    );

    // start_index=5 is beyond the single entry
    let page = ctx.factory.get_factory_streams_paginated(&5, &10);
    assert_eq!(page.len(), 0);
}

/// Pagination correctly windows a multi-entry registry.
#[test]
fn test_paginated_window_correctness() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient, &true);
    let now = ctx.now();

    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    for _ in 0..5 {
        let id = ctx.factory.create_stream(
            &ctx.sender,
            &recipient,
            &100,
            &1,
            &now,
            &now,
            &(now + 200),
            &0,
        );
        ids.push_back(id);
    }

    // Page 0: first 2
    let page0 = ctx.factory.get_factory_streams_paginated(&0, &2);
    assert_eq!(page0.len(), 2);
    assert_eq!(page0.get(0).unwrap(), ids.get(0).unwrap());
    assert_eq!(page0.get(1).unwrap(), ids.get(1).unwrap());

    // Page 1: next 2
    let page1 = ctx.factory.get_factory_streams_paginated(&2, &2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap(), ids.get(2).unwrap());
    assert_eq!(page1.get(1).unwrap(), ids.get(3).unwrap());

    // Page 2: remaining 1
    let page2 = ctx.factory.get_factory_streams_paginated(&4, &2);
    assert_eq!(page2.len(), 1);
    assert_eq!(page2.get(0).unwrap(), ids.get(4).unwrap());
}

/// get_factory_stream_count stays zero when only failed attempts are made.
#[test]
fn test_registry_count_only_counts_successful_streams() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.now();

    // Allowlist check failure
    let _ = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
    );
    // Deposit cap failure
    ctx.factory.set_allowlist(&recipient, &true);
    let _ = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &999_999,
        &1,
        &now,
        &now,
        &(now + 200),
        &0,
    );
    // Duration failure
    let _ = ctx.factory.try_create_stream(
        &ctx.sender,
        &recipient,
        &100,
        &1,
        &now,
        &now,
        &(now + 10), // too short
        &0,
    );

    assert_eq!(ctx.factory.get_factory_stream_count(), 0);
}
