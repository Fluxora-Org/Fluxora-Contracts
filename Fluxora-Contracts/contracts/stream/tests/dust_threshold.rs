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
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        TestContext {
            env,
            contract_id,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }
}

#[test]
fn test_withdraw_dust_threshold_enforced() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream with 100 threshold
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &100_i128, // threshold = 100
        &None,
    );

    // At t=50, withdrawable is 50. Threshold is 100.
    ctx.env.ledger().set_timestamp(50);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 0, "should return 0 when below threshold");
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    // At t=150, withdrawable is 150. Threshold is 100.
    ctx.env.ledger().set_timestamp(150);
    let withdrawn2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn2, 150, "should allow withdrawal above threshold");
    assert_eq!(ctx.token.balance(&ctx.recipient), 150);
}

#[test]
fn test_withdraw_dust_threshold_ignored_on_final_drain() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream with 100 threshold
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &500_i128, // threshold = 500
        &None,
    );

    // Withdraw 950 first (above threshold)
    ctx.env.ledger().set_timestamp(950);
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.token.balance(&ctx.recipient), 950);

    // Now 50 remains. Threshold is 500.
    // At t=1000, 50 more is accrued. Total 1000.
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 50,
        "should allow final drain even if below threshold"
    );
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
}

#[test]
fn test_withdraw_dust_threshold_ignored_in_terminal_state() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream with 100 threshold
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &500_i128, // threshold = 500
        &None,
    );

    // Cancel stream at t=100.
    ctx.env.ledger().set_timestamp(100);
    ctx.client().cancel_stream(&stream_id);
    // 100 accrued. Threshold 500.

    // Recipient tries to withdraw.
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 100,
        "should allow withdrawal in terminal state (Cancelled) even if below threshold"
    );
    assert_eq!(ctx.token.balance(&ctx.recipient), 100);
}

#[test]
fn test_withdraw_dust_threshold_ignored_past_end_time() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream with 500 threshold
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &500_i128,
        &None,
    );

    // Withdraw 900 at t=900 (above threshold)
    ctx.env.ledger().set_timestamp(900);
    ctx.client().withdraw(&stream_id);

    // At t=1100 (past end_time), 100 remains. Threshold 500.
    ctx.env.ledger().set_timestamp(1100);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 100,
        "should allow withdrawal past end_time even if below threshold"
    );
}
