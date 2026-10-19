//! Optional rate-per-second bounds (issue #1794).
//!
//! The factory may constrain the streaming rate a factory-mediated creation is
//! allowed to request. Both bounds are optional — `None` on a side means that
//! side is unrestricted — and the pair is set atomically, so there is no way to
//! half-update an interval. This module pins:
//!
//! * every shape of the pair round-trips, including the two one-sided forms;
//! * supplied sides replace their stored values while omitted sides stay unchanged;
//!   no update leaves behind an undecodable sentinel;
//! * `0` and equal bounds are valid — an inclusive interval may be a point;
//! * an inverted interval or a negative bound is refused with the named
//!   `InvalidRateBounds` (discriminant 6) and leaves storage untouched;
//! * setting bounds disturbs no other axis, and re-extends the instance TTL;
//! * before `init` the setter reports `NotInitialized`;
//! * only the current admin may call it.

use fluxora_factory::{load_policy, FactoryError, FluxoraFactory, FluxoraFactoryClient};
use soroban_sdk::testutils::{Address as _, Ledger as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};
use std::panic::AssertUnwindSafe;

/// One initialised factory.
struct Fixture {
    env: Env,
    fid: Address,
    admin: Address,
    stream_contract: Address,
}

impl Fixture {
    fn new() -> Fixture {
        let env = Env::default();
        env.mock_all_auths();
        let fid = env.register(FluxoraFactory, ());
        let admin = Address::generate(&env);
        let stream_contract = Address::generate(&env);
        let factory = FluxoraFactoryClient::new(&env, &fid);
        factory.init(&admin, &stream_contract, &10_000, &100);
        Fixture {
            env,
            fid,
            admin,
            stream_contract,
        }
    }

    fn factory(&self) -> FluxoraFactoryClient<'_> {
        FluxoraFactoryClient::new(&self.env, &self.fid)
    }

    /// The pair as the config view reports it.
    fn bounds(&self) -> (Option<i128>, Option<i128>) {
        let config = self.factory().get_factory_config();
        (config.min_rate_per_second, config.max_rate_per_second)
    }

    /// The pair as the creation paths see it, through the shared chokepoint.
    fn policy_bounds(&self) -> (Option<i128>, Option<i128>) {
        let policy = self
            .env
            .as_contract(&self.fid, || load_policy(&self.env))
            .expect("policy loads once initialised");
        (policy.min_rate_per_second, policy.max_rate_per_second)
    }

    /// Advance the ledger, so a follow-up read also proves the entry survived.
    fn advance(&self, ledgers: u32) {
        self.env
            .ledger()
            .set_sequence_number(self.env.ledger().sequence() + ledgers);
    }
}

fn assert_auth_fails<F: FnOnce()>(f: F) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    assert!(
        result.is_err(),
        "expected auth failure (panic) but call succeeded"
    );
}

/// Every initial bound shape round-trips, including one-sided intervals.
#[test]
fn every_shape_of_the_pair_round_trips() {
    for (min, max) in [
        (None, None),
        (Some(7), None),
        (None, Some(9_999)),
        (Some(1), Some(i128::MAX)),
    ] {
        let f = Fixture::new();
        f.factory().set_rate_bounds(&min, &max);
        f.advance(10_000);
        assert_eq!(f.bounds(), (min, max));
        assert_eq!(f.policy_bounds(), (min, max));
        assert_eq!(
            f.factory().get_factory_config().stream_contract,
            f.stream_contract
        );
    }
}

/// The documented setter updates only supplied sides; None preserves storage.
#[test]
fn supplied_bounds_replace_values_and_none_preserves_them() {
    let f = Fixture::new();
    let factory = f.factory();
    factory.set_rate_bounds(&Some(10), &Some(100));
    factory.set_rate_bounds(&Some(20), &None);
    assert_eq!(f.bounds(), (Some(20), Some(100)));
    factory.set_rate_bounds(&None, &Some(200));
    assert_eq!(f.bounds(), (Some(20), Some(200)));
    factory.set_rate_bounds(&None, &None);
    assert_eq!(f.bounds(), (Some(20), Some(200)));
    assert_eq!(f.policy_bounds(), f.bounds());
    factory.set_rate_bounds(&Some(2), &Some(3));
    factory.set_rate_bounds(&Some(4), &Some(5));
    assert_eq!(f.bounds(), (Some(4), Some(5)));
    assert_eq!(f.policy_bounds(), (Some(4), Some(5)));
}

/// Inclusive intervals permit zero and equal bounds, including partial updates.
#[test]
fn zero_and_equal_bounds_are_accepted() {
    let f = Fixture::new();
    let factory = f.factory();
    factory.set_rate_bounds(&Some(0), &Some(0));
    assert_eq!(f.bounds(), (Some(0), Some(0)));
    factory.set_rate_bounds(&Some(5), &Some(5));
    assert_eq!(f.bounds(), (Some(5), Some(5)));
    factory.set_rate_bounds(&Some(0), &None);
    assert_eq!(f.bounds(), (Some(0), Some(5)));
    factory.set_rate_bounds(&None, &Some(0));
    assert_eq!(f.bounds(), (Some(0), Some(0)));
}

/// An omitted bound must never write an undecodable Void sentinel over a value.
#[test]
fn omitted_bounds_preserve_decodable_policy_values() {
    let f = Fixture::new();
    let factory = f.factory();
    factory.set_rate_bounds(&Some(1), &Some(100));
    factory.set_rate_bounds(&None, &None);
    assert_eq!(f.bounds(), (Some(1), Some(100)));
    assert_eq!(f.policy_bounds(), (Some(1), Some(100)));
    assert!(!factory.is_factory_paused());
    factory.assert_creation_allowed();
    factory.set_rate_bounds(&Some(2), &None);
    assert_eq!(f.bounds(), (Some(2), Some(100)));
    factory.set_rate_bounds(&None, &Some(200));
    assert_eq!(f.bounds(), (Some(2), Some(200)));
    assert_eq!(f.policy_bounds(), (Some(2), Some(200)));
}

/// An inverted interval (`min > max`) is refused with the named
/// `InvalidRateBounds` (discriminant 6), and a refused call leaves the stored
/// interval exactly as it was — validation runs before either side is written,
/// so a rejected update can never be half-applied.
#[test]
fn an_inverted_interval_is_rejected_without_touching_storage() {
    let f = Fixture::new();
    let factory = f.factory();
    factory.set_rate_bounds(&Some(10), &Some(100));

    for (min, max) in [
        (Some(101), None),
        (None, Some(9)),
        (Some(200), Some(50)),
        (Some(101), Some(100)),
        (Some(i128::MAX), Some(0)),
        (Some(1), Some(0)),
    ] {
        assert_eq!(
            factory
                .try_set_rate_bounds(&min, &max)
                .unwrap_err()
                .unwrap(),
            FactoryError::InvalidRateBounds,
            "min {min:?} above max {max:?} must be refused",
        );
        assert_eq!(
            f.bounds(),
            (Some(10), Some(100)),
            "a refused call must leave both bounds untouched",
        );
    }
    assert_eq!(FactoryError::InvalidRateBounds as u32, 6);

    // The same inverted pair is fine once the sides are ordered, so the refusal
    // is about the interval and not about the values.
    factory.set_rate_bounds(&Some(50), &Some(200));
    assert_eq!(f.bounds(), (Some(50), Some(200)));
}

/// A negative bound is refused on either side, and a negative pair is refused
/// for being negative rather than for being inverted. As above, storage is
/// untouched by a refusal.
#[test]
fn a_negative_bound_is_rejected_on_either_side() {
    let f = Fixture::new();
    let factory = f.factory();
    factory.set_rate_bounds(&Some(100), &Some(200));

    for (min, max) in [
        (Some(-1), Some(200)),
        (Some(100), Some(-1)),
        (Some(-1), Some(-1)),
        (Some(-1), None),
        (None, Some(-1)),
        (Some(i128::MIN), Some(i128::MAX)),
    ] {
        assert_eq!(
            factory
                .try_set_rate_bounds(&min, &max)
                .unwrap_err()
                .unwrap(),
            FactoryError::InvalidRateBounds,
            "a negative bound must be refused: {min:?}..={max:?}",
        );
        assert_eq!(
            f.bounds(),
            (Some(100), Some(200)),
            "a refused call must leave both bounds untouched",
        );
    }

    // `0` is the boundary and is accepted on both sides.
    factory.set_rate_bounds(&Some(0), &Some(0));
    assert_eq!(f.bounds(), (Some(0), Some(0)));
}

/// The rate interval is independent of every other axis: setting it disturbs
/// nothing else, and moving the other axes neither sets nor clears it.
#[test]
fn rate_bounds_do_not_disturb_the_other_axes() {
    let f = Fixture::new();
    let factory = f.factory();

    let new_stream_contract = Address::generate(&f.env);
    let new_admin = Address::generate(&f.env);
    factory.set_stream_contract(&new_stream_contract);
    factory.set_cap(&7_500);
    factory.set_min_duration(&250);
    factory.set_batch_cap_enforcement(&false);
    factory.set_admin(&new_admin);

    let before = factory.get_factory_config();
    factory.set_rate_bounds(&Some(11), &Some(1_000));
    let after = factory.get_factory_config();

    assert_eq!(after.min_rate_per_second, Some(11));
    assert_eq!(after.max_rate_per_second, Some(1_000));
    assert_eq!(after.admin, before.admin);
    assert_eq!(after.stream_contract, before.stream_contract);
    assert_eq!(after.max_deposit, before.max_deposit);
    assert_eq!(after.min_duration, before.min_duration);
    assert_eq!(after.batch_cap_enforced, before.batch_cap_enforced);
    assert_eq!(after.creation_paused, before.creation_paused);

    // Moving every other axis leaves the interval alone.
    factory.set_cap(&1);
    factory.set_min_duration(&0);
    factory.set_batch_cap_enforcement(&true);
    factory.set_stream_contract(&Address::generate(&f.env));
    factory.set_factory_paused(&true);
    assert_eq!(f.bounds(), (Some(11), Some(1_000)));

    assert_eq!(f.policy_bounds(), (Some(11), Some(1_000)));
}

/// Before `init` there is no policy to update, so the setter reports the typed
/// `NotInitialized` (discriminant 2) and stores nothing.
#[test]
fn rate_bounds_before_init_are_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register(FluxoraFactory, ());
    let factory = FluxoraFactoryClient::new(&env, &fid);

    assert_eq!(
        factory
            .try_set_rate_bounds(&Some(1), &Some(2))
            .unwrap_err()
            .unwrap(),
        FactoryError::NotInitialized,
    );
    assert_eq!(FactoryError::NotInitialized as u32, 2);

    // Nothing was written, so `init` still succeeds and the interval starts
    // absent.
    let admin = Address::generate(&env);
    let stream_contract = Address::generate(&env);
    factory.init(&admin, &stream_contract, &10_000, &100);
    let config = factory.get_factory_config();
    assert_eq!(config.min_rate_per_second, None);
    assert_eq!(config.max_rate_per_second, None);
}

/// The interval is admin-only, and the privilege follows a rotation: a
/// non-admin caller is rejected and the stored interval is untouched, while the
/// new admin's call succeeds in the same ledger the old admin's would fail in.
#[test]
fn setting_rate_bounds_requires_the_admin() {
    let f = Fixture::new();
    let factory = f.factory();
    factory.set_rate_bounds(&Some(10), &Some(100));

    let non_admin = Address::generate(&f.env);
    f.env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &f.fid,
            fn_name: "set_rate_bounds",
            args: (Some(1i128), Some(2i128)).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_rate_bounds(&Some(1), &Some(2)));
    assert_eq!(
        f.bounds(),
        (Some(10), Some(100)),
        "a rejected call must not change the policy",
    );

    // Rotate: the old admin loses the privilege, the new one gains it.
    f.env.mock_all_auths();
    let new_admin = Address::generate(&f.env);
    let old_admin = f.admin.clone();
    factory.set_admin(&new_admin);

    f.env.mock_auths(&[MockAuth {
        address: &old_admin,
        invoke: &MockAuthInvoke {
            contract: &f.fid,
            fn_name: "set_rate_bounds",
            args: (Some(3i128), Some(4i128)).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_rate_bounds(&Some(3), &Some(4)));
    assert_eq!(f.bounds(), (Some(10), Some(100)));

    f.env.mock_all_auths();
    factory.set_rate_bounds(&Some(3), &Some(4));
    assert_eq!(f.bounds(), (Some(3), Some(4)));
}
