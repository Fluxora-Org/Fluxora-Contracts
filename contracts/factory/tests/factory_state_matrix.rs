//! Combined matrix test covering every public mutating entry point crossed
//! with every contract state (uninitialized, initialized-active, paused).
//!
//! Builds on existing coverage without duplicating it:
//! - `factory_setters.rs` — setters in Active state + most in Uninitialized
//! - `factory_init_security.rs` — init auth/validation edge cases
//! - `batch_pause_gate.rs` — `create_streams` in Paused state
//! - `adversarial_auth.rs` — `create_stream` dual-auth matrix
//!
//! This test focuses on the *state-machine* dimension: for each (function, state)
//! pair it confirms the outcome is intentional, then flags any surprising
//! allowed/disallowed combination for maintainer review.

#![cfg(test)]

extern crate std;

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{CreateStreamParams, FluxoraStream, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Vec,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_DEPOSIT: i128 = 10_000_000;
const MIN_DURATION: u64 = 86_400;
const LEDGER_TIMESTAMP: u64 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Fixtures — one per contract state
// ---------------------------------------------------------------------------

/// Deploy the factory contract **without** calling `init`.
fn uninitialized_factory() -> (Env, FluxoraFactoryClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(LEDGER_TIMESTAMP);
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    (env, factory)
}

/// Deploy + init factory. Returns `(env, factory, admin)`.
fn active_factory() -> (Env, FluxoraFactoryClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(LEDGER_TIMESTAMP);
    let stream = env.register_contract(None, FluxoraStream);
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    factory.init(&admin, &stream, &MAX_DEPOSIT, &MIN_DURATION);
    (env, factory, admin)
}

/// Deploy + init + pause the factory.
fn paused_factory() -> (Env, FluxoraFactoryClient<'static>, Address) {
    let (env, factory, admin) = active_factory();
    assert!(!factory.is_factory_paused());
    factory.set_factory_paused(&true);
    assert!(factory.is_factory_paused());
    (env, factory, admin)
}

/// Minimal valid `CreateStreamParams` — sufficient to reach policy guards.
fn dummy_stream_params(env: &Env, recipient: &Address) -> CreateStreamParams {
    let now = env.ledger().timestamp();
    CreateStreamParams {
        recipient: recipient.clone(),
        deposit_amount: 100_000,
        rate_per_second: 1,
        start_time: now,
        cliff_time: now,
        end_time: now + 200_000,
        withdraw_dust_threshold: None,
        memo: None,
        metadata: None,
        kind: StreamKind::Linear,
        irrevocable: None,
        witness: None,
    }
}

// ===========================================================================
// S1 — Uninitialized state
// ===========================================================================

#[test]
fn test_init_succeeds_in_uninitialized() {
    let (env, factory) = uninitialized_factory();
    let stream = env.register_contract(None, FluxoraStream);
    let admin = Address::generate(&env);
    let result = factory.try_init(&admin, &stream, &MAX_DEPOSIT, &MIN_DURATION);
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn test_second_init_returns_already_initialized() {
    let (env, factory) = uninitialized_factory();
    let stream = env.register_contract(None, FluxoraStream);
    let admin = Address::generate(&env);
    factory.init(&admin, &stream, &MAX_DEPOSIT, &MIN_DURATION);
    let result = factory.try_init(&admin, &stream, &MAX_DEPOSIT, &MIN_DURATION);
    assert_eq!(result, Err(Ok(FactoryError::AlreadyInitialized)));
}

/// Every admin-only setter returns `NotInitialized` before `init`.
#[test]
fn test_admin_setters_reject_uninitialized() {
    let (_env, factory) = uninitialized_factory();
    let addr = Address::generate(&_env);

    assert_eq!(
        factory.try_set_admin(&addr),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_stream_contract(&addr),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_allowlist(&addr, &true),
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
        factory.try_set_batch_cap_enforcement(&true),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_rate_bounds(&None, &None),
        Err(Ok(FactoryError::NotInitialized))
    );
    assert_eq!(
        factory.try_set_factory_paused(&true),
        Err(Ok(FactoryError::NotInitialized))
    );
}

/// `create_stream` before `init` returns `NotInitialized`.
#[test]
fn test_create_stream_rejects_uninitialized() {
    let (_env, factory) = uninitialized_factory();
    let sender = Address::generate(&_env);
    let recipient = Address::generate(&_env);
    let params = dummy_stream_params(&_env, &recipient);
    let result = factory.try_create_stream(&sender, &params);
    assert_eq!(result, Err(Ok(FactoryError::NotInitialized)));
}

/// `create_streams` before `init` returns `NotInitialized`.
#[test]
fn test_create_streams_rejects_uninitialized() {
    let (_env, factory) = uninitialized_factory();
    let sender = Address::generate(&_env);
    let params = dummy_stream_params(&_env, &Address::generate(&_env));
    let mut streams: Vec<CreateStreamParams> = Vec::new(&_env);
    streams.push_back(params);
    let result = factory.try_create_streams(&sender, &streams);
    assert_eq!(result, Err(Ok(FactoryError::NotInitialized)));
}

// ===========================================================================
// S2 — Initialized-Active state
// ===========================================================================

/// `init` after a successful init returns `AlreadyInitialized`.
#[test]
fn test_init_rejects_already_initialized() {
    let (_env, factory, _admin) = active_factory();
    let stream = _env.register_contract(None, FluxoraStream);
    let another_admin = Address::generate(&_env);
    let result = factory.try_init(&another_admin, &stream, &MAX_DEPOSIT, &MIN_DURATION);
    assert_eq!(result, Err(Ok(FactoryError::AlreadyInitialized)));
}

/// Every admin setter succeeds in the Active state (happy path).
#[test]
fn test_admin_setters_succeed_in_active() {
    let (_env, factory, _admin) = active_factory();
    let new_admin = Address::generate(&_env);
    let new_stream = _env.register_contract(None, FluxoraStream);
    let recipient = Address::generate(&_env);

    assert!(factory.try_set_admin(&new_admin).is_ok());
    assert!(factory.try_set_stream_contract(&new_stream).is_ok());
    assert!(factory.try_set_allowlist(&recipient, &true).is_ok());
    assert!(factory.try_set_cap(&5_000).is_ok());
    assert!(factory.try_set_min_duration(&200).is_ok());
    assert!(factory.try_set_batch_cap_enforcement(&false).is_ok());
    assert!(factory.try_set_rate_bounds(&Some(10), &Some(100)).is_ok());
    assert!(factory.try_set_factory_paused(&true).is_ok());
}

/// `create_stream` in Active state reaches policy evaluation (not blocked by
/// `NotInitialized` or `CreationPaused`). Proved by seeing an allowlist error
/// instead of a state-machine error.
#[test]
fn test_create_stream_reaches_policy_in_active() {
    let (_env, factory, _admin) = active_factory();
    let sender = Address::generate(&_env);
    // Recipient is NOT allowlisted → should get RecipientNotAllowlisted,
    // proving the pause gate and policy load passed.
    let recipient = Address::generate(&_env);
    let params = dummy_stream_params(&_env, &recipient);
    let result = factory.try_create_stream(&sender, &params);
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

/// `create_streams` similarly reaches policy evaluation in Active state.
#[test]
fn test_create_streams_reaches_policy_in_active() {
    let (_env, factory, _admin) = active_factory();
    let sender = Address::generate(&_env);
    let recipient = Address::generate(&_env);
    let params = dummy_stream_params(&_env, &recipient);
    let mut streams: Vec<CreateStreamParams> = Vec::new(&_env);
    streams.push_back(params);
    let result = factory.try_create_streams(&sender, &streams);
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

// ===========================================================================
// S3 — Paused state
// ===========================================================================

/// `init` is still `AlreadyInitialized` when paused.
#[test]
fn test_init_rejects_already_initialized_when_paused() {
    let (_env, factory, _admin) = paused_factory();
    let stream = _env.register_contract(None, FluxoraStream);
    let another_admin = Address::generate(&_env);
    let result = factory.try_init(&another_admin, &stream, &MAX_DEPOSIT, &MIN_DURATION);
    assert_eq!(result, Err(Ok(FactoryError::AlreadyInitialized)));
}

/// **Flagged behavior**: All admin setters succeed while the factory is paused.
/// This is intentional — it allows the admin to reconfigure policy (update
/// allowlist, adjust caps, rotate admin, swap stream contract, etc.) while
/// creation is halted, so the factory can be repaired before resuming.
#[test]
fn test_admin_setters_succeed_when_paused() {
    let (_env, factory, _admin) = paused_factory();
    let new_admin = Address::generate(&_env);
    let new_stream = _env.register_contract(None, FluxoraStream);
    let recipient = Address::generate(&_env);

    assert!(
        factory.try_set_admin(&new_admin).is_ok(),
        "set_admin must work while paused"
    );
    assert!(
        factory.try_set_stream_contract(&new_stream).is_ok(),
        "set_stream_contract must work while paused"
    );
    assert!(
        factory.try_set_allowlist(&recipient, &true).is_ok(),
        "set_allowlist must work while paused"
    );
    assert!(
        factory.try_set_cap(&5_000).is_ok(),
        "set_cap must work while paused"
    );
    assert!(
        factory.try_set_min_duration(&200).is_ok(),
        "set_min_duration must work while paused"
    );
    assert!(
        factory.try_set_batch_cap_enforcement(&false).is_ok(),
        "set_batch_cap_enforcement must work while paused"
    );
    assert!(
        factory.try_set_rate_bounds(&Some(10), &Some(100)).is_ok(),
        "set_rate_bounds must work while paused"
    );
    // Toggle pause off — the same function handles both directions.
    assert!(
        factory.try_set_factory_paused(&false).is_ok(),
        "set_factory_paused(false) must work while paused"
    );
    assert!(!factory.is_factory_paused(), "factory must be unpaused");
}

/// `create_stream` returns `CreationPaused` when the factory is paused.
#[test]
fn test_create_stream_blocked_when_paused() {
    let (_env, factory, _admin) = paused_factory();
    let sender = Address::generate(&_env);
    let recipient = Address::generate(&_env);
    let params = dummy_stream_params(&_env, &recipient);
    let result = factory.try_create_stream(&sender, &params);
    assert_eq!(result, Err(Ok(FactoryError::CreationPaused)));
}

/// `create_streams` returns `CreationPaused` when the factory is paused
/// (regression coverage: issue #726, already tested in `batch_pause_gate.rs`).
#[test]
fn test_create_streams_blocked_when_paused() {
    let (_env, factory, _admin) = paused_factory();
    let sender = Address::generate(&_env);
    let params = dummy_stream_params(&_env, &Address::generate(&_env));
    let mut streams: Vec<CreateStreamParams> = Vec::new(&_env);
    streams.push_back(params);
    let result = factory.try_create_streams(&sender, &streams);
    assert_eq!(result, Err(Ok(FactoryError::CreationPaused)));
}

/// After unpausing, `create_stream` returns to normal policy evaluation
/// (proved by seeing `RecipientNotAllowlisted` instead of `CreationPaused`).
#[test]
fn test_create_stream_unpause_restores_policy() {
    let (_env, factory, _admin) = paused_factory();
    factory.set_factory_paused(&false);
    assert!(!factory.is_factory_paused());

    let sender = Address::generate(&_env);
    let recipient = Address::generate(&_env);
    let params = dummy_stream_params(&_env, &recipient);
    let result = factory.try_create_stream(&sender, &params);
    assert_eq!(
        result,
        Err(Ok(FactoryError::RecipientNotAllowlisted)),
        "after unpause, create_stream must evaluate policy, not return CreationPaused"
    );
}

/// After unpausing, `create_streams` returns to normal policy evaluation.
#[test]
fn test_create_streams_unpause_restores_policy() {
    let (_env, factory, _admin) = paused_factory();
    factory.set_factory_paused(&false);
    assert!(!factory.is_factory_paused());

    let sender = Address::generate(&_env);
    let recipient = Address::generate(&_env);
    let params = dummy_stream_params(&_env, &recipient);
    let mut streams: Vec<CreateStreamParams> = Vec::new(&_env);
    streams.push_back(params);
    let result = factory.try_create_streams(&sender, &streams);
    assert_eq!(
        result,
        Err(Ok(FactoryError::RecipientNotAllowlisted)),
        "after unpause, create_streams must evaluate policy, not return CreationPaused"
    );
}
