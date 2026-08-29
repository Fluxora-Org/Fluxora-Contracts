//! Regression tests: no success event is emitted when a cross-contract
//! token transfer is reverted.
//!
//! ## Policy (decided here)
//!
//! Soroban transactions are **atomic**. When the token transfer inside
//! `create_stream`, `withdraw`, `cancel` or `top_up` fails, the host rolls
//! back the entire invocation frame — storage writes AND published events are
//! discarded together. There is therefore no such thing as a "failure event"
//! for these operations: the absence of the success event (`stream_created`,
//! `withdrawn`, `cancelled`, `topped_up`) is itself the signal that the
//! operation did not complete.
//!
//! The test host enforces this: `Events::all()` reports only the most recent
//! invocation and filters out events recorded on a `failed_call` frame, so a
//! reverted operation leaves an **empty** observable event log. These tests
//! pin that guarantee down so a future edit cannot quietly move an emission
//! before the token call.
//!
//! ## What is tested
//!
//! | Entry-point      | Token call | Success topic  |
//! |-----------------|------------|----------------|
//! | `withdraw`       | `transfer` | `"withdrawn"`  |
//! | `cancel`         | `transfer` | `"cancelled"`  |
//! | `top_up`         | `transfer` | `"topped_up"`  |
//! | `create_stream`  | `transfer` | `"stream_created"` |
//!
//! Each test:
//! 1. Boots a streaming contract whose token mock **panics on every real
//!    transfer** (non-zero amount).
//! 2. Calls `try_*` and asserts it returns `Err`.
//! 3. Asserts the observable event log is **empty** — no topic leaked — and
//!    that storage is unchanged too.
//!
//! For withdraw / cancel / top-up the stream must already exist before the
//! failing call. `OnceToken` allows exactly **one** `transfer` (consumed by
//! `create_stream`), then panics on every subsequent call, giving us a live
//! stream backed by an otherwise-broken token.
//!
//! ## Running
//! ```
//! cargo test -p fluxora-stream --test event_ordering_failed_transfer -- --nocapture
//! ```

extern crate std;

use fluxora_stream::{Error, FluxoraStream, FluxoraStreamClient, Stream, StreamStatus};
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Events, Ledger},
    xdr::ContractEventBody,
    Address, Env, Symbol, TryFromVal,
};

// ---------------------------------------------------------------------------
// Mock 1 — PanicToken
// Panics on every real transfer (non-zero amount). Used for the
// `create_stream` failure test, where the very first token call must fail.
// ---------------------------------------------------------------------------

#[contract]
pub struct PanicToken;

#[contractimpl]
impl PanicToken {
    /// Zero-amount transfers succeed; any other amount panics.
    pub fn transfer(_env: Env, _from: Address, _to: Address, amount: i128) {
        assert_eq!(
            amount, 0,
            "PanicToken: transfer always fails for amount > 0"
        );
    }
}

// ---------------------------------------------------------------------------
// Mock 2 — OnceToken
// Allows exactly ONE transfer (the deposit pull in create_stream), then
// panics on every subsequent call. Used for withdraw / cancel / top-up tests.
// ---------------------------------------------------------------------------

#[contract]
pub struct OnceToken;

#[contractimpl]
impl OnceToken {
    pub fn transfer(env: Env, _from: Address, _to: Address, amount: i128) {
        if amount == 0 {
            return;
        }
        let k = symbol_short!("used");
        if env.storage().instance().get::<_, bool>(&k).unwrap_or(false) {
            panic!("OnceToken: transfer already used");
        }
        env.storage().instance().set(&k, &true);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The events the *stream* contract published during the most recent call.
///
/// `Events::all()` only reports the most recent invocation, so this must be
/// the first thing read after the failing call — any further contract call
/// (even a read-only view) replaces the snapshot.
fn stream_events(
    env: &Env,
    contract_id: &Address,
) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    env.events()
        .all()
        .filter_by_contract(contract_id)
        .events()
        .to_vec()
}

/// Count the events whose first topic is the given symbol string.
fn count_topic(env: &Env, contract_id: &Address, topic: &str) -> usize {
    stream_events(env, contract_id)
        .iter()
        .filter(|e| {
            // `ContractEventBody` has a single `V0` variant, so this cannot
            // fail; the destructure is irrefutable.
            let ContractEventBody::V0(v0) = &e.body;
            let Some(first) = v0.topics.first() else {
                return false;
            };
            Symbol::try_from_val(env, first)
                .map(|s| s.to_string() == topic)
                .unwrap_or(false)
        })
        .count()
}

/// Boot a stream contract whose deposit pull succeeds exactly once, then
/// panics on every subsequent transfer. Returns the client, the stream
/// contract id, the token id and the created stream id.
fn live_stream_with_once_token(
    env: &Env,
) -> (
    FluxoraStreamClient<'_>,
    soroban_sdk::Address,
    soroban_sdk::Address,
    u64,
) {
    let token_id = env.register(OnceToken, ());
    let contract_id = env.register(FluxoraStream, ());
    let client = FluxoraStreamClient::new(env, &contract_id);

    let sender = Address::generate(env);
    let recipient = Address::generate(env);
    env.ledger().set_timestamp(0);

    // create_stream consumes the one allowed transfer.
    let stream_id = client.create_stream(
        &sender, &recipient, &token_id, &1_000, &0, &1_000, &0, &true, &true, &true,
    );
    (client, contract_id, token_id, stream_id)
}

// ---------------------------------------------------------------------------
// Test 1 — failed create_stream emits no "stream_created" event
// ---------------------------------------------------------------------------

/// When the deposit pull panics during `create_stream` the entire frame rolls
/// back. The `"stream_created"` event must not appear, no stream id may be
/// consumed, and no funds may move.
#[test]
fn failed_create_emits_no_created_event() {
    let env = Env::default();
    env.mock_all_auths();

    let token_id = env.register(PanicToken, ());
    let contract_id = env.register(FluxoraStream, ());
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);

    let count_before = client.stream_count();

    let result = client.try_create_stream(
        &sender, &recipient, &token_id, &1_000, &0, &1_000, &0, &true, &true, &true,
    );

    assert!(result.is_err(), "create_stream must fail when pull panics");
    assert!(
        env.events().all().events().is_empty(),
        "a reverted create publishes no events",
    );
    assert_eq!(
        count_topic(&env, &contract_id, "stream_created"),
        0,
        "no 'stream_created' topic on revert",
    );
    assert_eq!(
        client.stream_count(),
        count_before,
        "stream counter must not increment on revert",
    );
}

// ---------------------------------------------------------------------------
// Test 2 — failed withdraw emits no "withdrawn" event
// ---------------------------------------------------------------------------

/// When the payout transfer panics during `withdraw` the `"withdrawn"` event
/// must not appear and the stream's accounting must be untouched.
#[test]
fn failed_withdraw_emits_no_withdrew_event() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let (client, contract_id, _token_id, stream_id) = live_stream_with_once_token(&env);

    // Advance so there is something withdrawable; the payout now panics.
    env.ledger().set_timestamp(500);

    let result = client.try_withdraw(&stream_id, &None);

    assert!(result.is_err(), "withdraw must fail when the payout panics");
    assert!(
        env.events().all().events().is_empty(),
        "a reverted withdraw publishes no events",
    );
    assert_eq!(
        count_topic(&env, &contract_id, "withdrawn"),
        0,
        "no 'withdrawn' topic on revert",
    );

    // Accounting untouched: nothing withdrawn, still active.
    let s: Stream = client.get_stream(&stream_id);
    assert_eq!(s.withdrawn, 0, "reverted withdraw must not move accounting");
    assert_eq!(s.status, StreamStatus::Active);
}

// ---------------------------------------------------------------------------
// Test 3 — failed cancel emits no "cancelled" event
// ---------------------------------------------------------------------------

/// When the refund transfer panics during `cancel` the `"cancelled"` event
/// must not appear and the stream must still be live and whole.
#[test]
fn failed_cancel_emits_no_cancelled_event() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let (client, contract_id, _token_id, stream_id) = live_stream_with_once_token(&env);

    // Cancel at t=0 triggers a full refund → transfer → panic.
    let result = client.try_cancel(&stream_id);

    assert!(result.is_err(), "cancel must fail when the refund panics");
    assert!(
        env.events().all().events().is_empty(),
        "a reverted cancel publishes no events",
    );
    assert_eq!(
        count_topic(&env, &contract_id, "cancelled"),
        0,
        "no 'cancelled' topic on revert",
    );

    // The stream is still live with its full deposit.
    let s: Stream = client.get_stream(&stream_id);
    assert_eq!(s.status, StreamStatus::Active, "cancel must not stick");
    assert_eq!(
        s.deposited, 1_000,
        "reverted cancel must not shrink the deposit"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — failed top_up emits no "topped_up" event
// ---------------------------------------------------------------------------

/// When the pull transfer panics during `top_up` the `"topped_up"` event must
/// not appear and the schedule must be unchanged.
#[test]
fn failed_top_up_emits_no_top_up_event() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let (client, contract_id, _token_id, stream_id) = live_stream_with_once_token(&env);

    // Advance inside the stream window so top_up is allowed.
    env.ledger().set_timestamp(100);

    let result = client.try_top_up(&stream_id, &500);

    assert!(result.is_err(), "top_up must fail when the pull panics");
    assert!(
        env.events().all().events().is_empty(),
        "a reverted top-up publishes no events",
    );
    assert_eq!(
        count_topic(&env, &contract_id, "topped_up"),
        0,
        "no 'topped_up' topic on revert",
    );

    // Schedule and deposit unchanged.
    let s: Stream = client.get_stream(&stream_id);
    assert_eq!(s.deposited, 1_000, "reverted top-up must not add funds");
    assert_eq!(
        s.end_time, 1_000,
        "reverted top-up must not extend the schedule"
    );
}

// ---------------------------------------------------------------------------
// Sanity: the mocks actually fail the way the tests assume
// ---------------------------------------------------------------------------

/// The failing-call paths above all expect a typed stream error — the token
/// failure is bucketed, never leaked as a raw host abort. This pins the
/// mapping so a future change to `token_transfer` cannot silently turn these
/// into panics that pass for the wrong reason.
#[test]
fn reverted_token_calls_surface_typed_errors() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let (client, _contract_id, _token_id, stream_id) = live_stream_with_once_token(&env);

    env.ledger().set_timestamp(500);
    let err = client.try_withdraw(&stream_id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    let err = client.try_top_up(&stream_id, &500).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);
}
