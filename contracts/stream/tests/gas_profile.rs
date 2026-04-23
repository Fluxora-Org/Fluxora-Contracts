//! Gas profiling harness for Fluxora stream contract batch entrypoints.
//!
//! Measures Soroban CPU-instruction and memory-byte costs for `create_streams`
//! and `batch_withdraw` across realistic batch sizes (1, 5, 10, 20, 50).
//!
//! Each test:
//! 1. Resets the budget to unlimited before the measured call.
//! 2. Captures CPU instructions and memory bytes after the call.
//! 3. Asserts against a documented safe-limit guardrail.
//! 4. Prints the raw numbers so CI logs capture them for trend analysis.
//!
//! Run with:
//!   cargo test -p fluxora_stream gas_profile -- --nocapture
//!
//! Issue: #407

extern crate std;

use fluxora_stream::{ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct GasCtx<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> GasCtx<'a> {
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

        // Mint enough for large batches (50 streams × 1000 tokens each)
        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &1_000_000_i128);

        let token = TokenClient::new(&env, &token_id);

        Self {
            env,
            contract_id,
            token_id,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    /// Build N identical CreateStreamParams entries.
    fn make_params(&self, n: usize) -> soroban_sdk::Vec<CreateStreamParams> {
        let mut v = soroban_sdk::Vec::new(&self.env);
        for _ in 0..n {
            v.push_back(CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: 1_000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1_000,
                memo: None,
            });
        }
        v
    }

    /// Build N unique-recipient CreateStreamParams entries (avoids index contention).
    fn make_params_unique_recipients(&self, n: usize) -> soroban_sdk::Vec<CreateStreamParams> {
        let mut v = soroban_sdk::Vec::new(&self.env);
        for _ in 0..n {
            v.push_back(CreateStreamParams {
                recipient: Address::generate(&self.env),
                deposit_amount: 1_000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1_000,
                memo: None,
            });
        }
        v
    }
}

// ---------------------------------------------------------------------------
// create_streams — batch size sweep
// ---------------------------------------------------------------------------

/// Measure `create_streams` with a single stream (baseline).
#[test]
fn gas_create_streams_batch_1() {
    let ctx = GasCtx::setup();
    ctx.env.ledger().set_timestamp(0);
    let params = ctx.make_params_unique_recipients(1);

    ctx.env.budget().reset_unlimited();
    let ids = ctx.client().create_streams(&ctx.sender, &params);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(ids.len(), 1);
    soroban_sdk::log!(&ctx.env, "gas_create_streams_batch_1 cpu={} mem={}", cpu, mem);

    // Guardrail: single-stream create must stay well under 2 M CPU / 1 MB.
    assert!(cpu <= 2_000_000, "create_streams(1) cpu={} > 2_000_000", cpu);
    assert!(mem <= 1_000_000, "create_streams(1) mem={} > 1_000_000", mem);
}

/// Measure `create_streams` with 5 streams.
#[test]
fn gas_create_streams_batch_5() {
    let ctx = GasCtx::setup();
    ctx.env.ledger().set_timestamp(0);
    let params = ctx.make_params_unique_recipients(5);

    ctx.env.budget().reset_unlimited();
    let ids = ctx.client().create_streams(&ctx.sender, &params);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(ids.len(), 5);
    soroban_sdk::log!(&ctx.env, "gas_create_streams_batch_5 cpu={} mem={}", cpu, mem);

    assert!(cpu <= 4_000_000, "create_streams(5) cpu={} > 4_000_000", cpu);
    assert!(mem <= 2_000_000, "create_streams(5) mem={} > 2_000_000", mem);
}

/// Measure `create_streams` with 10 streams.
#[test]
fn gas_create_streams_batch_10() {
    let ctx = GasCtx::setup();
    ctx.env.ledger().set_timestamp(0);
    let params = ctx.make_params_unique_recipients(10);

    ctx.env.budget().reset_unlimited();
    let ids = ctx.client().create_streams(&ctx.sender, &params);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(ids.len(), 10);
    soroban_sdk::log!(&ctx.env, "gas_create_streams_batch_10 cpu={} mem={}", cpu, mem);

    // Documented safe limit from docs/gas.md
    assert!(cpu <= 6_000_000, "create_streams(10) cpu={} > 6_000_000", cpu);
    assert!(mem <= 3_000_000, "create_streams(10) mem={} > 3_000_000", mem);
}

/// Measure `create_streams` with 20 streams.
#[test]
fn gas_create_streams_batch_20() {
    let ctx = GasCtx::setup();
    ctx.env.ledger().set_timestamp(0);
    let params = ctx.make_params_unique_recipients(20);

    ctx.env.budget().reset_unlimited();
    let ids = ctx.client().create_streams(&ctx.sender, &params);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(ids.len(), 20);
    soroban_sdk::log!(&ctx.env, "gas_create_streams_batch_20 cpu={} mem={}", cpu, mem);

    assert!(cpu <= 12_000_000, "create_streams(20) cpu={} > 12_000_000", cpu);
    assert!(mem <= 5_000_000, "create_streams(20) mem={} > 5_000_000", mem);
}

/// Measure `create_streams` with 50 streams.
///
/// This is the practical upper bound for a single treasury batch operation.
/// Operators should not exceed this without verifying network budget limits.
#[test]
fn gas_create_streams_batch_50() {
    let ctx = GasCtx::setup();
    ctx.env.ledger().set_timestamp(0);
    let params = ctx.make_params_unique_recipients(50);

    ctx.env.budget().reset_unlimited();
    let ids = ctx.client().create_streams(&ctx.sender, &params);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(ids.len(), 50);
    soroban_sdk::log!(&ctx.env, "gas_create_streams_batch_50 cpu={} mem={}", cpu, mem);

    // Conservative upper bound for 50-stream batch.
    assert!(cpu <= 30_000_000, "create_streams(50) cpu={} > 30_000_000", cpu);
    assert!(mem <= 12_000_000, "create_streams(50) mem={} > 12_000_000", mem);
}

/// Measure `create_streams` with same recipient (worst-case index contention).
///
/// When all streams share the same recipient, the RecipientStreams Vec is read
/// and written N times. This is the worst-case for recipient-index I/O.
#[test]
fn gas_create_streams_batch_10_same_recipient() {
    let ctx = GasCtx::setup();
    ctx.env.ledger().set_timestamp(0);
    let params = ctx.make_params(10); // all same recipient

    ctx.env.budget().reset_unlimited();
    let ids = ctx.client().create_streams(&ctx.sender, &params);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(ids.len(), 10);
    soroban_sdk::log!(
        &ctx.env,
        "gas_create_streams_batch_10_same_recipient cpu={} mem={}",
        cpu,
        mem
    );

    // Same-recipient is more expensive due to repeated index reads/writes.
    assert!(
        cpu <= 8_000_000,
        "create_streams(10,same_recipient) cpu={} > 8_000_000",
        cpu
    );
    assert!(
        mem <= 4_000_000,
        "create_streams(10,same_recipient) mem={} > 4_000_000",
        mem
    );
}

// ---------------------------------------------------------------------------
// batch_withdraw — batch size sweep
// ---------------------------------------------------------------------------

/// Helper: create N streams for the same recipient and return their IDs.
fn setup_streams_for_withdraw(ctx: &GasCtx, n: usize) -> soroban_sdk::Vec<u64> {
    ctx.env.ledger().set_timestamp(0);
    let params = ctx.make_params(n);
    ctx.client().create_streams(&ctx.sender, &params)
}

/// Measure `batch_withdraw` with 1 stream (baseline).
#[test]
fn gas_batch_withdraw_1_stream() {
    let ctx = GasCtx::setup();
    let ids = setup_streams_for_withdraw(&ctx, 1);

    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().amount, 500);
    soroban_sdk::log!(&ctx.env, "gas_batch_withdraw_1 cpu={} mem={}", cpu, mem);

    assert!(cpu <= 1_500_000, "batch_withdraw(1) cpu={} > 1_500_000", cpu);
    assert!(mem <= 600_000, "batch_withdraw(1) mem={} > 600_000", mem);
}

/// Measure `batch_withdraw` with 5 streams.
#[test]
fn gas_batch_withdraw_5_streams() {
    let ctx = GasCtx::setup();
    let ids = setup_streams_for_withdraw(&ctx, 5);

    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(results.len(), 5);
    soroban_sdk::log!(&ctx.env, "gas_batch_withdraw_5 cpu={} mem={}", cpu, mem);

    assert!(cpu <= 4_000_000, "batch_withdraw(5) cpu={} > 4_000_000", cpu);
    assert!(mem <= 2_000_000, "batch_withdraw(5) mem={} > 2_000_000", mem);
}

/// Measure `batch_withdraw` with 10 streams.
#[test]
fn gas_batch_withdraw_10_streams() {
    let ctx = GasCtx::setup();
    let ids = setup_streams_for_withdraw(&ctx, 10);

    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(results.len(), 10);
    soroban_sdk::log!(&ctx.env, "gas_batch_withdraw_10 cpu={} mem={}", cpu, mem);

    assert!(cpu <= 6_000_000, "batch_withdraw(10) cpu={} > 6_000_000", cpu);
    assert!(mem <= 3_000_000, "batch_withdraw(10) mem={} > 3_000_000", mem);
}

/// Measure `batch_withdraw` with 20 streams.
///
/// This is the documented safe limit from docs/gas.md.
#[test]
fn gas_batch_withdraw_20_streams() {
    let ctx = GasCtx::setup();
    let ids = setup_streams_for_withdraw(&ctx, 20);

    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(results.len(), 20);
    for i in 0..20 {
        assert_eq!(results.get(i).unwrap().amount, 500);
    }
    soroban_sdk::log!(&ctx.env, "gas_batch_withdraw_20 cpu={} mem={}", cpu, mem);

    // Documented safe limit from docs/gas.md
    assert!(
        cpu <= 10_000_000,
        "batch_withdraw(20) cpu={} > 10_000_000",
        cpu
    );
    assert!(
        mem <= 4_000_000,
        "batch_withdraw(20) mem={} > 4_000_000",
        mem
    );
}

/// Measure `batch_withdraw` with 50 streams.
///
/// Practical upper bound. Operators should verify network budget before exceeding 20.
#[test]
fn gas_batch_withdraw_50_streams() {
    let ctx = GasCtx::setup();
    let ids = setup_streams_for_withdraw(&ctx, 50);

    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert_eq!(results.len(), 50);
    soroban_sdk::log!(&ctx.env, "gas_batch_withdraw_50 cpu={} mem={}", cpu, mem);

    assert!(
        cpu <= 25_000_000,
        "batch_withdraw(50) cpu={} > 25_000_000",
        cpu
    );
    assert!(
        mem <= 10_000_000,
        "batch_withdraw(50) mem={} > 10_000_000",
        mem
    );
}

// ---------------------------------------------------------------------------
// Scaling linearity check
// ---------------------------------------------------------------------------

/// Verify that batch_withdraw cost scales sub-quadratically with batch size.
///
/// Measures cost at N=5 and N=20. The ratio must be < 5× (linear would be 4×;
/// we allow 25% overhead for fixed per-call costs).
#[test]
fn gas_batch_withdraw_scaling_is_subquadratic() {
    let ctx = GasCtx::setup();

    // Measure N=5
    let ids5 = setup_streams_for_withdraw(&ctx, 5);
    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    ctx.client().batch_withdraw(&ctx.recipient, &ids5);
    let cpu5 = ctx.env.budget().cpu_instruction_cost();

    // Measure N=20 (need fresh context to avoid state pollution)
    let ctx2 = GasCtx::setup();
    let ids20 = setup_streams_for_withdraw(&ctx2, 20);
    ctx2.env.ledger().set_timestamp(500);
    ctx2.env.budget().reset_unlimited();
    ctx2.client().batch_withdraw(&ctx2.recipient, &ids20);
    let cpu20 = ctx2.env.budget().cpu_instruction_cost();

    soroban_sdk::log!(
        &ctx.env,
        "scaling check: cpu5={} cpu20={} ratio={}",
        cpu5,
        cpu20,
        cpu20 / cpu5.max(1)
    );

    // 20/5 = 4× streams; allow up to 5× CPU growth (linear + 25% overhead)
    assert!(
        cpu20 <= cpu5 * 5,
        "batch_withdraw scaling is super-linear: cpu5={} cpu20={} ratio={}",
        cpu5,
        cpu20,
        cpu20 / cpu5.max(1)
    );
}

/// Verify that create_streams cost scales sub-quadratically with batch size.
#[test]
fn gas_create_streams_scaling_is_subquadratic() {
    let ctx = GasCtx::setup();
    ctx.env.ledger().set_timestamp(0);

    // Measure N=5
    let params5 = ctx.make_params_unique_recipients(5);
    ctx.env.budget().reset_unlimited();
    ctx.client().create_streams(&ctx.sender, &params5);
    let cpu5 = ctx.env.budget().cpu_instruction_cost();

    // Measure N=20
    let ctx2 = GasCtx::setup();
    ctx2.env.ledger().set_timestamp(0);
    let params20 = ctx2.make_params_unique_recipients(20);
    ctx2.env.budget().reset_unlimited();
    ctx2.client().create_streams(&ctx2.sender, &params20);
    let cpu20 = ctx2.env.budget().cpu_instruction_cost();

    soroban_sdk::log!(
        &ctx.env,
        "create_streams scaling: cpu5={} cpu20={} ratio={}",
        cpu5,
        cpu20,
        cpu20 / cpu5.max(1)
    );

    assert!(
        cpu20 <= cpu5 * 5,
        "create_streams scaling is super-linear: cpu5={} cpu20={} ratio={}",
        cpu5,
        cpu20,
        cpu20 / cpu5.max(1)
    );
}

// ---------------------------------------------------------------------------
// Single withdraw baseline
// ---------------------------------------------------------------------------

/// Measure single `withdraw` call (baseline for comparison with batch).
#[test]
fn gas_single_withdraw_baseline() {
    let ctx = GasCtx::setup();
    ctx.env.ledger().set_timestamp(0);
    let params = ctx.make_params(1);
    let ids = ctx.client().create_streams(&ctx.sender, &params);
    let stream_id = ids.get(0).unwrap();

    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    ctx.client().withdraw(&stream_id);
    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    soroban_sdk::log!(
        &ctx.env,
        "gas_single_withdraw_baseline cpu={} mem={}",
        cpu,
        mem
    );

    // Documented safe limit from docs/gas.md
    assert!(cpu <= 1_000_000, "single withdraw cpu={} > 1_000_000", cpu);
    assert!(mem <= 500_000, "single withdraw mem={} > 500_000", mem);
}
