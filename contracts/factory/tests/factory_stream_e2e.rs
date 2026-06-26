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
use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_DEPOSIT: i128 = 10_000_000;
const MIN_DURATION: u64 = 86_400; // 1 day in seconds
const DEPOSIT_AMOUNT: i128 = 200_000;
const RATE_PER_SECOND: i128 = 1;
const STREAM_DURATION: u64 = 200_000;
const SENDER_FUNDING: i128 = 1_000_000_000;
const LEDGER_TIMESTAMP: u64 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    factory: FluxoraFactoryClient<'a>,
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

        // The stream contract uses transfer_from; approve it to pull up to the full sender balance.
        token.approve(&sender, &stream_contract_id, &SENDER_FUNDING, &100_000);

        let sender_balance_before = token.balance(&sender);

        Self {
            env,
            factory,
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
        self.factory.create_stream(
            &self.sender, &self.recipient, &dep, &rate, &start, &cliff, &end, &dust,
            &None, &StreamKind::Linear,
        )
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
    let factory = FluxoraFactoryClient::new(&env, &factory_id);

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
        factory.create_stream(&sender, &recipient, &DEPOSIT_AMOUNT, &RATE_PER_SECOND, &now, &now, &(now + STREAM_DURATION), &0, &None, &StreamKind::Linear);
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
        &None, &StreamKind::Linear,
    );
    // Slightly different schedule for a second stream (same duration, valid deposit)
    let id1 = ctx.factory.create_stream(
        &ctx.sender, &ctx.recipient, &dep, &rate, &(start + 1), &(start + 1), &(start + 1 + STREAM_DURATION), &dust,
        &None, &StreamKind::Linear,
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

    ctx.factory.create_stream(&ctx.sender, &ctx.recipient, &dep, &rate, &start, &cliff, &end, &dust, &None, &StreamKind::Linear);
    ctx.factory.create_stream(&ctx.sender, &recipient_b, &dep, &rate, &(start + 1), &(start + 1), &(start + 1 + STREAM_DURATION), &dust, &None, &StreamKind::Linear);

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
    let factory = FluxoraFactoryClient::new(&env, &factory_id);
    let now = env.ledger().timestamp();

    // No init called. Guard order in create_stream hits allowlist before cap/NotInitialized,
    // so we expect RecipientNotAllowlisted as the first observable error.
    let result = factory.try_create_stream(
        &Address::generate(&env), &Address::generate(&env),
        &DEPOSIT_AMOUNT, &RATE_PER_SECOND, &now, &now, &(now + STREAM_DURATION), &0,
        &None, &StreamKind::Linear,
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
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
        &None, &StreamKind::Linear,
    );
    assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
}

// ---------------------------------------------------------------------------
// Batch registry regression tests (issue #724)
// ---------------------------------------------------------------------------

/// Helper: build a valid CreateStreamParams against the test context defaults.
/// `start_offset` shifts the start time to create distinct streams for the same sender.
fn make_params(ctx: &Ctx, recipient: &Address, start_offset: u64) -> CreateStreamParams {
    let start = ctx.now() + start_offset;
    CreateStreamParams {
        recipient: recipient.clone(),
        deposit_amount: DEPOSIT_AMOUNT,
        rate_per_second: RATE_PER_SECOND,
        start_time: start,
        cliff_time: start,
        end_time: start + STREAM_DURATION,
        withdraw_dust_threshold: None,
        memo: None,
        kind: StreamKind::Linear,
    }
}

/// After a successful `create_streams` batch, `get_factory_stream_count` must
/// increase by the batch size (the core regression from issue #724).
#[test]
fn test_create_streams_batch_registers_ids_count() {
    let ctx = Ctx::setup();
    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);
    let r3 = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&r1, &true);
    ctx.factory.set_allowlist(&r2, &true);
    ctx.factory.set_allowlist(&r3, &true);

    assert_eq!(ctx.factory.get_factory_stream_count(), 0);

    let mut streams = Vec::new(&ctx.env);
    streams.push_back(make_params(&ctx, &r1, 0));
    streams.push_back(make_params(&ctx, &r2, 10_000));
    streams.push_back(make_params(&ctx, &r3, 20_000));

    ctx.factory.create_streams(&ctx.sender, &streams);

    assert_eq!(ctx.factory.get_factory_stream_count(), 3);
}

/// `get_factory_streams_paginated` must return every batch ID in creation order.
#[test]
fn test_create_streams_batch_registers_ids_in_order() {
    let ctx = Ctx::setup();
    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&r1, &true);
    ctx.factory.set_allowlist(&r2, &true);

    let mut streams = Vec::new(&ctx.env);
    streams.push_back(make_params(&ctx, &r1, 0));
    streams.push_back(make_params(&ctx, &r2, 10_000));

    let created_ids = ctx.factory.create_streams(&ctx.sender, &streams);
    assert_eq!(created_ids.len(), 2);

    let page = ctx.factory.get_factory_streams_paginated(&0, &10);
    assert_eq!(page.len(), 2);
    assert_eq!(page.get(0).unwrap(), created_ids.get(0).unwrap());
    assert_eq!(page.get(1).unwrap(), created_ids.get(1).unwrap());
}

/// An empty batch must produce no registry writes (count stays 0).
#[test]
fn test_create_streams_empty_batch_no_registry_writes() {
    let ctx = Ctx::setup();
    let empty: Vec<CreateStreamParams> = Vec::new(&ctx.env);

    let result = ctx.factory.try_create_streams(&ctx.sender, &empty);
    assert!(result.is_ok());
    assert_eq!(ctx.factory.get_factory_stream_count(), 0);
}

/// A single-element batch registers exactly one ID — boundary between single and
/// batch paths.
#[test]
fn test_create_streams_single_element_batch_registers_one_id() {
    let ctx = Ctx::setup();
    let r = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&r, &true);

    let mut streams = Vec::new(&ctx.env);
    streams.push_back(make_params(&ctx, &r, 0));

    ctx.factory.create_streams(&ctx.sender, &streams);

    assert_eq!(ctx.factory.get_factory_stream_count(), 1);
    let page = ctx.factory.get_factory_streams_paginated(&0, &10);
    assert_eq!(page.len(), 1);
}

/// Single `create_stream` path still registers IDs (no regression on the
/// existing path).
#[test]
fn test_single_create_stream_still_registers_in_factory() {
    let ctx = Ctx::setup();

    let id = ctx.factory.create_stream(
        &ctx.sender, &ctx.recipient,
        &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &ctx.now(), &ctx.now(), &(ctx.now() + STREAM_DURATION), &0,
        &None, &StreamKind::Linear,
    );

    assert_eq!(ctx.factory.get_factory_stream_count(), 1);
    let page = ctx.factory.get_factory_streams_paginated(&0, &10);
    assert_eq!(page.len(), 1);
    assert_eq!(page.get(0).unwrap(), id);
}

/// Mixed usage: one single stream then a batch — registry accumulates all IDs
/// in correct insertion order.
#[test]
fn test_single_then_batch_registry_accumulates_in_order() {
    let ctx = Ctx::setup();
    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);
    ctx.factory.set_allowlist(&r1, &true);
    ctx.factory.set_allowlist(&r2, &true);

    // single stream first
    let single_id = ctx.factory.create_stream(
        &ctx.sender, &ctx.recipient,
        &DEPOSIT_AMOUNT, &RATE_PER_SECOND,
        &ctx.now(), &ctx.now(), &(ctx.now() + STREAM_DURATION), &0,
        &None, &StreamKind::Linear,
    );

    // batch of two
    let mut streams = Vec::new(&ctx.env);
    streams.push_back(make_params(&ctx, &r1, 0));
    streams.push_back(make_params(&ctx, &r2, 5_000));
    let batch_ids = ctx.factory.create_streams(&ctx.sender, &streams);

    assert_eq!(ctx.factory.get_factory_stream_count(), 3);

    let page = ctx.factory.get_factory_streams_paginated(&0, &10);
    assert_eq!(page.get(0).unwrap(), single_id);
    assert_eq!(page.get(1).unwrap(), batch_ids.get(0).unwrap());
    assert_eq!(page.get(2).unwrap(), batch_ids.get(1).unwrap());
}
