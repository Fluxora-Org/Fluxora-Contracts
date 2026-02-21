#[cfg(test)]
extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

use crate::{FluxoraStream, FluxoraStreamClient, StreamStatus};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct TestContext {
    env: Env,
    contract_id: Address,
    token_id: Address,
    #[allow(dead_code)]
    admin: Address,
    sender: Address,
    recipient: Address,
}

impl TestContext {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        TestContext {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
        )
    }

    fn create_cliff_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &500u64,
            &1000u64,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests — init
// ---------------------------------------------------------------------------

#[test]
fn test_init_success() {
    let ctx = TestContext::setup();
    let config = ctx.client().get_config();
    assert_eq!(config.token, ctx.token_id);
    assert_eq!(config.admin, ctx.admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn test_init_already_initialized() {
    let ctx = TestContext::setup();
    ctx.client().init(&ctx.token_id, &ctx.admin);
}

// ---------------------------------------------------------------------------
// Tests — create_stream
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_initial_state() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    assert_eq!(stream_id, 0, "first stream id should be 0");

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token().balance(&ctx.contract_id), 1000);
    assert_eq!(ctx.token().balance(&ctx.sender), 9000);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_zero_deposit_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &0_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_invalid_times_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &500u64,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_zero_rate_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &0_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_sender_equals_recipient_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.sender,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_cliff_before_start_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &50u64,
        &1100u64,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_cliff_after_end_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1500u64,
        &1000u64,
    );
}

#[test]
fn test_create_stream_cliff_equals_start_succeeds() {
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
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 0);
}

#[test]
fn test_create_stream_cliff_equals_end_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1000u64,
        &1000u64,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_deposit_less_than_total_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

#[test]
fn test_create_stream_deposit_equals_total_succeeds() {
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
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1000);
}

#[test]
fn test_create_stream_deposit_greater_than_total_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 2000);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_create_stream_insufficient_balance_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &20_000_i128,
        &20_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

#[test]
fn test_create_stream_transfer_failure_no_state_change() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &20_000_i128,
            &20_i128,
            &0u64,
            &0u64,
            &1000u64,
        )
    }));

    assert!(
        result.is_err(),
        "should have panicked on insufficient balance"
    );
}

// ---------------------------------------------------------------------------
// Tests — calculate_accrued
// ---------------------------------------------------------------------------

#[test]
fn test_calculate_accrued_at_start() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(0);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "nothing accrued at start_time");
}

#[test]
fn test_calculate_accrued_mid_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(300);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 300, "300s × 1/s = 300");
}

#[test]
fn test_calculate_accrued_capped_at_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(9999);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1000, "accrued must be capped at deposit_amount");
}

#[test]
fn test_calculate_accrued_before_cliff_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(200);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "nothing accrued before cliff");
}

#[test]
fn test_calculate_accrued_after_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(600);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 600,
        "600s × 1/s = 600 (uses start_time, not cliff)"
    );
}

// ---------------------------------------------------------------------------
// Tests — pause / resume
// ---------------------------------------------------------------------------

#[test]
fn test_pause_and_resume() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn test_admin_can_resume_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_pause_already_paused_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().pause_stream(&stream_id);
    ctx.client().pause_stream(&stream_id);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_resume_active_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().resume_stream(&stream_id);
}

// ---------------------------------------------------------------------------
// Tests — cancel_stream
// ---------------------------------------------------------------------------

#[test]
fn test_cancel_stream_full_refund() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let sender_balance_before = ctx.token().balance(&ctx.sender);

    ctx.env.ledger().set_timestamp(0);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    let sender_balance_after = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_balance_after - sender_balance_before, 1000);
}

#[test]
fn test_cancel_stream_partial_refund() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    let sender_balance_before = ctx.token().balance(&ctx.sender);

    ctx.client().cancel_stream(&stream_id);

    let sender_balance_after = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_balance_after - sender_balance_before, 700);
}

#[test]
fn test_cancel_stream_as_admin() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(0);

    ctx.client().cancel_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_cancel_already_cancelled_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().cancel_stream(&stream_id);
    ctx.client().cancel_stream(&stream_id);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_cancel_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    ctx.client().cancel_stream(&stream_id);
}

#[test]
fn test_cancel_paused_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().pause_stream(&stream_id);
    ctx.client().cancel_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Tests — withdraw
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_after_cancel_gets_accrued_amount() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 400);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_withdraw_twice_after_cancel_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);
    ctx.client().withdraw(&stream_id);
    ctx.client().withdraw(&stream_id);
}

#[test]
fn test_withdraw_mid_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 500);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_withdraw_before_cliff_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&stream_id);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_withdraw_paused_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.client().withdraw(&stream_id);
}

#[test]
fn test_withdraw_after_resume_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    ctx.client().pause_stream(&stream_id);
    ctx.client().resume_stream(&stream_id);

    let recipient_before = ctx.token().balance(&ctx.recipient);
    let amount = ctx.client().withdraw(&stream_id);

    assert_eq!(amount, 500);
    assert_eq!(ctx.token().balance(&ctx.recipient) - recipient_before, 500);
}

// ---------------------------------------------------------------------------
// Tests — stream count / multiple streams
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_streams_independent() {
    let ctx = TestContext::setup();
    let id0 = ctx.create_default_stream();
    let id1 = ctx
        .client()
        .create_stream(&ctx.sender, &ctx.recipient, &200, &2, &0, &0, &100);

    assert_eq!(id0, 0);
    assert_eq!(id1, 1);

    ctx.client().cancel_stream(&id0);
    assert_eq!(
        ctx.client().get_stream_state(&id0).status,
        StreamStatus::Cancelled
    );
    assert_eq!(
        ctx.client().get_stream_state(&id1).status,
        StreamStatus::Active
    );
}

// ---------------------------------------------------------------------------
// Tests — stream not found errors
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_get_stream_state_not_found_panics() {
    let ctx = TestContext::setup();
    ctx.client().get_stream_state(&999u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_pause_stream_not_found_panics() {
    let ctx = TestContext::setup();
    ctx.client().pause_stream(&999u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_resume_stream_not_found_panics() {
    let ctx = TestContext::setup();
    ctx.client().resume_stream(&999u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_cancel_stream_not_found_panics() {
    let ctx = TestContext::setup();
    ctx.client().cancel_stream(&999u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_withdraw_stream_not_found_panics() {
    let ctx = TestContext::setup();
    ctx.client().withdraw(&999u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_calculate_accrued_not_found_panics() {
    let ctx = TestContext::setup();
    ctx.client().calculate_accrued(&999u64);
}

// ---------------------------------------------------------------------------
// Tests — completed stream state
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_withdraw_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    ctx.client().withdraw(&stream_id);
}

// ---------------------------------------------------------------------------
// Tests — negative values
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_negative_deposit_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &-100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_create_stream_negative_rate_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &-1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

// ---------------------------------------------------------------------------
// Tests — not initialized errors
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_get_config_not_initialized_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.get_config();
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_create_stream_not_initialized_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.create_stream(&sender, &recipient, &1000, &1, &0, &0, &1000);
}
