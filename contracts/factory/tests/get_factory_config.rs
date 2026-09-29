//! Dedicated acceptance tests for issue #1787 — `get_factory_config` and its
//! uninitialised branch.
//!
//! `factory_setters.rs` specifies the factory as a whole. This file pins the
//! one entry point #1787 is about, on its own terms, so the behaviour cannot
//! regress without a test named after it failing:
//!
//! * the returned config carries **admin**, **stream contract**, **max deposit**
//!   and **min duration**;
//! * calling it before `init` returns `FactoryError::NotInitialized`;
//! * the view performs **no storage writes** — measured as the instance entry's
//!   TTL and the ledger sequence being untouched after repeated calls.
//!
//! The no-write property is the one that is easy to lose. `get_factory_config`
//! is called through simulation by SDKs and dashboards; a view that paid rent
//! or bumped a TTL would be an observable behaviour change for a caller who only
//! wanted to read, exactly the rule `test::read_methods_no_side_effects` pins
//! for the stream contract (#1686).

#![cfg(test)]

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{Address, Env};

/// Read the instance entry's remaining TTL from inside the factory's own
/// context, the way a contract call would see it.
fn instance_ttl(env: &Env, fid: &Address) -> u32 {
    env.as_contract(fid, || env.storage().instance().get_ttl())
}

/// The returned config carries all four fields #1787 names.
#[test]
fn test_get_factory_config_carries_admin_stream_contract_cap_and_min_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &5_000, &200);

    let cfg = factory.get_factory_config();
    assert_eq!(cfg.admin, admin, "config must report the stored admin");
    assert_eq!(
        cfg.stream_contract, sc,
        "config must report the downstream stream contract"
    );
    assert_eq!(cfg.max_deposit, 5_000, "config must report the deposit cap");
    assert_eq!(
        cfg.min_duration, 200,
        "config must report the duration floor"
    );
}

/// The same four fields survive extreme-but-valid values, so the view is not
/// quietly widening or narrowing an `i128`/`u64` on the way out.
#[test]
fn test_get_factory_config_round_trips_large_and_zero_valued_fields() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    // Largest cap the setter accepts, and a duration floor of zero (disabled).
    factory.init(&admin, &sc, &i128::MAX, &0);

    let cfg = factory.get_factory_config();
    assert_eq!(cfg.max_deposit, i128::MAX, "i128::MAX cap must round-trip");
    assert_eq!(cfg.min_duration, 0, "a zero floor means 'no floor'");
}

/// Before `init` the view must fail with the typed pre-init error, not panic
/// and not invent a default configuration.
#[test]
fn test_get_factory_config_before_init_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);

    let result = factory.try_get_factory_config();
    assert_eq!(
        result,
        Err(Ok(FactoryError::NotInitialized)),
        "an uninitialised factory has no configuration to report"
    );
    // The discriminant is ABI: a client branches on the number, so pin it.
    assert_eq!(FactoryError::NotInitialized as u32, 2);
}

/// After a failed `get_factory_config` the factory is still uninitialised —
/// a view that failed partway must not have written anything.
#[test]
fn test_failed_get_factory_config_leaves_factory_uninitialised() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);

    assert!(factory.try_get_factory_config().is_err());
    assert!(factory.try_get_factory_config().is_err());

    // A real `init` therefore still succeeds — nothing was half-written.
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);
    factory.init(&admin, &sc, &1_000, &10);
    assert_eq!(factory.get_factory_config().max_deposit, 1_000);
}

/// The view performs no storage writes: repeated calls leave the instance
/// entry's TTL and the ledger sequence exactly where they were.
///
/// A read that accidentally extended the entry (or wrote a value back) would
/// move `instance_ttl` or the sequence, so both are compared as exact integers.
#[test]
fn test_get_factory_config_performs_no_storage_writes() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &5_000, &200);

    let ttl_before = instance_ttl(&env, &fid);
    let sequence_before = env.ledger().sequence();

    for _ in 0..8 {
        let cfg = factory.get_factory_config();
        assert_eq!(cfg.max_deposit, 5_000);
    }

    assert_eq!(
        instance_ttl(&env, &fid),
        ttl_before,
        "get_factory_config must not extend the instance TTL",
    );
    assert_eq!(
        env.ledger().sequence(),
        sequence_before,
        "get_factory_config must not advance the ledger",
    );

    // And the stored values are demonstrably untouched: they still read back
    // exactly as written.
    let cfg = factory.get_factory_config();
    assert_eq!(cfg.admin, admin);
    assert_eq!(cfg.stream_contract, sc);
    assert_eq!(cfg.max_deposit, 5_000);
    assert_eq!(cfg.min_duration, 200);
}

/// A TTL that has decayed is *not* repaired by reading — the view is inert.
///
/// This is the sharper form of the previous test: with a small maximum entry
/// TTL, a mutating call visibly re-extends the entry, so the assertion that a
/// read does not is meaningful rather than accidentally true.
#[test]
fn test_get_factory_config_does_not_restore_a_decayed_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_max_entry_ttl(50_000);
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &5_000, &200);
    let full_ttl = instance_ttl(&env, &fid);
    assert!(
        full_ttl > 40_000,
        "init should fund a full window: {full_ttl}"
    );

    // Let the entry decay to a sliver.
    env.ledger()
        .set_sequence_number(env.ledger().sequence() + full_ttl - 1_000);
    let decayed = instance_ttl(&env, &fid);
    assert!(decayed < 2_000, "entry should be nearly expired: {decayed}");

    // The view must leave the decayed TTL alone.
    for _ in 0..4 {
        let _ = factory.get_factory_config();
    }
    assert_eq!(
        instance_ttl(&env, &fid),
        decayed,
        "get_factory_config must not re-extend a decayed instance entry",
    );

    // Whereas an admin setter is a state change and does re-extend it.
    factory.set_cap(&6_000);
    assert!(
        instance_ttl(&env, &fid) > decayed + 40_000,
        "a setter must refresh the instance entry",
    );
}

/// `get_factory_config` reports the *effective* configuration, so it tracks
/// each admin setter rather than a snapshot taken at `init`.
#[test]
fn test_get_factory_config_tracks_every_admin_setter() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let sc = Address::generate(&env);

    factory.init(&admin, &sc, &10_000, &100);

    let new_sc = Address::generate(&env);
    factory.set_stream_contract(&new_sc);
    factory.set_cap(&7_500);
    factory.set_min_duration(&250);
    factory.set_batch_cap_enforcement(&false);
    factory.set_factory_paused(&true);
    factory.set_rate_bounds(&Some(50), &Some(1_000));

    let cfg = factory.get_factory_config();
    assert_eq!(cfg.stream_contract, new_sc);
    assert_eq!(cfg.max_deposit, 7_500);
    assert_eq!(cfg.min_duration, 250);
    assert!(!cfg.batch_cap_enforced);
    assert!(cfg.creation_paused);
    assert_eq!(cfg.min_rate_per_second, Some(50));
    assert_eq!(cfg.max_rate_per_second, Some(1_000));
}
