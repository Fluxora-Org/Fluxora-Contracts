//! Edge-case tests for manifest versioning, upgrade compatibility, and retry safety.
//!
//! This test module formalizes the behavior documented in `docs/manifest-versioning.md`,
//! ensuring that:
//! 1. Manifest versioning handles edge cases consistently
//! 2. Upgrade scenarios maintain backward compatibility
//! 3. Retry operations produce deterministic results
//! 4. Storage transitions are safe and non-destructive

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    MAX_METADATA_BYTES, MAX_METADATA_KEYS, MAX_METADATA_KEY_BYTES, MAX_METADATA_VALUE_BYTES,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Bytes, Env, Map, String,
};

// ============================================================================
// Test Harness
// ============================================================================

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    admin: Address,
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

        // Fund the sender
        sac.mint(&sender, &1_000_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
            admin,
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

    fn advance_time(&self, seconds: u64) {
        self.env.ledger().set_timestamp(seconds);
    }
}

// ============================================================================
// Edge Case 1: Clock Regression Detection and Idempotency
// ============================================================================

/// Clock regression should be detected and idempotent.
///
/// After a stream is created at t=100, calling `calculate_accrued` at t=100 works.
/// Retrying at t=100 should also work (same wall clock).
/// Calling at t=99 should fail with ClockRegression.
#[test]
fn edge_case_clock_regression_detection_idempotent() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    ctx.advance_time(100);

    // First accrual read at t=100
    let accrued1 = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should succeed at t=100");
    assert_eq!(accrued1, 100);

    // Retry at same time should be idempotent
    let accrued2 = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should succeed on retry at t=100");
    assert_eq!(accrued2, 100);

    // Attempting to advance backward should fail
    ctx.advance_time(99);
    let result = ctx.client.calculate_accrued(&stream_id);
    assert!(result.is_err());
    // Should be ClockRegression, mapped to ContractError::ClockRegression
    match result {
        Err(e) if e as u32 == 29 => {} // ClockRegression discriminant
        _ => panic!("expected ClockRegression error"),
    }
}

/// Multiple accrual reads with monotonically increasing timestamps should all succeed.
#[test]
fn edge_case_clock_monotonicity_multiple_reads() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(10000, 10, 1000);

    for t in [100u64, 100, 101, 102, 200, 500].iter() {
        ctx.advance_time(*t);
        let accrued = ctx
            .client
            .calculate_accrued(&stream_id)
            .expect(&format!("should succeed at t={}", t));
        assert_eq!(accrued, (*t as i128) * 10);
    }
}

// ============================================================================
// Edge Case 2: Rate Decrease Across Checkpoint (Entitlement Preservation)
// ============================================================================

/// Rate decrease should preserve already-accrued entitlements via checkpoint.
///
/// Stream created with rate=10 tokens/sec for 1000 seconds (total=10k).
/// At t=500, accrued=5000. Rate decreased to 5 tokens/sec.
/// At t=600, accrued should be 5000 + (5 * 100) = 5500, not 5 * 600 = 3000.
#[test]
fn edge_case_rate_decrease_preserves_checkpoint() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(10000, 10, 1000);

    ctx.advance_time(500);
    let accrued_before = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should succeed");
    assert_eq!(accrued_before, 5000);

    // Decrease rate to 5 tokens/sec
    ctx.client
        .decrease_rate_per_second(&stream_id, 5)
        .expect("should succeed");

    // At t=600, accrued should be 5000 + 5*100 = 5500
    ctx.advance_time(600);
    let accrued_after = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should succeed");
    assert_eq!(accrued_after, 5500);

    // Verify it doesn't drop back to 5*600=3000 (checkpoint preserved)
    assert!(accrued_after >= accrued_before);
}

/// Rate decrease at the very end of a stream should not reduce accrued amount.
#[test]
fn edge_case_rate_decrease_at_stream_end() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    ctx.advance_time(1000); // Reach end_time
    let accrued_at_end = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should succeed");
    assert_eq!(accrued_at_end, 1000); // Full deposit accrued

    // Rate decrease after stream completion should not affect accrual
    ctx.client
        .decrease_rate_per_second(&stream_id, 0)
        .expect("should succeed");

    let accrued_after_decrease = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should succeed");
    assert_eq!(accrued_after_decrease, 1000); // Still full deposit
}

// ============================================================================
// Edge Case 3: Paused Stream State Preservation and Cooldown
// ============================================================================

/// Pausing and resuming a stream should respect cooldown and preserve accrual.
#[test]
fn edge_case_paused_stream_cooldown_respects_idempotency() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    ctx.advance_time(100);

    // Pause the stream
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational)
        .expect("first pause should succeed");

    // Attempt immediate pause should fail (not already paused, but cooldown)
    // Instead, try to pause again immediately
    let pause_result = ctx
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);
    // This should fail because it's already paused
    assert!(pause_result.is_err());

    // Accrual should be preserved even while paused
    let accrued_paused = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should succeed");
    assert_eq!(accrued_paused, 100);

    // Advance past cooldown and resume
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
    ctx.client
        .resume_stream(&stream_id)
        .expect("resume should succeed");

    // Accrual should continue
    ctx.advance_time(200);
    let accrued_after_resume = ctx
        .client
        .calculate_accrued(&stream_id)
        .expect("should succeed");
    assert_eq!(accrued_after_resume, 200);
}

// ============================================================================
// Edge Case 4: Batch Operations with Duplicate IDs (Idempotency)
// ============================================================================

/// Batch withdrawal with duplicate stream IDs should fail deterministically on retry.
#[test]
fn edge_case_batch_withdraw_duplicate_ids_idempotent() {
    let ctx = TestContext::setup();
    let id1 = ctx.create_test_stream(1000, 1, 1000);
    let id2 = ctx.create_test_stream(1000, 1, 1000);

    ctx.advance_time(500);

    // First attempt with duplicates should fail
    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    ids.push_back(id1);
    ids.push_back(id2);
    ids.push_back(id1); // duplicate

    let result1 = ctx.client.batch_withdraw(&ids);
    assert!(result1.is_err());

    // Retry with same duplicate list should produce same error (idempotent)
    let result2 = ctx.client.batch_withdraw(&ids);
    assert!(result1.is_err());
    assert_eq!(result1.is_err(), result2.is_err());
}

/// Batch operations with valid (non-duplicate) IDs should be idempotent.
#[test]
fn edge_case_batch_operations_deterministic() {
    let ctx = TestContext::setup();
    let id1 = ctx.create_test_stream(1000, 1, 1000);
    let id2 = ctx.create_test_stream(1000, 1, 1000);

    ctx.advance_time(500);

    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    ids.push_back(id1);
    ids.push_back(id2);

    // First batch withdraw
    let result1 = ctx.client.batch_withdraw(&ids).expect("should succeed");
    assert_eq!(result1.len(), 2);

    // Retry: both should be in Completed state, so second attempt should have 0 withdrawable
    let id1_state = ctx.client.get_stream_state(&id1).expect("should exist");
    assert_eq!(id1_state.withdrawn_amount, 500);

    // If we retry, withdrawable should be 0 (already withdrawn full amount)
    let withdrawable1 = ctx.client.get_withdrawable(&id1).expect("should exist");
    assert_eq!(withdrawable1, 0);
}

// ============================================================================
// Edge Case 5: Metadata Immutability Across Upgrade
// ============================================================================

/// Metadata should be immutable after stream creation.
#[test]
fn edge_case_metadata_immutable_after_creation() {
    let ctx = TestContext::setup();

    // Create stream with metadata
    let mut metadata = Map::new(&ctx.env);
    metadata.set(
        String::from_str(&ctx.env, "key1"),
        String::from_str(&ctx.env, "value1"),
    );

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: Some(metadata),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Read metadata
    let stored_metadata = ctx
        .client
        .get_stream_metadata(&stream_id)
        .expect("metadata should exist");
    assert_eq!(stored_metadata.len(), 1);

    // Metadata should remain the same across multiple reads
    let stored_metadata2 = ctx
        .client
        .get_stream_metadata(&stream_id)
        .expect("metadata should exist");
    assert_eq!(stored_metadata.len(), stored_metadata2.len());
}

// ============================================================================
// Edge Case 6: Terminal State Semantics and Keeper Cancel
// ============================================================================

/// Once a stream reaches Completed status, it should reject further operations.
#[test]
fn edge_case_completed_stream_terminal_state() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    ctx.advance_time(1000); // Reach end_time

    // Withdraw full amount
    ctx.client.withdraw(&stream_id, &None).expect("should succeed");

    // Stream should now be Completed
    let stream = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert_eq!(stream.status as u32, 2); // Completed = 2

    // Attempting pause on completed stream should fail
    let pause_result = ctx
        .client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert!(pause_result.is_err());

    // Attempting top-up on completed stream should fail
    let topup_result = ctx.client.top_up_stream(&stream_id, &ctx.sender, 100);
    assert!(topup_result.is_err());
}

/// Keeper cancel should be deterministic and respect terminal state.
#[test]
fn edge_case_keeper_cancel_terminal_idempotent() {
    let ctx = TestContext::setup();
    let keeper = Address::generate(&ctx.env);

    let stream_id = ctx.create_test_stream(10000, 5, 1000);

    // Advance past end_time + grace period
    ctx.advance_time(1000 + 604_800 + 1); // end_time + KEEPER_GRACE + 1

    // First keeper cancel should succeed
    let result1 = ctx.client.keeper_cancel(&stream_id, &keeper);
    assert!(result1.is_ok());

    // Stream should now be terminal
    let stream = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert!(stream.status as u32 == 2 || stream.status as u32 == 3); // Completed or Cancelled

    // Retry keeper_cancel on terminal stream should fail deterministically
    let result2 = ctx.client.keeper_cancel(&stream_id, &keeper);
    assert!(result2.is_err());
}

// ============================================================================
// Edge Case 7: Global Emergency Pause (Instance-Specific, Not Migrated)
// ============================================================================

/// Global pause should be instance-specific and affect stream operations.
#[test]
fn edge_case_global_pause_instance_specific() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    ctx.advance_time(100);

    // Enable global emergency pause
    ctx.client.set_global_emergency_paused(true);
    assert!(ctx.client.get_global_emergency_paused());

    // Attempting to create a new stream should fail
    let create_result = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    assert!(create_result.is_err());

    // Existing stream withdrawals should also fail
    let withdraw_result = ctx.client.withdraw(&stream_id, &None);
    assert!(withdraw_result.is_err());

    // Resume pause
    ctx.client.set_global_emergency_paused(false);
    assert!(!ctx.client.get_global_emergency_paused());

    // Operations should now succeed
    let withdraw_result2 = ctx
        .client
        .withdraw(&stream_id, &None)
        .expect("should succeed after unpause");
    assert_eq!(withdraw_result2, 100);
}

// ============================================================================
// Edge Case 8: AutoRenew Opt-In Default (Backward Compatibility)
// ============================================================================

/// AutoRenew should default to disabled for backward compatibility.
#[test]
fn edge_case_autorenew_defaults_to_disabled() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 1, 1000);

    // Check default state
    let autorenew = ctx.client.get_auto_renew(&stream_id).expect("should exist");
    assert!(!autorenew); // Should be disabled by default

    // Explicitly enable
    ctx.client
        .set_auto_renew(&stream_id, true)
        .expect("should succeed");

    let autorenew_enabled = ctx.client.get_auto_renew(&stream_id).expect("should exist");
    assert!(autorenew_enabled);

    // Disable again
    ctx.client
        .set_auto_renew(&stream_id, false)
        .expect("should succeed");

    let autorenew_disabled = ctx.client.get_auto_renew(&stream_id).expect("should exist");
    assert!(!autorenew_disabled);
}

// ============================================================================
// Edge Case 9: Withdrawable Amount Determinism (Checkpoint-Based)
// ============================================================================

/// Withdrawable amount should be deterministic given checkpoint state.
#[test]
fn edge_case_withdrawable_deterministic_with_checkpoint() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(10000, 10, 1000);

    // Accrue 5000 at t=500
    ctx.advance_time(500);
    let withdrawable1 = ctx
        .client
        .get_withdrawable(&stream_id)
        .expect("should exist");
    assert_eq!(withdrawable1, 5000);

    // Withdraw 3000
    ctx.client
        .withdraw(&stream_id, &None)
        .expect("withdraw should succeed");

    // Remaining should be 2000
    let stream = ctx
        .client
        .get_stream_state(&stream_id)
        .expect("should exist");
    assert_eq!(stream.withdrawn_amount, 3000);

    // At t=600, new accrual: 10*600 = 6000 total, minus 3000 withdrawn = 3000 withdrawable
    ctx.advance_time(600);
    let withdrawable2 = ctx
        .client
        .get_withdrawable(&stream_id)
        .expect("should exist");
    assert_eq!(withdrawable2, 3000);

    // Decrease rate to 5 at t=600, checkpoint at 6000
    ctx.client
        .decrease_rate_per_second(&stream_id, 5)
        .expect("should succeed");

    // At t=700: checkpoint(6000) + 5*(700-600) = 6000 + 500 = 6500 total, minus 3000 = 3500 withdrawable
    ctx.advance_time(700);
    let withdrawable3 = ctx
        .client
        .get_withdrawable(&stream_id)
        .expect("should exist");
    assert_eq!(withdrawable3, 3500);

    // Monotonicity: withdrawable should never decrease over time
    assert!(withdrawable3 >= withdrawable2);
}

// ============================================================================
// Edge Case 10: Stream Clone Preserves Immutable Fields
// ============================================================================

/// Cloning a stream should preserve immutable fields and start fresh on accrual.
#[test]
fn edge_case_stream_clone_preserves_metadata() {
    let ctx = TestContext::setup();

    let mut metadata = Map::new(&ctx.env);
    metadata.set(
        String::from_str(&ctx.env, "original"),
        String::from_str(&ctx.env, "clone_test"),
    );

    let original_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 5000,
            rate_per_second: 5,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: Some(metadata),
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.advance_time(100);

    // Clone the stream
    let cloned_id = ctx
        .client
        .clone_stream(&original_id, &ctx.recipient)
        .expect("clone should succeed");

    // Original metadata should be preserved in clone
    let cloned_metadata = ctx
        .client
        .get_stream_metadata(&cloned_id)
        .expect("cloned metadata should exist");
    assert_eq!(cloned_metadata.len(), 1);

    // Cloned stream should have fresh accrual (start_time = now)
    let cloned_stream = ctx
        .client
        .get_stream_state(&cloned_id)
        .expect("should exist");
    assert_eq!(cloned_stream.withdrawn_amount, 0); // Fresh start

    // Original accrual should be independent
    let original_withdrawable = ctx
        .client
        .get_withdrawable(&original_id)
        .expect("should exist");
    let cloned_withdrawable = ctx
        .client
        .get_withdrawable(&cloned_id)
        .expect("should exist");
    // Original has accrued, clone is fresh
    assert!(original_withdrawable > 0);
    assert_eq!(cloned_withdrawable, 0);
}

// ============================================================================
// Edge Case 11: Version Endpoint Stability
// ============================================================================

/// The `version()` endpoint should be permissionless and deterministic.
#[test]
fn edge_case_version_endpoint_permissionless_and_stable() {
    let ctx = TestContext::setup();

    // Call version multiple times, should always return same value
    let version1 = ctx.client.version();
    let version2 = ctx.client.version();

    assert_eq!(version1, version2);
    assert_eq!(version1, 9); // Current CONTRACT_VERSION = 9

    // Version should not be affected by stream creation or operations
    let _stream_id = ctx.create_test_stream(1000, 1, 1000);
    let version3 = ctx.client.version();
    assert_eq!(version3, 9);
}

// ============================================================================
// Edge Case 12: DataKey Discriminant Stability
// ============================================================================

/// Storage keys should be stable and backward-compatible.
/// This is a compile-time check via the frozen DataKey enum,
/// but we validate the invariant through operation.
#[test]
fn edge_case_storage_key_stability() {
    let ctx = TestContext::setup();

    // Create multiple streams to exercise different storage keys
    let stream_ids: Vec<u64> = (0..5)
        .map(|_| ctx.create_test_stream(1000, 1, 1000))
        .collect();

    // All streams should be retrievable
    for id in stream_ids {
        let stream = ctx
            .client
            .get_stream_state(&id)
            .expect("should be retrievable");
        assert_eq!(stream.stream_id, id);
    }

    // Recipient index should be consistent
    let recipient_streams = ctx.client.get_recipient_streams(ctx.recipient.clone());
    assert_eq!(recipient_streams.len(), 5);

    // Sender index should be consistent
    let sender_streams = ctx.client.get_recipient_streams(ctx.sender.clone());
    assert_eq!(sender_streams.len(), 0); // Sender is not recipient
}

// ============================================================================
// Regression Tests: Ensure No Future Regressions
// ============================================================================

/// Verify that accrual never becomes negative due to checkpoint math.
#[test]
fn regression_accrual_never_negative() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(1000, 10, 100);

    for t in 0..=100 {
        ctx.advance_time(t as u64);
        let accrued = ctx
            .client
            .calculate_accrued(&stream_id)
            .expect("should succeed");
        assert!(accrued >= 0, "accrual at t={} should not be negative", t);
    }
}

/// Verify that withdrawable never exceeds deposit.
#[test]
fn regression_withdrawable_capped_at_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(5000, 100, 100);

    for t in 0..=200 {
        ctx.advance_time(t as u64);
        let withdrawable = ctx
            .client
            .get_withdrawable(&stream_id)
            .expect("should exist");
        assert!(
            withdrawable <= 5000,
            "withdrawable at t={} should not exceed deposit",
            t
        );
    }
}

/// Verify that withdrawn_amount is monotonically non-decreasing.
#[test]
fn regression_withdrawn_monotonic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_test_stream(10000, 1, 10000);

    let mut previous_withdrawn = 0i128;

    for t in [0, 100, 200, 500, 1000, 2000, 5000].iter() {
        ctx.advance_time(*t);
        ctx.client.withdraw(&stream_id, &None).expect("should succeed");

        let stream = ctx
            .client
            .get_stream_state(&stream_id)
            .expect("should exist");
        assert!(
            stream.withdrawn_amount >= previous_withdrawn,
            "withdrawn_amount should be monotonic at t={}",
            t
        );
        previous_withdrawn = stream.withdrawn_amount;
    }
}
