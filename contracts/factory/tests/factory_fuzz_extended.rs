//! Extended fuzz coverage for FluxoraFactory entry points.
//!
//! Gap analysis: The original `factory_fuzz.rs` only fuzzes `create_stream`.
//! This harness adds coverage for admin setter functions that take numeric
//! and address-shaped inputs:
//!   - `init` — validates max_deposit (i128), min_duration (u64)
//!   - `set_cap` — validates max_deposit (i128)
//!   - `set_min_duration` — validates min_duration (u64)
//!   - `set_rate_bounds` — validates min_rate/max_rate (Option<i128>)
//!   - `set_factory_paused` — validates bool
//!   - `get_factory_streams_paginated` — validates start_index/limit (u32)
//!
//! Priority: numeric overflow, underflow, boundary conditions.

use fluxora_factory::{
    FactoryError, FluxoraFactory, FluxoraFactoryClient, MAX_MIN_DURATION_SECONDS,
};
use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

/// Setup a fresh factory environment for fuzzing.
fn setup_factory() -> (Env, FluxoraFactoryClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let stream_cid = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &stream_cid);

    let factory_cid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_cid);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);
    let stellar_asset = StellarAssetClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);

    stellar_asset.mint(&sender, &1_000_000_000_i128);
    stream_client.init(&token_id, &admin);
    token.approve(&sender, &stream_cid, &i128::MAX, &100_000);

    // Initialize factory with reasonable defaults
    factory.init(&admin, &stream_cid, &1_000_000_i128, &60_u64);

    (env, factory, admin)
}

// ─────────────────────────────────────────────────────────────────────────────
// Fuzz target: init validation
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn prop_init_cap_validation(
        max_deposit in i128::MIN..i128::MAX,
        min_duration in 0u64..(MAX_MIN_DURATION_SECONDS + 1000),
    ) {
        let env = Env::default();
        env.mock_all_auths();

        let stream_cid = env.register_contract(None, FluxoraStream);
        let stream_client = FluxoraStreamClient::new(&env, &stream_cid);

        let factory_cid = env.register_contract(None, FluxoraFactory);
        let factory = FluxoraFactoryClient::new(&env, &factory_cid);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        stream_client.init(&token_id, &admin);

        let result = factory.try_init(&admin, &stream_cid, &max_deposit, &min_duration);

        // Property: InvalidCap iff max_deposit <= 0
        if max_deposit <= 0 {
            assert_eq!(result, Err(Ok(FactoryError::InvalidCap)));
        } else if min_duration > MAX_MIN_DURATION_SECONDS {
            // Property: InvalidMinDuration iff min_duration > MAX_MIN_DURATION_SECONDS
            assert_eq!(result, Err(Ok(FactoryError::InvalidMinDuration)));
        } else {
            // Valid config should succeed
            assert!(result.is_ok(), "Valid init was rejected: {:?}", result);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fuzz target: set_cap validation
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn prop_set_cap_validation(new_cap in i128::MIN..i128::MAX) {
        let (_env, factory, _admin) = setup_factory();

        let result = factory.try_set_cap(&new_cap);

        // Property: InvalidCap iff new_cap <= 0
        if new_cap <= 0 {
            assert_eq!(result, Err(Ok(FactoryError::InvalidCap)));
        } else {
            assert!(result.is_ok(), "Valid cap was rejected: {:?}", result);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fuzz target: set_min_duration validation
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn prop_set_min_duration_validation(new_min_duration in 0u64..(MAX_MIN_DURATION_SECONDS + 10000)) {
        let (_env, factory, _admin) = setup_factory();

        let result = factory.try_set_min_duration(&new_min_duration);

        // Property: InvalidMinDuration iff new_min_duration > MAX_MIN_DURATION_SECONDS
        if new_min_duration > MAX_MIN_DURATION_SECONDS {
            assert_eq!(result, Err(Ok(FactoryError::InvalidMinDuration)));
        } else {
            assert!(result.is_ok(), "Valid min_duration was rejected: {:?}", result);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fuzz target: set_rate_bounds validation
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn prop_set_rate_bounds_validation(
        min_rate_opt in proptest::option::of(i128::MIN..i128::MAX),
        max_rate_opt in proptest::option::of(i128::MIN..i128::MAX),
    ) {
        let (_env, factory, _admin) = setup_factory();

        let result = factory.try_set_rate_bounds(&min_rate_opt, &max_rate_opt);

        // Property: InvalidRateBounds iff:
        //   - min_rate < 0, OR
        //   - max_rate < 0, OR
        //   - both set and min_rate > max_rate
        let expect_invalid = match (min_rate_opt, max_rate_opt) {
            (Some(min), _) if min < 0 => true,
            (_, Some(max)) if max < 0 => true,
            (Some(min), Some(max)) if min > max => true,
            _ => false,
        };

        if expect_invalid {
            assert_eq!(result, Err(Ok(FactoryError::InvalidRateBounds)));
        } else {
            assert!(result.is_ok(), "Valid rate bounds were rejected: {:?}", result);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fuzz target: set_factory_paused (bool toggle)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn prop_set_factory_paused_toggle(paused in proptest::bool::ANY) {
        let (_env, factory, _admin) = setup_factory();

        let result = factory.try_set_factory_paused(&paused);
        assert!(result.is_ok(), "set_factory_paused failed: {:?}", result);

        // Verify state matches
        let actual_paused = factory.is_factory_paused();
        assert_eq!(actual_paused, paused, "Pause state mismatch");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fuzz target: get_factory_streams_paginated boundary conditions
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn prop_paginated_boundary_conditions(
        start_index in 0u32..1000u32,
        limit in 0u32..200u32,
    ) {
        let (_env, factory, _admin) = setup_factory();

        // This should never panic, even with extreme values
        let _result = factory.get_factory_streams_paginated(&start_index, &limit);
        // No assertion on content — just verify it doesn't panic
    }
}
