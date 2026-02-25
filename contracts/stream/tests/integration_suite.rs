extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient, StreamStatus};
use soroban_sdk::{
    log,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
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

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        let token = TokenClient::new(&env, &token_id);

        Self {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
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

    fn create_stream_with_cliff(&self, cliff_time: u64) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &cliff_time,
            &1000u64,
        )
    }
}

#[test]
fn init_sets_config_and_keeps_token_address() {
    let ctx = TestContext::setup();

    let config = ctx.client().get_config();
    assert_eq!(config.admin, ctx.admin);
    assert_eq!(config.token, ctx.token_id);
}

#[test]
#[should_panic]
fn init_twice_panics() {
    let ctx = TestContext::setup();
    ctx.client().init(&ctx.token_id, &ctx.admin);
}

#[test]
fn get_stream_count_reports_next_stream_id() {
    let ctx = TestContext::setup();
    assert_eq!(ctx.client().get_stream_count(), 0);

    let id0 = ctx.create_default_stream();
    assert_eq!(id0, 0);
    assert_eq!(ctx.client().get_stream_count(), 1);

    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(id1, 1);
    assert_eq!(ctx.client().get_stream_count(), 2);
}

// ---------------------------------------------------------------------------
// Tests — Issue #62: config immutability after re-init attempt
// ---------------------------------------------------------------------------

/// After a failed re-init with different params, config must still hold the
/// original token and admin addresses.
#[test]
fn reinit_with_different_params_preserves_config() {
    let ctx = TestContext::setup();

    // Snapshot original config
    let original = ctx.client().get_config();

    // Attempt re-init with completely different addresses
    let new_token = Address::generate(&ctx.env);
    let new_admin = Address::generate(&ctx.env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().init(&new_token, &new_admin);
    }));
    assert!(result.is_err(), "re-init should have panicked");

    // Config must be unchanged
    let after = ctx.client().get_config();
    assert_eq!(
        after.token, original.token,
        "token must survive reinit attempt"
    );
    assert_eq!(
        after.admin, original.admin,
        "admin must survive reinit attempt"
    );
}

/// Stream counter must remain unaffected by a failed re-init attempt.
#[test]
fn stream_counter_unaffected_by_reinit_attempt() {
    let ctx = TestContext::setup();

    // Create first stream (id = 0)
    let id0 = ctx.create_default_stream();
    assert_eq!(id0, 0);

    // Attempt re-init (should fail)
    let new_admin = Address::generate(&ctx.env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().init(&ctx.token_id, &new_admin);
    }));
    assert!(result.is_err(), "re-init should have panicked");

    // Create second stream — counter must still be 1
    ctx.env.ledger().set_timestamp(0);
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(
        id1, 1,
        "stream counter must continue from 1 after failed reinit"
    );
}

#[test]
fn create_stream_persists_state_and_moves_deposit() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();
    let state = ctx.client().get_stream_state(&stream_id);

    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 0);
    assert_eq!(state.end_time, 1000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
}

#[test]
fn withdraw_accrued_amount_updates_balances_and_state() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(250);
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(withdrawn, 250);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 250);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.recipient), 250);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 750);
}

/// Test withdraw before cliff returns 0 (idempotent behavior).
#[test]
fn withdraw_before_cliff_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream_with_cliff(500);

    ctx.env.ledger().set_timestamp(100);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 0);
}

#[test]
fn get_stream_state_returns_latest_status() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn full_lifecycle_create_withdraw_to_completion() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Mid-stream withdrawal.
    ctx.env.ledger().set_timestamp(400);
    let first = ctx.client().withdraw(&stream_id);
    assert_eq!(first, 400);

    // Final withdrawal at end of stream should complete the stream.
    ctx.env.ledger().set_timestamp(1000);
    let second = ctx.client().withdraw(&stream_id);
    assert_eq!(second, 600);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);

    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

#[test]
#[should_panic]
fn get_stream_state_unknown_id_panics() {
    let ctx = TestContext::setup();
    ctx.client().get_stream_state(&99);
}

#[test]
fn create_stream_rejects_underfunded_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
        );
    }));

    assert!(result.is_err());
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

#[test]
fn harness_mints_sender_balance() {
    let ctx = TestContext::setup();
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
}

/// End-to-end integration test: create stream, advance time in steps,
/// withdraw multiple times, verify amounts and final Completed status.
///
/// This test covers:
/// - Stream creation and initial state
/// - Multiple partial withdrawals at different time points
/// - Balance verification after each withdrawal
/// - Final withdrawal that completes the stream
/// - Status transition to Completed
/// - Correct final balances for all parties
#[test]
fn integration_full_flow_multiple_withdraws_to_completed() {
    let ctx = TestContext::setup();

    // Initial balances
    let sender_initial = ctx.token.balance(&ctx.sender);
    assert_eq!(sender_initial, 10_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Create stream: 5000 tokens over 5000 seconds (1 token/sec), no cliff
    ctx.env.ledger().set_timestamp(1000);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &6000u64,
    );

    // Verify stream created and deposit transferred
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 1000);
    assert_eq!(state.end_time, 6000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.sender), 5_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 5_000);

    // First withdrawal at 20% progress (1000 seconds elapsed)
    ctx.env.ledger().set_timestamp(2000);
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4000);

    // Second withdrawal at 50% progress (1500 more seconds)
    ctx.env.ledger().set_timestamp(3500);
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 1500);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 2500);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2500);

    // Third withdrawal at 80% progress (1000 more seconds)
    ctx.env.ledger().set_timestamp(4500);
    let withdrawn_3 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_3, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 3500);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 3500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1500);

    // Final withdrawal at 100% (end_time reached)
    ctx.env.ledger().set_timestamp(6000);
    let withdrawn_4 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_4, 1500);

    // Verify stream is now Completed
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 5000);
    assert_eq!(state.status, StreamStatus::Completed);

    // Verify final balances
    assert_eq!(ctx.token.balance(&ctx.recipient), 5000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    assert_eq!(ctx.token.balance(&ctx.sender), 5000);

    // Verify total withdrawn equals deposit
    assert_eq!(withdrawn_1 + withdrawn_2 + withdrawn_3 + withdrawn_4, 5000);
}

/// Integration test: multiple withdrawals with time advancement beyond end_time.
/// Verifies that accrual caps at deposit_amount and status transitions correctly.
#[test]
fn integration_withdraw_beyond_end_time() {
    let ctx = TestContext::setup();

    // Create stream: 2000 tokens over 1000 seconds (2 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    // Withdraw at 25%
    ctx.env.ledger().set_timestamp(250);
    let w1 = ctx.client().withdraw(&stream_id);
    assert_eq!(w1, 500);

    // Withdraw at 75%
    ctx.env.ledger().set_timestamp(750);
    let w2 = ctx.client().withdraw(&stream_id);
    assert_eq!(w2, 1000);

    // Advance time well beyond end_time
    ctx.env.ledger().set_timestamp(5000);
    let w3 = ctx.client().withdraw(&stream_id);
    assert_eq!(w3, 500); // Only remaining 500, not more

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 2000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2000);
}

/// Integration test: create stream → cancel immediately → sender receives full refund.
///
/// This test covers:
/// - Stream creation with deposit transfer
/// - Immediate cancellation (no time elapsed, no accrual)
/// - Full refund to sender
/// - Stream status transitions to Cancelled
/// - All balances are correct (sender gets full deposit back, recipient gets nothing)
#[test]
fn integration_cancel_immediately_full_refund() {
    let ctx = TestContext::setup();

    // Record initial balances
    let sender_initial = ctx.token.balance(&ctx.sender);
    let recipient_initial = ctx.token.balance(&ctx.recipient);
    let contract_initial = ctx.token.balance(&ctx.contract_id);

    assert_eq!(sender_initial, 10_000);
    assert_eq!(recipient_initial, 0);
    assert_eq!(contract_initial, 0);

    // Create stream: 3000 tokens over 3000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(1000);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &4000u64,
    );

    // Verify deposit transferred
    assert_eq!(ctx.token.balance(&ctx.sender), 7_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);

    // Cancel immediately (no time elapsed)
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 0);

    // Verify sender received full refund
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Integration test: create stream → advance time → cancel → sender receives partial refund.
///
/// This test covers:
/// - Stream creation and time advancement
/// - Partial accrual (30% of stream duration)
/// - Cancellation with partial refund
/// - Sender receives unstreamed amount (70% of deposit)
/// - Accrued amount (30%) remains in contract for recipient
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_partial_accrual_partial_refund() {
    let ctx = TestContext::setup();

    // Create stream: 5000 tokens over 5000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &5000u64,
    );

    // Verify initial state after creation
    assert_eq!(ctx.token.balance(&ctx.sender), 5_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 5_000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.deposit_amount, 5000);

    // Advance time to 30% completion (1500 seconds)
    ctx.env.ledger().set_timestamp(1500);

    // Verify accrued amount before cancel
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1500);

    // Cancel stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received refund of unstreamed amount (3500 tokens)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 3500);
    assert_eq!(sender_after_cancel, 8_500);

    // Verify accrued amount (1500) remains in contract for recipient
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    // Verify recipient can withdraw the accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Integration test: create stream → advance to 100% → cancel → no refund.
///
/// This test covers:
/// - Stream creation and full time advancement
/// - Full accrual (100% of deposit)
/// - Cancellation when fully accrued
/// - Sender receives no refund (all tokens accrued to recipient)
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_fully_accrued_no_refund() {
    let ctx = TestContext::setup();

    // Create stream: 2000 tokens over 1000 seconds (2 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 8_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_000);

    // Advance time to 100% completion (or beyond)
    ctx.env.ledger().set_timestamp(1000);

    // Verify full accrual
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2000);

    // Cancel stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received NO refund (balance unchanged)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    assert_eq!(sender_after_cancel, sender_before_cancel);
    assert_eq!(sender_after_cancel, 8_000);

    // Verify all tokens remain in contract for recipient
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_000);

    // Verify recipient can withdraw full amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 2000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Integration test: create stream → withdraw partially → cancel → correct refund.
///
/// This test covers:
/// - Stream creation and partial withdrawal
/// - Cancellation after partial withdrawal
/// - Sender receives refund of unstreamed amount (not withdrawn amount)
/// - Accrued but not withdrawn amount remains for recipient
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_after_partial_withdrawal() {
    let ctx = TestContext::setup();

    // Create stream: 4000 tokens over 4000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &4000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &4000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 6_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4_000);

    // Advance to 25% and withdraw
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);

    // Advance to 60% and cancel
    ctx.env.ledger().set_timestamp(2400);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2400);

    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received refund of unstreamed amount
    // Unstreamed = deposit - accrued = 4000 - 2400 = 1600
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 1600);
    assert_eq!(sender_after_cancel, 7_600);

    // Verify accrued but not withdrawn amount remains in contract
    // Accrued = 2400, Withdrawn = 1000, Remaining = 1400
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_400);

    // Verify recipient can withdraw remaining accrued amount
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 1400);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2_400);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Verify total withdrawn equals accrued
    assert_eq!(withdrawn_1 + withdrawn_2, 2400);
}

/// Integration test: create stream → multiple partial withdrawals → cancel → correct refund.
///
/// This test covers:
/// - Multiple partial withdrawals before cancellation
/// - Cancellation after multiple withdrawal transactions
/// - Sender receives refund of unstreamed amount (independent of withdrawal count)
/// - Accrued but not withdrawn remains for recipient
/// - All balances remain consistent through multiple withdrawal operations
#[test]
fn integration_cancel_after_multiple_partial_withdrawals() {
    let ctx = TestContext::setup();

    // Create stream: 5000 tokens over 5000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &5000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 5_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 5_000);

    // First withdrawal at t=1000 (20% accrual)
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4_000);

    // Second withdrawal at t=2500 (50% accrual)
    ctx.env.ledger().set_timestamp(2500);
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 1500); // 2500 accrued - 1000 already withdrawn
    assert_eq!(ctx.token.balance(&ctx.recipient), 2_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_500);

    // Third withdrawal at t=3500 (70% accrual)
    ctx.env.ledger().set_timestamp(3500);
    let withdrawn_3 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_3, 1000); // 3500 accrued - 2500 already withdrawn
    assert_eq!(ctx.token.balance(&ctx.recipient), 3_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_500);

    // Cancel at t=4200 (84% accrual)
    ctx.env.ledger().set_timestamp(4200);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 4200);

    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 3500);

    // Verify sender received refund of unstreamed amount
    // Unstreamed = deposit - accrued = 5000 - 4200 = 800
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 800);
    assert_eq!(sender_after_cancel, 5_800);

    // Verify accrued but not withdrawn amount remains in contract
    // Accrued = 4200, Withdrawn = 3500, Remaining = 700
    assert_eq!(ctx.token.balance(&ctx.contract_id), 700);

    // Verify recipient can withdraw remaining accrued amount
    let withdrawn_final = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_final, 700);
    assert_eq!(ctx.token.balance(&ctx.recipient), 4_200);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Verify total withdrawn across all calls equals accrued
    assert_eq!(
        withdrawn_1 + withdrawn_2 + withdrawn_3 + withdrawn_final,
        4200
    );
}

/// Integration test: create stream with cliff → cancel before cliff → full refund.
///
/// This test covers:
/// - Stream creation with cliff
/// - Cancellation before cliff time
/// - Full refund to sender (no accrual before cliff)
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_before_cliff_full_refund() {
    let ctx = TestContext::setup();

    // Create stream with cliff: 3000 tokens over 3000 seconds, cliff at 1500
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &1500u64, // cliff at 50%
        &3000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 7_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);

    // Advance time before cliff (1000 seconds, before 1500 cliff)
    ctx.env.ledger().set_timestamp(1000);

    // Verify no accrual before cliff
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0);

    // Cancel stream
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received full refund
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
}

/// Integration test: create stream with cliff → cancel after cliff → partial refund.
///
/// This test covers:
/// - Stream creation with cliff
/// - Cancellation after cliff time
/// - Partial refund based on accrual from start_time (not cliff_time)
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_after_cliff_partial_refund() {
    let ctx = TestContext::setup();

    // Create stream with cliff: 4000 tokens over 4000 seconds, cliff at 2000
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &4000_i128,
        &1_i128,
        &0u64,
        &2000u64, // cliff at 50%
        &4000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 6_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4_000);

    // Advance time after cliff (2500 seconds, past 2000 cliff)
    ctx.env.ledger().set_timestamp(2500);

    // Verify accrual after cliff (calculated from start_time)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2500);

    // Cancel stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received refund of unstreamed amount (1500)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 1500);
    assert_eq!(sender_after_cancel, 7_500);

    // Verify accrued amount remains in contract
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_500);

    // Verify recipient can withdraw accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 2500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

// ---------------------------------------------------------------------------
// Integration tests — stream_id generation and uniqueness
// ---------------------------------------------------------------------------

/// Creating N streams must produce IDs 0, 1, 2, …, N-1 with no gaps or duplicates.
///
/// Verifies:
/// - Counter starts at 0 after init
/// - Each create_stream call advances the counter by exactly 1
/// - The returned stream_id matches the value stored in the Stream struct
/// - No two streams share the same id
#[test]
fn integration_stream_ids_are_unique_and_sequential() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    const N: u64 = 10;
    let mut collected: std::vec::Vec<u64> = std::vec::Vec::new();

    for expected in 0..N {
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100_i128,
            &1_i128,
            &0u64,
            &0u64,
            &100u64,
        );

        // Returned id must be sequential
        assert_eq!(
            id, expected,
            "stream {expected}: id must equal counter value"
        );

        // Id stored inside the struct must match the returned id
        let state = ctx.client().get_stream_state(&id);
        assert_eq!(
            state.stream_id, id,
            "stream {expected}: stored stream_id must equal returned id"
        );

        collected.push(id);
    }

    // Pairwise uniqueness — no duplicate ids
    for i in 0..collected.len() {
        for j in (i + 1)..collected.len() {
            assert_ne!(
                collected[i], collected[j],
                "stream_ids at positions {i} and {j} must be unique"
            );
        }
    }
}

/// A create_stream call that fails validation must NOT advance NextStreamId;
/// the following successful call must receive the id that would have been next.
///
/// Verifies:
/// - Validation failures (underfunded deposit) leave the counter unchanged
/// - Subsequent successful calls receive the correct sequential id
#[test]
fn integration_failed_creation_does_not_advance_counter() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // First successful stream → id = 0
    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(id0, 0, "first stream must be id 0");

    // Attempt a stream with an underfunded deposit → must panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1_i128, // deposit < rate * duration
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
        );
    }));
    assert!(result.is_err(), "underfunded create_stream must panic");

    // Next successful stream must be id = 1, not 2
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(
        id1, 1,
        "counter must not advance after a failed create_stream"
    );

    // Verify both streams are independently retrievable
    assert_eq!(ctx.client().get_stream_state(&id0).stream_id, 0);
    assert_eq!(ctx.client().get_stream_state(&id1).stream_id, 1);
}

/// Integration test: create stream → pause → cancel → correct refund.
///
/// This test covers:
/// - Stream creation and pause
/// - Cancellation of paused stream
/// - Correct refund calculation (accrual continues even when paused)
/// - Stream status transitions from Paused to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_paused_stream() {
    let ctx = TestContext::setup();

    // Create stream: 3000 tokens over 3000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &3000u64,
    );

    // Advance to 40% and pause
    ctx.env.ledger().set_timestamp(1200);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Advance time further (accrual continues even when paused)
    ctx.env.ledger().set_timestamp(2000);

    // Verify accrual continues based on time (not affected by pause)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2000);

    // Cancel paused stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received refund of unstreamed amount (1000)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 1000);
    assert_eq!(sender_after_cancel, 8_000);

    // Verify accrued amount remains in contract
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_000);
}

/// Integration test: create stream, pause, advance time, resume, advance time, withdraw.
/// Asserts accrual and withdrawals reflect paused period (accrual continues, withdrawals blocked).
///
/// Test flow:
/// 1. Create a 1000-token stream over 1000 seconds (1 token/sec), starting at t=0
/// 2. Advance to t=300, verify 300 tokens accrued, pause the stream
/// 3. Advance to t=700 (400 more seconds), verify accrual continues during pause (700 total)
/// 4. Attempt withdrawal while paused (should fail)
/// 5. Resume stream at t=700
/// 6. Withdraw 700 tokens accrued
/// 7. Advance to t=1000 (end of stream)
/// 8. Withdraw remaining 300 tokens
/// 9. Verify stream completes and final balances are correct
///
/// Key assertions:
/// - Accrual is time-based and unaffected by pause state
/// - Withdrawals are blocked while stream is paused
/// - After resume, withdrawals work with all accrued amounts
/// - Total withdrawn equals deposit amount
/// - Status transitions through Active -> Paused -> Active -> Completed
#[test]
fn integration_pause_resume_withdraw_lifecycle() {
    let ctx = TestContext::setup();

    // -----------------------------------------------------------------------
    // Phase 1: Create stream (t=0)
    // -----------------------------------------------------------------------
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
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.withdrawn_amount, 0);

    // Verify deposit transferred to contract
    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    // -----------------------------------------------------------------------
    // Phase 2: Advance to t=300 and pause
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(300);

    // Verify 300 tokens accrued
    let accrued_at_300 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_at_300, 300);

    // Pause stream (sender authorization required)
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
    assert_eq!(
        state.withdrawn_amount, 0,
        "no withdrawals should occur during pause"
    );

    // -----------------------------------------------------------------------
    // Phase 3: Advance to t=700 while paused, verify accrual continues
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(700);

    // Verify accrual continues during pause (time-based, not status-based)
    let accrued_at_700 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_at_700, 700,
        "accrual must continue during pause period"
    );

    // Attempt to withdraw while paused — should fail
    let withdrawal_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw(&stream_id);
    }));
    let err = withdrawal_result.expect_err("withdrawal should panic while stream is paused");
    // Ensure the panic reason matches the expected paused-stream invariant
    let panic_msg = err
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| err.downcast_ref::<String>().map(|s| s.as_str()))
        .unwrap_or("<non-string panic payload>");
    assert!(
        panic_msg.contains("cannot withdraw from paused stream"),
        "unexpected panic message when withdrawing from paused stream: {}",
        panic_msg
    );

    // Verify stream still paused and no tokens transferred
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    // -----------------------------------------------------------------------
    // Phase 4: Resume stream at t=700
    // -----------------------------------------------------------------------
    ctx.client().resume_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.withdrawn_amount, 0);

    // -----------------------------------------------------------------------
    // Phase 5: Withdraw all accrued amount (700 tokens) at t=700
    // -----------------------------------------------------------------------
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 700, "should withdraw all 700 accrued tokens");

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.withdrawn_amount, 700);

    // Verify balances after withdrawal
    assert_eq!(ctx.token.balance(&ctx.recipient), 700);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 300);

    // -----------------------------------------------------------------------
    // Phase 6: Advance to t=1000 (end of stream) and withdraw remaining
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(1000);

    // Verify 1000 tokens accrued at end
    let accrued_at_1000 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_at_1000, 1000);

    // Withdraw final 300 tokens (1000 - 700 already withdrawn)
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 300, "should withdraw remaining 300 tokens");

    // Verify stream is now Completed
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);

    // Verify final balances
    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Verify total withdrawn equals deposit
    assert_eq!(withdrawn_1 + withdrawn_2, 1000);
}

/// Integration test: multiple pause/resume cycles with time advancement.
/// Verifies that accrual is unaffected by repeated pause/resume operations.
///
/// Test flow:
/// 1. Create 2000-token stream over 2000 seconds
/// 2. Advance to t=500, pause
/// 3. Advance to t=1000, resume
/// 4. Advance to t=1500, pause
/// 5. Advance to t=1800, resume
/// 6. Withdraw at t=1800 (1800 tokens should be accrued)
/// 7. Advance to t=2000 (end)
/// 8. Withdraw final 200 tokens
///
/// Verifies accrual accumulates correctly through multiple pause/resume cycles.
#[test]
fn integration_multiple_pause_resume_cycles() {
    let ctx = TestContext::setup();

    // Create stream: 2000 tokens over 2000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
    );

    // First pause/resume cycle
    ctx.env.ledger().set_timestamp(500);
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.env.ledger().set_timestamp(1000);
    let accrued_at_1000 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_at_1000, 1000, "accrual continues during pause");

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Second pause/resume cycle
    ctx.env.ledger().set_timestamp(1500);
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.env.ledger().set_timestamp(1800);
    let accrued_at_1800 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_at_1800, 1800,
        "accrual continues through multiple pauses"
    );

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Withdraw at t=1800
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1800);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1800);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1800);

    // Final withdrawal at end
    ctx.env.ledger().set_timestamp(2000);
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 200);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 2000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2000);
}

/// Integration test: pause, advance past end_time, resume, verify capped accrual.
/// Ensures accrual remains capped at deposit_amount even with pause during stream.
///
/// Test flow:
/// 1. Create 1000-token stream over 1000 seconds
/// 2. Advance to t=300, pause
/// 3. Advance to t=2000 (well past end_time)
/// 4. Resume stream
/// 5. Verify accrual is capped at 1000 (not 2000)
/// 6. Withdraw all 1000 tokens
/// 7. Stream completes
#[test]
fn integration_pause_resume_past_end_time_accrual_capped() {
    let ctx = TestContext::setup();

    // Create stream: 1000 tokens over 1000 seconds
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

    // Pause at t=300
    ctx.env.ledger().set_timestamp(300);
    ctx.client().pause_stream(&stream_id);

    // Advance far past end_time (t=2000)
    ctx.env.ledger().set_timestamp(2000);

    // Verify accrual is still capped at deposit_amount
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 1000,
        "accrual must be capped at deposit_amount even past end_time"
    );

    // Resume and withdraw
    ctx.client().resume_stream(&stream_id);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);
}

/// Integration test: pause stream, then cancel while paused.
/// Verifies that accrual reflects time elapsed even during pause,
/// and sender receives correct refund for unstreamed amount.
///
/// Test flow:
/// 1. Create 3000-token stream over 1000 seconds (3 tokens/sec)
/// 2. Advance to t=300, pause
/// 3. Advance to t=600 (paused, 1800 tokens accrued but blocked from withdrawal)
/// 4. Cancel stream as sender
/// 5. Verify sender receives refund for unstreamed amount (1200 tokens)
/// 6. Verify recipient can still withdraw accrued 1800 tokens
#[test]
fn integration_pause_then_cancel_preserves_accrual() {
    let ctx = TestContext::setup();

    // Create stream: 3000 tokens over 1000 seconds (3 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &3_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    assert_eq!(ctx.token.balance(&ctx.sender), 7_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);

    // Pause at t=300 (900 tokens accrued)
    ctx.env.ledger().set_timestamp(300);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Advance to t=600 while paused (1800 tokens accrued, recipient cannot withdraw)
    ctx.env.ledger().set_timestamp(600);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1800, "accrual continues during pause");

    // Cancel paused stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender receives refund of unstreamed amount (3000 - 1800 = 1200)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 1200, "refund should be deposit - accrued");
    assert_eq!(sender_after_cancel, 8_200);

    // Verify accrued amount (1800) remains in contract for recipient
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1800);

    // Recipient can still withdraw accrued amount from cancelled stream
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1800);

    assert_eq!(ctx.token.balance(&ctx.recipient), 1800);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Integration test: same sender creates multiple streams to different recipients.
///
/// This test verifies that:
/// 1. create_stream returns distinct stream IDs for each stream
/// 2. Each stream maintains independent state in persistent storage
/// 3. get_stream_state returns correct stream for each ID
/// 4. Multiple streams can be withdrawn from independently
/// 5. Token balances are correctly managed across multiple streams
/// 6. Each stream lifecycle (create, withdraw, complete) is independent
///
/// Test flow:
/// 1. Setup test context with sender and mint tokens
/// 2. Create first stream: sender -> recipient1 (1000 tokens, 1 token/sec, 1000s duration)
/// 3. Create second stream: sender -> recipient2 (2000 tokens, 2 tokens/sec, 1000s duration)
/// 4. Create third stream: sender -> recipient1 (500 tokens, 1 token/sec, 500s duration)
/// 5. Verify all three streams have distinct IDs (0, 1, 2)
/// 6. Verify initial balances: sender loses 3500 tokens, contract holds 3500
/// 7. Advance time and withdraw from stream 1 independently
/// 8. Advance time and withdraw from stream 0 independently
/// 9. Verify stream 2 is unaffected by withdrawals from other streams
/// 10. Withdraw from stream 2 and verify completion
/// 11. Verify final balances and all recipient accounts
#[test]
fn integration_same_sender_multiple_streams() {
    let ctx = TestContext::setup();

    // Setup additional recipient for testing
    let recipient2 = Address::generate(&ctx.env);

    // Initial state
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&recipient2), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // === Stream 1: sender -> recipient (1000 tokens, 1 token/sec, start=0, end=1000)
    ctx.env.ledger().set_timestamp(0);
    let stream_id_0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(stream_id_0, 0, "first stream should have id=0");

    // === Stream 2: sender -> recipient2 (2000 tokens, 2 tokens/sec, start=0, end=1000)
    ctx.env.ledger().set_timestamp(0);
    let stream_id_1 = ctx.client().create_stream(
        &ctx.sender,
        &recipient2,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(stream_id_1, 1, "second stream should have id=1");

    // === Stream 3: sender -> recipient (500 tokens, 1 token/sec, start=0, end=500)
    ctx.env.ledger().set_timestamp(0);
    let stream_id_2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
    );
    assert_eq!(stream_id_2, 2, "third stream should have id=2");

    // Verify all stream IDs are distinct
    assert_ne!(stream_id_0, stream_id_1, "stream ids must be unique");
    assert_ne!(stream_id_1, stream_id_2, "stream ids must be unique");
    assert_ne!(stream_id_0, stream_id_2, "stream ids must be unique");

    // Verify balances after creating all three streams
    // Total deposit: 1000 + 2000 + 500 = 3500
    let sender_balance = ctx.token.balance(&ctx.sender);
    assert_eq!(sender_balance, 6_500, "sender should have 10000 - 3500");
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        3_500,
        "contract should hold 3500"
    );

    // === Verify stream metadata for each stream
    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(state_0.stream_id, stream_id_0);
    assert_eq!(state_0.sender, ctx.sender);
    assert_eq!(state_0.recipient, ctx.recipient);
    assert_eq!(state_0.deposit_amount, 1000);
    assert_eq!(state_0.rate_per_second, 1);
    assert_eq!(state_0.end_time, 1000);
    assert_eq!(state_0.withdrawn_amount, 0);
    assert_eq!(state_0.status, StreamStatus::Active);

    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state_1.stream_id, stream_id_1);
    assert_eq!(state_1.sender, ctx.sender);
    assert_eq!(state_1.recipient, recipient2);
    assert_eq!(state_1.deposit_amount, 2000);
    assert_eq!(state_1.rate_per_second, 2);
    assert_eq!(state_1.end_time, 1000);
    assert_eq!(state_1.withdrawn_amount, 0);
    assert_eq!(state_1.status, StreamStatus::Active);

    let state_2 = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state_2.stream_id, stream_id_2);
    assert_eq!(state_2.sender, ctx.sender);
    assert_eq!(state_2.recipient, ctx.recipient);
    assert_eq!(state_2.deposit_amount, 500);
    assert_eq!(state_2.rate_per_second, 1);
    assert_eq!(state_2.end_time, 500);
    assert_eq!(state_2.withdrawn_amount, 0);
    assert_eq!(state_2.status, StreamStatus::Active);

    // === Independent withdrawals from multiple streams
    // Withdraw from stream_id_1 (recipient2) at t=250 (500 tokens accrued)
    ctx.env.ledger().set_timestamp(250);
    let withdrawn_1_at_250 = ctx.client().withdraw(&stream_id_1);
    assert_eq!(withdrawn_1_at_250, 500);

    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state_1.withdrawn_amount, 500);
    assert_eq!(state_1.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&recipient2), 500);

    // Verify stream 0 and 2 are unaffected
    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(state_0.withdrawn_amount, 0, "stream 0 should be unaffected");

    let state_2 = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state_2.withdrawn_amount, 0, "stream 2 should be unaffected");

    // Withdraw from stream_id_0 (recipient) at t=300 (300 tokens accrued since start)
    ctx.env.ledger().set_timestamp(300);
    let withdrawn_0_at_300 = ctx.client().withdraw(&stream_id_0);
    assert_eq!(withdrawn_0_at_300, 300);

    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(state_0.withdrawn_amount, 300);
    assert_eq!(state_0.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 300);

    // Verify stream 1 state is preserved (still has 1500 accrued, 500 withdrawn)
    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state_1.withdrawn_amount, 500);
    assert_eq!(state_1.status, StreamStatus::Active);

    // Verify stream 2 state is preserved
    let state_2 = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state_2.withdrawn_amount, 0);
    assert_eq!(state_2.status, StreamStatus::Active);

    // Verify contract balance reflects withdrawals
    // Initial: 3500, Withdrawn: 500 + 300 = 800
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_700);

    // === Complete stream 2 (should reach end_time at t=500)
    ctx.env.ledger().set_timestamp(500);
    let withdrawn_2_at_500 = ctx.client().withdraw(&stream_id_2);
    assert_eq!(withdrawn_2_at_500, 500);

    let state_2 = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state_2.withdrawn_amount, 500);
    assert_eq!(state_2.status, StreamStatus::Completed);
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        800,
        "recipient gets 300 + 500"
    );

    // Verify streams 0 and 1 are still active
    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(state_0.status, StreamStatus::Active);

    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state_1.status, StreamStatus::Active);

    // === Complete stream 0 at t=1000
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn_0_at_1000 = ctx.client().withdraw(&stream_id_0);
    assert_eq!(
        withdrawn_0_at_1000, 700,
        "700 tokens remaining (1000 - 300)"
    );

    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(state_0.withdrawn_amount, 1000);
    assert_eq!(state_0.status, StreamStatus::Completed);
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        1500,
        "recipient gets 800 + 700"
    );

    // === Complete stream 1 at t=1000
    let withdrawn_1_at_1000 = ctx.client().withdraw(&stream_id_1);
    assert_eq!(
        withdrawn_1_at_1000, 1500,
        "1500 tokens remaining (2000 - 500)"
    );

    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state_1.withdrawn_amount, 2000);
    assert_eq!(state_1.status, StreamStatus::Completed);
    assert_eq!(ctx.token.balance(&recipient2), 2000);

    // === Final balance verification
    assert_eq!(
        ctx.token.balance(&ctx.sender),
        6_500,
        "sender balance unchanged"
    );
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        1500,
        "recipient total: 300+500+700"
    );
    assert_eq!(
        ctx.token.balance(&recipient2),
        2000,
        "recipient2 total: 500+1500"
    );
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        0,
        "contract should be empty after all withdrawals"
    );

    // Verify total tokens are conserved
    let total = ctx.token.balance(&ctx.sender)
        + ctx.token.balance(&ctx.recipient)
        + ctx.token.balance(&recipient2)
        + ctx.token.balance(&ctx.contract_id);
    assert_eq!(total, 10_000, "total tokens must be conserved");
}

/// Integration test: same sender creates multiple streams to SAME recipient.
///
/// This test covers an important edge case: multiple streams to the same recipient
/// must maintain independent state even though they share a recipient address.
///
/// This test verifies that:
/// 1. Multiple streams to same recipient get distinct stream IDs
/// 2. Each stream maintains independent state despite shared recipient
/// 3. get_stream_state correctly differentiates between streams with same recipient
/// 4. Withdrawals are tracked independently per stream
/// 5. Recipient receives funds from all streams correctly
/// 6. Token balances properly account for multiple independent stream deposits
///
/// Test flow:
/// 1. Create stream 0: sender -> recipient (1000 tokens, 1 token/sec, 0-1000s)
/// 2. Create stream 1: sender -> recipient (1000 tokens, 1 token/sec, 0-1000s)
/// 3. Create stream 2: sender -> recipient (1000 tokens, 1 token/sec, 0-500s)
/// 4. Verify distinct IDs and independent metadata
/// 5. Withdraw from stream 1 at t=200
/// 6. Verify stream 0 and 2 are unaffected
/// 7. Withdraw from stream 2 at t=500 (completion)
/// 8. Verify stream 0 state unchanged, stream 1 still has different balance
/// 9. Complete both remaining streams independently
/// 10. Verify recipient receives tokens from all three streams
#[test]
fn integration_same_sender_same_recipient_multiple_streams() {
    let ctx = TestContext::setup();

    // Initial state
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // === Create multiple streams to SAME recipient
    // Stream 0: 1000 tokens, 1 token/sec, 0-1000s
    ctx.env.ledger().set_timestamp(0);
    let stream_id_0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(stream_id_0, 0, "first stream to recipient should have id=0");

    // Stream 1: 1000 tokens, 1 token/sec, 0-1000s (same recipient as stream 0)
    ctx.env.ledger().set_timestamp(0);
    let stream_id_1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(
        stream_id_1, 1,
        "second stream to same recipient should have id=1"
    );

    // Stream 2: 500 tokens, 1 token/sec, 0-500s (same recipient, shorter duration)
    // Rate * duration = 1 * 500 = 500 tokens (matches deposit)
    ctx.env.ledger().set_timestamp(0);
    let stream_id_2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
    );
    assert_eq!(
        stream_id_2, 2,
        "third stream to same recipient should have id=2"
    );

    // === Verify distinct stream IDs
    assert_ne!(
        stream_id_0, stream_id_1,
        "streams to same recipient must have different IDs"
    );
    assert_ne!(
        stream_id_1, stream_id_2,
        "streams to same recipient must have different IDs"
    );
    assert_ne!(
        stream_id_0, stream_id_2,
        "streams to same recipient must have different IDs"
    );

    // === Verify balances
    // Total deposit: 1000 + 1000 + 500 = 2500, all to same recipient
    assert_eq!(ctx.token.balance(&ctx.sender), 7_500, "sender loses 2500");
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        2_500,
        "contract holds 2500 total"
    );
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        0,
        "recipient has no balance yet"
    );

    // === Verify independent stream metadata despite shared recipient
    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(state_0.stream_id, 0, "stream 0 id must be 0");
    assert_eq!(state_0.recipient, ctx.recipient);
    assert_eq!(state_0.deposit_amount, 1000);
    assert_eq!(state_0.end_time, 1000, "stream 0 ends at 1000s");
    assert_eq!(state_0.withdrawn_amount, 0);

    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state_1.stream_id, 1, "stream 1 id must be 1");
    assert_eq!(
        state_1.recipient, ctx.recipient,
        "stream 1 has same recipient"
    );
    assert_eq!(state_1.deposit_amount, 1000);
    assert_eq!(state_1.end_time, 1000, "stream 1 ends at 1000s");
    assert_eq!(state_1.withdrawn_amount, 0);

    let state_2 = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state_2.stream_id, 2, "stream 2 id must be 2");
    assert_eq!(
        state_2.recipient, ctx.recipient,
        "stream 2 has same recipient"
    );
    assert_eq!(state_2.deposit_amount, 500, "stream 2 deposit is 500");
    assert_eq!(
        state_2.end_time, 500,
        "stream 2 ends at 500s (shorter duration)"
    );
    assert_eq!(state_2.withdrawn_amount, 0);

    // === Independent withdrawal from stream 1 at t=200
    ctx.env.ledger().set_timestamp(200);
    let withdrawn_1_at_200 = ctx.client().withdraw(&stream_id_1);
    assert_eq!(
        withdrawn_1_at_200, 200,
        "stream 1 accrues 200 tokens by t=200"
    );

    // Verify stream 1 state after withdrawal
    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(
        state_1.withdrawn_amount, 200,
        "stream 1 withdrawn_amount = 200"
    );
    assert_eq!(state_1.status, StreamStatus::Active);

    // Verify streams 0 and 2 are unaffected by stream 1 withdrawal
    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(
        state_0.withdrawn_amount, 0,
        "stream 0 unaffected by stream 1 withdrawal"
    );
    assert_eq!(state_0.status, StreamStatus::Active);

    let state_2 = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(
        state_2.withdrawn_amount, 0,
        "stream 2 unaffected by stream 1 withdrawal"
    );
    assert_eq!(state_2.status, StreamStatus::Active);

    // Recipient receives 200 from stream 1
    assert_eq!(ctx.token.balance(&ctx.recipient), 200);

    // === Complete stream 2 at t=500 (it has shorter duration)
    ctx.env.ledger().set_timestamp(500);
    let withdrawn_2_at_500 = ctx.client().withdraw(&stream_id_2);
    assert_eq!(withdrawn_2_at_500, 500, "stream 2 completes at t=500");

    // Verify stream 2 is now Completed
    let state_2 = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state_2.withdrawn_amount, 500);
    assert_eq!(state_2.status, StreamStatus::Completed);

    // Verify streams 0 and 1 are still Active and independent
    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(
        state_0.withdrawn_amount, 0,
        "stream 0 still has 0 withdrawn"
    );
    assert_eq!(state_0.status, StreamStatus::Active);

    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(
        state_1.withdrawn_amount, 200,
        "stream 1 still has 200 withdrawn"
    );
    assert_eq!(state_1.status, StreamStatus::Active);

    // Recipient now has 200 + 500 = 700
    assert_eq!(ctx.token.balance(&ctx.recipient), 700);

    // === Withdraw more from stream 0 at t=600
    ctx.env.ledger().set_timestamp(600);
    let withdrawn_0_at_600 = ctx.client().withdraw(&stream_id_0);
    assert_eq!(withdrawn_0_at_600, 600, "stream 0 accrues 600 by t=600");

    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(state_0.withdrawn_amount, 600);
    assert_eq!(state_0.status, StreamStatus::Active);

    // Recipient now has 700 + 600 = 1300
    assert_eq!(ctx.token.balance(&ctx.recipient), 1300);

    // === Complete stream 1 at t=1000
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn_1_at_1000 = ctx.client().withdraw(&stream_id_1);
    assert_eq!(
        withdrawn_1_at_1000, 800,
        "stream 1 has 800 remaining (1000-200)"
    );

    let state_1 = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state_1.withdrawn_amount, 1000);
    assert_eq!(state_1.status, StreamStatus::Completed);

    // Recipient now has 1300 + 800 = 2100
    assert_eq!(ctx.token.balance(&ctx.recipient), 2100);

    // === Complete stream 0 at t=1000
    let withdrawn_0_at_1000 = ctx.client().withdraw(&stream_id_0);
    assert_eq!(
        withdrawn_0_at_1000, 400,
        "stream 0 has 400 remaining (1000-600)"
    );

    let state_0 = ctx.client().get_stream_state(&stream_id_0);
    assert_eq!(state_0.withdrawn_amount, 1000);
    assert_eq!(state_0.status, StreamStatus::Completed);

    // Recipient now has 2100 + 400 = 2500
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        2500,
        "recipient total from all streams"
    );

    // === Final balance verification
    assert_eq!(ctx.token.balance(&ctx.sender), 7_500, "sender unchanged");
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        2500,
        "recipient: 200+500+600+1000=2500"
    );
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0, "contract empty");

    // Verify total tokens conserved
    let total = ctx.token.balance(&ctx.sender)
        + ctx.token.balance(&ctx.recipient)
        + ctx.token.balance(&ctx.contract_id);
    assert_eq!(total, 10_000, "total tokens conserved");
}

#[test]
fn test_create_many_streams_from_same_sender() {
    let ctx = TestContext::setup();
    ctx.env.budget().reset_default();

    ctx.env.ledger().set_timestamp(0);

    let counter_stop = 50;
    let mut counter = 0;
    let mut stream_vec = Vec::new(&ctx.env);
    let deposit = 10_i128;
    let rate = 1_i128;
    let start = 0u64;
    let cliff = 0u64;
    let end = 10u64;
    loop {
        let recipient = Address::generate(&ctx.env);
        let stream_id = ctx.client().create_stream(
            &ctx.sender,
            &recipient,
            &deposit,
            &rate,
            &start,
            &cliff,
            &end,
        );

        let state = ctx.client().get_stream_state(&stream_id);
        assert_eq!(state.stream_id, stream_id);
        assert_eq!(state.stream_id, counter);
        assert_eq!(state.sender, ctx.sender);
        assert_eq!(state.recipient, recipient);
        assert_eq!(state.deposit_amount, deposit);
        assert_eq!(state.rate_per_second, rate);
        assert_eq!(state.start_time, start);
        assert_eq!(state.cliff_time, cliff);
        assert_eq!(state.end_time, end);
        assert_eq!(state.withdrawn_amount, 0);
        assert_eq!(state.status, StreamStatus::Active);

        counter += 1;

        stream_vec.push_back(stream_id);
        if counter == counter_stop {
            break;
        }
    }

    let cpu_insns = ctx.env.budget().cpu_instruction_cost();
    log!(&ctx.env, "cpu_insns", cpu_insns);
    assert!(cpu_insns == 21_796_255);

    // Check memory bytes consumed
    let mem_bytes = ctx.env.budget().memory_bytes_cost();
    log!(&ctx.env, "mem_bytes", mem_bytes);
    assert!(mem_bytes == 4_326_285);
}
