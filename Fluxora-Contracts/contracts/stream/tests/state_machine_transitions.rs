// State machine transition tests for Issue #572
// Verifies every state transition documented in docs/state-machine.md

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason,
    StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Events},
    vec, Address, Env, String as SorobanString,
};

// Test fixture setup
struct TestFixture<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    token: soroban_sdk::token::Client<'a>,
    sender: Address,
    recipient: Address,
    admin: Address,
}

impl<'a> TestFixture<'a> {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin.clone()).address();
        let token = soroban_sdk::token::Client::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Initialize contract
        client.init(&token_id, &admin);

        // Mint tokens to sender
        token.mint(&sender, &1_000_000_000);

        Self {
            env,
            client,
            token,
            sender,
            recipient,
            admin,
        }
    }

    fn create_active_stream(&self) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &1000,
            &1,
            &now,
            &now,
            &(now + 1000),
            &0,
            &None,
        )
    }

    fn create_paused_stream(&self) -> u64 {
        let stream_id = self.create_active_stream();
        self.client
            .pause_stream(&stream_id, &PauseReason::Operational);
        stream_id
    }
}

// ============================================================================
// TRANSITION: [*] → Active (Creation)
// ============================================================================

#[test]
fn test_transition_initial_to_active_create_stream() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
    assert_eq!(stream.sender, fixture.sender);
    assert_eq!(stream.recipient, fixture.recipient);
}

#[test]
fn test_transition_initial_to_active_create_streams() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    let params = vec![
        &fixture.env,
        CreateStreamParams {
            recipient: fixture.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: now,
            cliff_time: now,
            end_time: now + 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
        },
    ];

    let stream_ids = fixture.client.create_streams(&fixture.sender, &params);
    assert_eq!(stream_ids.len(), 1);

    let stream = fixture.client.get_stream_state(&stream_ids.get(0).unwrap());
    assert_eq!(stream.status, StreamStatus::Active);
}

#[test]
fn test_transition_initial_to_active_create_stream_relative() {
    let fixture = TestFixture::new();

    let params = fluxora_stream::CreateStreamRelativeParams {
        recipient: fixture.recipient.clone(),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_delay: 0,
        cliff_delay: 0,
        duration: 1000,
        withdraw_dust_threshold: Some(0),
        memo: None,
    };

    let stream_id = fixture.client.create_stream_relative(&fixture.sender, &params);

    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
}

#[test]
#[should_panic(expected = "StartTimeInPast")]
fn test_creation_guard_start_time_in_past() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Attempt to create stream with start_time in the past
    fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &(now - 100), // Past start time
        &now,
        &(now + 1000),
        &0,
        &None,
    );
}

#[test]
#[should_panic(expected = "InvalidParams")]
fn test_creation_guard_sender_equals_recipient() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Attempt to create stream where sender == recipient
    fixture.client.create_stream(
        &fixture.sender,
        &fixture.sender, // Same as sender
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );
}

#[test]
#[should_panic(expected = "InvalidParams")]
fn test_creation_guard_invalid_time_range() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Attempt to create stream where start_time >= end_time
    fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &now, // end_time == start_time
        &0,
        &None,
    );
}

#[test]
#[should_panic(expected = "InsufficientDeposit")]
fn test_creation_guard_insufficient_deposit() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Attempt to create stream where deposit < rate × duration
    fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &500,  // Insufficient
        &1,
        &now,
        &now,
        &(now + 1000), // Needs 1000
        &0,
        &None,
    );
}

// ============================================================================
// TRANSITION: Active → Paused
// ============================================================================

#[test]
fn test_transition_active_to_paused() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Verify initial state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);

    // Transition to Paused
    fixture
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);

    // Verify final state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Paused);
}

#[test]
fn test_transition_active_to_paused_as_admin() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Admin can pause any stream
    fixture
        .client
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Paused);
}

#[test]
#[should_panic(expected = "StreamAlreadyPaused")]
fn test_pause_guard_already_paused() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_paused_stream();

    // Attempt to pause already-paused stream
    fixture
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);
}

#[test]
#[should_panic(expected = "StreamTerminalState")]
fn test_pause_guard_time_terminal() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Create stream that ends immediately
    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1000, // High rate
        &now,
        &now,
        &(now + 1), // Ends in 1 second
        &0,
        &None,
    );

    // Advance time past end_time
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 10;
    });

    // Attempt to pause time-terminal stream
    fixture
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);
}

// ============================================================================
// TRANSITION: Paused → Active
// ============================================================================

#[test]
fn test_transition_paused_to_active() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_paused_stream();

    // Verify initial state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Paused);

    // Transition to Active
    fixture.client.resume_stream(&stream_id);

    // Verify final state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
}

#[test]
fn test_transition_paused_to_active_as_admin() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_paused_stream();

    // Admin can resume any stream
    fixture.client.resume_stream_as_admin(&stream_id);

    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
}

#[test]
#[should_panic(expected = "StreamNotPaused")]
fn test_resume_guard_not_paused() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Attempt to resume active stream
    fixture.client.resume_stream(&stream_id);
}

#[test]
#[should_panic(expected = "StreamTerminalState")]
fn test_resume_guard_time_terminal() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Create and pause stream
    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1000,
        &now,
        &now,
        &(now + 1),
        &0,
        &None,
    );
    fixture
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);

    // Advance time past end_time
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 10;
    });

    // Attempt to resume time-terminal stream
    fixture.client.resume_stream(&stream_id);
}

// ============================================================================
// TRANSITION: Active → Cancelled
// ============================================================================

#[test]
fn test_transition_active_to_cancelled() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Verify initial state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);

    let sender_balance_before = fixture.token.balance(&fixture.sender);

    // Transition to Cancelled
    fixture.client.cancel_stream(&stream_id);

    // Verify final state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert!(stream.cancelled_at.is_some());

    // Verify refund was issued
    let sender_balance_after = fixture.token.balance(&fixture.sender);
    assert!(sender_balance_after > sender_balance_before);
}

#[test]
fn test_transition_active_to_cancelled_as_admin() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Admin can cancel any stream
    fixture.client.cancel_stream_as_admin(&stream_id);

    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

// ============================================================================
// TRANSITION: Paused → Cancelled
// ============================================================================

#[test]
fn test_transition_paused_to_cancelled() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_paused_stream();

    // Verify initial state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Paused);

    // Transition to Cancelled
    fixture.client.cancel_stream(&stream_id);

    // Verify final state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

#[test]
#[should_panic(expected = "InvalidState")]
fn test_cancel_guard_already_cancelled() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Cancel once
    fixture.client.cancel_stream(&stream_id);

    // Attempt to cancel again
    fixture.client.cancel_stream(&stream_id);
}

#[test]
#[should_panic(expected = "InvalidState")]
fn test_cancel_guard_completed() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Create stream
    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    // Advance time and withdraw all
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 1000;
    });
    fixture.client.withdraw(&stream_id);

    // Verify completed
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Completed);

    // Attempt to cancel completed stream
    fixture.client.cancel_stream(&stream_id);
}

// ============================================================================
// TRANSITION: Active → Completed
// ============================================================================

#[test]
fn test_transition_active_to_completed() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Create stream
    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    // Verify initial state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);

    // Advance time to end
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 1000;
    });

    let recipient_balance_before = fixture.token.balance(&fixture.recipient);

    // Withdraw all (triggers completion)
    let withdrawn = fixture.client.withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);

    // Verify final state
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Completed);
    assert_eq!(stream.withdrawn_amount, stream.deposit_amount);

    // Verify recipient received tokens
    let recipient_balance_after = fixture.token.balance(&fixture.recipient);
    assert_eq!(recipient_balance_after, recipient_balance_before + 1000);
}

#[test]
fn test_transition_active_to_completed_withdraw_to() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();
    let destination = Address::generate(&fixture.env);

    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 1000;
    });

    // Withdraw to destination
    let withdrawn = fixture.client.withdraw_to(&stream_id, &destination);
    assert_eq!(withdrawn, 1000);

    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Completed);

    // Verify destination received tokens
    assert_eq!(fixture.token.balance(&destination), 1000);
}

// ============================================================================
// TRANSITION: Paused → Completed (Terminal Liquidity)
// ============================================================================

#[test]
fn test_transition_paused_to_completed_terminal_liquidity() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Create and pause stream
    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );
    fixture
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);

    // Verify paused
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Paused);

    // Advance time past end_time
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 1000;
    });

    // Withdraw from paused stream (terminal liquidity)
    let withdrawn = fixture.client.withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);

    // Verify completed
    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Completed);
}

#[test]
#[should_panic(expected = "InvalidState")]
fn test_withdraw_guard_paused_before_end_time() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_paused_stream();

    // Attempt to withdraw from paused stream before end_time
    fixture.client.withdraw(&stream_id);
}

// ============================================================================
// TRANSITION: Completed → [*] (Close)
// ============================================================================

#[test]
fn test_transition_completed_to_closed() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    // Complete the stream
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 1000;
    });
    fixture.client.withdraw(&stream_id);

    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Completed);

    // Close the stream (permissionless)
    fixture.client.close_completed_stream(&stream_id);

    // Verify stream is removed
    let result = fixture.client.try_get_stream_state(&stream_id);
    assert!(result.is_err());
}

// ============================================================================
// TRANSITION: Cancelled → [*] (Close)
// ============================================================================

#[test]
fn test_transition_cancelled_to_closed() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    // Advance time and cancel
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 500;
    });
    fixture.client.cancel_stream(&stream_id);

    // Withdraw frozen accrued amount
    fixture.client.withdraw(&stream_id);

    // Verify no claimable balance remains
    let stream = fixture.client.get_stream_state(&stream_id);
    let accrued = fixture.client.calculate_accrued(&stream_id);
    assert_eq!(stream.withdrawn_amount, accrued);

    // Close the cancelled stream
    fixture.client.close_completed_stream(&stream_id);

    // Verify stream is removed
    let result = fixture.client.try_get_stream_state(&stream_id);
    assert!(result.is_err());
}

#[test]
#[should_panic(expected = "InvalidState")]
fn test_close_guard_cancelled_with_claimable_balance() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    // Advance time and cancel
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 500;
    });
    fixture.client.cancel_stream(&stream_id);

    // Do NOT withdraw (claimable balance remains)

    // Attempt to close with claimable balance
    fixture.client.close_completed_stream(&stream_id);
}

#[test]
#[should_panic(expected = "InvalidState")]
fn test_close_guard_active_stream() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Attempt to close active stream
    fixture.client.close_completed_stream(&stream_id);
}

// ============================================================================
// BATCH OPERATIONS
// ============================================================================

#[test]
fn test_batch_withdraw_to_completed() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Create two streams
    let stream_id_1 = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );
    let stream_id_2 = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &2000,
        &2,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    // Advance time to end
    fixture.env.ledger().with_mut(|li| {
        li.timestamp = now + 1000;
    });

    // Batch withdraw
    let stream_ids = vec![&fixture.env, stream_id_1, stream_id_2];
    let results = fixture
        .client
        .batch_withdraw(&fixture.recipient, &stream_ids);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().amount, 1000);
    assert_eq!(results.get(1).unwrap().amount, 2000);

    // Verify both completed
    let stream_1 = fixture.client.get_stream_state(&stream_id_1);
    let stream_2 = fixture.client.get_stream_state(&stream_id_2);
    assert_eq!(stream_1.status, StreamStatus::Completed);
    assert_eq!(stream_2.status, StreamStatus::Completed);
}

// ============================================================================
// GLOBAL PAUSE GUARDS
// ============================================================================

#[test]
fn test_global_pause_blocks_creation() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Pause protocol
    fixture.client.pause_protocol(
        &fixture.admin,
        &Some(SorobanString::from_str(&fixture.env, "emergency")),
    );

    // Attempt to create stream
    let result = fixture.client.try_create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    assert!(result.is_err());
}

#[test]
fn test_global_pause_blocks_withdrawal() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Pause protocol
    fixture.client.pause_protocol(
        &fixture.admin,
        &Some(SorobanString::from_str(&fixture.env, "emergency")),
    );

    // Attempt to withdraw
    let result = fixture.client.try_withdraw(&stream_id);
    assert!(result.is_err());
}

#[test]
fn test_global_pause_blocks_cancellation() {
    let fixture = TestFixture::new();
    let stream_id = fixture.create_active_stream();

    // Pause protocol
    fixture.client.pause_protocol(
        &fixture.admin,
        &Some(SorobanString::from_str(&fixture.env, "emergency")),
    );

    // Attempt to cancel
    let result = fixture.client.try_cancel_stream(&stream_id);
    assert!(result.is_err());
}

#[test]
fn test_global_resume_restores_operations() {
    let fixture = TestFixture::new();
    let now = fixture.env.ledger().timestamp();

    // Pause protocol
    fixture.client.pause_protocol(
        &fixture.admin,
        &Some(SorobanString::from_str(&fixture.env, "emergency")),
    );

    // Resume protocol
    fixture.client.resume_protocol(&fixture.admin);

    // Verify creation works again
    let stream_id = fixture.client.create_stream(
        &fixture.sender,
        &fixture.recipient,
        &1000,
        &1,
        &now,
        &now,
        &(now + 1000),
        &0,
        &None,
    );

    let stream = fixture.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
}
