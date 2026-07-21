extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

#[allow(dead_code)]
struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
    sac: StellarAssetClient<'a>,
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
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        sac.mint(&sender, &100_000_i128);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
            token,
            sac,
            contract_id,
        }
    }

    /// Create a default linear stream: deposit=1000, rate=1/s, 0..1000s, no cliff.
    fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
        )
    }
}

// ---------------------------------------------------------------------------
// 1. Active stream top-up
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_active_stream_deposit_reflected() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    ctx.client.top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.end_time, 1_000);
}

// ---------------------------------------------------------------------------
// 2. Paused stream top-up — matches documented behaviour
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_paused_stream_matches_spec() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance ledger sequence past cooldown, then pause the stream
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
    ctx.env.ledger().set_timestamp(400);
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Top up while paused
    ctx.client.top_up_stream(&stream_id, &ctx.sender, &300_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_300);
    assert_eq!(state.status, StreamStatus::Paused); // status unchanged
    assert_eq!(state.end_time, 1_000); // schedule unchanged
}

// ---------------------------------------------------------------------------
// 3. Completed stream top-up — rejected with InvalidState
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_completed_stream_rejected() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance ledger sequence past withdrawal frequency check, then complete
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
    ctx.env.ledger().set_timestamp(1000);
    ctx.client.withdraw(&stream_id);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    let result = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &100_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    // Verify no side effects
    let state_after = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state_after.deposit_amount, state.deposit_amount);
}

// ---------------------------------------------------------------------------
// 4. Cancelled stream top-up — rejected with InvalidState
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_cancelled_stream_rejected() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel the stream
    ctx.env.ledger().set_timestamp(100);
    ctx.client.cancel_stream(&stream_id);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    let result = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &100_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    // Verify no side effects
    let state_after = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state_after.deposit_amount, state.deposit_amount);
}

// ---------------------------------------------------------------------------
// 5. Near end_time (T-1) top-up — accrual/withdrawable update correctly
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_near_end_updates_accrual() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // deposit=1000, rate=1/s, 0..1000

    // At T-1: accrued = min(1 * 999, 1000) = 999
    ctx.env.ledger().set_timestamp(999);
    let accrued_before = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 999);

    // Top up by 500 → deposit becomes 1500
    ctx.client.top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);

    // Accrual at same timestamp: min(1 * 999, 1500) = 999 (unchanged at same time)
    let accrued_after = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued_after, 999);

    // Move to end_time: accrued = min(1 * 1000, 1500) = 1000
    ctx.env.ledger().set_timestamp(1000);
    let accrued_at_end = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued_at_end, 1_000);

    // withdrawable = accrued - withdrawn = 1000 - 0 = 1000
    let withdrawable = ctx.client.get_withdrawable(&stream_id);
    assert_eq!(withdrawable, 1_000);
}
