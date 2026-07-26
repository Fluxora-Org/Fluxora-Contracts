#![cfg(test)]
extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, KeeperCancelled, StreamKind, StreamStatus,
    StreamCancelled,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Symbol, TryFromVal, vec
};

// Grace period in seconds (mirrors KEEPER_GRACE_PERIOD_SECONDS in lib.rs).
const GRACE: u64 = 604_800;

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    keeper: Address,
    admin: Address,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
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
        let keeper = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &1_000_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &200_000);

        Ctx {
            env,
            contract_id,
            sender,
            recipient,
            keeper,
            admin,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn create_cliff_slope_stream(&self, deposit: i128, rate: i128, start: u64, cliff: u64, end: u64) -> u64 {
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &deposit,
            &rate,
            &start,
            &cliff,
            &end,
            &0,
            &None,
            &StreamKind::CliffSlope,
        )
    }
}

// Helper: create a CliffOnly stream
fn create_cliff_stream(ctx: &Ctx<'_>, deposit: i128, start: u64, cliff: u64, end: u64) -> u64 {
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &1_i128, // Rate doesn't matter for CliffOnly
        &start,
        &cliff,
        &end,
        &0_i128,
        &None,
        &StreamKind::CliffOnly,
    )
}

#[test]
fn test_cliffonly_keeper_cancel_after_cliff() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    // Stream: deposit=1000, start=0, cliff=500, end=1000
    let deposit = 1000;
    let stream_id = create_cliff_stream(&ctx, deposit, 0, 500, 1000);

    // Advance past end_time + grace period
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);

    let client = ctx.client();
    
    // Initial balances
    let contract_bal_before = ctx.token.balance(&ctx.contract_id);
    assert_eq!(contract_bal_before, deposit);
    let liabilities_before = client.get_total_liabilities();
    assert_eq!(liabilities_before, deposit);

    let sender_bal_before = ctx.token.balance(&ctx.sender);
    let recipient_bal_before = ctx.token.balance(&ctx.recipient);
    let keeper_bal_before = ctx.token.balance(&ctx.keeper);

    let result = client.keeper_cancel(&stream_id, &ctx.keeper);

    // Assert return values
    assert_eq!(result.recipient_amount, deposit, "Recipient gets full deposit");
    assert_eq!(result.sender_refund_gross, 0, "Sender gets nothing");
    assert_eq!(result.keeper_fee, 0, "Keeper fee is 0 because refund gross is 0");
    assert_eq!(result.sender_refund_net, 0, "Net refund is 0");

    // Balances
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    assert_eq!(client.get_total_liabilities(), 0);
    assert_eq!(ctx.token.balance(&ctx.sender), sender_bal_before);
    assert_eq!(ctx.token.balance(&ctx.recipient), recipient_bal_before + deposit);
    assert_eq!(ctx.token.balance(&ctx.keeper), keeper_bal_before); // Keeper gets 0

    // Stream state
    let state = client.get_stream(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

#[test]
fn test_cliffonly_bulk_cancel_before_cliff() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1000;
    let stream_id = create_cliff_stream(&ctx, deposit, 0, 1500, 2000);

    // Advance past start but before cliff
    ctx.env.ledger().set_timestamp(500); // before cliff=1500

    let client = ctx.client();
    let streams = vec![&ctx.env, stream_id];
    
    // bulk_cancel_streams is admin only
    let results = client.bulk_cancel_streams(&streams);
    assert_eq!(results.len(), 1);
    let result = results.get(0).unwrap();

    assert_eq!(result.recipient_amount, 0, "Recipient gets 0 before cliff");
    assert_eq!(result.sender_refund_gross, deposit, "Sender gets full refund gross");
    assert_eq!(result.keeper_fee, 0, "Admin cancel doesn't take keeper fee");
    assert_eq!(result.sender_refund_net, deposit, "Sender gets full refund net");

    // Balances
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    assert_eq!(client.get_total_liabilities(), 0);
}

/// 6. CliffSlope Timeline Testing:
/// - 0 before cliff
/// - linear from cliff to end
#[test]
fn test_cliff_slope_accrual_timeline() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1000_i128;
    let rate = 2_i128; // 2 tokens per sec
    let start = 100u64;
    let cliff = 500u64;
    let end = 1000u64; // duration 500 secs, 2 * 500 = 1000 deposit

    let stream_id = ctx.create_cliff_slope_stream(deposit, rate, start, cliff, end);

    // 1 second before cliff (t = 499)
    ctx.env.ledger().set_timestamp(499);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), 0);

    // Exactly at cliff (t = 500)
    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), 0);

    // After cliff (t = 600 -> 100 secs passed -> 200 accrued)
    ctx.env.ledger().set_timestamp(600);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), 200);

    // At end (t = 1000 -> 500 secs passed -> 1000 accrued)
    ctx.env.ledger().set_timestamp(1000);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), deposit);

    // Long after end (t = 1500)
    ctx.env.ledger().set_timestamp(1500);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), deposit);
}

/// 7. CliffSlope Mutation Restriction Verification
#[test]
fn test_cliff_slope_rejects_mutations() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_slope_stream(1000, 2, 100, 500, 1000);

    let state = ctx.client.get_stream_state(&stream_id);
    assert!(matches!(state.kind, StreamKind::CliffSlope));

    let res = ctx.client.try_update_rate_per_second(&stream_id, &5_i128);
    assert_eq!(res, Err(Ok(ContractError::UnsupportedStreamKind)));

    let res = ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128);
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

/// 6. Exact Boundary Testing - 1 second before cliff (cliff_time - 1):
/// Asserts that withdrawable is 0 at cliff_time - 1 and withdrawal returns 0.
#[test]
fn test_cliff_only_withdrawable_one_second_before_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 5000_i128;
    let start = 100u64;
    let cliff = 500u64;
    let end = 1000u64;

    let stream_id = ctx.create_cliff_only_stream(deposit, start, cliff, end);

    // Timestamp set to cliff_time - 1 (499)
    let t_before = cliff - 1;
    ctx.env.ledger().set_timestamp(t_before);

    // Accrued and withdrawable must be exactly 0
    assert_eq!(ctx.client.get_withdrawable(&stream_id), 0);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), 0);
    assert_eq!(ctx.client.get_claimable_at(&stream_id, &t_before), 0);

    // Attempting to withdraw must return 0 and not transfer any tokens
    let withdrawn = ctx.client.withdraw(&stream_id);
    assert_eq!(withdrawn, 0);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
}

/// 7. Exact Boundary Testing - Exactly at cliff (cliff_time):
/// Asserts that withdrawable is the full deposit_amount at exactly cliff_time
/// confirming inclusive semantics (ledger.timestamp() == cliff_time).
#[test]
fn test_cliff_only_withdrawable_exactly_at_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 5000_i128;
    let start = 100u64;
    let cliff = 500u64;
    let end = 1000u64;

    let stream_id = ctx.create_cliff_only_stream(deposit, start, cliff, end);

    // Timestamp set exactly to cliff_time (500)
    ctx.env.ledger().set_timestamp(cliff);

    // Accrued and withdrawable must equal full deposit_amount (inclusive cliff semantics)
    assert_eq!(ctx.client.get_withdrawable(&stream_id), deposit);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), deposit);
    assert_eq!(ctx.client.get_claimable_at(&stream_id, &cliff), deposit);

    // Recipient successfully withdraws 100% of deposit at cliff_time
    let withdrawn = ctx.client.withdraw(&stream_id);
    assert_eq!(withdrawn, deposit);
    assert_eq!(ctx.token.balance(&ctx.recipient), deposit);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
}

/// 8. Exact Boundary Testing - After cliff (cliff_time + N):
/// Asserts that withdrawable remains capped at deposit_amount for timestamps after cliff_time
/// (no further "accrual" past the cliff for a CliffOnly stream).
#[test]
fn test_cliff_only_withdrawable_after_cliff_capped() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 5000_i128;
    let start = 100u64;
    let cliff = 500u64;
    let end = 1000u64;

    let stream_id = ctx.create_cliff_only_stream(deposit, start, cliff, end);

    // Test multiple timestamps after cliff_time (cliff + 1, cliff + 100, end_time, far past end_time)
    for &t_after in &[cliff + 1, cliff + 100, end, end + 5000] {
        ctx.env.ledger().set_timestamp(t_after);
        assert_eq!(
            ctx.client.get_withdrawable(&stream_id),
            deposit,
            "Withdrawable past cliff at t={} should be capped at deposit_amount",
            t_after
        );
        assert_eq!(
            ctx.client.calculate_accrued(&stream_id),
            deposit,
            "Accrued past cliff at t={} should be capped at deposit_amount",
            t_after
        );
        assert_eq!(
            ctx.client.get_claimable_at(&stream_id, &t_after),
            deposit,
            "Claimable at t={} should be capped at deposit_amount",
            t_after
        );
    }

    // Recipient withdraws full deposit amount
    let withdrawn = ctx.client.withdraw(&stream_id);
    assert_eq!(withdrawn, deposit);
    assert_eq!(ctx.token.balance(&ctx.recipient), deposit);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
}

