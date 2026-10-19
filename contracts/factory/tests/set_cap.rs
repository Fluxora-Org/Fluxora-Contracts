//! Dedicated acceptance tests for issue #1790 — `FluxoraFactory::set_cap` with
//! admin authorisation.
//!
//! `factory_setters.rs` specifies the factory as a whole; this file pins the one
//! setter #1790 is about, so its four acceptance criteria each have a test
//! named after them:
//!
//! * only the current admin may call it (and a rotation moves the privilege);
//! * a non-admin caller is rejected, leaving the stored cap untouched;
//! * calling it before `init` returns `FactoryError::NotInitialized`;
//! * the cap round-trips through `get_factory_config`.
//!
//! The boundary cases at the end pin the guard the setter adds on top of the
//! raw storage write: a cap below the minimum is a misconfiguration that would
//! reject every stream, so it is refused rather than stored, and a refused call
//! must not leave a half-applied policy behind.

#![cfg(test)]

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient, MIN_DEPOSIT_CAP};
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};
use std::panic::AssertUnwindSafe;

/// Assert a call panics — the Soroban test host's behaviour for an unsatisfied
/// `require_auth`.
fn assert_auth_fails<F: FnOnce()>(f: F) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    assert!(
        result.is_err(),
        "expected auth failure (panic) but the call succeeded"
    );
}

/// Initialise a factory and return the pieces every test needs.
fn init_factory(env: &Env) -> (Address, FluxoraFactoryClient<'static>, Address, Address) {
    let fid = env.register(FluxoraFactory, ());
    let factory = FluxoraFactoryClient::new(env, &fid);
    let admin = Address::generate(env);
    let sc = Address::generate(env);
    factory.init(&admin, &sc, &10_000, &100);
    (fid, factory, admin, sc)
}

/// The cap round-trips through `get_factory_config`, and the previous value is
/// gone — `set_cap` replaces rather than appends.
#[test]
fn test_set_cap_round_trips_through_get_factory_config() {
    let env = Env::default();
    env.mock_all_auths();
    let (_fid, factory, _admin, _sc) = init_factory(&env);

    assert_eq!(factory.get_factory_config().max_deposit, 10_000);

    factory.set_cap(&7_500);
    assert_eq!(
        factory.get_factory_config().max_deposit,
        7_500,
        "the new cap must be what the config view reports"
    );

    // A second call replaces it again; nothing accumulates.
    factory.set_cap(&2_000);
    assert_eq!(factory.get_factory_config().max_deposit, 2_000);
}

/// Only the current admin may call `set_cap`: after a rotation the *new* admin
/// sets the cap and the *old* admin is locked out.
#[test]
fn test_set_cap_follows_admin_rotation() {
    let env = Env::default();
    let fid = env.register(FluxoraFactory, ());
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let old_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let sc = Address::generate(&env);

    env.mock_auths(&[MockAuth {
        address: &old_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "init",
            args: (&old_admin, &sc, 10_000i128, 100u64).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    factory.init(&old_admin, &sc, &10_000, &100);

    // Rotate, authorised by the outgoing admin.
    env.mock_auths(&[MockAuth {
        address: &old_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_admin",
            args: (&new_admin,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    factory.set_admin(&new_admin);

    // The old admin may no longer move the cap…
    env.mock_auths(&[MockAuth {
        address: &old_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_cap",
            args: (1_234i128,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_cap(&1_234));
    assert_eq!(
        factory.get_factory_config().max_deposit,
        10_000,
        "a rejected call must not have moved the cap"
    );

    // …but the new admin can, in the same ledger.
    env.mock_auths(&[MockAuth {
        address: &new_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_cap",
            args: (3_000i128,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    factory.set_cap(&3_000);
    assert_eq!(factory.get_factory_config().max_deposit, 3_000);
}

/// A non-admin caller is rejected, and the rejection is atomic: the cap is
/// exactly what it was before the attempted call.
#[test]
fn test_set_cap_rejects_non_admin_and_leaves_the_cap_untouched() {
    let env = Env::default();
    let fid = env.register(FluxoraFactory, ());
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

    // Re-enable the permissive mocking just to read the state back: the value
    // must be the original 10_000, not the rejected 5_000.
    env.mock_all_auths();
    assert_eq!(
        factory.get_factory_config().max_deposit,
        10_000,
        "a rejected non-admin call must not change the stored cap"
    );
}

/// Before `init` there is no admin to authenticate against, so `set_cap` fails
/// with the typed pre-init error rather than an opaque auth trap.
#[test]
fn test_set_cap_before_init_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register(FluxoraFactory, ());
    let factory = FluxoraFactoryClient::new(&env, &fid);

    assert_eq!(
        factory.try_set_cap(&1_000),
        Err(Ok(FactoryError::NotInitialized)),
    );
    assert_eq!(FactoryError::NotInitialized as u32, 2);
}

/// The minimum accepted cap is `MIN_DEPOSIT_CAP`; anything below it is refused
/// and the previously stored cap survives.
#[test]
fn test_set_cap_below_minimum_is_refused_atomically() {
    let env = Env::default();
    env.mock_all_auths();
    let (_fid, factory, _admin, _sc) = init_factory(&env);

    for bad in [0i128, -1, i128::MIN] {
        assert_eq!(
            factory.try_set_cap(&bad),
            Err(Ok(FactoryError::InvalidCap)),
            "a cap of {bad} would reject every stream and must be refused",
        );
    }

    assert_eq!(
        factory.get_factory_config().max_deposit,
        10_000,
        "a refused cap must leave the stored policy unchanged"
    );
}

/// The boundary is inclusive: `MIN_DEPOSIT_CAP` itself is accepted, as is
/// `i128::MAX`.
#[test]
fn test_set_cap_accepts_both_boundaries() {
    let env = Env::default();
    env.mock_all_auths();
    let (_fid, factory, _admin, _sc) = init_factory(&env);

    factory.set_cap(&MIN_DEPOSIT_CAP);
    assert_eq!(factory.get_factory_config().max_deposit, MIN_DEPOSIT_CAP);

    factory.set_cap(&i128::MAX);
    assert_eq!(factory.get_factory_config().max_deposit, i128::MAX);

    // Setting the value already stored is a successful no-op, not an error.
    factory.set_cap(&i128::MAX);
    assert_eq!(factory.get_factory_config().max_deposit, i128::MAX);
}

/// `set_cap` is a state change, so it must refresh the instance entry — an
/// actively-administered factory must never let its own config archive.
#[test]
fn test_set_cap_bumps_instance_ttl() {
    use soroban_sdk::testutils::storage::Instance as _;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_max_entry_ttl(50_000);
    let (fid, factory, _admin, _sc) = init_factory(&env);

    let full_ttl = env.as_contract(&fid, || env.storage().instance().get_ttl());
    env.ledger()
        .set_sequence_number(env.ledger().sequence() + full_ttl - 1_000);
    let decayed = env.as_contract(&fid, || env.storage().instance().get_ttl());
    assert!(decayed < 2_000, "entry should be nearly expired: {decayed}");

    factory.set_cap(&1_500);
    let after = env.as_contract(&fid, || env.storage().instance().get_ttl());
    assert!(
        after > decayed + 40_000,
        "set_cap must re-extend the instance entry, {decayed} -> {after}",
    );
    assert_eq!(factory.get_factory_config().max_deposit, 1_500);
}
