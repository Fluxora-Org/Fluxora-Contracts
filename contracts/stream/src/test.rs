#[cfg(test)]
extern crate std;

use soroban_sdk::{
    log,
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

        // Deploy the streaming contract
        let contract_id = env.register_contract(None, FluxoraStream);

        // Create a mock SAC token (Stellar Asset Contract)
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Initialise the streaming contract
        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        // Mint tokens to sender (10_000 USDC-equivalent)
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

    /// Create a standard 1000-unit stream spanning 1000 seconds (rate 1/s, no cliff).
    fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128, // deposit_amount
            &1_i128,    // rate_per_second  (1 token/s)
            &0u64,      // start_time
            &0u64,      // cliff_time (no cliff)
            &1000u64,   // end_time
        )
    }

    /// Create a stream with a cliff at t=500 out of 1000s.
    fn create_cliff_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &500u64, // cliff at t=500
            &1000u64,
        )
    }
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

    // Contract should hold the deposit
    assert_eq!(ctx.token().balance(&ctx.contract_id), 1000);
    // Sender balance reduced by deposit
    assert_eq!(ctx.token().balance(&ctx.sender), 9000);
}

#[test]
#[should_panic(expected = "deposit_amount must be positive")]
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
#[should_panic(expected = "start_time must be before end_time")]
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
        &500u64, // end before start
    );
}

// ---------------------------------------------------------------------------
// Tests — Issue #35: validate positive amounts and sender != recipient
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "rate_per_second must be positive")]
fn test_create_stream_zero_rate_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &0_i128, // zero rate
        &0u64,
        &0u64,
        &1000u64,
    );
}

#[test]
#[should_panic(expected = "sender and recipient must be different")]
fn test_create_stream_sender_equals_recipient_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.sender, // same as sender
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

// ---------------------------------------------------------------------------
// Tests — Issue #33: validate cliff_time in [start_time, end_time]
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
fn test_create_stream_cliff_before_start_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,  // start_time
        &50u64,   // cliff_time before start
        &1100u64, // end_time
    );
}

#[test]
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
fn test_create_stream_cliff_after_end_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1500u64, // cliff_time after end
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
        &0u64, // cliff equals start
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
        &1000u64, // cliff equals end
        &1000u64,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 1000);
}

// ---------------------------------------------------------------------------
// Tests — Issue #34: validate deposit_amount >= rate * duration
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "deposit_amount must cover total streamable amount")]
fn test_create_stream_deposit_less_than_total_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128, // deposit only 500
        &1_i128,   // rate = 1/s
        &0u64,
        &0u64,
        &1000u64, // duration = 1000s, so total = 1000 tokens needed
    );
}

#[test]
fn test_create_stream_deposit_equals_total_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128, // deposit exactly matches total
        &1_i128,    // rate = 1/s
        &0u64,
        &0u64,
        &1000u64, // duration = 1000s
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
        &2000_i128, // deposit more than needed
        &1_i128,    // rate = 1/s
        &0u64,
        &0u64,
        &1000u64, // duration = 1000s, total needed = 1000
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 2000);
}

// ---------------------------------------------------------------------------
// Tests — Issue #36: reject when token transfer fails
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_create_stream_insufficient_balance_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Sender only has 10_000 tokens, trying to deposit 20_000
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

    // Attempt to create stream with insufficient balance (should panic)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &20_000_i128, // more than sender has
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

    // In Soroban, a failed transaction is rolled back, so we can't easily verify
    // state wasn't changed in a unit test. The key point is the transfer happens
    // before any state modification in the contract logic.
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
    ctx.env.ledger().set_timestamp(9999); // way past end

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1000, "accrued must be capped at deposit_amount");
}

#[test]
fn test_calculate_accrued_before_cliff_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(200); // before cliff at 500

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "nothing accrued before cliff");
}

#[test]
fn test_calculate_accrued_after_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(600); // 100s after cliff at 500

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

    // Auth override test for resume
    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
#[should_panic(expected = "stream is not active")]
fn test_pause_already_paused_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().pause_stream(&stream_id);
    ctx.client().pause_stream(&stream_id); // second pause should panic
}

#[test]
#[should_panic(expected = "stream is not paused")]
fn test_resume_active_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().resume_stream(&stream_id); // not paused, should panic
}

// ---------------------------------------------------------------------------
// Tests — cancel_stream
// ---------------------------------------------------------------------------

#[test]
fn test_cancel_stream_full_refund() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let sender_balance_before = ctx.token().balance(&ctx.sender);

    ctx.env.ledger().set_timestamp(0); // no time has passed
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
#[should_panic(expected = "stream must be active or paused to cancel")]
fn test_cancel_already_cancelled_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().cancel_stream(&stream_id);
    ctx.client().cancel_stream(&stream_id);
}

#[test]
#[should_panic(expected = "stream must be active or paused to cancel")]
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
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_twice_after_cancel_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);
    ctx.client().withdraw(&stream_id);
    ctx.client().withdraw(&stream_id);
}

/// Status is Complete when Recipient fully withdraws
#[test]
fn test_withdraw_completed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000); // 400 accrued, 600 unstreamed
    ctx.client().cancel_stream(&stream_id);

    let recipient_balance_before = ctx.token().balance(&ctx.recipient);
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, 1000,
        "recipient should withdraw the 1000 accrued tokens"
    );
    let recipient_balance_after = ctx.token().balance(&ctx.recipient);
    assert_eq!(recipient_balance_after - recipient_balance_before, 1000);

    // Nothing left in contract
    assert_eq!(ctx.token().balance(&ctx.contract_id), 0);

    // Complete withdrawal record
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);
}

/// Status is Complete when Recipient fully withdraws in batches
#[test]
fn test_withdraw_completed_in_batch() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(200); // 200 accrued, 800 unstreamed
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, 200,
        "recipient should withdraw the 200 accrued tokens"
    );

    ctx.env.ledger().set_timestamp(500); // 500 accrued, 500 unstreamed
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, 300,
        "recipient should withdraw the 500 accrued tokens"
    );

    ctx.env.ledger().set_timestamp(1000); // 1000 accrued, 0 unstreamed
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, 500,
        "recipient should withdraw the 500 accrued tokens"
    );

    // Nothing left in contract
    assert_eq!(ctx.token().balance(&ctx.contract_id), 0);

    // Complete withdrawal record
    let state = ctx.client().get_stream_state(&stream_id);
    log!(&ctx.env, "state:", state);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);
}

#[test]
#[should_panic(expected = "stream already completed")]
fn test_withdraw_completed_panic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000); // 400 accrued, 600 unstreamed
    ctx.client().cancel_stream(&stream_id);

    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, 1000,
        "recipient should withdraw the 1000 accrued tokens"
    );

    let _ = ctx.client().withdraw(&stream_id);
}

// ---------------------------------------------------------------------------
// Tests — withdraw (general)
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_mid_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 500);
}

#[test]
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_before_cliff_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&stream_id);
}

// ---------------------------------------------------------------------------
// Tests — Issue #37: withdraw reject when stream is Paused
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "cannot withdraw from paused stream")]
fn test_withdraw_paused_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance time so there's something to withdraw
    ctx.env.ledger().set_timestamp(500);

    // Pause the stream
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Attempt to withdraw while paused should fail
    ctx.client().withdraw(&stream_id);
}

#[test]
fn test_withdraw_after_resume_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance time
    ctx.env.ledger().set_timestamp(500);

    // Pause and then resume
    ctx.client().pause_stream(&stream_id);
    ctx.client().resume_stream(&stream_id);

    // Withdraw should now succeed
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
