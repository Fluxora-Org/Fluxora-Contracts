//! `load_policy` and the `FactoryPolicy` structure (issue #1793).
//!
//! `load_policy` is the single read chokepoint every creation path goes through:
//! it reads the whole policy in one pass so no path can consult a subset and
//! silently skip a constraint. This module pins:
//!
//! * the loaded policy agrees with the `get_factory_config` view on every field
//!   it shares, and reflects each setter immediately;
//! * each of the four **required** axes is genuinely required — removing any one
//!   of them makes the load fail with `NotInitialized` rather than yield a
//!   default;
//! * the **optional** axes are absent by default and read as permissive values
//!   (`false`, `None`), never as zeros;
//! * on an uninitialised factory the load fails with the named
//!   `NotInitialized` (discriminant 2) instead of returning a partial policy;
//! * `FactoryPolicy` equality is structural, so callers may compare snapshots.

use fluxora_factory::{
    load_policy, DataKey, FactoryError, FactoryPolicy, FluxoraFactory, FluxoraFactoryClient,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

/// One initialised factory.
struct Fixture {
    env: Env,
    fid: Address,
    admin: Address,
}

impl Fixture {
    fn new() -> Fixture {
        let env = Env::default();
        env.mock_all_auths();
        let fid = env.register(FluxoraFactory, ());
        let admin = Address::generate(&env);
        let stream_contract = Address::generate(&env);
        FluxoraFactoryClient::new(&env, &fid).init(&admin, &stream_contract, &10_000, &100);
        Fixture { env, fid, admin }
    }

    fn factory(&self) -> FluxoraFactoryClient<'_> {
        FluxoraFactoryClient::new(&self.env, &self.fid)
    }

    /// The policy exactly as a creation path loads it: with the factory as the
    /// current contract.
    fn policy(&self) -> Result<FactoryPolicy, FactoryError> {
        self.env.as_contract(&self.fid, || load_policy(&self.env))
    }

    /// Remove a required axis from instance storage, simulating a factory whose
    /// bootstrap was incomplete.
    fn remove(&self, key: &DataKey) {
        self.env.as_contract(&self.fid, || {
            self.env.storage().instance().remove(key);
        });
    }
}

/// The policy a creation path loads must agree, field for field, with the
/// `get_factory_config` view — otherwise a path that consults the chokepoint and
/// a dashboard that reads the view could disagree about what is enforced.
///
/// Every setter is exercised first, so the agreement is asserted on a policy
/// that is fully off its defaults, and the closing block shows that
/// `FactoryPolicy` equality really is structural.
#[test]
fn load_policy_agrees_with_the_config_view_on_every_field() {
    let f = Fixture::new();
    let factory = f.factory();

    let new_stream_contract = Address::generate(&f.env);
    factory.set_stream_contract(&new_stream_contract);
    factory.set_cap(&7_500);
    factory.set_min_duration(&250);
    factory.set_batch_cap_enforcement(&false);
    factory.set_factory_paused(&true);
    factory.set_rate_bounds(&Some(50), &Some(1_000));

    let policy = f.policy().expect("the policy loads");
    let config = factory.get_factory_config();

    assert_eq!(policy.stream_contract, config.stream_contract);
    assert_eq!(policy.max_deposit, config.max_deposit);
    assert_eq!(policy.min_duration, config.min_duration);
    assert_eq!(policy.batch_cap_enforced, config.batch_cap_enforced);
    assert_eq!(policy.creation_paused, config.creation_paused);
    assert_eq!(policy.min_rate_per_second, config.min_rate_per_second);
    assert_eq!(policy.max_rate_per_second, config.max_rate_per_second);

    // And they are the values that were actually set, not a coincidence of
    // both being empty.
    assert_eq!(policy.stream_contract, new_stream_contract);
    assert_eq!(policy.max_deposit, 7_500);
    assert_eq!(policy.min_duration, 250);
    assert!(!policy.batch_cap_enforced);
    assert!(policy.creation_paused);
    assert_eq!(policy.min_rate_per_second, Some(50));
    assert_eq!(policy.max_rate_per_second, Some(1_000));

    // The view carries exactly one field the policy does not: the admin. The
    // policy describes *what is enforced*, not *who may change it*.
    assert_eq!(
        config.admin, f.admin,
        "the admin belongs to the view, not to the policy",
    );

    // Structural equality: an unchanged load compares equal, and one changed
    // field compares unequal.
    factory.set_rate_bounds(&Some(10), &Some(100));
    let first = f.policy().expect("loads");
    let second = f.policy().expect("loads");
    assert_eq!(first, second);

    let mut flipped = first.clone();
    flipped.max_deposit += 1;
    assert_ne!(first, flipped, "equality is field-by-field");
    let mut flipped = first.clone();
    flipped.max_rate_per_second = Some(1);
    assert_ne!(first, flipped);
}

/// Each of the four required axes is genuinely required: remove any one of them
/// and the load reports `NotInitialized` instead of substituting a default.
///
/// A default here would be dangerous in exactly the direction a policy must not
/// fail — an absent cap read as unlimited, or an absent duration floor read as
/// zero, would admit streams the operator never authorised.
#[test]
fn every_required_axis_must_be_present() {
    // The complete factory loads.
    assert!(Fixture::new().policy().is_ok());

    for key in [
        DataKey::StreamContract,
        DataKey::MaxDepositCap,
        DataKey::MinDuration,
        DataKey::BatchCapEnforced,
    ] {
        let f = Fixture::new();
        f.remove(&key);
        assert_eq!(
            f.policy(),
            Err(FactoryError::NotInitialized),
            "removing {key:?} must make the policy unloadable",
        );

        // The failure is the policy load, not the views: the pause view still
        // answers, because it reads its own (optional) key.
        assert!(!f.factory().is_factory_paused());
    }

    // Removing an *optional* key is not a failure: it is absent on a fresh
    // factory anyway.
    let f = Fixture::new();
    f.remove(&DataKey::CreationPaused);
    assert!(f.policy().is_ok());
}

/// The optional axes are absent by default and read as permissive values:
/// `creation_paused == false` and both rate bounds `None`.
///
/// Reading an absent rate bound as `0` would reject every stream with a
/// `RateBelowMin`-style failure, so this is asserted on the value rather than on
/// the storage.
#[test]
fn absent_optional_axes_default_to_permissive() {
    let f = Fixture::new();
    let factory = f.factory();

    // Freshly initialised: the pause key was never written.
    let policy = f.policy().expect("loads");
    assert!(!policy.creation_paused);
    assert_eq!(policy.min_rate_per_second, None);
    assert_eq!(policy.max_rate_per_second, None);

    // Explicitly storing `false` and clearing the bounds keep the same reading.
    factory.set_factory_paused(&true);
    assert!(f.policy().expect("loads").creation_paused);
    factory.set_factory_paused(&false);
    let policy = f.policy().expect("loads");
    assert!(!policy.creation_paused, "an explicit false is still false");

    factory.set_rate_bounds(&Some(3), &Some(9));
    factory.set_rate_bounds(&None, &None);
    let policy = f.policy().expect("loads");
    assert_eq!(policy.min_rate_per_second, None);
    assert_eq!(policy.max_rate_per_second, None);

    // The defaults are permissive, not "anything goes": a required axis still
    // has to be present for the load to succeed at all.
    assert!(policy.max_deposit > 0);
    assert_eq!(policy.min_duration, 100);
}

/// On an uninitialised factory the load fails with `NotInitialized`
/// (discriminant 2). It does not return a zeroed or partially-filled policy, so
/// a creation path cannot mistake "unconfigured" for "unrestricted".
#[test]
fn load_policy_on_an_uninitialised_factory_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let fid = env.register(FluxoraFactory, ());

    let loaded = env.as_contract(&fid, || load_policy(&env));
    assert_eq!(loaded, Err(FactoryError::NotInitialized));
    assert_eq!(
        FactoryError::NotInitialized as u32,
        2,
        "the discriminant is part of the ABI",
    );

    // Partially initialising is not enough either: the first required read
    // fails, and the factory is still initialisable afterwards.
    env.as_contract(&fid, || {
        env.storage()
            .instance()
            .set(&DataKey::StreamContract, &Address::generate(&env));
    });
    assert_eq!(
        env.as_contract(&fid, || load_policy(&env)),
        Err(FactoryError::NotInitialized),
    );

    let factory = FluxoraFactoryClient::new(&env, &fid);
    let admin = Address::generate(&env);
    let stream_contract = Address::generate(&env);
    factory.init(&admin, &stream_contract, &10_000, &100);
    assert!(env.as_contract(&fid, || load_policy(&env)).is_ok());
}
