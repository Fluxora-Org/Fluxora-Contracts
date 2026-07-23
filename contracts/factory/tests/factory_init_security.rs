//! Security tests for `FluxoraFactory::init` and `set_stream_contract`.
//!
//! Covers:
//! - `init` cannot succeed without the declared admin's authorization.
//! - `init` and `set_stream_contract` reject a `stream_contract` address that
//!   does not implement the `FluxoraStream` interface (EOA / non-contract /
//!   wrong contract), returning a typed `FactoryError::InvalidStreamContract`
//!   instead of host-trapping.
//! - The `InvalidStreamContract` discriminant is stable and existing
//!   discriminants are unchanged.

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

fn deploy_factory(env: &Env) -> FluxoraFactoryClient<'_> {
    let factory_id = env.register_contract(None, FluxoraFactory);
    FluxoraFactoryClient::new(env, &factory_id)
}

fn deploy_stream(env: &Env) -> Address {
    env.register_contract(None, FluxoraStream)
}

// ---------------------------------------------------------------------------
// init requires admin auth
// ---------------------------------------------------------------------------

/// `init` must panic (auth failure) if the declared admin did not authorize
/// the call, even when the supplied `stream_contract` is a valid deployment.
#[test]
#[should_panic]
fn test_init_without_admin_auth_panics() {
    let env = Env::default();
    // No mock_all_auths / mock_auths configured at all: any require_auth fails.
    let factory = deploy_factory(&env);
    let stream_contract = deploy_stream(&env);
    let admin = Address::generate(&env);

    factory.init(&admin, &stream_contract, &10_000, &100);
}

/// `try_init` confirms the panic is specifically an auth failure (not, say,
/// a coincidental `InvalidStreamContract`) by using a valid stream contract
/// and asserting the call never returns — it must trap before any storage
/// write or typed-error return path is reached.
#[test]
fn test_init_fails_when_only_unrelated_address_authorizes() {
    let env = Env::default();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let stream_contract = deploy_stream(&env);
    let admin = Address::generate(&env);
    let unrelated = Address::generate(&env);

    // Mock auth for a *different* address than the declared admin.
    env.mock_auths(&[MockAuth {
        address: &unrelated,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "init",
            args: (&admin, &stream_contract, 10_000i128, 100u64).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        factory.init(&admin, &stream_contract, &10_000, &100);
    }));
    assert!(
        result.is_err(),
        "init must reject a call not authorized by the declared admin"
    );
}

/// With the declared admin's auth correctly mocked, `init` succeeds.
#[test]
fn test_init_succeeds_with_admin_auth_and_valid_stream_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let factory = deploy_factory(&env);
    let stream_contract = deploy_stream(&env);
    let admin = Address::generate(&env);

    let result = factory.try_init(&admin, &stream_contract, &10_000, &100);
    assert_eq!(result, Ok(Ok(())));

    let config = factory.get_factory_config();
    assert_eq!(config.admin, admin);
    assert_eq!(config.stream_contract, stream_contract);
}

// ---------------------------------------------------------------------------
// init / set_stream_contract validate the stream contract address
// ---------------------------------------------------------------------------

/// `init` rejects an EOA-style address (no deployed contract at all) as
/// `stream_contract` with a typed error instead of host-trapping.
#[test]
fn test_init_rejects_eoa_stream_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let factory = deploy_factory(&env);
    let admin = Address::generate(&env);
    let eoa = Address::generate(&env); // never registered as a contract

    let result = factory.try_init(&admin, &eoa, &10_000, &100);
    assert_eq!(result, Err(Ok(FactoryError::InvalidStreamContract)));
}

/// `init` rejects a deployed contract that does not implement the
/// `FluxoraStream` interface (here, the factory contract itself).
#[test]
fn test_init_rejects_non_fluxora_stream_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let factory = deploy_factory(&env);
    let admin = Address::generate(&env);
    // A real deployed contract, but not a FluxoraStream — it has no version().
    let wrong_contract = env.register_contract(None, FluxoraFactory);

    let result = factory.try_init(&admin, &wrong_contract, &10_000, &100);
    assert_eq!(result, Err(Ok(FactoryError::InvalidStreamContract)));
}

/// A failed `init` due to an invalid stream contract must not leave the
/// factory partially initialized: a subsequent `init` with a valid stream
/// contract must still succeed.
#[test]
fn test_init_invalid_stream_contract_is_side_effect_free() {
    let env = Env::default();
    env.mock_all_auths();
    let factory = deploy_factory(&env);
    let admin = Address::generate(&env);
    let eoa = Address::generate(&env);

    let first = factory.try_init(&admin, &eoa, &10_000, &100);
    assert_eq!(first, Err(Ok(FactoryError::InvalidStreamContract)));

    let valid_stream = deploy_stream(&env);
    let second = factory.try_init(&admin, &valid_stream, &10_000, &100);
    assert_eq!(second, Ok(Ok(())));
}

/// `set_stream_contract` rejects an EOA-style address, leaving the
/// previously configured stream contract untouched.
#[test]
fn test_set_stream_contract_rejects_eoa_address() {
    let env = Env::default();
    env.mock_all_auths();
    let factory = deploy_factory(&env);
    let admin = Address::generate(&env);
    let original_stream = deploy_stream(&env);
    factory.init(&admin, &original_stream, &10_000, &100);

    let eoa = Address::generate(&env);
    let result = factory.try_set_stream_contract(&eoa);
    assert_eq!(result, Err(Ok(FactoryError::InvalidStreamContract)));

    // The previously configured stream contract must remain in place.
    let config = factory.get_factory_config();
    assert_eq!(config.stream_contract, original_stream);
}

/// `set_stream_contract` rejects a deployed-but-wrong-interface contract.
#[test]
fn test_set_stream_contract_rejects_non_fluxora_stream_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let factory = deploy_factory(&env);
    let admin = Address::generate(&env);
    let original_stream = deploy_stream(&env);
    factory.init(&admin, &original_stream, &10_000, &100);

    let wrong_contract = env.register_contract(None, FluxoraFactory);
    let result = factory.try_set_stream_contract(&wrong_contract);
    assert_eq!(result, Err(Ok(FactoryError::InvalidStreamContract)));

    let config = factory.get_factory_config();
    assert_eq!(config.stream_contract, original_stream);
}

/// `set_stream_contract` succeeds when swapping to another valid
/// `FluxoraStream` deployment.
#[test]
fn test_set_stream_contract_accepts_valid_stream_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let factory = deploy_factory(&env);
    let admin = Address::generate(&env);
    let original_stream = deploy_stream(&env);
    factory.init(&admin, &original_stream, &10_000, &100);

    let new_stream = deploy_stream(&env);
    let result = factory.try_set_stream_contract(&new_stream);
    assert_eq!(result, Ok(Ok(())));

    let config = factory.get_factory_config();
    assert_eq!(config.stream_contract, new_stream);
}

/// `set_stream_contract` still requires the *declared admin's* auth even
/// when the new address is a valid `FluxoraStream` deployment: an
/// unrelated caller's authorization does not satisfy `require_admin`.
#[test]
fn test_set_stream_contract_requires_admin_auth() {
    let env = Env::default();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let original_stream = deploy_stream(&env);

    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "init",
            args: (&admin, &original_stream, 10_000i128, 100u64).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    factory.init(&admin, &original_stream, &10_000, &100);

    let new_stream = deploy_stream(&env);
    env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &factory_id,
            fn_name: "set_stream_contract",
            args: (&new_stream,).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        factory.set_stream_contract(&new_stream);
    }));
    assert!(
        result.is_err(),
        "set_stream_contract must reject auth from a non-admin address"
    );

    // Original stream contract must remain unchanged.
    let config = factory.get_factory_config();
    assert_eq!(config.stream_contract, original_stream);
}

// ---------------------------------------------------------------------------
// version() smoke-check sanity: confirm it exercises a real FluxoraStream
// entrypoint and is unaffected by the stream contract's own init state.
// ---------------------------------------------------------------------------

/// The smoke check (`version()`) succeeds against a `FluxoraStream` contract
/// that has not yet had its own `init` called, matching the documented
/// "works even on an uninitialised contract" behavior of `version()`.
#[test]
fn test_uninitialized_stream_contract_passes_smoke_check() {
    let env = Env::default();
    env.mock_all_auths();
    let factory = deploy_factory(&env);
    let admin = Address::generate(&env);
    let stream_contract = deploy_stream(&env); // never call stream.init(...)

    let stream_client = FluxoraStreamClient::new(&env, &stream_contract);
    // Sanity: the stream contract is reachable and responds to version().
    let _ = stream_client.version();

    let result = factory.try_init(&admin, &stream_contract, &10_000, &100);
    assert_eq!(result, Ok(Ok(())));
}

// ---------------------------------------------------------------------------
// Discriminant stability
// ---------------------------------------------------------------------------
//
// A `test_factory_error_discriminants_are_stable` used to live here, but it
// hardcoded a stale, pre-#788 discriminant set (it asserted
// `InvalidStreamContract as u32 == 9`, from before `CreationPaused` through
// `InvalidMemo` were inserted ahead of it, after which `InvalidStreamContract`
// became discriminant 17). It was never updated in lockstep with
// `contracts/factory/src/lib.rs::FactoryError`, so it silently bit-rotted
// until `cargo test --workspace` actually exercised it.
//
// The canonical, actively-maintained version of this guard — kept in sync
// with `docs/error.md` and covering all 17 variants — lives in
// `contracts/factory/tests/factory_error_discriminants.rs`. Removed the
// duplicate here rather than re-fixing two copies of the same assertion set.
