//! Acceptance tests for issue #1801 — factory policy enforcement on stream creation.
//!
//! `create_stream_via_factory` must consult the factory's policy before
//! creating a stream. Each acceptance criterion gets its own test:
//!
//! 1. A deposit above the cap is rejected (`DepositExceedsCap`).
//! 2. A duration below the minimum is rejected (`DurationBelowMinimum`).
//! 3. A token absent from the allowlist is rejected (`TokenNotAllowlisted`).
//! 4. A rate outside the configured bounds is rejected (`RateBelowMin` / `RateAboveMax`).
//! 5. Factory pause blocks creation (`FactoryPaused`).
//! 6. A valid stream that satisfies all policy axes is accepted.

#![cfg(test)]
extern crate std;

use fluxora_factory::{FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{Error, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    env: Env,
    factory_id: Address,
    stream_id: Address,
    token: Address,
    sender: Address,
    recipient: Address,
}

impl Fixture {
    /// Deploy both contracts, wire them together and give the sender tokens.
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let factory_id = env.register(FluxoraFactory, ());
        let stream_id = env.register(FluxoraStream, ());

        let token_admin = Address::generate(&env);
        let token = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Mint plenty of tokens to the sender.
        StellarAssetClient::new(&env, &token).mint(&sender, &1_000_000_000);

        let admin = Address::generate(&env);

        // Initialise factory: cap=100_000, min_duration=60 (1 minute).
        FluxoraFactoryClient::new(&env, &factory_id).init(&admin, &stream_id, &100_000, &60);

        // Allowlist the token.
        FluxoraFactoryClient::new(&env, &factory_id).set_allowlist(&token, &true);

        Fixture {
            env,
            factory_id,
            stream_id,
            token,
            sender,
            recipient,
        }
    }

    fn stream_client(&self) -> FluxoraStreamClient {
        FluxoraStreamClient::new(&self.env, &self.stream_id)
    }

    fn factory_client(&self) -> FluxoraFactoryClient {
        FluxoraFactoryClient::new(&self.env, &self.factory_id)
    }

    /// Call create_stream_via_factory with default valid params, overriding specific ones.
    fn create(
        &self,
        deposit: i128,
        duration_secs: u64,
        token: &Address,
    ) -> Result<u64, fluxora_stream::Error> {
        let now = self.env.ledger().timestamp();
        let start = now + 10;
        let end = start + duration_secs;
        self.stream_client()
            .try_create_stream_via_factory(
                &self.factory_id,
                &self.sender,
                &self.recipient,
                token,
                &deposit,
                &start,
                &end,
                &start, // cliff == start (no cliff)
                &false,
                &false,
                &false,
            )
            .map(|r| r.unwrap())
            .map_err(|e| e.unwrap())
    }
}

// ---------------------------------------------------------------------------
// AC1 — deposit above cap is rejected
// ---------------------------------------------------------------------------

#[test]
fn deposit_above_cap_is_rejected() {
    let f = Fixture::new();
    // Cap is 100_000; deposit 100_001 must fail.
    let result = f.create(100_001, 200, &f.token.clone());
    assert_eq!(result, Err(Error::DepositExceedsCap));
}

#[test]
fn deposit_at_cap_is_accepted() {
    let f = Fixture::new();
    // Exactly at the cap is fine.
    let result = f.create(100_000, 200, &f.token.clone());
    assert!(
        result.is_ok(),
        "deposit == cap must be accepted, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// AC2 — duration below minimum is rejected
// ---------------------------------------------------------------------------

#[test]
fn duration_below_minimum_is_rejected() {
    let f = Fixture::new();
    // min_duration is 60 s; use 59.
    let result = f.create(1_000, 59, &f.token.clone());
    assert_eq!(result, Err(Error::DurationBelowMinimum));
}

#[test]
fn duration_at_minimum_is_accepted() {
    let f = Fixture::new();
    // Exactly at the floor is fine.
    let result = f.create(1_000, 60, &f.token.clone());
    assert!(
        result.is_ok(),
        "duration == min_duration must be accepted, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// AC3 — token absent from allowlist is rejected
// ---------------------------------------------------------------------------

#[test]
fn unlisted_token_is_rejected() {
    let f = Fixture::new();
    // Generate a token that was never allowlisted.
    let other_token = Address::generate(&f.env);
    let result = f.create(1_000, 200, &other_token);
    assert_eq!(result, Err(Error::TokenNotAllowlisted));
}

#[test]
fn allowlisted_token_is_accepted() {
    let f = Fixture::new();
    // The fixture already allowlisted `f.token`.
    let result = f.create(1_000, 200, &f.token.clone());
    assert!(
        result.is_ok(),
        "allowlisted token must be accepted, got {result:?}"
    );
}

#[test]
fn removing_from_allowlist_blocks_creation() {
    let f = Fixture::new();
    // Remove the token and verify it is now rejected.
    f.factory_client().set_allowlist(&f.token, &false);
    let result = f.create(1_000, 200, &f.token.clone());
    assert_eq!(result, Err(Error::TokenNotAllowlisted));
}

// ---------------------------------------------------------------------------
// AC4 — rate outside configured bounds is rejected
// ---------------------------------------------------------------------------

#[test]
fn rate_below_minimum_is_rejected() {
    let f = Fixture::new();
    // Set min_rate = 100 stroops/s. Use deposit=100, duration=200 -> rate=0, below min.
    f.factory_client().set_rate_bounds(&Some(100), &None);
    let result = f.create(100, 200, &f.token.clone());
    assert_eq!(result, Err(Error::RateBelowMin));
}

#[test]
fn rate_above_maximum_is_rejected() {
    let f = Fixture::new();
    // Set max_rate = 5 stroops/s. Use deposit=10_000, duration=60 -> rate=166, above max.
    f.factory_client().set_rate_bounds(&None, &Some(5));
    let result = f.create(10_000, 60, &f.token.clone());
    assert_eq!(result, Err(Error::RateAboveMax));
}

#[test]
fn rate_within_bounds_is_accepted() {
    let f = Fixture::new();
    // min=1, max=1000. deposit=500, duration=100 -> rate=5, within bounds.
    f.factory_client().set_rate_bounds(&Some(1), &Some(1000));
    let result = f.create(500, 100, &f.token.clone());
    assert!(
        result.is_ok(),
        "rate within bounds must be accepted, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// AC5 — factory pause blocks creation
// ---------------------------------------------------------------------------

#[test]
fn factory_pause_blocks_creation() {
    let f = Fixture::new();
    f.factory_client().set_factory_paused(&true);
    let result = f.create(1_000, 200, &f.token.clone());
    assert_eq!(result, Err(Error::FactoryPaused));
}

#[test]
fn unpausing_factory_allows_creation() {
    let f = Fixture::new();
    f.factory_client().set_factory_paused(&true);
    f.factory_client().set_factory_paused(&false);
    let result = f.create(1_000, 200, &f.token.clone());
    assert!(
        result.is_ok(),
        "unpaused factory must allow creation, got {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Happy path — all policy axes satisfied
// ---------------------------------------------------------------------------

#[test]
fn valid_stream_passes_all_policy_axes() {
    let f = Fixture::new();
    f.factory_client().set_rate_bounds(&Some(1), &Some(10_000));
    // deposit=5_000, duration=200 -> rate=25, within [1,10_000]; cap 100_000 ok; min_duration 60 ok.
    let result = f.create(5_000, 200, &f.token.clone());
    assert!(
        result.is_ok(),
        "valid stream must be created, got {result:?}"
    );
}
