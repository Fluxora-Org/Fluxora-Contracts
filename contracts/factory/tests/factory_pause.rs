//! The factory-level creation pause (issue #1796).
//!
//! `set_factory_paused` is the one policy axis that is a *switch* rather than a
//! value: it decides whether factory-mediated creation is accepted at all. This
//! module pins it end to end through the public ABI:
//!
//! * the guard refuses creation while paused, and stops refusing once unpaused;
//! * the toggle touches the pause bit and nothing else — every other axis is
//!   bit-identical across a flip in both directions;
//! * the guard is a pure read, so a creation path can call it without paying
//!   rent or emitting an event, and the flag survives every other setter;
//! * the pause is a named error (`FactoryPaused`, discriminant 7) so an
//!   integrator can branch on the reason rather than parse a trap;
//! * an uninitialised factory reports `NotInitialized` instead of an auth trap;
//! * a non-admin cannot toggle it in either direction.

use fluxora_factory::{
    load_policy, FactoryConfig, FactoryError, FluxoraFactory, FluxoraFactoryClient,
};
use soroban_sdk::testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Address, Env, IntoVal};
use std::panic::AssertUnwindSafe;

/// One initialised factory, identified by its contract id.
struct Fixture {
    env: Env,
    fid: Address,
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
        Fixture { env, fid }
    }

    /// A registered factory that was never initialised.
    fn uninitialised() -> Fixture {
        let env = Env::default();
        env.mock_all_auths();
        let fid = env.register(FluxoraFactory, ());
        Fixture { env, fid }
    }

    fn factory(&self) -> FluxoraFactoryClient<'_> {
        FluxoraFactoryClient::new(&self.env, &self.fid)
    }

    fn config(&self) -> FactoryConfig {
        self.factory().get_factory_config()
    }

    /// The policy as the creation paths see it, through the shared chokepoint.
    fn policy_paused(&self) -> bool {
        self.env
            .as_contract(&self.fid, || load_policy(&self.env))
            .expect("policy loads once initialised")
            .creation_paused
    }

    /// Whether factory-mediated creation is allowed right now.
    ///
    /// The guard traps on refusal — a creation path cannot continue past a
    /// refused guard — so the boolean form catches that trap.
    fn creation_allowed(&self) -> bool {
        std::panic::catch_unwind(AssertUnwindSafe(|| {
            self.factory().assert_creation_allowed()
        }))
        .is_ok()
    }

    /// The typed error a refused guard reports.
    fn creation_error(&self) -> FactoryError {
        self.factory()
            .try_assert_creation_allowed()
            .unwrap_err()
            .unwrap()
    }

    /// How many events the most recent invocation of this factory emitted.
    ///
    /// The test host exposes only the events of the last invocation, so this is
    /// read straight after the call being pinned — never after an unrelated
    /// call in between.
    fn event_count(&self) -> usize {
        self.env
            .events()
            .all()
            .filter_by_contract(&self.fid)
            .events()
            .len()
    }

    /// Every boolean payload carried by the most recent invocation's events, in
    /// order.
    ///
    /// The pause announcement encodes its state as a boolean field; collecting
    /// every boolean in the topics and body keeps this helper indifferent to
    /// which of the two the SDK puts the field in.
    fn event_bools(&self) -> Vec<bool> {
        let mut out = Vec::new();
        for event in self
            .env
            .events()
            .all()
            .filter_by_contract(&self.fid)
            .events()
            .iter()
        {
            let ContractEventBody::V0(v0) = &event.body;
            for topic in v0.topics.iter() {
                collect_bools(topic, &mut out);
            }
            collect_bools(&v0.data, &mut out);
        }
        out
    }
}

fn collect_bools(value: &ScVal, out: &mut Vec<bool>) {
    match value {
        ScVal::Bool(flag) => out.push(*flag),
        ScVal::Map(Some(map)) => {
            for entry in map.iter() {
                collect_bools(&entry.key, out);
                collect_bools(&entry.val, out);
            }
        }
        ScVal::Vec(Some(items)) => {
            for item in items.iter() {
                collect_bools(item, out);
            }
        }
        _ => {}
    }
}

/// Helper: assert that a closure panics (Soroban testutils behaviour for an
/// unauthorized `require_auth` call).
fn assert_auth_fails<F: FnOnce()>(f: F) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    assert!(
        result.is_err(),
        "expected auth failure (panic) but call succeeded"
    );
}

/// Toggling the pause flips the guard from allowed to refused and back, and the
/// refusal is the *named* `FactoryError::FactoryPaused` carrying the documented
/// discriminant (7) so the ABI is pinned, not just the behaviour.
///
/// The emitted payload is checked too: the event that announces the change must
/// carry the new state, so an off-chain indexer cannot be told the opposite of
/// what the contract now enforces.
#[test]
fn a_paused_factory_refuses_creation_and_unpausing_restores_it() {
    let f = Fixture::new();
    let factory = f.factory();

    assert!(f.creation_allowed(), "creation starts allowed");
    assert!(!factory.is_factory_paused());
    assert!(!f.policy_paused());

    factory.set_factory_paused(&true);
    // Read the announcement immediately: the host only exposes the events of the
    // most recent invocation, so a call in between would hide it.
    assert_eq!(
        f.event_bools(),
        std::vec![true],
        "the event must announce the state that was stored",
    );
    assert!(!f.creation_allowed(), "a paused factory admits nothing");
    assert_eq!(
        f.creation_error(),
        FactoryError::FactoryPaused,
        "the refusal names the pause",
    );
    assert_eq!(
        FactoryError::FactoryPaused as u32,
        7,
        "the discriminant is part of the ABI",
    );
    assert!(factory.is_factory_paused());
    assert!(
        f.policy_paused(),
        "the policy the creation paths read agrees"
    );

    factory.set_factory_paused(&false);
    assert_eq!(
        f.event_bools(),
        std::vec![false],
        "lifting the pause is announced too",
    );
    assert!(f.creation_allowed(), "creation is allowed again");
    assert!(!factory.is_factory_paused());
    assert!(!f.policy_paused());
}

/// The pause is the *only* axis a toggle touches: with every other axis moved
/// off its default, a flip in either direction leaves all of them bit-identical
/// and changes only `creation_paused`.
///
/// This is what makes the pause safe as an incident switch — flipping it can
/// never silently change a cap, a duration, the target stream contract, the
/// batch policy or the rate interval.
#[test]
fn the_pause_is_the_only_policy_axis_a_toggle_touches() {
    let f = Fixture::new();
    let factory = f.factory();

    // Move every other axis off its default.
    let new_stream_contract = Address::generate(&f.env);
    let new_admin = Address::generate(&f.env);
    factory.set_stream_contract(&new_stream_contract);
    factory.set_cap(&7_500);
    factory.set_min_duration(&250);
    factory.set_batch_cap_enforcement(&false);
    factory.set_rate_bounds(&Some(50), &Some(1_000));
    factory.set_admin(&new_admin);

    let before = f.config();
    assert!(!before.creation_paused);

    factory.set_factory_paused(&true);
    let paused = f.config();
    assert!(paused.creation_paused);
    assert_eq!(paused.admin, before.admin);
    assert_eq!(paused.stream_contract, before.stream_contract);
    assert_eq!(paused.max_deposit, before.max_deposit);
    assert_eq!(paused.min_duration, before.min_duration);
    assert_eq!(paused.batch_cap_enforced, before.batch_cap_enforced);
    assert_eq!(paused.min_rate_per_second, before.min_rate_per_second);
    assert_eq!(paused.max_rate_per_second, before.max_rate_per_second);

    factory.set_factory_paused(&false);
    let unpaused = f.config();
    assert!(!unpaused.creation_paused);
    assert_eq!(unpaused, before, "an off-then-on flip is a no-op");

    // The guard depends on the pause and on nothing else: it refuses while
    // paused even though every other axis was moved, and accepts once the pause
    // is off with all of those still in place.
    factory.set_factory_paused(&true);
    assert!(!f.creation_allowed());
    factory.set_factory_paused(&false);
    assert!(f.creation_allowed());
}

/// The pause and the other axes are independent in both directions: changing
/// any axis neither sets nor clears the pause, and the pause survives those
/// changes.
#[test]
fn the_pause_survives_other_policy_changes_and_vice_versa() {
    let f = Fixture::new();
    let factory = f.factory();

    factory.set_factory_paused(&true);
    factory.set_cap(&1);
    factory.set_min_duration(&0);
    factory.set_stream_contract(&Address::generate(&f.env));
    factory.set_batch_cap_enforcement(&true);
    factory.set_rate_bounds(&None, &None);
    factory.set_admin(&Address::generate(&f.env));

    assert!(
        factory.is_factory_paused(),
        "no other setter may clear the pause",
    );
    assert!(!f.creation_allowed());

    factory.set_factory_paused(&false);
    factory.set_cap(&9_999);
    factory.set_min_duration(&3_600);
    factory.set_rate_bounds(&Some(1), &Some(2));
    factory.set_batch_cap_enforcement(&false);

    assert!(
        !factory.is_factory_paused(),
        "no other setter may raise the pause",
    );
    assert!(f.creation_allowed());
}

/// Pausing twice, or unpausing twice, is a successful no-op that still announces
/// itself: the state is unchanged, the guard's answer is unchanged, and exactly
/// one event is emitted per call — a caller retrying a toggle cannot end up with
/// a different state or with a silent call.
#[test]
fn pausing_is_idempotent_in_both_directions() {
    let f = Fixture::new();
    let factory = f.factory();

    // The host exposes only the most recent invocation's events, so each
    // observation follows its own call.
    factory.set_factory_paused(&true);
    assert_eq!(f.event_count(), 1, "a toggle announces once, not twice");
    assert_eq!(f.event_bools(), std::vec![true]);

    factory.set_factory_paused(&true);
    assert_eq!(f.event_count(), 1, "a repeat is announced, not swallowed");
    assert_eq!(f.event_bools(), std::vec![true]);
    assert!(factory.is_factory_paused());
    assert!(!f.creation_allowed());

    factory.set_factory_paused(&false);
    assert_eq!(f.event_count(), 1);
    assert_eq!(f.event_bools(), std::vec![false]);

    factory.set_factory_paused(&false);
    assert_eq!(f.event_count(), 1);
    assert_eq!(f.event_bools(), std::vec![false]);
    assert!(!factory.is_factory_paused());
    assert!(f.creation_allowed());

    let config = f.config();
    assert!(!config.creation_paused);
    assert_eq!(config.max_deposit, 10_000, "no axis drifted");
    assert_eq!(config.min_duration, 100);
}

/// Before `init` there is no policy to consult, so the guard reports the typed
/// `NotInitialized` (discriminant 2) rather than an auth trap or a default of
/// "allowed" — a creation path must never be able to proceed against a factory
/// that was never configured.
#[test]
fn the_guard_before_init_is_not_initialized() {
    let f = Fixture::uninitialised();
    let factory = f.factory();

    assert!(
        !f.creation_allowed(),
        "an unconfigured factory admits nothing"
    );
    assert_eq!(f.creation_error(), FactoryError::NotInitialized);
    assert_eq!(
        FactoryError::NotInitialized as u32,
        2,
        "the discriminant is part of the ABI",
    );

    // The flag itself is absent rather than false-but-stored, and the setter is
    // refused for the same reason.
    assert!(!factory.is_factory_paused());
    assert_eq!(
        factory.try_set_factory_paused(&true).unwrap_err().unwrap(),
        FactoryError::NotInitialized,
    );
    assert!(!factory.is_factory_paused());
}

/// Reading the pause never writes: neither the flag view nor the creation guard
/// changes any policy field, and neither emits an event. A creation path may
/// therefore consult the guard as often as it likes — including on a path that
/// then fails or reverts — without leaving a trace or paying rent.
#[test]
fn reading_the_pause_flag_does_not_change_the_policy() {
    let f = Fixture::new();
    let factory = f.factory();

    factory.set_cap(&4_242);
    factory.set_factory_paused(&true);

    let before = f.config();
    let events_before = f.event_count();

    for _ in 0..5 {
        assert!(factory.is_factory_paused());
        assert!(!f.creation_allowed());
        assert!(f.policy_paused());
    }

    assert_eq!(f.config(), before, "a read is not a policy change");
    assert_eq!(
        f.event_count(),
        events_before,
        "a read must not emit an event",
    );

    // Even a successful guard leaves nothing behind.
    factory.set_factory_paused(&false);
    let before = f.config();
    let events_before = f.event_count();
    for _ in 0..5 {
        assert!(f.creation_allowed());
    }
    assert_eq!(f.config(), before);
    assert_eq!(f.event_count(), events_before);
}

/// Only the admin can move the switch, in either direction: a non-admin caller
/// is rejected by the host's `require_auth`, and the stored flag is provably
/// unchanged afterwards.
#[test]
fn a_non_admin_cannot_toggle_the_pause() {
    let f = Fixture::new();
    let factory = f.factory();
    let non_admin = Address::generate(&f.env);

    // Pausing is refused.
    f.env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &f.fid,
            fn_name: "set_factory_paused",
            args: (true,).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_factory_paused(&true));
    assert!(!factory.is_factory_paused(), "the flag must be untouched");
    assert!(f.creation_allowed());

    // Unpausing is refused too — an attacker cannot lift an incident pause.
    f.env.mock_all_auths();
    factory.set_factory_paused(&true);
    f.env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &f.fid,
            fn_name: "set_factory_paused",
            args: (false,).into_val(&f.env),
            sub_invokes: &[],
        },
    }]);
    assert_auth_fails(|| factory.set_factory_paused(&false));
    assert!(factory.is_factory_paused(), "the pause must stand");
    assert!(!f.creation_allowed());
}
