use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{token::Client as TokenClient, Address, Env, Vec};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
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

        // Fund the sender using the admin's minting power
        token.mint(&sender, &1_000_000_i128);

        Self {
            env,
            client,
            sender,
            recipient,
            token,
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
fn test_create_streams_gas() {
    let ctx = TestContext::setup();
    let sizes = [1, 5, 10];

    for &size in &sizes {
        let mut streams = soroban_sdk::Vec::new(&ctx.env);
        for _ in 0..size {
            streams.push_back(CreateStreamParams {
                kind: StreamKind::Linear,
                withdraw_dust_threshold: None,
                recipient: Address::generate(&ctx.env),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: 1000u64,
                memo: None,
                metadata: None,
            });
        }

        let cost = measure_gas(&ctx, |ctx| {
            ctx.client.create_streams(&ctx.sender, &streams);
        });

        println!("GAS_MEASUREMENT: create_streams: {}: {}", size, cost);
    }
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

        let mut streams = Vec::new();
        for _ in 0..size {
            streams.push(ctx.create_default_stream());
        }

        ctx.env.ledger().set_timestamp(500); // Accrue tokens for all

        let cost = measure_gas(&ctx, |ctx| {
            let streams_val = streams.clone().into_val(&ctx.env);
            ctx.client.batch_withdraw(&streams_val);
        });

        println!("GAS_MEASUREMENT: batch_withdraw: {}: {}", size, cost);
    }
}

#[test]
fn test_batch_withdraw_mixed_state_gas() {
        let ctx = TestContext::setup();

        let active_id = ctx.create_default_stream();
        let completed_id = ctx.create_default_stream();
        let cancelled_id = ctx.create_default_stream();

        ctx.env.ledger().set_timestamp(500);
        ctx.client.cancel_stream(&cancelled_id);

        ctx.env.ledger().set_timestamp(1000);
        ctx.client.withdraw(&completed_id);

        let mut stream_ids = Vec::new(&ctx.env);
        stream_ids.push_back(active_id);
        stream_ids.push_back(cancelled_id);
        stream_ids.push_back(completed_id);

        let cost = measure_gas(&ctx, |ctx| {
            ctx.env.ledger().set_timestamp(1000);
            ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
        });

        println!("GAS_MEASUREMENT: batch_withdraw: mixed-state: {}", cost);
    }
