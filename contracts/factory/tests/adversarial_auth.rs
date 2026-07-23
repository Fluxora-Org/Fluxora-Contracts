//! Adversarial authorization tests for `FluxoraFactory::create_stream`.
//!
//! Issue #647 — The factory's `create_stream` holds a dual-authorization
//! requirement: the sender must authorize both the outer factory invocation
//! **and** the inner `FluxoraStream::create_stream` cross-contract
//! sub-invocation. These tests assert that:
//!
//! 1. A call with **no** sender auth fails (missing top-level auth).
//! 2. A **spoofed sender** (auth supplied for a different address) fails.
//! 3. **Only the factory-level auth** — without the required cross-contract
//!    sub-invocation — fails the auth check.
//! 4. The happy path with correct dual-auth succeeds.

#![cfg(test)]

extern crate std;

use fluxora_factory::{FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, IntoVal,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_DEPOSIT: i128 = 10_000_000;
const MIN_DURATION: u64 = 86_400;
const DEPOSIT: i128 = 100_000;
const RATE: i128 = 1;
const DURATION: u64 = 200_000;
const SENDER_FUNDING: i128 = 1_000_000_000;
const NOW: u64 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Test context — real contracts, no mock boundaries
// ---------------------------------------------------------------------------

struct Ctx {
    env: Env,
    factory: FluxoraFactoryClient,
    stream: FluxoraStreamClient,
    sender: Address,
    recipient: Address,
    factory_id: Address,
    stream_id: Address,
}

impl Ctx {
    fn setup() -> Self {
        let env = Env::default();
        env.ledger().set_timestamp(NOW);

        let stream_id = env.register_contract(None, FluxoraStream);
        let factory_id = env.register_contract(None, FluxoraFactory);

        let stream = FluxoraStreamClient::new(&env, &stream_id);
        let factory = FluxoraFactoryClient::new(&env, &factory_id);

        let token_admin = Address::generate(&env);
        let token_addr = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let stellar_asset = StellarAssetClient::new(&env, &token_addr);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        stellar_asset.mint(&sender, &SENDER_FUNDING);

        // Init both contracts under mock_all_auths, then drop it.
        env.mock_all_auths();
        stream.init(&token_addr, &stream_id);
        factory.init(&admin, &stream_id, &MAX_DEPOSIT, &MIN_DURATION);
        factory.set_allowlist(&recipient, &true);

        Self {
            env,
            factory,
            stream,
            sender,
            recipient,
            factory_id,
            stream_id,
        }
    }

    fn start(&self) -> u64 {
        self.env.ledger().timestamp()
    }
}

// ---------------------------------------------------------------------------
// Helper: build the full dual-auth that `create_stream` legitimately requires.
//
// The sender must authorize:
//   1. The factory invocation (`create_stream` on `factory_id`).
//   2. The cross-contract sub-invocation (`create_stream` on `stream_id`).
// ---------------------------------------------------------------------------
fn dual_auth<'a>(
    env: &'a Env,
    sender: &'a Address,
    factory_id: &'a Address,
    stream_id: &'a Address,
    deposit: i128,
    rate: i128,
    start: u64,
    cliff: u64,
    end: u64,
    recipient: &'a Address,
) -> MockAuth<'a> {
    MockAuth {
        address: sender,
        invoke: &MockAuthInvoke {
            contract: factory_id,
            fn_name: "create_stream",
            args: (
                sender.clone(),
                recipient.clone(),
                deposit,
                rate,
                start,
                cliff,
                end,
                0i128,
            )
                .into_val(env),
            sub_invokes: &[MockAuthInvoke {
                contract: stream_id,
                fn_name: "create_stream",
                args: (
                    sender.clone(),
                    recipient.clone(),
                    deposit,
                    rate,
                    start,
                    cliff,
                    end,
                    0i128,
                )
                    .into_val(env),
                sub_invokes: &[],
            }],
        },
    }
}

// ---------------------------------------------------------------------------
// Test 1: No auth supplied — must fail
// ---------------------------------------------------------------------------

/// Calling `create_stream` with **no** authorization at all must panic/fail.
/// Soroban `require_auth` panics when the auth tree is empty.
#[test]
fn test_create_stream_no_auth_fails() {
    let ctx = Ctx::setup();
    // Explicitly supply an empty auth list — no MockAuth entries.
    ctx.env.mock_auths(&[]);

    let start = ctx.start();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.factory.try_create_stream(
            &ctx.sender,
            &ctx.recipient,
            &DEPOSIT,
            &RATE,
            &start,
            &start,
            &(start + DURATION),
            &0,
        )
    }));

    assert!(
        result.is_err(),
        "create_stream must fail when no auth is provided"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Spoofed sender — auth for a different address must fail
// ---------------------------------------------------------------------------

/// Supplying auth for `attacker` while passing `sender` as the `sender`
/// argument must fail: the factory calls `sender.require_auth()`, which checks
/// that the auth tree contains an entry for `sender`, not `attacker`.
#[test]
fn test_create_stream_spoofed_sender_fails() {
    let ctx = Ctx::setup();
    let attacker = Address::generate(&ctx.env);
    let start = ctx.start();

    // Auth is for `attacker`, but the factory will require auth for `sender`.
    ctx.env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &ctx.factory_id,
            fn_name: "create_stream",
            args: (
                ctx.sender.clone(),
                ctx.recipient.clone(),
                DEPOSIT,
                RATE,
                start,
                start,
                start + DURATION,
                0i128,
            )
                .into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.factory.try_create_stream(
            &ctx.sender,
            &ctx.recipient,
            &DEPOSIT,
            &RATE,
            &start,
            &start,
            &(start + DURATION),
            &0,
        )
    }));

    assert!(
        result.is_err(),
        "create_stream must fail when auth is for a different (spoofed) address"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Factory-level auth only, missing sub-invocation auth — must fail
// ---------------------------------------------------------------------------

/// The sender authorizes the factory invocation but does **not** authorize the
/// cross-contract `create_stream` sub-invocation on the stream contract.
/// The sub-invocation's `require_auth` must cause the transaction to fail.
#[test]
fn test_create_stream_missing_sub_invocation_auth_fails() {
    let ctx = Ctx::setup();
    let start = ctx.start();

    // Auth covers only the factory wrapper — no sub_invokes entry for the stream contract.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.factory_id,
            fn_name: "create_stream",
            args: (
                ctx.sender.clone(),
                ctx.recipient.clone(),
                DEPOSIT,
                RATE,
                start,
                start,
                start + DURATION,
                0i128,
            )
                .into_val(&ctx.env),
            sub_invokes: &[], // <-- missing the stream sub-invocation
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.factory.try_create_stream(
            &ctx.sender,
            &ctx.recipient,
            &DEPOSIT,
            &RATE,
            &start,
            &start,
            &(start + DURATION),
            &0,
        )
    }));

    assert!(
        result.is_err(),
        "create_stream must fail when the cross-contract sub-invocation auth is missing"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Correct dual-auth — happy path must succeed
// ---------------------------------------------------------------------------

/// With both the factory-level and cross-contract sub-invocation auth in place,
/// `create_stream` must succeed and return a valid stream ID.
#[test]
fn test_create_stream_correct_dual_auth_succeeds() {
    let ctx = Ctx::setup();
    let start = ctx.start();

    ctx.env.mock_auths(&[dual_auth(
        &ctx.env,
        &ctx.sender,
        &ctx.factory_id,
        &ctx.stream_id,
        DEPOSIT,
        RATE,
        start,
        start,
        start + DURATION,
        &ctx.recipient,
    )]);

    let result = ctx.factory.try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &DEPOSIT,
        &RATE,
        &start,
        &start,
        &(start + DURATION),
        &0,
    );

    assert!(
        result.is_ok(),
        "create_stream must succeed with correct dual-auth: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// Test 5: Sender tries to authorize as a different recipient's stream
// ---------------------------------------------------------------------------

/// The sender provides full dual-auth but for a *different* recipient than the
/// one passed in the call. The factory's `require_auth` checks the exact
/// arguments, so the mismatched auth must fail.
#[test]
fn test_create_stream_auth_recipient_mismatch_fails() {
    let ctx = Ctx::setup();
    let start = ctx.start();
    let wrong_recipient = Address::generate(&ctx.env);

    // Auth is constructed for `wrong_recipient`, but the call uses `ctx.recipient`.
    ctx.env.mock_auths(&[dual_auth(
        &ctx.env,
        &ctx.sender,
        &ctx.factory_id,
        &ctx.stream_id,
        DEPOSIT,
        RATE,
        start,
        start,
        start + DURATION,
        &wrong_recipient,
    )]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.factory.try_create_stream(
            &ctx.sender,
            &ctx.recipient, // does not match what was authorized
            &DEPOSIT,
            &RATE,
            &start,
            &start,
            &(start + DURATION),
            &0,
        )
    }));

    assert!(
        result.is_err(),
        "create_stream must fail when auth was issued for a different recipient"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Third party attempts to invoke create_stream on behalf of sender
// ---------------------------------------------------------------------------

/// A third party (`attacker`) supplies auth that names `sender` as the
/// invoking address but signs as `attacker`. This models a relay attack where
/// someone tries to create a stream debiting the sender's balance without the
/// sender's consent.
#[test]
fn test_create_stream_third_party_relay_fails() {
    let ctx = Ctx::setup();
    let attacker = Address::generate(&ctx.env);
    let start = ctx.start();

    // Attacker signs their own address — but caller passes `ctx.sender`.
    ctx.env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &ctx.factory_id,
            fn_name: "create_stream",
            args: (
                attacker.clone(), // attacker claims to be the sender
                ctx.recipient.clone(),
                DEPOSIT,
                RATE,
                start,
                start,
                start + DURATION,
                0i128,
            )
                .into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.factory.try_create_stream(
            &ctx.sender, // actual sender argument is ctx.sender, not attacker
            &ctx.recipient,
            &DEPOSIT,
            &RATE,
            &start,
            &start,
            &(start + DURATION),
            &0,
        )
    }));

    assert!(
        result.is_err(),
        "create_stream must fail when a third party tries to relay a stream creation for sender"
    );
}
