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
