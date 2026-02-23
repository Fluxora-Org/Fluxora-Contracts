extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        let token = TokenClient::new(&env, &token_id);

        Self {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
        )
    }

    fn create_stream_with_cliff(&self, cliff_time: u64) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &cliff_time,
            &1000u64,
        )
    }
}

#[test]
fn init_sets_config_and_keeps_token_address() {
    let ctx = TestContext::setup();

    let config = ctx.client().get_config();
    assert_eq!(config.admin, ctx.admin);
    assert_eq!(config.token, ctx.token_id);
}

#[test]
#[should_panic(expected = "already initialised")]
fn init_twice_panics() {
    let ctx = TestContext::setup();
    ctx.client().init(&ctx.token_id, &ctx.admin);
}

#[test]
fn create_stream_persists_state_and_moves_deposit() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();
    let state = ctx.client().get_stream_state(&stream_id);

    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 0);
    assert_eq!(state.end_time, 1000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
}

#[test]
fn withdraw_accrued_amount_updates_balances_and_state() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(250);
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(withdrawn, 250);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 250);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.recipient), 250);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 750);
}

#[test]
#[should_panic(expected = "nothing to withdraw")]
fn withdraw_before_cliff_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream_with_cliff(500);

    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&stream_id);
}

#[test]
fn get_stream_state_returns_latest_status() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn full_lifecycle_create_withdraw_to_completion() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Mid-stream withdrawal.
    ctx.env.ledger().set_timestamp(400);
    let first = ctx.client().withdraw(&stream_id);
    assert_eq!(first, 400);

    // Final withdrawal at end of stream should complete the stream.
    ctx.env.ledger().set_timestamp(1000);
    let second = ctx.client().withdraw(&stream_id);
    assert_eq!(second, 600);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);

    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

#[test]
#[should_panic(expected = "stream not found")]
fn get_stream_state_unknown_id_panics() {
    let ctx = TestContext::setup();
    ctx.client().get_stream_state(&99);
}

#[test]
fn create_stream_rejects_underfunded_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
        );
    }));

    assert!(result.is_err());
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

#[test]
fn harness_mints_sender_balance() {
    let ctx = TestContext::setup();
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
}

/// End-to-end integration test: create stream, advance time in steps,
/// withdraw multiple times, verify amounts and final Completed status.
///
/// This test covers:
/// - Stream creation and initial state
/// - Multiple partial withdrawals at different time points
/// - Balance verification after each withdrawal
/// - Final withdrawal that completes the stream
/// - Status transition to Completed
/// - Correct final balances for all parties
#[test]
fn integration_full_flow_multiple_withdraws_to_completed() {
    let ctx = TestContext::setup();

    // Initial balances
    let sender_initial = ctx.token.balance(&ctx.sender);
    assert_eq!(sender_initial, 10_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Create stream: 5000 tokens over 5000 seconds (1 token/sec), no cliff
    ctx.env.ledger().set_timestamp(1000);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &6000u64,
    );

    // Verify stream created and deposit transferred
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 1000);
    assert_eq!(state.end_time, 6000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.sender), 5_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 5_000);

    // First withdrawal at 20% progress (1000 seconds elapsed)
    ctx.env.ledger().set_timestamp(2000);
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4000);

    // Second withdrawal at 50% progress (1500 more seconds)
    ctx.env.ledger().set_timestamp(3500);
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 1500);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 2500);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2500);

    // Third withdrawal at 80% progress (1000 more seconds)
    ctx.env.ledger().set_timestamp(4500);
    let withdrawn_3 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_3, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 3500);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 3500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1500);

    // Final withdrawal at 100% (end_time reached)
    ctx.env.ledger().set_timestamp(6000);
    let withdrawn_4 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_4, 1500);

    // Verify stream is now Completed
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 5000);
    assert_eq!(state.status, StreamStatus::Completed);

    // Verify final balances
    assert_eq!(ctx.token.balance(&ctx.recipient), 5000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    assert_eq!(ctx.token.balance(&ctx.sender), 5000);

    // Verify total withdrawn equals deposit
    assert_eq!(withdrawn_1 + withdrawn_2 + withdrawn_3 + withdrawn_4, 5000);
}

/// Integration test: multiple withdrawals with time advancement beyond end_time.
/// Verifies that accrual caps at deposit_amount and status transitions correctly.
#[test]
fn integration_withdraw_beyond_end_time() {
    let ctx = TestContext::setup();

    // Create stream: 2000 tokens over 1000 seconds (2 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    // Withdraw at 25%
    ctx.env.ledger().set_timestamp(250);
    let w1 = ctx.client().withdraw(&stream_id);
    assert_eq!(w1, 500);

    // Withdraw at 75%
    ctx.env.ledger().set_timestamp(750);
    let w2 = ctx.client().withdraw(&stream_id);
    assert_eq!(w2, 1000);

    // Advance time well beyond end_time
    ctx.env.ledger().set_timestamp(5000);
    let w3 = ctx.client().withdraw(&stream_id);
    assert_eq!(w3, 500); // Only remaining 500, not more

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 2000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2000);
}
