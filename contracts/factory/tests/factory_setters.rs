//! Direct unit tests for `FluxoraFactory` admin setters and config views.
//!
//! Covers issue #684: `set_admin`, `set_cap`, `set_min_duration`, `set_allowlist`,
//! `is_allowlisted`, and `get_factory_config` — including auth enforcement,
//! `NotInitialized` / `AlreadyInitialized` branches, and admin rotation.

#![cfg(test)]

use fluxora_factory::{load_policy, FactoryError, FactoryPolicy, FluxoraFactory, FluxoraFactoryClient};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};
use std::panic::AssertUnwindSafe;

// ---------------------------------------------------------------------------
// init — happy path and error branches
// ---------------------------------------------------------------------------

/// `init` succeeds and persists all supplied parameters.
#[test]
fn test_init_happy_path() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &5_000, &200);

    let cfg = factory.get_factory_config();
    assert_eq!(cfg.admin, admin);
    assert_eq!(cfg.stream_contract, sc);
    assert_eq!(cfg.max_deposit, 5_000);
    assert_eq!(cfg.min_duration, 200);
}

/// Calling `init` a second time returns `AlreadyInitialized`.
#[test]
fn test_init_double_returns_already_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);
    let result = factory.try_init(&admin, &sc, &1_000, &10);
    assert_eq!(result, Err(Ok(FactoryError::AlreadyInitialized)));
}

// ---------------------------------------------------------------------------
// Views before init — NotInitialized
// ---------------------------------------------------------------------------

/// `get_factory_config` before `init` returns `NotInitialized`.
#[test]
fn test_get_factory_config_before_init() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);

    assert_eq!(
        factory.try_get_factory_config(),
        Err(Ok(FactoryError::NotInitialized))
    );
}

/// Each admin-only setter before `init` returns `NotInitialized`.
#[test]
fn test_setters_before_init_return_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let addr = Address::generate(&env);

    assert_eq!(
        factory.try_set_admin(&addr),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_stream_contract(&addr),
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
    assert_eq!(
        factory.try_set_allowlist(&addr, &true),
        Err(Ok(FactoryError::NotInitialized))
    );
}

// ---------------------------------------------------------------------------
// set_admin — rotation and auth transfer
// ---------------------------------------------------------------------------

/// `set_admin` updates the stored admin; `get_factory_config` reflects the change.
#[test]
fn test_set_admin_updates_config() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    let new_admin = Address::generate(&env);
    factory.set_admin(&new_admin);
    assert_eq!(factory.get_factory_config().admin, new_admin);
}

/// After rotation, the new admin can call setters (mock_all_auths covers both).
#[test]
fn test_set_admin_new_admin_can_call_setters() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    let new_admin = Address::generate(&env);
    factory.set_admin(&new_admin);

    factory.set_cap(&3_000);
    assert_eq!(factory.get_factory_config().max_deposit, 3_000);
}

/// Setting admin to the same address is a no-op and does not error.
#[test]
fn test_set_admin_same_address_noop() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    factory.set_admin(&admin);
    assert_eq!(factory.get_factory_config().admin, admin);
}

// ---------------------------------------------------------------------------
// set_cap — round-trip
// ---------------------------------------------------------------------------

/// `set_cap` persists and is reflected by `get_factory_config`.
#[test]
fn test_set_cap_round_trip() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    factory.set_cap(&7_500);
    assert_eq!(factory.get_factory_config().max_deposit, 7_500);
}

// ---------------------------------------------------------------------------
// set_min_duration — round-trip
// ---------------------------------------------------------------------------

/// `set_min_duration` persists and is reflected by `get_factory_config`.
#[test]
fn test_set_min_duration_round_trip() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    factory.set_min_duration(&300);
    assert_eq!(factory.get_factory_config().min_duration, 300);
}

// ---------------------------------------------------------------------------
// set_allowlist / is_allowlisted
// ---------------------------------------------------------------------------

/// `is_allowlisted` returns false for an address never added.
#[test]
fn test_is_allowlisted_default_false() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    let recipient = Address::generate(&env);
    assert!(!factory.is_allowlisted(&recipient));
}

/// Adding a recipient flips `is_allowlisted` to true.
#[test]
fn test_set_allowlist_add() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    let recipient = Address::generate(&env);
    factory.set_allowlist(&recipient, &true);
    assert!(factory.is_allowlisted(&recipient));
}

/// Removing a recipient flips `is_allowlisted` back to false and removes the
/// underlying persistent key.
#[test]
fn test_set_allowlist_remove() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    let recipient = Address::generate(&env);
    factory.set_allowlist(&recipient, &true);
    factory.set_allowlist(&recipient, &false);
    assert!(!factory.is_allowlisted(&recipient));
}

/// Removing a recipient that was never added is a safe no-op.
#[test]
fn test_set_allowlist_remove_non_allowlisted_noop() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &10_000, &100);

    let recipient = Address::generate(&env);
    factory.set_allowlist(&recipient, &false); // never added — should not panic
    assert!(!factory.is_allowlisted(&recipient));
}

// ---------------------------------------------------------------------------
// Negative auth tests — each admin-only setter must fail without admin auth
// ---------------------------------------------------------------------------

/// Helper: assert that a closure panics (Soroban testutils behaviour for
/// unauthorized `require_auth` calls).
fn assert_auth_fails<F: FnOnce()>(f: F) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    assert!(result.is_err(), "expected auth failure (panic) but call succeeded");
}

/// `set_admin` rejects a non-admin caller.
#[test]
fn test_set_admin_rejects_non_admin() {
    let env = Env::default();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let sc = Address::generate(&env);

    env.mock_all_auths();
    factory.init(&admin, &sc, &10_000, &100);

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_admin",
            args: (&new_admin,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_admin(&new_admin));
}

/// `set_stream_contract` rejects a non-admin caller.
#[test]
fn test_set_stream_contract_rejects_non_admin() {
    let env = Env::default();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let sc = Address::generate(&env);
    let new_sc = Address::generate(&env);

    env.mock_all_auths();
    factory.init(&admin, &sc, &10_000, &100);

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_stream_contract",
            args: (&new_sc,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_stream_contract(&new_sc));
}

/// `set_cap` rejects a non-admin caller.
#[test]
fn test_set_cap_rejects_non_admin() {
    let env = Env::default();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let sc = Address::generate(&env);

    env.mock_all_auths();
    factory.init(&admin, &sc, &10_000, &100);

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_cap",
            args: (5_000i128,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_cap(&5_000));
}

/// `set_min_duration` rejects a non-admin caller.
#[test]
fn test_set_min_duration_rejects_non_admin() {
    let env = Env::default();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let sc = Address::generate(&env);

    env.mock_all_auths();
    factory.init(&admin, &sc, &10_000, &100);

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_min_duration",
            args: (500u64,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_min_duration(&500));
}

/// `set_allowlist` rejects a non-admin caller.
#[test]
fn test_set_allowlist_rejects_non_admin() {
    let env = Env::default();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let sc = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.mock_all_auths();
    factory.init(&admin, &sc, &10_000, &100);

    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_allowlist",
            args: (&recipient, true).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_allowlist(&recipient, &true));
}

// ---------------------------------------------------------------------------
// Instance storage TTL regression tests (issue #728)
// ---------------------------------------------------------------------------

/// Test that `init` bumps instance TTL, allowing config to survive idle periods.
///
/// This test simulates a long-idle factory by advancing the ledger clock
/// after initialization. The config should remain accessible if TTL was
/// properly bumped during `init`.
#[test]
fn test_init_bumps_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    // Initialize the factory — this should bump instance TTL.
    factory.init(&admin, &sc, &5_000, &200);

    // Verify config is immediately accessible after init.
    let cfg = factory.get_factory_config();
    assert_eq!(cfg.admin, admin);

    // Advance ledger by a significant amount (simulating idle time).
    // The threshold is 17_280 ledgers; we advance less than that to show
    // that a single TTL bump keeps the entry alive past one idle window.
    env.ledger().set_sequence_number(env.ledger().sequence() + 10_000);

    // Config should still be accessible after simulated idle time.
    let cfg = factory.get_factory_config();
    assert_eq!(cfg.admin, admin);
    assert_eq!(cfg.stream_contract, sc);
    assert_eq!(cfg.max_deposit, 5_000);
    assert_eq!(cfg.min_duration, 200);
}

/// Test that each admin setter bumps instance TTL.
///
/// This test verifies that calling each setter extends the instance TTL,
/// ensuring that repeated admin activity keeps the config alive.
#[test]
fn test_setters_bump_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);

    // Test set_admin bumps TTL.
    let new_admin = Address::generate(&env);
    factory.set_admin(&new_admin);
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    assert_eq!(factory.get_factory_config().admin, new_admin);

    // Test set_stream_contract bumps TTL.
    let new_sc = Address::generate(&env);
    factory.set_stream_contract(&new_sc);
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    assert_eq!(factory.get_factory_config().stream_contract, new_sc);

    // Test set_cap bumps TTL.
    factory.set_cap(&7_500);
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    assert_eq!(factory.get_factory_config().max_deposit, 7_500);

    // Test set_min_duration bumps TTL.
    factory.set_min_duration(&250);
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    assert_eq!(factory.get_factory_config().min_duration, 250);

    // Test set_batch_cap_enforcement bumps TTL.
    factory.set_batch_cap_enforcement(&false);
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    assert_eq!(factory.get_factory_config().batch_cap_enforced, false);

    // Test set_factory_paused bumps TTL.
    factory.set_factory_paused(&true);
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    assert!(factory.is_factory_paused());

    // Test set_rate_bounds bumps TTL (by checking config still accessible).
    factory.set_rate_bounds(&Some(100), &Some(1_000));
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    assert_eq!(factory.get_factory_config().max_deposit, 7_500); // config still accessible
}

/// Test repeated setter calls near TTL threshold keep config alive.
///
/// This test simulates a factory receiving frequent admin updates (near
/// the TTL threshold window) and verifies that repeated bumps keep the
/// config from expiring.
#[test]
fn test_repeated_setter_calls_prevent_expiration() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);

    // Simulate a busy factory: repeatedly advance and call setters.
    for i in 0..5 {
        env.ledger().set_sequence_number(env.ledger().sequence() + 3_000);
        factory.set_cap(&(10_000 + (i as i128 * 100)));
        let cfg = factory.get_factory_config();
        assert_eq!(cfg.max_deposit, 10_000 + (i as i128 * 100));
    }

    // After 5 updates spaced 3000 ledgers apart (15_000 total),
    // the config should still be accessible.
    let cfg = factory.get_factory_config();
    assert_eq!(cfg.max_deposit, 10_400); // last update
}

/// Test that config remains accessible after many idlereads.
///
/// This test verifies that simple read operations (like `is_factory_paused`)
/// do NOT bump TTL (they are read-only), so a truly idle factory will
/// eventually expire. However, the first setter after the idle period
/// should successfully bump TTL and restore accessibility.
#[test]
fn test_idle_factory_recovers_on_first_setter() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);
    assert_eq!(factory.get_factory_config().max_deposit, 10_000);

    // Simulate an idle period: don't call any setters, just advance ledger.
    // The instance entries may approach expiration but should not yet expire
    // if the TTL bump from init was sufficient.
    env.ledger().set_sequence_number(env.ledger().sequence() + 10_000);

    // A read operation (read-only, no TTL bump) should still work.
    let paused = factory.is_factory_paused();
    assert!(!paused);

    // Now call a setter — this should successfully bump TTL.
    factory.set_cap(&15_000);
    assert_eq!(factory.get_factory_config().max_deposit, 15_000);

    // Advance ledger again and verify config is still accessible.
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    assert_eq!(factory.get_factory_config().max_deposit, 15_000);
}

/// Test that set_rate_bounds bumps instance TTL.
#[test]
fn test_set_rate_bounds_bumps_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);

    // Set rate bounds — this should bump TTL.
    factory.set_rate_bounds(&Some(50), &Some(5_000));

    // Advance ledger and verify config is still accessible.
    env.ledger().set_sequence_number(env.ledger().sequence() + 5_000);
    let cfg = factory.get_factory_config();
    assert_eq!(cfg.max_deposit, 10_000); // config still accessible
}

// ---------------------------------------------------------------------------
// load_policy helper — centralised policy-load chokepoint
// ---------------------------------------------------------------------------

/// `load_policy` returns `NotInitialized` before any `init` call. All required
/// fields are absent, so the first required read fails.
#[test]
fn test_load_policy_before_init_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let _fid = env.register_contract(None, FluxoraFactory);

    let result = load_policy(&env);
    assert_eq!(result, Err(FactoryError::NotInitialized));
}

/// Immediately after `init`, `load_policy` reflects the init-supplied values
/// plus the documented defaults for optional fields (pause = false, rate
/// bounds = None, batch_cap = true).
#[test]
fn test_load_policy_reflects_initial_state() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);

    let policy = load_policy(&env).expect("policy should load after init");
    assert_eq!(policy.stream_contract, sc);
    assert_eq!(policy.max_deposit, 10_000);
    assert_eq!(policy.min_duration, 100);
    assert!(policy.batch_cap_enforced, "init defaults batch-cap to true");
    assert!(!policy.creation_paused, "init defaults pause to false");
    assert_eq!(policy.min_rate_per_second, None, "no rate bounds by default");
    assert_eq!(policy.max_rate_per_second, None, "no rate bounds by default");
}

/// Applying each policy setter (cap / min_duration / stream_contract /
/// batch_cap / pause / rate bounds) is reflected by `load_policy`. This is
/// the primary regression test for the centralised load helper.
#[test]
fn test_load_policy_reflects_all_setters() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    // Initial cap/min_duration
    factory.init(&admin, &sc, &10_000, &100);

    // Replace each policy axis through its setter and verify via load_policy.
    let new_sc = Address::generate(&env);
    factory.set_stream_contract(&new_sc);
    factory.set_cap(&7_500);
    factory.set_min_duration(&250);
    factory.set_batch_cap_enforcement(&false);
    factory.set_factory_paused(&true);
    factory.set_rate_bounds(&Some(50), &Some(1_000));

    let policy: FactoryPolicy = load_policy(&env).expect("policy should load");

    assert_eq!(policy.stream_contract, new_sc);
    assert_eq!(policy.max_deposit, 7_500);
    assert_eq!(policy.min_duration, 250);
    assert!(!policy.batch_cap_enforced);
    assert!(policy.creation_paused);
    assert_eq!(policy.min_rate_per_second, Some(50));
    assert_eq!(policy.max_rate_per_second, Some(1_000));
}

/// When rate bounds are absent, `load_policy` reports `None` for both fields.
/// This guards against accidentally reading them as `0` (which would silently
/// turn into RateBelowMin / RateAboveMax failures on every stream creation).
#[test]
fn test_load_policy_defaults_rate_bounds_to_none() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);

    let policy = load_policy(&env).expect("policy should load");
    assert_eq!(policy.min_rate_per_second, None);
    assert_eq!(policy.max_rate_per_second, None);

    // Toggling pause does not implicitly change rate-bound visibility.
    factory.set_factory_paused(&true);
    let policy = load_policy(&env).expect("policy should load");
    assert_eq!(policy.min_rate_per_second, None);
    assert_eq!(policy.max_rate_per_second, None);
}

/// `set_batch_cap_enforcement` flips `batch_cap_enforced` in both directions,
/// which `load_policy` must surface verbatim for the batch path to honour.
#[test]
fn test_load_policy_reflects_batch_cap_toggle() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);
    assert!(load_policy(&env).unwrap().batch_cap_enforced);

    factory.set_batch_cap_enforcement(&false);
    assert!(!load_policy(&env).unwrap().batch_cap_enforced);

    factory.set_batch_cap_enforcement(&true);
    assert!(load_policy(&env).unwrap().batch_cap_enforced);
}

/// `set_factory_paused` flips `creation_paused`, which is the very first
/// semantic guard after the policy load on both creation paths.
#[test]
fn test_load_policy_reflects_pause_toggle() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);
    assert!(!load_policy(&env).unwrap().creation_paused);

    factory.set_factory_paused(&true);
    assert!(load_policy(&env).unwrap().creation_paused);

    factory.set_factory_paused(&false);
    assert!(!load_policy(&env).unwrap().creation_paused);
}

/// `FactoryPolicy` instances returned from `load_policy` comparing equal must
/// be a strict struct-equality (not e.g. partial memcmp) so callers can
/// rely on field-by-field exactness for snapshot tests.
#[test]
fn test_load_policy_equality_is_struct_equality() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);
    factory.set_rate_bounds(&Some(10), &Some(100));

    let p1 = load_policy(&env).unwrap();
    let p2 = load_policy(&env).unwrap();
    assert_eq!(p1, p2);

    let admin_addr = p1.stream_contract.clone();
    let mut different = p1.clone();
    different.stream_contract = admin_addr; // identical -> still equal
    assert_eq!(p1, different);

    // Flip a single field — equality must break.
    let mut different = p1.clone();
    different.max_deposit += 1;
    assert_ne!(p1, different);
}
