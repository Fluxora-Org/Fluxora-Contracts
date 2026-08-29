//! Regression tests: no success event is emitted when a cross-contract
//! token transfer is reverted.
//!
//! ## Policy (decided here)
//!
//! Soroban transactions are **atomic**. When `push_token` or `pull_token`
//! panics, the host rolls back the entire invocation frame — storage writes
//! AND published events are discarded together. There is therefore no such
//! thing as a "failure event" for these operations: the absence of the
//! success event (`"withdrew"`, `"cancelled"`, `"top_up"`) is itself the
//! signal that the operation did not complete.
//!
//! ## What is tested
//!
//! | Entry-point     | Token call        | Success topic  |
//! |----------------|-------------------|----------------|
//! | `withdraw`      | `push_token`      | `"withdrew"`   |
//! | `cancel_stream` | `push_token`      | `"cancelled"`  |
//! | `top_up_stream` | `pull_token`      | `"top_up"`     |
//! | `create_stream` | `pull_token`      | `"created"`    |
//!
//! Each test:
//! 1. Boots a streaming contract whose token mock **panics on every real
//!    transfer** (non-zero amount).
//! 2. Records the event-log length immediately before the failing call.
//! 3. Calls `try_*` and asserts it returns `Err`.
//! 4. Asserts the event-log length is **unchanged** — no topic leaked.
//!
//! For withdraw / cancel / top-up the stream must already exist before the
//! failing call.  `OnceToken` allows exactly **one** `transfer_from`
//! (consumed by `create_stream`), then panics on every subsequent call,
//! giving us a live stream backed by an otherwise-broken token.
//!
//! ## Running
//! ```
//! cargo test -p fluxora_stream --test event_ordering_failed_transfer -- --nocapture
//! ```

extern crate std;

use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Events, Ledger},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Mock 1 — PanicToken
// Passes the `verify_token_behavior` zero-amount smoke test in `init`, then
// panics on every real transfer.  Used for the `create_stream` failure test.
// ---------------------------------------------------------------------------

#[contract]
pub struct PanicToken;

#[contractimpl]
impl PanicToken {
    pub fn balance(_env: Env, _id: Address) -> i128 {
        0
    }
    /// Zero-amount self-transfer succeeds (init smoke test). Any other amount panics.
    pub fn transfer(_env: Env, _from: Address, _to: Address, amount: i128) {
        assert_eq!(
            amount, 0,
            "PanicToken: transfer always fails for amount > 0"
        );
    }
    pub fn transfer_from(_env: Env, _sp: Address, _from: Address, _to: Address, _amt: i128) {
        panic!("PanicToken: transfer_from always fails");
    }
    pub fn decimals(_env: Env) -> u32 {
        7
    }
}

// ---------------------------------------------------------------------------
// Mock 2 — OnceToken
// Allows exactly ONE transfer_from (the deposit pull in create_stream), then
// panics on every subsequent call.  Used for withdraw / cancel / top-up tests.
// ---------------------------------------------------------------------------

#[contract]
pub struct OnceToken;

#[contractimpl]
impl OnceToken {
    pub fn balance(_env: Env, _id: Address) -> i128 {
        // Return a balance large enough for the contract balance-cap check.
        1_000_000
    }
    pub fn transfer(env: Env, _from: Address, _to: Address, amount: i128) {
        if amount == 0 {
            return;
        } // init smoke test
        let k = symbol_short!("used");
        if env.storage().instance().get::<_, bool>(&k).unwrap_or(false) {
            panic!("OnceToken: transfer already used");
        }
        env.storage().instance().set(&k, &true);
    }
    pub fn transfer_from(env: Env, _sp: Address, _from: Address, _to: Address, amount: i128) {
        if amount == 0 {
            return;
        }
        let k = symbol_short!("used");
        if env.storage().instance().get::<_, bool>(&k).unwrap_or(false) {
            panic!("OnceToken: transfer_from already used");
        }
        env.storage().instance().set(&k, &true);
    }
    pub fn decimals(_env: Env) -> u32 {
        7
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_params(env: &Env, recipient: Address) -> CreateStreamParams {
    CreateStreamParams {
        recipient,
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
    }
}

/// Count events with a given first-topic symbol emitted by `contract_id`.
fn count_topic(env: &Env, contract_id: &Address, topic: &str) -> usize {
    env.events()
        .all()
        .iter()
        .filter(|e| {
            if &e.0 != contract_id {
                return false;
            }
            if let Some(v) = e.1.iter().next() {
                if let Ok(s) = soroban_sdk::Symbol::try_from_val(env, &v) {
                    return s.to_string() == topic;
                }
            }
            false
        })
        .count()
}

// ---------------------------------------------------------------------------
// Test 1 — failed create_stream emits no "created" event
// ---------------------------------------------------------------------------

/// When `pull_token` panics during `create_stream` the entire frame rolls
/// back.  The `"created"` event must not appear and the stream counter must
/// be unchanged.
#[test]
fn failed_create_emits_no_created_event() {
    let env = Env::default();
    env.mock_all_auths();

    let token_id = env.register_contract(None, PanicToken);
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);

    let count_before = client.get_stream_count();
    let events_before = env.events().all().len();

    let result = client.try_create_stream(&sender, &default_params(&env, recipient));

    assert!(
        result.is_err(),
        "create_stream must fail when pull_token panics"
    );
    assert_eq!(
        env.events().all().len(),
        events_before,
        "event log must not grow on reverted create"
    );
    assert_eq!(
        count_topic(&env, &contract_id, "created"),
        0,
        "no 'created' topic on revert"
    );
    assert_eq!(
        client.get_stream_count(),
        count_before,
        "stream counter must not increment on revert"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — failed withdraw emits no "withdrew" event
// ---------------------------------------------------------------------------

/// When `push_token` panics during `withdraw` the `"withdrew"` event must
/// not appear in the log.
#[test]
fn failed_withdraw_emits_no_withdrew_event() {
    let env = Env::default();
    env.mock_all_auths();

    let token_id = env.register_contract(None, OnceToken);
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);

    // create_stream consumes the one allowed transfer_from.
    let stream_id = client.create_stream(&sender, &default_params(&env, recipient));

    // Advance time so there is something withdrawable.
    env.ledger().set_timestamp(500);
    let events_before = env.events().all().len();

    // push_token now panics → withdrawal reverts.
    let result = client.try_withdraw(&stream_id);

    assert!(result.is_err(), "withdraw must fail when push_token panics");
    assert_eq!(
        env.events().all().len(),
        events_before,
        "event log must not grow on reverted withdraw"
    );
    assert_eq!(
        count_topic(&env, &contract_id, "withdrew"),
        0,
        "no 'withdrew' topic on revert"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — failed cancel_stream emits no "cancelled" event
// ---------------------------------------------------------------------------

/// When `push_token` panics during `cancel_stream` (refund path) the
/// `"cancelled"` event must not appear.
#[test]
fn failed_cancel_emits_no_cancelled_event() {
    let env = Env::default();
    env.mock_all_auths();

    let token_id = env.register_contract(None, OnceToken);
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);

    // create_stream consumes the one allowed transfer_from.
    let stream_id = client.create_stream(&sender, &default_params(&env, recipient));

    // Cancel at t=0 triggers a full refund → push_token → panic.
    let events_before = env.events().all().len();

    let result = client.try_cancel_stream(&stream_id);

    assert!(
        result.is_err(),
        "cancel_stream must fail when push_token panics"
    );
    assert_eq!(
        env.events().all().len(),
        events_before,
        "event log must not grow on reverted cancel"
    );
    assert_eq!(
        count_topic(&env, &contract_id, "cancelled"),
        0,
        "no 'cancelled' topic on revert"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — failed top_up_stream emits no "top_up" event
// ---------------------------------------------------------------------------

/// When `pull_token` panics during `top_up_stream` the `"top_up"` event must
/// not appear.
#[test]
fn failed_top_up_emits_no_top_up_event() {
    let env = Env::default();
    env.mock_all_auths();

    let token_id = env.register_contract(None, OnceToken);
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);

    // create_stream consumes the one allowed transfer_from.
    let stream_id = client.create_stream(&sender, &default_params(&env, recipient));

    // Advance inside the stream window so top_up is allowed.
    env.ledger().set_timestamp(100);
    let events_before = env.events().all().len();

    // pull_token now panics → top-up reverts.
    let result = client.try_top_up_stream(&stream_id, &sender, &500);

    assert!(
        result.is_err(),
        "top_up_stream must fail when pull_token panics"
    );
    assert_eq!(
        env.events().all().len(),
        events_before,
        "event log must not grow on reverted top-up"
    );
    assert_eq!(
        count_topic(&env, &contract_id, "top_up"),
        0,
        "no 'top_up' topic on revert"
    );
}
