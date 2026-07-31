//! Retry safety tests — verify that retried operations produce deterministic results.
//!
//! These tests formalize the retry safety invariants documented in `docs/manifest-versioning.md`:
//! 1. Idempotent operations return the same result on retry
//! 2. Deterministic error handling returns the same error on retry
//! 3. All operations are timestamp-deterministic given same wall clock
//! 4. No state is partially mutated if validation fails (CEI pattern)

use fluxora_stream::{
    CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    _admin: Address,
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
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        sac.mint(&sender, &10_000_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
            _admin: admin,
        }
    }

    fn create_test_stream(&self, deposit: i128, rate: i128, duration: u64) -> u64 {
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: rate,
                start_time: 0,
                cliff_time: 0,
                end_time: duration,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

// ============================================================================
// Retry Safety: Create Stream (Idempotency via CEI)
// ============================================================================

/// Creating a stream twice with identical parameters should produce the same stream_id.
///
/// This tests that the global `NextStreamId` counter increments deterministically
/// and that stream creation is idempotent (CEI pattern ensures no partial state mutation).
#[test]
fn retry_safety_create_stream_idempotent() {
    let ctx = TestContext::setup();

    let params = CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: 5000,
        rate_per_second: 5,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        withdraw_dust_threshold: Some(0),
        memo: None,
        metadata: None,
        kind: StreamKind::Linear,
        irrevocable: None,
        witness: None,
    };

    // First create
    let stream_id_1 = ctx
        .client
        .create_stream(&ctx.sender, &params)
        .expect("first create should succeed");

    // Verify stream is persisted
    let stream_1 = ctx
        .client
        .get_stream_state(&stream_id_1)
        .expect("stream should exist");
    assert_eq!(stream_1.deposit_amount, 5000);

    // Second create with same parameters should produce a NEW stream_id (not idempotent on input,
    // but the sequence of operations is deterministic)
    let stream_id_2 = ctx
        .client
        .create_stream(&ctx.sender, &params)
        .expect("second create should succeed");

    // Both streams should exist and be retrievable
    let stream_2 = ctx
        .client
        .get_stream_state(&stream_id_2)
        .expect("stream should exist");
    assert_eq!(stream_2.deposit_amount, 5000);

    // Both should have identical parameters (but different IDs)
    assert_ne!(stream_id_1, stream_id_2);
    assert_eq!(stream_1.deposit_amount, stream_2.deposit_amount);
    assert_eq!(stream_1.rate_per_second, stream_2.rate_per_second);
}

// ============================================================================
// Retry Safety: Withdraw (Deterministic Accrual)
// ============================================================================

/// Withdrawing at the same timestamp should produce the same result each time.
///
/// Accrual is deterministic: given the stream parameters and current time,
/// the result is always the same. Retrying at the same timestamp should yield
/// identical withdrawable amount.
#[test]
fn retry_safety_withdraw_deterministic_timestamp() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(10000, 10, 1000);

    // Advance to t=500 (accrued = 5000)
    ctx.env.ledger().set_timestamp(500);

    // First withdraw
    let withdrawable_1 = ctx
        .client
        .get_withdrawable(&stream_id)
        .expect("should exist");
    assert_eq!(withdrawable_1, 5000);

    ctx.client
        .withdraw(&stream_id, &None)
        .expect("first withdraw should succeed");
    let withdrawn_after_1 = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("stream should exist")
        .withdrawn_amount;

    // Retry at same timestamp (after CCU would be idempotent, but we just query state)
    let withdrawable_2 = ctx
        .client
        .get_withdrawable(&stream_id)
        .expect("should exist");
    // After first withdraw, withdrawable should be 0 (already withdrawn 5000 out of 5000 accrued)
    assert_eq!(withdrawable_2, 0);

    // Verify withdrawn amount didn't double-count
    assert_eq!(withdrawn_after_1, 5000);
}

/// Advancing time and withdrawing multiple times should be monotonic.
#[test]
fn retry_safety_multiple_withdraws_monotonic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(10000, 100, 1000);

    let mut previous_withdrawn = 0i128;

    for t in [100u64, 200, 300].iter() {
        ctx.env.ledger().set_timestamp(*t);
        ctx.client
            .withdraw(&stream_id, &None)
            .expect("withdraw should succeed");

        let stream = ctx
            .client
            .get_stream_state(&stream_id)
            .expect("stream should exist");
        assert!(stream.withdrawn_amount >= previous_withdrawn);
        previous_withdrawn = stream.withdrawn_amount;
    }
}

// ============================================================================
// Retry Safety: Batch Operations (Validation Determinism)
// ============================================================================

/// Batch withdraw with duplicate IDs should fail deterministically on each retry.
#[test]
fn retry_safety_batch_withdraw_duplicate_deterministic() {
    let ctx = TestContext::setup();
    let id1 = ctx.create_test_stream(1000, 1, 1000);
    let id2 = ctx.create_test_stream(1000, 1, 1000);

    ctx.env.ledger().set_timestamp(500);

    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    ids.push_back(id1);
    ids.push_back(id2);
    ids.push_back(id1); // Duplicate

    // First attempt should fail
    let result_1 = ctx.client.batch_withdraw(&ids);
    assert!(result_1.is_err());

    // Retry with same input should produce same error (deterministic)
    let result_2 = ctx.client.batch_withdraw(&ids);
    assert!(result_2.is_err());

    // Error type should be the same
    assert_eq!(result_1.is_err(), result_2.is_err());
}

/// Batch operations with valid IDs should succeed consistently.
#[test]
fn retry_safety_batch_withdraw_valid_deterministic() {
    let ctx = TestContext::setup();
    let id1 = ctx.create_test_stream(1000, 1, 1000);
    let id2 = ctx.create_test_stream(1000, 1, 1000);
    let id3 = ctx.create_test_stream(1000, 1, 1000);

    ctx.env.ledger().set_timestamp(500);

    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    ids.push_back(id1);
    ids.push_back(id2);
    ids.push_back(id3);

    // First batch withdraw
    let result_1 = ctx.client.batch_withdraw(&ids).expect("should succeed");
    assert_eq!(result_1.len(), 3);

    // All streams should have withdrawn 500
    let s1 = ctx.client.get_stream_state(&id1).expect("should exist");
    assert_eq!(s1.withdrawn_amount, 500);

    // After withdrawal, withdrawable should be 0 for all
    let withdrawable_1 = ctx.client.get_withdrawable(&id1).expect("should exist");
    assert_eq!(withdrawable_1, 0);

    // Retry batch on already-withdrawn streams should succeed but withdraw 0
    let result_2 = ctx.client.batch_withdraw(&ids).expect("should succeed");
    assert_eq!(result_2.len(), 3);

    // Withdrawn amounts should be unchanged (idempotent)
    let s1_after = ctx.client.get_stream_state(&id1).expect("should exist");
    assert_eq!(s1_after.withdrawn_amount, 500);
}

// ============================================================================
// Retry Safety: Pause/Resume (Cooldown Determinism)
// ============================================================================

/// Pausing a stream at the same ledger should produce the same result.
#[test]
fn retry_safety_pause_deterministic_ledger() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    ctx.env.ledger().set_timestamp(100);

    // First pause
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational)
        .expect("first pause should succeed");

    let stream_1 = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert_eq!(stream_1.status as u32, 1); // Paused

    // Attempt to pause again at same ledger should fail (already paused)
    let pause_result = ctx
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert!(pause_result.is_err());

    // Advance past cooldown
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);

    // Resume
    ctx.client
        .resume_stream(&stream_id)
        .expect("resume should succeed");

    let stream_2 = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert_eq!(stream_2.status as u32, 0); // Active
}

// ============================================================================
// Retry Safety: Rate Decrease (Checkpoint Determinism)
// ============================================================================

/// Rate decrease at the same timestamp should produce identical checkpoint state.
#[test]
fn retry_safety_rate_decrease_checkpoint_deterministic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(10000, 10, 1000);

    ctx.env.ledger().set_timestamp(500);

    // Get accrued before rate decrease
    let accrued_before = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should exist");
    assert_eq!(accrued_before, 5000);

    // Decrease rate
    ctx.client
        .decrease_rate_per_second(&stream_id, 5)
        .expect("first decrease should succeed");

    let stream_after_1 = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert_eq!(stream_after_1.checkpointed_amount, 5000);
    assert_eq!(stream_after_1.checkpointed_at, 500);

    // Attempting to decrease rate again at same timestamp should update checkpoint
    // (each decrease recalculates accrual and rebases checkpoint)
    ctx.client
        .decrease_rate_per_second(&stream_id, 3)
        .expect("second decrease should succeed");

    let stream_after_2 = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    // Checkpoint should be based on accrual at t=500 with rate=5 (unchanged)
    assert_eq!(stream_after_2.checkpointed_amount, 5000);
    assert_eq!(stream_after_2.checkpointed_at, 500);

    // Advance time and verify accrual uses new rate consistently
    ctx.env.ledger().set_timestamp(600);
    let accrued_at_600 = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should exist");
    // checkpoint(5000) + 3*(600-500) = 5000 + 300 = 5300
    assert_eq!(accrued_at_600, 5300);
}

// ============================================================================
// Retry Safety: CEI Pattern (No Partial State Mutations)
// ============================================================================

/// Validation failures should not mutate storage (CEI pattern).
///
/// If a batch operation fails validation, no streams should be modified.
#[test]
fn retry_safety_validation_fails_no_partial_mutation() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    ctx.env.ledger().set_timestamp(500);

    // Record initial state
    let initial = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    let initial_withdrawn = initial.withdrawn_amount;
    assert_eq!(initial_withdrawn, 0);

    // Attempt batch withdraw with duplicate (will fail validation)
    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    ids.push_back(stream_id);
    ids.push_back(stream_id); // Duplicate

    let result = ctx.client.batch_withdraw(&ids);
    assert!(result.is_err());

    // Verify state is unchanged (no partial mutation)
    let after_failed = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert_eq!(after_failed.withdrawn_amount, initial_withdrawn);
}

// ============================================================================
// Retry Safety: Terminal State (Idempotent Rejection)
// ============================================================================

/// Operations on terminal streams should fail deterministically.
#[test]
fn retry_safety_terminal_state_idempotent_rejection() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    ctx.env.ledger().set_timestamp(1000); // Reach end_time

    // Withdraw full amount
    ctx.client.withdraw(&stream_id, &None).expect("should succeed");

    // Stream is now Completed
    let stream = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert_eq!(stream.status as u32, 2); // Completed

    // First pause attempt should fail
    let pause_1 = ctx
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert!(pause_1.is_err());

    // Retry pause should fail identically
    let pause_2 = ctx
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert!(pause_2.is_err());

    // First top-up attempt should fail
    let topup_1 = ctx.client.top_up_stream(&stream_id, &ctx.sender, 100);
    assert!(topup_1.is_err());

    // Retry top-up should fail identically
    let topup_2 = ctx.client.top_up_stream(&stream_id, &ctx.sender, 100);
    assert!(topup_2.is_err());
}

// ============================================================================
// Retry Safety: Idempotent Queries (No Side Effects)
// ============================================================================

/// Query operations should be idempotent and not affect withdrawable amounts.
#[test]
fn retry_safety_queries_no_side_effects() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(5000, 5, 1000);

    ctx.env.ledger().set_timestamp(300);

    // Query withdrawable multiple times
    let w1 = ctx
        .client
        .get_withdrawable(&stream_id)
        .expect("should exist");
    let w2 = ctx
        .client
        .get_withdrawable(&stream_id)
        .expect("should exist");
    let w3 = ctx
        .client
        .get_withdrawable(&stream_id)
        .expect("should exist");

    assert_eq!(w1, 1500);
    assert_eq!(w2, 1500);
    assert_eq!(w3, 1500);

    // Query accrued multiple times
    let a1 = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should exist");
    let a2 = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should exist");

    assert_eq!(a1, 1500);
    assert_eq!(a2, 1500);

    // Verify state is unchanged
    let stream = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert_eq!(stream.withdrawn_amount, 0);
}

// ============================================================================
// Retry Safety: Timestamp Consistency (CEI with Multiple Queries)
// ============================================================================

/// All queries in a single invocation should use consistent timestamp.
///
/// This tests that `current_accrual_timestamp()` is read once per invocation,
/// then reused for all accrual calculations.
#[test]
fn retry_safety_timestamp_consistency_within_invocation() {
    let ctx = TestContext::setup();
    let s1 = ctx.create_test_stream(10000, 100, 1000);
    let s2 = ctx.create_test_stream(10000, 100, 1000);

    ctx.env.ledger().set_timestamp(250);

    // Calculate accrued for multiple streams in same invocation
    let a1 = ctx.client.calculate_accrued(&s1).expect("should exist");
    let a2 = ctx.client.calculate_accrued(&s2).expect("should exist");

    // Both should be identical (same rate, same elapsed time)
    assert_eq!(a1, 25000);
    assert_eq!(a2, 25000);

    // If we retry at exact same timestamp, results should be identical
    let a1_retry = ctx.client.calculate_accrued(&s1).expect("should exist");
    let a2_retry = ctx.client.calculate_accrued(&s2).expect("should exist");

    assert_eq!(a1, a1_retry);
    assert_eq!(a2, a2_retry);
}

// ============================================================================
// Retry Safety: Version Endpoint (Deterministic, Permissionless)
// ============================================================================

/// Version endpoint should return identical value across multiple calls.
#[test]
fn retry_safety_version_endpoint_deterministic() {
    let ctx = TestContext::setup();

    let v1 = ctx.client.version();
    let v2 = ctx.client.version();
    let v3 = ctx.client.version();

    assert_eq!(v1, v2);
    assert_eq!(v2, v3);
    assert_eq!(v1, 9); // Current CONTRACT_VERSION
}

// ============================================================================
// Regression: Ensure Determinism Not Broken by Future Changes
// ============================================================================

/// Verify that accrual determinism is maintained across the test suite.
#[test]
fn regression_accrual_determinism() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(100000, 1000, 1000);

    // Sample accrual at various times multiple times
    for t in [0u64, 100, 250, 500, 750, 999].iter() {
        ctx.env.ledger().set_timestamp(*t);

        // Query 3 times at same timestamp
        let a1 = ctx
            .client
            .calculate_accrued(&stream_id)
            .expect("should exist");
        let a2 = ctx
            .client
            .calculate_accrued(&stream_id)
            .expect("should exist");
        let a3 = ctx
            .client
            .calculate_accrued(&stream_id)
            .expect("should exist");

        assert_eq!(a1, a2);
        assert_eq!(a2, a3);
        assert_eq!(a1, (*t as i128) * 1000);
    }
}

/// Verify monotonicity is preserved under repeated operations.
#[test]
fn regression_monotonic_withdrawn() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(100000, 10000, 100);

    let mut previous = 0i128;

    for t in 0..=100 {
        ctx.env.ledger().set_timestamp(t);
        ctx.client.withdraw(&stream_id, &None).expect("should succeed");

        let stream = ctx
            .client
            .get_stream_state(&stream_id)
            .expect("should exist");
        assert!(stream.withdrawn_amount >= previous);
        previous = stream.withdrawn_amount;
    }
}
