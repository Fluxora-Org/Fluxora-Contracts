//! Dedicated acceptance tests for issue #1791 — `FluxoraFactory::set_min_duration`
//! with admin authorisation.
//!
//! `factory_setters.rs` specifies the factory as a whole; this file pins the one
//! setter #1791 is about, so its acceptance criteria each have a test named
//! after them:
//!
//! * only the current admin may call it (and a rotation moves the privilege);
//! * a non-admin caller is rejected, leaving the stored duration untouched;
//! * calling it before `init` returns `FactoryError::NotInitialized`;
//! * the minimum duration round-trips through `get_factory_config`.
//!
//! The boundary cases at the end pin the guard the setter adds on top of the
//! raw storage write: `0` deliberately disables the factory-level minimum, a
//! value at [`MAX_MIN_DURATION_SECONDS`] is accepted, and anything above it is
//! refused rather than stored, atomically.

#![cfg(test)]

use fluxora_factory::{
    FactoryError, FluxoraFactory, FluxoraFactoryClient, MAX_MIN_DURATION_SECONDS,
};
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

/// The minimum duration round-trips through `get_factory_config`, and the
/// previous value is gone — `set_min_duration` replaces rather than appends.
#[test]
fn test_set_min_duration_round_trips_through_get_factory_config() {
    let env = Env::default();
    env.mock_all_auths();
    let (_fid, factory, _admin, _sc) = init_factory(&env);

    assert_eq!(factory.get_factory_config().min_duration, 100);

    factory.set_min_duration(&300);
    assert_eq!(
        factory.get_factory_config().min_duration,
        300,
        "the new minimum duration must be what the config view reports"
    );

    // A second call replaces it again; nothing accumulates.
    factory.set_min_duration(&3_600);
    assert_eq!(factory.get_factory_config().min_duration, 3_600);
}

/// Only the current admin may call `set_min_duration`: after a rotation the
/// *new* admin sets the duration and the *old* admin is locked out.
#[test]
fn test_set_min_duration_follows_admin_rotation() {
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

    // The old admin may no longer move the minimum duration…
    env.mock_auths(&[MockAuth {
        address: &old_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_min_duration",
            args: (1_234u64,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_min_duration(&1_234));
    assert_eq!(
        factory.get_factory_config().min_duration,
        100,
        "a rejected call must not have moved the minimum duration"
    );

    // …but the new admin can, in the same ledger.
    env.mock_auths(&[MockAuth {
        address: &new_admin,
        invoke: &MockAuthInvoke {
            contract: &fid,
            fn_name: "set_min_duration",
            args: (600u64,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    factory.set_min_duration(&600);
    assert_eq!(factory.get_factory_config().min_duration, 600);
}

/// A non-admin caller is rejected, and the rejection is atomic: the minimum
/// duration is exactly what it was before the attempted call.
#[test]
fn test_set_min_duration_rejects_non_admin_and_leaves_the_duration_untouched() {
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
            fn_name: "set_min_duration",
            args: (500u64,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_min_duration(&500));

    // Re-enable the permissive mocking just to read the state back: the value
    // must be the original 100, not the rejected 500.
    env.mock_all_auths();
    assert_eq!(
        factory.get_factory_config().min_duration,
        100,
        "a rejected non-admin call must not change the stored minimum duration"
    );
}

/// Before `init` there is no admin to authenticate against, so
/// `set_min_duration` fails with the typed pre-init error rather than an opaque
/// auth trap.
#[test]
fn test_set_min_duration_before_init_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register(FluxoraFactory, ());
    let factory = FluxoraFactoryClient::new(&env, &fid);

    assert_eq!(
        factory.try_set_min_duration(&100),
        Err(Ok(FactoryError::NotInitialized)),
    );
    assert_eq!(FactoryError::NotInitialized as u32, 2);
}

/// A duration above [`MAX_MIN_DURATION_SECONDS`] is refused and the previously
/// stored policy survives.
#[test]
fn test_set_min_duration_above_maximum_is_refused_atomically() {
    let env = Env::default();
    env.mock_all_auths();
    let (_fid, factory, _admin, _sc) = init_factory(&env);

    for bad in [MAX_MIN_DURATION_SECONDS + 1, u64::MAX] {
        assert_eq!(
            factory.try_set_min_duration(&bad),
            Err(Ok(FactoryError::InvalidMinDuration)),
            "a minimum duration of {bad} exceeds the ceiling and must be refused",
        );
    }

    assert_eq!(
        factory.get_factory_config().min_duration,
        100,
        "a refused duration must leave the stored policy unchanged"
    );
}

/// The accepted interval is `0..=MAX_MIN_DURATION_SECONDS`: `0` disables the
/// factory-level minimum and the ceiling itself is accepted.
#[test]
fn test_set_min_duration_accepts_both_boundaries() {
    let env = Env::default();
    env.mock_all_auths();
    let (_fid, factory, _admin, _sc) = init_factory(&env);

    // Zero is the documented "no factory-level minimum" value, not an error.
    factory.set_min_duration(&0);
    assert_eq!(factory.get_factory_config().min_duration, 0);

    factory.set_min_duration(&MAX_MIN_DURATION_SECONDS);
    assert_eq!(
        factory.get_factory_config().min_duration,
        MAX_MIN_DURATION_SECONDS
    );

    // Setting the value already stored is a successful no-op, not an error.
    factory.set_min_duration(&MAX_MIN_DURATION_SECONDS);
    assert_eq!(
        factory.get_factory_config().min_duration,
        MAX_MIN_DURATION_SECONDS
    );
}

/// `set_min_duration` is a state change, so it must refresh the instance
/// entry — an actively-administered factory must never let its own config
/// archive.
#[test]
fn test_set_min_duration_bumps_instance_ttl() {
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

    factory.set_min_duration(&250);
    let after = env.as_contract(&fid, || env.storage().instance().get_ttl());
    assert!(
        after > decayed + 40_000,
        "set_min_duration must re-extend the instance entry, {decayed} -> {after}",
    );
    assert_eq!(factory.get_factory_config().min_duration, 250);
}
