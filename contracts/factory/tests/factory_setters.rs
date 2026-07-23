//! Direct unit tests for `FluxoraFactory` admin setters and config views.
//!
//! Covers issue #684: `set_admin`, `set_cap`, `set_min_duration`, `set_allowlist`,
//! `is_allowlisted`, and `get_factory_config` — including auth enforcement,
//! `NotInitialized` / `AlreadyInitialized` branches, and admin rotation.

#![cfg(test)]

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
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
    assert!(
        result.is_err(),
        "expected auth failure (panic) but call succeeded"
    );
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
