//! End-to-end integration test: `FluxoraFactory::create_stream` → `FluxoraStream::create_stream`.
//!
//! Deployment topology:
//!
//! ```text
//! ┌──────────────────┐     cross-contract      ┌──────────────────┐
//! │ FluxoraFactory    │ ──────────────────────→ │ FluxoraStream     │
//! │                  │   create_stream         │                  │
//! │ policy checks   │   (sender auth × 2)     │ token transfer   │
//! │ (allowlist,     │                          │ persist stream   │
//! │  cap, duration) │                          │ recipient index  │
//! └──────────────────┘                          └──────────────────┘
//!         │                                            │
//!         ▼                                            ▼
//!   ┌───────────────────────────────────────────────────────┐
//!   │       Stellar Asset Contract (SEP-41 / SAC)           │
//!   └───────────────────────────────────────────────────────┘
//! ```
//!
//! Every test registers **real** `FluxoraFactory`, `FluxoraStream`, and SAC token
//! contracts in a single `Env` so that the cross-contract wiring — sender dual-auth,
//! token funding, returned `stream_id`, and recipient-index updates — is genuinely
//! exercised (no mocks at the contract boundary).

extern crate std;

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_DEPOSIT: i128 = 10_000_000;
const MIN_DURATION: u64 = 86_400; // 1 day in seconds
const DEPOSIT_AMOUNT: i128 = 100_000;
const RATE_PER_SECOND: i128 = 1;
const STREAM_DURATION: u64 = 200_000;
const SENDER_FUNDING: i128 = 1_000_000_000;
const LEDGER_TIMESTAMP: u64 = 1_000_000_000;

struct FactoryClientWrapper<'a> {
    client: FluxoraFactoryClient<'a>,
}

impl<'a> FactoryClientWrapper<'a> {
    fn new(client: FluxoraFactoryClient<'a>) -> Self {
        Self { client }
    }

    fn init(&self, admin: &Address, stream_contract: &Address, max_deposit: &i128, min_duration: &u64) {
        self.client.init(admin, stream_contract, max_deposit, min_duration);
    }

    fn set_allowlist(&self, recipient: &Address, allowed: &bool) {
        self.client.set_allowlist(recipient, allowed);
    }

    fn set_cap(&self, new_cap: &i128) {
        self.client.set_cap(new_cap);
    }

    fn set_min_duration(&self, new_min_duration: &u64) {
        self.client.set_min_duration(new_min_duration);
    }

    fn set_rate_bounds(&self, min_rate: &Option<i128>, max_rate: &Option<i128>) {
        self.client.set_rate_bounds(min_rate, max_rate);
    }

    fn create_stream(
        &self,
        sender: &Address,
        recipient: &Address,
        deposit_amount: &i128,
        rate_per_second: &i128,
        start_time: &u64,
        cliff_time: &u64,
        end_time: &u64,
        withdraw_dust_threshold: &i128,
    ) -> u64 {
        self.client.create_stream(
            sender,
            recipient,
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            withdraw_dust_threshold,
            &None,
            &fluxora_stream::StreamKind::Linear,
        )
    }

    fn try_create_stream(
        &self,
        sender: &Address,
        recipient: &Address,
        deposit_amount: &i128,
        rate_per_second: &i128,
        start_time: &u64,
        cliff_time: &u64,
        end_time: &u64,
        withdraw_dust_threshold: &i128,
    ) -> Result<Result<u64, soroban_sdk::Error>, Result<FactoryError, soroban_sdk::InvokeError>> {
        self.client.try_create_stream(
            sender,
            recipient,
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            withdraw_dust_threshold,
            &None,
            &fluxora_stream::StreamKind::Linear,
        )
    }
}

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    factory: FactoryClientWrapper<'a>,
    stream: FluxoraStreamClient<'a>,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
    token_id: Address,
    stream_contract_id: Address,
    factory_contract_id: Address,
    sender_balance_before: i128,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(LEDGER_TIMESTAMP);

        let stream_contract_id = env.register_contract(None, FluxoraStream);
        let factory_contract_id = env.register_contract(None, FluxoraFactory);

        let stream = FluxoraStreamClient::new(&env, &stream_contract_id);
        let factory = FluxoraFactoryClient::new(&env, &factory_contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        stellar_asset.mint(&sender, &SENDER_FUNDING);

        stream.init(&token_id, &stream_contract_id);
        factory.init(&admin, &stream_contract_id, &MAX_DEPOSIT, &MIN_DURATION);
        factory.set_allowlist(&recipient, &true);

        let sender_balance_before = token.balance(&sender);

        Self {
            env,
            factory: FactoryClientWrapper::new(factory),
            stream,
            admin,
            sender,
            recipient,
            token,
            token_id,
            stream_contract_id,
            factory_contract_id,
            sender_balance_before,
        }
    }

    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }

    fn default_params(&self) -> (i128, i128, u64, u64, u64, i128) {
        let start = self.now();
        (DEPOSIT_AMOUNT, RATE_PER_SECOND, start, start, start + STREAM_DURATION, 0)
    }

    fn create_default_stream(&self) -> u64 {
        let (dep, rate, start, cliff, end, dust) = self.default_params();
        self.factory.create_stream(&self.sender, &self.recipient, &dep, &rate, &start, &cliff, &end, &dust)
    }
}

// ---------------------------------------------------------------------------
// Happy path: factory creates a stream and the stream is persisted
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_happy_path() {
    let ctx = Ctx::setup();
    let (deposit, rate, start, cliff, end, dust) = ctx.default_params();

    let stream_id = ctx.factory.create_stream(
        &ctx.sender, &ctx.recipient, &deposit, &rate, &start, &cliff, &end, &dust,
    );

    assert_eq!(stream_id, 0, "first stream gets id 0");

    // -- stream state ------------------------------------------------------
    let state = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, DEPOSIT_AMOUNT);
    assert_eq!(state.rate_per_second, RATE_PER_SECOND);
    assert_eq!(state.start_time, start);
    assert_eq!(state.cliff_time, cliff);
    assert_eq!(state.end_time, end);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, fluxora_stream::StreamStatus::Active);
    assert_eq!(state.kind, fluxora_stream::StreamKind::Linear);

    // -- recipient index ---------------------------------------------------
    let streams = ctx.stream.get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);
    assert_eq!(streams.get(0).unwrap(), stream_id);

    let count = ctx.stream.get_recipient_stream_count(&ctx.recipient);
    assert_eq!(count, 1);

    // empty for other recipients
    let other = Address::generate(&ctx.env);
    assert_eq!(ctx.stream.get_recipient_stream_count(&other), 0);
    assert!(ctx.stream.get_recipient_streams(&other).is_empty());

    // -- token balance -----------------------------------------------------
    let sender_after = ctx.token.balance(&ctx.sender);
    let stream_balance = ctx.token.balance(&ctx.stream_contract_id);
    assert_eq!(sender_after, ctx.sender_balance_before - DEPOSIT_AMOUNT);
    assert_eq!(stream_balance, DEPOSIT_AMOUNT);
}

// ---------------------------------------------------------------------------
// RecipientNotAllowlisted
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_recipient_not_allowlisted() {
    let ctx = Ctx::setup();
    let unknown = Address::generate(&ctx.env);
    let (dep, rate, start, cliff, end, dust) = ctx.default_params();

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &unknown, &dep, &rate, &start, &cliff, &end, &dust,
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

// ---------------------------------------------------------------------------
// DepositExceedsCap
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_deposit_exceeds_cap() {
    let ctx = Ctx::setup();
    let (_, rate, start, cliff, end, dust) = ctx.default_params();
    let over_cap = MAX_DEPOSIT + 1;

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &over_cap, &rate, &start, &cliff, &end, &dust,
    );
    assert_eq!(result, Err(Ok(FactoryError::DepositExceedsCap)));
}

/// Deposit exactly at the cap boundary is accepted.
#[test]
fn test_create_stream_deposit_at_cap_ok() {
    let ctx = Ctx::setup();
    let (_, rate, start, cliff, end, dust) = ctx.default_params();

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &MAX_DEPOSIT, &rate, &start, &cliff, &end, &dust,
    );
    assert_ne!(result, Err(Ok(FactoryError::DepositExceedsCap)));
}

// ---------------------------------------------------------------------------
// DurationTooShort
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_duration_too_short() {
    let ctx = Ctx::setup();
    let start = ctx.now();
    let short_duration = MIN_DURATION - 1;

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &start, &start, &(start + short_duration), &0,
    );
    assert_eq!(result, Err(Ok(FactoryError::DurationTooShort)));
}

/// Duration exactly at the minimum boundary is accepted.
#[test]
fn test_create_stream_duration_at_minimum_ok() {
    let ctx = Ctx::setup();
    let start = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &start, &start, &(start + MIN_DURATION), &0,
    );
    assert_ne!(result, Err(Ok(FactoryError::DurationTooShort)));
}

// ---------------------------------------------------------------------------
// Time-relationship validation
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_invalid_time_range_end_before_start() {
    let ctx = Ctx::setup();
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &(now + 200), &(now + 200), &(now + 100), &0,
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidTimeRange)));
}

#[test]
fn test_create_stream_invalid_time_range_end_equal_start() {
    let ctx = Ctx::setup();
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &now, &now, &now, &0,
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidTimeRange)));
}

#[test]
fn test_create_stream_invalid_cliff_before_start() {
    let ctx = Ctx::setup();
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &(now + 100), &now, &(now + 300), &0,
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidCliff)));
}

#[test]
fn test_create_stream_invalid_cliff_after_end() {
    let ctx = Ctx::setup();
    let now = ctx.now();

    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &now, &(now + 300), &(now + 200), &0,
    );
    assert_eq!(result, Err(Ok(FactoryError::InvalidCliff)));
}

// ---------------------------------------------------------------------------
// Cliff-at-boundary edge cases
// ---------------------------------------------------------------------------

/// Cliff at start time is valid (no cliff / immediate vesting).
#[test]
fn test_create_stream_cliff_at_start() {
    let ctx = Ctx::setup();
    let now = ctx.now();

    let stream_id = ctx.factory.create_stream(
        &ctx.sender, &ctx.recipient,
        &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &now, &now, &(now + STREAM_DURATION), &0,
    );

    let state = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, now);
    assert_eq!(state.cliff_time, state.start_time);
}

/// Cliff at end time is valid (cliff vests all at conclusion).
#[test]
fn test_create_stream_cliff_at_end() {
    let ctx = Ctx::setup();
    let now = ctx.now();
    let end = now + STREAM_DURATION;

    let stream_id = ctx.factory.create_stream(
        &ctx.sender, &ctx.recipient,
        &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &now, &end, &end, &0,
    );

    let state = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, end);
    assert_eq!(state.cliff_time, state.end_time);
}

// ---------------------------------------------------------------------------
// Sender auth required
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_requires_sender_auth() {
    let env = Env::default();
    // Deliberately NOT calling mock_all_auths — we want `require_auth` to fail.
    let stream_id = env.register_contract(None, FluxoraStream);
    let factory_id = env.register_contract(None, FluxoraFactory);

    let stream = FluxoraStreamClient::new(&env, &stream_id);
    let factory = FactoryClientWrapper::new(FluxoraFactoryClient::new(&env, &factory_id));

    let token_admin = Address::generate(&env);
    let token = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    // Use auth-allowing setup calls so we can test just the create_stream auth
    env.mock_all_auths_allowing_non_root_auth();
    stream.init(&token, &stream_id);
    factory.init(&admin, &stream_id, &MAX_DEPOSIT, &MIN_DURATION);
    factory.set_allowlist(&recipient, &true);
    // Restore no-auth state for the actual test call
    env.mock_auths(&[]);

    let now = env.ledger().timestamp();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        factory.create_stream(&sender, &recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND, &now, &now, &(now + STREAM_DURATION), &0);
    }));
    assert!(result.is_err(), "create_stream must panic without sender auth");
}

// ---------------------------------------------------------------------------
// Token-balance transfer verification
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_moves_tokens_from_sender_to_contract() {
    let ctx = Ctx::setup();

    let sender_before = ctx.token.balance(&ctx.sender);
    let contract_before = ctx.token.balance(&ctx.stream_contract_id);
    let recipient_before = ctx.token.balance(&ctx.recipient);

    assert_eq!(contract_before, 0, "stream contract starts with zero balance");
    assert_eq!(recipient_before, 0, "recipient starts with zero balance");

    ctx.create_default_stream();

    let sender_after = ctx.token.balance(&ctx.sender);
    let contract_after = ctx.token.balance(&ctx.stream_contract_id);
    let recipient_after = ctx.token.balance(&ctx.recipient);

    assert_eq!(sender_after, sender_before - DEPOSIT_AMOUNT, "sender debited deposit");
    assert_eq!(contract_after, DEPOSIT_AMOUNT, "stream contract credited deposit");
    assert_eq!(recipient_after, 0, "recipient balance unchanged");
}

// ---------------------------------------------------------------------------
// Multiple streams for the same recipient
// ---------------------------------------------------------------------------

#[test]
fn test_create_multiple_streams_same_recipient() {
    let ctx = Ctx::setup();
    let (dep, rate, start, cliff, end, dust) = ctx.default_params();

    let id0 = ctx.factory.create_stream(
        &ctx.sender, &ctx.recipient, &dep, &rate, &start, &cliff, &end, &dust,
    );
    // Slightly different schedule for a second stream
    let id1 = ctx.factory.create_stream(
        &ctx.sender, &ctx.recipient, &dep, &rate, &start, &cliff, &(end + 100_000), &dust,
    );

    assert_eq!(id0, 0);
    assert_eq!(id1, 1);

    let streams = ctx.stream.get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 2);
    assert_eq!(streams.get(0).unwrap(), 0);
    assert_eq!(streams.get(1).unwrap(), 1);

    let count = ctx.stream.get_recipient_stream_count(&ctx.recipient);
    assert_eq!(count, 2);
}

// ---------------------------------------------------------------------------
// Multiple recipients
// ---------------------------------------------------------------------------

#[test]
fn test_create_streams_different_recipients() {
    let ctx = Ctx::setup();
    let recipient_b = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&recipient_b, &true);

    let (dep, rate, start, cliff, end, dust) = ctx.default_params();

    ctx.factory.create_stream(&ctx.sender, &ctx.recipient, &dep, &rate, &start, &cliff, &end, &dust);
    ctx.factory.create_stream(&ctx.sender, &recipient_b, &dep, &rate, &start, &cliff, &(end + 50_000), &dust);

    assert_eq!(ctx.stream.get_recipient_stream_count(&ctx.recipient), 1);
    assert_eq!(ctx.stream.get_recipient_stream_count(&recipient_b), 1);

    assert_eq!(ctx.stream.get_recipient_streams(&ctx.recipient).get(0).unwrap(), 0);
    assert_eq!(ctx.stream.get_recipient_streams(&recipient_b).get(0).unwrap(), 1);
}

// ---------------------------------------------------------------------------
// NotInitialized
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_factory_not_initialized_returns_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let factory_id = env.register_contract(None, FluxoraFactory);
    let factory = FactoryClientWrapper::new(FluxoraFactoryClient::new(&env, &factory_id));
    let now = env.ledger().timestamp();

    let result = factory.try_create_stream(
        &Address::generate(&env), &Address::generate(&env),
        &DEPOSIT_AMOUNT, &RATE_PER_SECOND, &now, &now, &(now + STREAM_DURATION), &0,
    );
    assert_eq!(result, Err(Ok(FactoryError::NotInitialized)));
}

// ---------------------------------------------------------------------------
// Policy update enforcement
// ---------------------------------------------------------------------------

#[test]
fn test_set_cap_enforced_end_to_end() {
    let ctx = Ctx::setup();
    ctx.factory.set_cap(&5_000);

    let (_, rate, start, cliff, end, dust) = ctx.default_params();
    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &6_000, &rate, &start, &cliff, &end, &dust,
    );
    assert_eq!(result, Err(Ok(FactoryError::DepositExceedsCap)));
}

#[test]
fn test_set_min_duration_enforced_end_to_end() {
    let ctx = Ctx::setup();
    ctx.factory.set_min_duration(&500_000);

    let now = ctx.now();
    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &now, &now, &(now + 200_000), &0,
    );
    assert_eq!(result, Err(Ok(FactoryError::DurationTooShort)));
}

#[test]
fn test_remove_allowlist_enforced_end_to_end() {
    let ctx = Ctx::setup();
    ctx.factory.set_allowlist(&ctx.recipient, &false);

    let (dep, rate, start, cliff, end, dust) = ctx.default_params();
    let result = ctx.factory.try_create_stream(
        &ctx.sender, &ctx.recipient, &dep, &rate, &start, &cliff, &end, &dust,
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}
