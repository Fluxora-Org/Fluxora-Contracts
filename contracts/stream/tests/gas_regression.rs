use fluxora_stream::{
    CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind,
};
use soroban_sdk::{token::Client as TokenClient, Address, Bytes, Env, Map};

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

// ---------------------------------------------------------------------------
// Gas regression: metadata operations
// ---------------------------------------------------------------------------

/// Helper: build a metadata map with `count` entries "k0"→"v0", … "kN"→"vN".
fn metadata_n(env: &Env, count: u32) -> Map<Bytes, Bytes> {
    let mut m: Map<Bytes, Bytes> = Map::new(env);
    for i in 0..count {
        let k = Bytes::from_slice(env, format!("k{}", i).as_bytes());
        let v = Bytes::from_slice(env, format!("v{}", i).as_bytes());
        m.set(k, v);
    }
    m
}

/// Measure gas for `create_streams_partial` with a single entry carrying metadata.
#[test]
fn test_create_stream_with_metadata_gas() {
    let ctx = TestContext::setup();

    // Full metadata: MAX_METADATA_KEYS × (32-byte key + 128-byte value) at max aggregate.
    let meta = metadata_n(&ctx.env, fluxora_stream::MAX_METADATA_KEYS);
    let recipient = Address::generate(&ctx.env);
    let params = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(meta),
        },
    ];

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client
            .create_streams_partial(&ctx.sender, &params);
    });

    println!("GAS_MEASUREMENT: create_stream_with_metadata: full: {}", cost);

    // Also measure without metadata for comparison
    let recipient2 = Address::generate(&ctx.env);
    let params_no_meta = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient: recipient2,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
        },
    ];

    let cost_no_meta = measure_gas(&ctx, |ctx| {
        ctx.client
            .create_streams_partial(&ctx.sender, &params_no_meta);
    });

    println!(
        "GAS_MEASUREMENT: create_stream_without_metadata: baseline: {}",
        cost_no_meta
    );
}

/// Measure gas for `get_stream_metadata` on a stream with full metadata.
#[test]
fn test_get_stream_metadata_gas() {
    let ctx = TestContext::setup();

    // Create a stream with metadata so we can read it back
    let meta = metadata_n(&ctx.env, fluxora_stream::MAX_METADATA_KEYS);
    let recipient = Address::generate(&ctx.env);
    let params = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: None,
            memo: None,
            kind: StreamKind::Linear,
            metadata: Some(meta),
        },
    ];
    let results = ctx
        .client
        .create_streams_partial(&ctx.sender, &params);
    let stream_id = results.get(0).unwrap().stream_id.unwrap();

    let cost = measure_gas(&ctx, |ctx| {
        ctx.client.get_stream_metadata(&stream_id);
    });

    println!(
        "GAS_MEASUREMENT: get_stream_metadata: full: {}",
        cost
    );
}
