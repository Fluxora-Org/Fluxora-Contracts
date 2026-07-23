// See docs/gas.md for the baseline update process and review bar.
use fluxora_stream::{FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// Grace period (mirrors KEEPER_GRACE_PERIOD_SECONDS in lib.rs).
const KEEPER_GRACE: u64 = 604_800;

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    keeper: Address,
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
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let keeper = Address::generate(&env);

        client.init(&token_id, &admin);

        // Fund the sender using the admin's minting power
        sac.mint(&sender, &1_000_000_i128);
        // Provide default allowance so create_stream can pull the deposit.
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
            keeper,
        }
    }

    fn create_default_stream(&self) -> u64 {
        let amount = 1000_i128;
        let rate = 1_i128;
        let start_time = 0u64;
        let cliff_time = 0u64;
        let end_time = 1000u64;

        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &amount,
            &rate,
            &start_time,
            &cliff_time,
            &end_time,
            &0,
            &None,
            &StreamKind::Linear,
        )
    }
}

fn measure_gas<F>(ctx: &TestContext, f: F) -> u64
where
    F: FnOnce(&TestContext),
{
    ctx.env.budget().reset_unlimited();
    f(ctx);
    ctx.env.budget().cpu_instruction_cost()
}

#[test]
fn test_create_stream_gas() {
    let ctx = TestContext::setup();

    let cost = measure_gas(&ctx, |ctx| {
        ctx.create_default_stream();
    });

    println!("GAS_MEASUREMENT: create_stream: single: {}", cost);
}

#[test]
fn test_withdraw_gas() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500); // Accrue 500 tokens

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.withdraw(&stream_id);
    });

    println!("GAS_MEASUREMENT: withdraw: single: {}", cost);
}

#[test]
fn test_batch_withdraw_gas() {
    let sizes = [1, 10, 50, 100];

    for &size in &sizes {
        let ctx = TestContext::setup();

        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(ctx.create_default_stream());
        }

        ctx.env.ledger().set_timestamp(500); // Accrue tokens for all

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.batch_withdraw(&ctx.recipient, &streams);
        });

        println!("GAS_MEASUREMENT: batch_withdraw: {}: {}", size, cost);
    }
}

// ---------------------------------------------------------------------------
// keeper_cancel gas measurements
//
// Two variants capture the two meaningful cost paths:
//
//   partial_accrual — the common keeper incentive case: the stream expired with
//     an unstreamed balance, so the contract makes three token transfers
//     (recipient, sender, keeper).  This is the hot path for economically
//     rational keeper bots and the cost documented in docs/gas.md's
//     break-even formula.
//
//   fully_accrued   — the degenerate case: deposit == rate × duration, so
//     sender_refund_gross == 0, keeper_fee == 0 and no keeper transfer is
//     issued.  Only one token transfer (to the recipient) occurs.  Cost is
//     slightly lower than the partial_accrual variant.
//
// Both variants print a GAS_MEASUREMENT line that validate_gas.py picks up
// and compares against the JSON baseline in docs/gas.md.
// ---------------------------------------------------------------------------

/// keeper_cancel on a stream that still has an unstreamed balance (3 transfers).
///
/// Setup:
///   deposit = 10 000, rate = 5 token/s, start = 0, end = 1 000
///   → accrued at end_time = min(5 × 1 000, 10 000) = 5 000
///   → sender_refund_gross = 5 000
///   → keeper_fee = 5 000 × 50 / 10 000 = 25
///   → three token transfers: recipient 5 000, sender 4 975, keeper 25
#[test]
fn test_keeper_cancel_gas_partial_accrual() {
    let ctx = TestContext::setup();

    // Create the stream at t=0.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &5_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
    );

    // Advance past end_time + grace period so the stream is eligible.
    ctx.env.ledger().set_timestamp(1_000 + KEEPER_GRACE + 1);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.keeper_cancel(&stream_id, &ctx.keeper);
    });

    // Print in the canonical GAS_MEASUREMENT format so validate_gas.py can
    // parse this line and compare it against the baseline in docs/gas.md.
    println!("GAS_MEASUREMENT: keeper_cancel: partial_accrual: {}", cost);
}

/// keeper_cancel on a stream that is fully accrued (1 transfer, keeper fee == 0).
///
/// Setup:
///   deposit = 1 000, rate = 1 token/s, start = 0, end = 1 000
///   → accrued at end_time = 1 000 == deposit
///   → sender_refund_gross = 0, keeper_fee = 0
///   → one token transfer: recipient 1 000; no sender or keeper transfers
#[test]
fn test_keeper_cancel_gas_fully_accrued() {
    let ctx = TestContext::setup();

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &0_i128,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1_000 + KEEPER_GRACE + 1);

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.keeper_cancel(&stream_id, &ctx.keeper);
    });

    println!("GAS_MEASUREMENT: keeper_cancel: fully_accrued: {}", cost);
}
