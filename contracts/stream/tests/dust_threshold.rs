#![cfg(any())]

extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
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

#[test]
fn test_create_stream_rejects_excessive_dust_threshold() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Try to create stream with threshold (1100) > deposit (1000)
    let res = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &1100_i128, // threshold > deposit
        &None,
    );

    match res {
        Err(Ok(fluxora_stream::ContractError::InvalidDustThreshold)) => {}
        _ => panic!("Expected InvalidDustThreshold error, got {:?}", res),
    }
}

// ---------------------------------------------------------------------------
// Additional coverage
// ---------------------------------------------------------------------------

/// threshold = 0 is a no-op: every withdrawal, including micro-amounts, is allowed.
#[test]
fn test_zero_threshold_allows_all_withdrawals() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128, // no filter
        &None,
    );

    // At t=1, only 1 raw unit has accrued — still allowed with threshold=0.
    ctx.env.ledger().set_timestamp(1);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1, "threshold=0 must allow any positive amount");
    assert_eq!(ctx.token.balance(&ctx.recipient), 1);
}

/// threshold = deposit_amount is valid at creation but blocks all non-terminal
/// withdrawals; the recipient can only drain after end_time.
#[test]
fn test_threshold_equal_to_deposit_blocks_until_terminal() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1000_i128;
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &deposit, // threshold == deposit
        &None,
    );

    // Mid-stream: withdrawable < deposit → blocked.
    ctx.env.ledger().set_timestamp(500);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 0,
        "threshold == deposit should block mid-stream withdrawals"
    );

    // Past end_time: terminal bypass kicks in.
    ctx.env.ledger().set_timestamp(1001);
    let withdrawn_terminal = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn_terminal, 1000,
        "terminal bypass must allow full drain when threshold == deposit"
    );
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
}

/// batch_withdraw respects the dust threshold for each stream in the batch.
#[test]
fn test_batch_withdraw_respects_dust_threshold() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Stream A: threshold = 200 (will be below threshold at t=100)
    let stream_a = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &200_i128,
        &None,
    );

    // Stream B: threshold = 0 (always allowed)
    let stream_b = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
        &None,
    );

    ctx.env.ledger().set_timestamp(100);
    let results = ctx
        .client()
        .batch_withdraw(&ctx.recipient, &vec![&ctx.env, stream_a, stream_b]);

    // Stream A: 100 < 200 threshold → 0
    assert_eq!(results.get(0).unwrap().amount, 0);
    // Stream B: 100 >= 0 threshold → 100
    assert_eq!(results.get(1).unwrap().amount, 100);

    assert_eq!(ctx.token.balance(&ctx.recipient), 100);
}

/// Threshold exactly at withdrawable amount is allowed (check is strictly less-than).
#[test]
fn test_threshold_exactly_at_withdrawable_is_allowed() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // rate=1, threshold=100 → at t=100, withdrawable==threshold → allowed
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &100_i128,
        &None,
    );

    ctx.env.ledger().set_timestamp(100);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 100,
        "withdrawable == threshold must be allowed (strictly less-than check)"
    );
}

/// Short-duration stream: threshold larger than per-second accrual but smaller than
/// total deposit. Recipient is blocked mid-stream and can only withdraw at end_time.
#[test]
fn test_short_stream_threshold_blocks_until_end_time() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // 10 s stream, rate=100/s, deposit=1000, threshold=600
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &100_i128,
        &0u64,
        &0u64,
        &10u64,
        &600_i128, // requires 6 s of accrual before first withdrawal
        &None,
    );

    // At t=5: 500 accrued < 600 threshold → blocked
    ctx.env.ledger().set_timestamp(5);
    assert_eq!(ctx.client().withdraw(&stream_id), 0);

    // At t=7: 700 accrued > 600 threshold → allowed
    ctx.env.ledger().set_timestamp(7);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 700);
    assert_eq!(ctx.token.balance(&ctx.recipient), 700);
}

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

#[test]
fn test_create_stream_rejects_excessive_dust_threshold() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Try to create stream with threshold (1100) > deposit (1000)
    let res = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &1100_i128, // threshold > deposit
        &None,
    );

    match res {
        Err(Ok(fluxora_stream::ContractError::InvalidDustThreshold)) => {}
        _ => panic!("Expected InvalidDustThreshold error, got {:?}", res),
    }
}
