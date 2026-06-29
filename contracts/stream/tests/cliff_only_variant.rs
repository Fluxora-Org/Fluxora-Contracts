extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
    contract_id: Address,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let token = TokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        // Fund sender with tokens
        let stellar_asset = env.register_stellar_asset_contract_v2(token_admin);
        let token_admin_client = StellarAssetClient::new(&env, &token_id);
        token_admin_client.mint(&sender, &10_000);

        Self {
            env,
            client,
            sender,
            recipient,
            token,
            contract_id,
        }
    }

    fn create_cliff_only_stream(&self, deposit: i128, start: u64, cliff: u64, end: u64) -> u64 {
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &deposit,
            &0_i128, // rate forced/set to 0 for CliffOnly
            &start,
            &cliff,
            &end,
            &0,    // dust threshold
            &None, // memo
            &StreamKind::CliffOnly,
        )
    }
}

/// 1. Timeline Boundary Testing:
/// - Query exactly 1 second before the cliff timestamp (cliff_time - 1) and assert accrued/withdrawable is exactly 0.
/// - Query exactly at the cliff timestamp (cliff_time) and assert accrued/withdrawable is deposit_amount.
/// - Query in the future (cliff_time + 100) and assert accrued is capped at deposit_amount.
#[test]
fn test_cliff_only_accrual_timeline() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1000_i128;
    let start = 100u64;
    let cliff = 500u64;
    let end = 1000u64;

    let stream_id = ctx.create_cliff_only_stream(deposit, start, cliff, end);

    // 1 second before cliff (t = 499)
    ctx.env.ledger().set_timestamp(499);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), 0);
    assert_eq!(ctx.client.get_withdrawable(&stream_id), 0);
    assert_eq!(ctx.client.get_claimable_at(&stream_id, &499), 0);

    // Exactly at cliff (t = 500)
    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), deposit);
    assert_eq!(ctx.client.get_withdrawable(&stream_id), deposit);
    assert_eq!(ctx.client.get_claimable_at(&stream_id, &500), deposit);

    // Long after cliff (t = 800)
    ctx.env.ledger().set_timestamp(800);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), deposit);
    assert_eq!(ctx.client.get_withdrawable(&stream_id), deposit);
    assert_eq!(ctx.client.get_claimable_at(&stream_id, &800), deposit);
}

/// 2. Mutation Restriction Verification:
/// Assert all mutations are rejected with UnsupportedStreamKind.
#[test]
fn test_cliff_only_rejects_mutations() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_only_stream(1000, 100, 500, 1000);

    // Ensure CliffOnly stream kind is stored properly
    let state = ctx.client.get_stream_state(&stream_id);
    assert!(matches!(state.kind, StreamKind::CliffOnly));

    // Attempt: update_rate_per_second
    let res = ctx.client.try_update_rate_per_second(&stream_id, &5_i128);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedStreamKind)));

    // Attempt: decrease_rate_per_second
    let res = ctx.client.try_decrease_rate_per_second(&stream_id, &2_i128);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedStreamKind)));

    // Attempt: shorten_stream_end_time
    let res = ctx.client.try_shorten_stream_end_time(&stream_id, &800u64);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedStreamKind)));

    // Attempt: extend_stream_end_time
    let res = ctx.client.try_extend_stream_end_time(&stream_id, &1200u64);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedStreamKind)));

    // Attempt: top_up_stream
    let res = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &500_i128);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedStreamKind)));
}

/// 3. Withdrawal Verification:
/// - Recipient cannot withdraw before cliff.
/// - Recipient can successfully withdraw 100% of deposit after cliff.
#[test]
fn test_cliff_only_withdrawals() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1000_i128;
    let stream_id = ctx.create_cliff_only_stream(deposit, 100, 500, 1000);

    // Try withdraw before cliff
    ctx.env.ledger().set_timestamp(499);
    let withdrawn = ctx.client.withdraw(&stream_id);
    assert_eq!(withdrawn, 0);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    // Withdraw after cliff
    ctx.env.ledger().set_timestamp(500);
    let withdrawn = ctx.client.withdraw(&stream_id);
    assert_eq!(withdrawn, deposit);
    assert_eq!(ctx.token.balance(&ctx.recipient), deposit);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
}

/// 4. Cancellation Verification (Before Cliff):
/// Sender cancelled before cliff -> gets 100% refund.
#[test]
fn test_cliff_only_cancel_before_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1000_i128;
    let sender_balance_before = ctx.token.balance(&ctx.sender);
    let stream_id = ctx.create_cliff_only_stream(deposit, 100, 500, 1000);

    assert_eq!(
        ctx.token.balance(&ctx.sender),
        sender_balance_before - deposit
    );

    // Cancel before cliff (t = 400)
    ctx.env.ledger().set_timestamp(400);
    ctx.client.cancel_stream(&stream_id);

    // Sender gets 100% refund, recipient gets 0
    assert_eq!(ctx.token.balance(&ctx.sender), sender_balance_before);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

/// 5. Cancellation Verification (After Cliff):
/// Sender cancelled after cliff -> recipient keeps 100%, sender gets 0 refund.
#[test]
fn test_cliff_only_cancel_after_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1000_i128;
    let sender_balance_before = ctx.token.balance(&ctx.sender);
    let stream_id = ctx.create_cliff_only_stream(deposit, 100, 500, 1000);

    // Cancel after cliff (t = 600)
    ctx.env.ledger().set_timestamp(600);
    ctx.client.cancel_stream(&stream_id);

    // Sender gets 0 refund, recipient is entitled to 100%
    assert_eq!(
        ctx.token.balance(&ctx.sender),
        sender_balance_before - deposit
    );

    // Recipient pulls their funds
    let withdrawn = ctx.client.withdraw(&stream_id);
    assert_eq!(withdrawn, deposit);
    assert_eq!(ctx.token.balance(&ctx.recipient), deposit);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}
