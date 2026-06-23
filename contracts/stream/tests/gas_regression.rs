use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{token::Client as TokenClient, Address, Env};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
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

        client.init(&token_id, &admin);

        // Fund the sender using the admin's minting power
        token.mint(&sender, &1_000_000_i128);

        Self {
            env,
            client,
            sender,
            token,
        }
    }

    fn create_default_stream(&self) -> u64 {
        let recipient = Address::generate(&self.env);
        let amount = 1000_i128;
        let rate = 1_i128;
        let start_time = 0u64;
        let cliff_time = 0u64;
        let end_time = 1000u64;

        let stream_id = self.client.create_stream(
            &self.sender,
            &recipient,
            &amount,
            &rate,
            &start_time,
            &cliff_time,
            &end_time,
            &0,
            &None,
        );
        stream_id
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
