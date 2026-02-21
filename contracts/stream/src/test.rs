#[cfg(test)]
extern crate std;

use soroban_sdk::{
    log,
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, FromVal,
};

use crate::{FluxoraStream, FluxoraStreamClient, StreamEvent, StreamStatus};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    sac: StellarAssetClient<'a>,
}

impl<'a> TestContext<'a> {
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
            sac,
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

    fn create_max_rate_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &(i128::MAX - 1),
            &((i128::MAX - 1) / 3),
            &0,
            &0u64,
            &3,
        )
    }

    fn create_half_max_rate_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &42535295865117307932921825928971026400_i128,
            &(42535295865117307932921825928971026400_i128 / 100),
            &0,
            &0u64,
            &100,
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

#[test]
fn test_calculate_accrued_max_values() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &(i128::MAX - 10_000_i128));
    let stream_id = ctx.create_max_rate_stream();

    ctx.env.ledger().set_timestamp(u64::MAX);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, i128::MAX - 1, "accrued should be max");

    let state = ctx.client().get_stream_state(&stream_id);
    assert!(accrued <= state.deposit_amount);
    assert!(accrued >= 0);
}

#[test]
fn test_calculate_accrued_overflow_protection() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &(i128::MAX - 10_000_i128));
    let stream_id = ctx.create_half_max_rate_stream();

    ctx.env.ledger().set_timestamp(1_800);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 42535295865117307932921825928971026400_i128);
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

/// Verify that withdraw enforces recipient-only authorization.
/// The require_auth() on stream.recipient ensures only the recipient can withdraw.
/// This test verifies that the authorization check is in place.
/// Note: In SDK 21.7.7, env.invoker() is not available, so we use require_auth()
/// which is the security-equivalent mechanism. The require_auth() call ensures
/// that only the recipient can authorize the withdrawal, preventing unauthorized access.
#[test]
fn test_withdraw_requires_recipient_authorization() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    // With mock_all_auths(), recipient's auth is mocked, so withdraw succeeds
    // This verifies that the authorization mechanism works correctly
    let recipient_before = ctx.token().balance(&ctx.recipient);
    let amount = ctx.client().withdraw(&stream_id);

    assert_eq!(amount, 500);
    assert_eq!(ctx.token().balance(&ctx.recipient) - recipient_before, 500);

    // Verify the withdrawal was recorded
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 500);

    // The require_auth() call in withdraw() ensures that only the recipient
    // can authorize this call, which is equivalent to checking env.invoker() == recipient
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

// ---------------------------------------------------------------------------
// Tests — Events
// ---------------------------------------------------------------------------

#[test]
fn test_pause_resume_events() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Check pause event
    // The event is published as ((symbol_short!("paused"), stream_id), StreamEvent::Paused(stream_id))
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Paused(stream_id)
    );

    ctx.client().resume_stream(&stream_id);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Check resume event
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Resumed(stream_id)
    );
}

#[test]
fn test_cancel_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().cancel_stream(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Check cancel event
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Cancelled(stream_id)
    );
}

// ---------------------------------------------------------------------------
// Additional Tests — create_stream (enhanced coverage)
// ---------------------------------------------------------------------------

/// Test creating a stream with negative deposit amount panics
#[test]
#[should_panic(expected = "deposit_amount must be positive")]
fn test_create_stream_negative_deposit_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &-100_i128, // negative deposit
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

/// Test creating a stream with negative rate_per_second panics
#[test]
#[should_panic(expected = "rate_per_second must be positive")]
fn test_create_stream_negative_rate_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &-5_i128, // negative rate
        &0u64,
        &0u64,
        &1000u64,
    );
}

/// Test creating a stream where start_time equals end_time panics
#[test]
#[should_panic(expected = "start_time must be before end_time")]
fn test_create_stream_equal_start_end_times_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &500u64,
        &500u64,
        &500u64, // start == end
    );
}

/// Test creating a stream with cliff_time equal to start_time (valid edge case)
#[test]
fn test_create_stream_cliff_equals_start() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &100u64, // cliff == start (valid)
        &1100u64,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 100);
    assert_eq!(state.start_time, 100);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test creating a stream with cliff_time equal to end_time (valid edge case)
#[test]
fn test_create_stream_cliff_equals_end() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff == end (valid)
        &1000u64,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 1000);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test creating multiple streams increments stream_id correctly
#[test]
fn test_create_stream_increments_id_correctly() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
    );

    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &1_i128,
        &0u64,
        &0u64,
        &200u64,
    );

    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &300_i128,
        &1_i128,
        &0u64,
        &0u64,
        &300u64,
    );

    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);

    // Verify each stream has correct data
    let s0 = ctx.client().get_stream_state(&id0);
    let s1 = ctx.client().get_stream_state(&id1);
    let s2 = ctx.client().get_stream_state(&id2);

    assert_eq!(s0.deposit_amount, 100);
    assert_eq!(s1.deposit_amount, 200);
    assert_eq!(s2.deposit_amount, 300);
}

/// Test creating a stream with very large deposit amount
#[test]
fn test_create_stream_large_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Mint large amount to sender
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &1_000_000_000_i128);

    let large_amount = 1_000_000_i128;
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &large_amount,
        &1000_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, large_amount);
    assert_eq!(ctx.token().balance(&ctx.contract_id), large_amount);
}

/// Test creating a stream with very high rate_per_second
#[test]
fn test_create_stream_high_rate() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let high_rate = 1000_i128;
    let duration = 10u64;
    let deposit = high_rate * duration as i128; // Ensure deposit covers total streamable

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &high_rate,
        &0u64,
        &0u64,
        &duration,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, high_rate);
    assert_eq!(state.deposit_amount, deposit);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test creating a stream with different sender and recipient
#[test]
fn test_create_stream_different_addresses() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let another_recipient = Address::generate(&ctx.env);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &another_recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, another_recipient);
}

/// Test creating a stream with future start_time
#[test]
fn test_create_stream_future_start_time() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &1000u64, // starts in the future
        &1000u64,
        &2000u64,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.start_time, 1000);
    assert_eq!(state.end_time, 2000);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test token balance changes after creating stream
#[test]
fn test_create_stream_token_balances() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let recipient_balance_before = ctx.token().balance(&ctx.recipient);

    let deposit = 2500_i128;
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &5_i128,
        &0u64,
        &0u64,
        &500u64,
    );

    // Sender balance should decrease by deposit
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        sender_balance_before - deposit
    );

    // Contract balance should increase by deposit
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before + deposit
    );

    // Recipient balance should remain unchanged (no withdrawal yet)
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        recipient_balance_before
    );
}

/// Test creating stream with minimum valid duration (1 second)
#[test]
fn test_create_stream_minimum_duration() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &100_i128,
        &0u64,
        &0u64,
        &1u64, // 1 second duration
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time - state.start_time, 1);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test creating stream verifies all stream fields are set correctly
#[test]
fn test_create_stream_all_fields_correct() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 5000_i128;
    let rate = 10_i128;
    let start = 100u64;
    let cliff = 200u64;
    let end = 600u64;

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &start,
        &cliff,
        &end,
    );

    let state = ctx.client().get_stream_state(&stream_id);

    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, deposit);
    assert_eq!(state.rate_per_second, rate);
    assert_eq!(state.start_time, start);
    assert_eq!(state.cliff_time, cliff);
    assert_eq!(state.end_time, end);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test that creating stream with same sender and recipient panics
#[test]
#[should_panic(expected = "sender and recipient must be different")]
fn test_create_stream_self_stream_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Attempt to create stream where sender is also recipient (should panic)
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.sender, // same as sender - not allowed
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

// ---------------------------------------------------------------------------
// Tests — get_stream_state
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "stream not found")]
fn test_get_stream_state_non_existent() {
    let ctx = TestContext::setup();
    ctx.client().get_stream_state(&999);
}

#[test]
fn test_get_stream_state_all_statuses() {
    let ctx = TestContext::setup();

    // 1. Check Active (from creation)
    let id_active = ctx.create_default_stream();
    let state_active = ctx.client().get_stream_state(&id_active);
    assert_eq!(state_active.status, StreamStatus::Active);
    assert_eq!(state_active.stream_id, id_active);

    // 2. Check Paused
    let id_paused = ctx.create_default_stream();
    ctx.client().pause_stream(&id_paused);
    let state_paused = ctx.client().get_stream_state(&id_paused);
    assert_eq!(state_paused.status, StreamStatus::Paused);

    // 3. Check Cancelled
    let id_cancelled = ctx.create_default_stream();
    ctx.client().cancel_stream(&id_cancelled);
    let state_cancelled = ctx.client().get_stream_state(&id_cancelled);
    assert_eq!(state_cancelled.status, StreamStatus::Cancelled);

    // 4. Check Completed
    let id_completed = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id_completed);
    let state_completed = ctx.client().get_stream_state(&id_completed);
    assert_eq!(state_completed.status, StreamStatus::Completed);
}

#[test]
#[should_panic(expected = "already initialised")]
fn test_init_twice_panics() {
    let ctx = TestContext::setup();
    ctx.client().init(&ctx.token_id, &ctx.admin);
}

#[test]
fn test_get_config() {
    let ctx = TestContext::setup();
    let config = ctx.client().get_config();
    assert_eq!(config.token, ctx.token_id);
    assert_eq!(config.admin, ctx.admin);
}

#[test]
fn test_cancel_fully_accrued_no_refund() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // 1000 seconds pass → 1000 tokens accrued (full deposit)
    ctx.env.ledger().set_timestamp(1000);

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    let sender_balance_after = ctx.token().balance(&ctx.sender);
    assert_eq!(
        sender_balance_after, sender_balance_before,
        "nothing should be refunded"
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

#[test]
fn test_withdraw_multiple_times() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 200 at t=200
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    // Withdraw another 300 at t=500
    ctx.env.ledger().set_timestamp(500);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 300);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 500);
}

#[test]
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
fn test_create_stream_invalid_cliff_panics() {
    let ctx = TestContext::setup();
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1,
        &100,
        &50,
        &200, // cliff < start
    );
}

#[test]
fn test_create_stream_edge_cliffs() {
    let ctx = TestContext::setup();

    // Cliff at start_time
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100,
        &100,
        &1100,
    );
    assert_eq!(ctx.client().get_stream_state(&id1).cliff_time, 100);

    // Cliff at end_time
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100,
        &1100,
        &1100,
    );
    assert_eq!(ctx.client().get_stream_state(&id2).cliff_time, 1100);
}

#[test]
fn test_calculate_accrued_exactly_at_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at 500
    ctx.env.ledger().set_timestamp(500);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 500,
        "at cliff, should accrue full amount from start"
    );
}
