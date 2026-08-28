//! Regression tests: event ordering around failed cross-contract token calls.
//!
//! # Policy
//!
//! Soroban transactions are atomic. If the token transfer panics (insufficient
//! balance, allowance revoked, or a malicious token that always reverts), the
//! entire transaction is rolled back — including any storage mutations AND any
//! events that were published during that invocation.
//!
//! Therefore: **a consumer can never observe a success event (`withdrew`,
//! `cancelled`, `top_up`) for a transaction that was ultimately reverted.**
//! No explicit "failure event" is needed; the absence of the success event IS
//! the signal.
//!
//! # What these tests verify
//!
//! For each of the three lifecycle operations that cross-call the token contract:
//!
//! | Operation        | Token call   | Success event |
//! |-----------------|--------------|---------------|
//! | `withdraw`       | `push_token` | `"withdrew"`  |
//! | `cancel_stream`  | `push_token` | `"cancelled"` |
//! | `top_up_stream`  | `pull_token` | `"top_up"`    |
//!
//! Each test:
//! 1. Registers a `PanicToken` — a contract whose `transfer` /
//!    `transfer_from` always panics.
//! 2. Initialises a streaming contract backed by that token (bypassing the
//!    normal `verify_token_behavior` guard via a compliant `balance` impl).
//! 3. Records the event-log length before the failing call.
//! 4. Confirms the call panics (via `try_*` returning `Err`).
//! 5. Asserts the event-log length is **unchanged** — no partial event was
//!    left behind.
//!
//! Run with:
//! ```bash
//! cargo test -p fluxora-stream events -- --nocapture
//! ```

extern crate std;

use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Events, Ledger},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Failing token mock
// ---------------------------------------------------------------------------

/// A minimal SEP-41 lookalike whose `transfer` and `transfer_from` always panic.
///
/// `balance` returns 0 normally so that `verify_token_behavior` (which only
/// calls `transfer(self, self, 0)` as a smoke test) passes during `init`.
/// The zero-amount self-transfer path in `transfer` succeeds; any non-zero
/// amount panics, simulating an always-reverting token.
#[contract]
pub struct PanicToken;

#[contractimpl]
impl PanicToken {
    /// Always returns 0 — satisfies `verify_token_behavior`.
    pub fn balance(_env: Env, _id: Address) -> i128 {
        0
    }

    /// Zero-amount self-transfer succeeds (SEP-41 smoke test in `init`).
    /// Any real transfer panics.
    pub fn transfer(_env: Env, _from: Address, _to: Address, amount: i128) {
        if amount != 0 {
            panic!("PanicToken: transfer always fails");
        }
    }

    /// Always panics — used by `pull_token` / `push_token` paths that call
    /// `transfer_from`.
    pub fn transfer_from(
        _env: Env,
        _spender: Address,
        _from: Address,
        _to: Address,
        _amount: i128,
    ) {
        panic!("PanicToken: transfer_from always fails");
    }

    /// Decimals shim — not called by the streaming contract, but some SDK
    /// introspection paths expect it to exist.
    pub fn decimals(_env: Env) -> u32 {
        7
    }
}

// ---------------------------------------------------------------------------
// Test context (PanicToken — used for failed create test only)
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    #[allow(dead_code)]
    contract_id: Address,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let token_id = env.register_contract(None, PanicToken);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.init(&token_id, &admin);

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        env.ledger().set_timestamp(0);

        Ctx {
            env,
            client,
            sender,
            recipient,
            contract_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Semi-panic token: allows the first transfer_from (create deposit pull),
// then panics on every subsequent transfer/transfer_from call.
// ---------------------------------------------------------------------------

mod once_token {
    use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env};

    #[contract]
    pub struct OnceToken;

    #[contractimpl]
    impl OnceToken {
        pub fn balance(_env: Env, _id: Address) -> i128 {
            // Return a large enough balance so balance-cap checks pass.
            1_000_000
        }

        pub fn transfer(env: Env, _from: Address, _to: Address, amount: i128) {
            if amount == 0 {
                return; // zero-amount smoke test in verify_token_behavior
            }
            let key = symbol_short!("used");
            let used: bool = env.storage().instance().get(&key).unwrap_or(false);
            if used {
                panic!("OnceToken: transfer not allowed after first use");
            }
            env.storage().instance().set(&key, &true);
        }

        pub fn transfer_from(
            env: Env,
            _spender: Address,
            _from: Address,
            _to: Address,
            amount: i128,
        ) {
            if amount == 0 {
                return;
            }
            let key = symbol_short!("used");
            let used: bool = env.storage().instance().get(&key).unwrap_or(false);
            if used {
                panic!("OnceToken: transfer_from not allowed after first use");
            }
            env.storage().instance().set(&key, &true);
        }

        pub fn decimals(_env: Env) -> u32 {
            7
        }
    }
}
use once_token::OnceToken;

/// A context backed by `OnceToken`: the first token transfer succeeds
/// (stream creation deposit pull), every subsequent one panics.
struct OnceCtx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
}

impl<'a> OnceCtx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let token_id = env.register_contract(None, OnceToken);
        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.init(&token_id, &admin);

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        env.ledger().set_timestamp(0);

        OnceCtx {
            env,
            client,
            sender,
            recipient,
        }
    }

    /// Create a stream — consumes the one allowed token transfer.
    fn create_stream(&self) -> u64 {
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1000,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Regression 1: failed withdrawal emits no "withdrew" event
// ---------------------------------------------------------------------------

/// Policy: when `push_token` panics during `withdraw`, the transaction is
/// rolled back atomically. No `"withdrew"` event must appear in the log.
#[test]
fn failed_withdraw_emits_no_withdrew_event() {
    let ctx = OnceCtx::setup();
    // Consume the one allowed transfer on stream creation.
    let stream_id = ctx.create_stream();

    // Advance time so there is something to withdraw.
    ctx.env.ledger().set_timestamp(500);

    let events_before = ctx.env.events().all().len();

    // withdraw calls push_token which panics on the second transfer.
    let result = ctx.client.try_withdraw(&stream_id);
    assert!(
        result.is_err(),
        "withdraw must fail when push_token panics"
    );

    // Event log must be unchanged — no partial "withdrew" event.
    let events_after = ctx.env.events().all().len();
    assert_eq!(
        events_after, events_before,
        "no events must be emitted for a reverted withdrawal"
    );

    // Double-check no "withdrew" topic is present at all since creation.
    let all_events = ctx.env.events().all();
    let withdrew_count = all_events
        .iter()
        .filter(|e| {
            if let Some(topic_val) = e.1.iter().next() {
                if let Ok(sym) = soroban_sdk::Symbol::try_from_val(&ctx.env, &topic_val) {
                    return sym.to_string() == "withdrew";
                }
            }
            false
        })
        .count();
    assert_eq!(
        withdrew_count, 0,
        "\"withdrew\" topic must never appear when withdrawal is reverted"
    );
}

// ---------------------------------------------------------------------------
// Regression 2: failed cancellation emits no "cancelled" event
// ---------------------------------------------------------------------------

/// Policy: when `push_token` panics during `cancel_stream` (refund path),
/// the transaction rolls back. No `"cancelled"` event must appear.
#[test]
fn failed_cancel_emits_no_cancelled_event() {
    let ctx = OnceCtx::setup();
    // Consume the one allowed transfer.
    let stream_id = ctx.create_stream();

    // Cancel at t=0: full refund path — push_token fires immediately.
    ctx.env.ledger().set_timestamp(0);

    let events_before = ctx.env.events().all().len();

    let result = ctx.client.try_cancel_stream(&stream_id);
    assert!(
        result.is_err(),
        "cancel_stream must fail when push_token panics"
    );

    let events_after = ctx.env.events().all().len();
    assert_eq!(
        events_after, events_before,
        "no events must be emitted for a reverted cancellation"
    );

    let all_events = ctx.env.events().all();
    let cancelled_count = all_events
        .iter()
        .filter(|e| {
            if let Some(topic_val) = e.1.iter().next() {
                if let Ok(sym) = soroban_sdk::Symbol::try_from_val(&ctx.env, &topic_val) {
                    return sym.to_string() == "cancelled";
                }
            }
            false
        })
        .count();
    assert_eq!(
        cancelled_count, 0,
        "\"cancelled\" topic must never appear when cancellation is reverted"
    );
}

// ---------------------------------------------------------------------------
// Regression 3: failed top-up emits no "top_up" event
// ---------------------------------------------------------------------------

/// Policy: when `pull_token` panics during `top_up_stream`, the transaction
/// rolls back. No `"top_up"` event must appear.
///
/// For this test we need the CREATE to succeed (so stream exists) but the
/// TOP-UP transfer to fail.  We use a fresh `OnceCtx` — create consumes the
/// one allowed transfer, leaving top_up unable to pull tokens.
#[test]
fn failed_top_up_emits_no_top_up_event() {
    let ctx = OnceCtx::setup();
    let stream_id = ctx.create_stream(); // consumes the one token transfer

    ctx.env.ledger().set_timestamp(100); // still before end_time=1000

    let events_before = ctx.env.events().all().len();

    let result = ctx.client.try_top_up_stream(&stream_id, &ctx.sender, &500);
    assert!(
        result.is_err(),
        "top_up_stream must fail when pull_token panics"
    );

    let events_after = ctx.env.events().all().len();
    assert_eq!(
        events_after, events_before,
        "no events must be emitted for a reverted top-up"
    );

    let all_events = ctx.env.events().all();
    let top_up_count = all_events
        .iter()
        .filter(|e| {
            if let Some(topic_val) = e.1.iter().next() {
                if let Ok(sym) = soroban_sdk::Symbol::try_from_val(&ctx.env, &topic_val) {
                    return sym.to_string() == "top_up";
                }
            }
            false
        })
        .count();
    assert_eq!(
        top_up_count, 0,
        "\"top_up\" topic must never appear when top-up is reverted"
    );
}

// ---------------------------------------------------------------------------
// Regression 4: create_stream with failing pull emits no "created" event
// ---------------------------------------------------------------------------

/// Policy: if `pull_token` panics during `create_stream`, no `"created"`
/// event must be observable and the stream counter must not increment.
///
/// Uses `PanicToken` directly (all transfers panic, including the initial
/// deposit pull during create).
#[test]
fn failed_create_emits_no_created_event() {
    let ctx = Ctx::setup(); // backed by PanicToken

    let events_before = ctx.env.events().all().len();
    let count_before = ctx.client.get_stream_count();

    let result = ctx.client.try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    assert!(
        result.is_err(),
        "create_stream must fail when pull_token panics"
    );

    // Event count and stream counter must be unchanged.
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no events must be emitted for a reverted create_stream"
    );
    assert_eq!(
        ctx.client.get_stream_count(),
        count_before,
        "stream counter must not increment when create_stream reverts"
    );
}
