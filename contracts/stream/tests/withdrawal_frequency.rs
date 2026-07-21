#![cfg(test)]
extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token::Client as TokenClient,
    Address, Env,
};

struct TestContext {
    env: Env,
    client: FluxoraStreamClient<'static>,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'static>,
}

impl TestContext {
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
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        // Mint tokens to sender
        token.mint(&sender, &1_000_000_000);

        Self {
            env,
            client,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn create_stream(&self) -> u64 {
        self.client
            .create_stream(
                &self.sender,
                &self.recipient,
                &1000,
                &1, // 1 token per second
                &0,
                &0,
                &1000,
                &0,
                &None,
            )
            .unwrap()
    }

    fn advance_ledger(&self, ledgers: u32) {
        let current = self.env.ledger().sequence();
        self.env.ledger().set(LedgerInfo {
            timestamp: self.env.ledger().timestamp() + (ledgers as u64 * 5),
            protocol_version: 20,
            sequence_number: current + ledgers,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 16,
            max_entry_ttl: 6312000,
        });
    }
}

#[test]
fn test_first_withdrawal_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First withdrawal should succeed
    let result = ctx.client.withdraw(&stream_id);
    assert!(result.is_ok());
    assert!(result.unwrap() > 0);
}

#[test]
fn test_second_withdrawal_same_ledger_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First withdrawal succeeds
    let result = ctx.client.withdraw(&stream_id);
    assert!(result.is_ok());

    // Second withdrawal at same ledger should fail
    let result = ctx.client.try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));
}

#[test]
fn test_withdrawal_before_interval_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First withdrawal succeeds
    ctx.client.withdraw(&stream_id).unwrap();

    // Advance by MIN_WITHDRAW_INTERVAL_LEDGERS - 1 (16 ledgers)
    ctx.advance_ledger(16);

    // Second withdrawal should fail (only 16 ledgers elapsed, need 17)
    let result = ctx.client.try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));
}

#[test]
fn test_withdrawal_at_exact_interval_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First withdrawal succeeds
    let first_amount = ctx.client.withdraw(&stream_id).unwrap();
    assert!(first_amount > 0);

    // Advance by exactly MIN_WITHDRAW_INTERVAL_LEDGERS (17 ledgers)
    ctx.advance_ledger(17);

    // Second withdrawal should succeed
    let result = ctx.client.withdraw(&stream_id);
    assert!(result.is_ok());
    let second_amount = result.unwrap();
    assert!(second_amount > 0);
}

#[test]
fn test_withdrawal_after_interval_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First withdrawal succeeds
    ctx.client.withdraw(&stream_id).unwrap();

    // Advance by more than MIN_WITHDRAW_INTERVAL_LEDGERS (20 ledgers)
    ctx.advance_ledger(20);

    // Second withdrawal should succeed
    let result = ctx.client.withdraw(&stream_id);
    assert!(result.is_ok());
    assert!(result.unwrap() > 0);
}

#[test]
fn test_third_withdrawal_resets_window() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First withdrawal at ledger 100
    ctx.client.withdraw(&stream_id).unwrap();

    // Advance by 17 ledgers to ledger 117
    ctx.advance_ledger(17);

    // Second withdrawal at ledger 117
    ctx.client.withdraw(&stream_id).unwrap();

    // Advance by 16 ledgers to ledger 133 (only 16 from second withdrawal)
    ctx.advance_ledger(16);

    // Third withdrawal should fail (only 16 ledgers since second withdrawal)
    let result = ctx.client.try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));

    // Advance by 1 more ledger to ledger 134 (17 from second withdrawal)
    ctx.advance_ledger(1);

    // Third withdrawal should now succeed
    let result = ctx.client.withdraw(&stream_id);
    assert!(result.is_ok());
}

#[test]
fn test_delegated_withdraw_enforces_rate_limit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // Create ed25519 keypair for recipient
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let public_key_bytes = soroban_sdk::Bytes::from_slice(&ctx.env, verifying_key.as_bytes());

    let relayer = Address::generate(&ctx.env);
    let nonce = ctx.client.get_delegated_nonce(&ctx.recipient);
    let deadline = ctx.env.ledger().timestamp() + 3600;
    let expected_minimum = 0i128;

    // Build message
    let mut msg_bytes = Vec::new();
    msg_bytes.extend_from_slice(&stream_id.to_be_bytes());
    msg_bytes.extend_from_slice(&nonce.to_be_bytes());
    msg_bytes.extend_from_slice(&deadline.to_be_bytes());
    msg_bytes.extend_from_slice(&expected_minimum.to_be_bytes());

    // Sign message
    use ed25519_dalek::Signer;
    let signature = signing_key.sign(&msg_bytes);
    let signature_bytes = soroban_sdk::Bytes::from_slice(&ctx.env, &signature.to_bytes());

    // First delegated withdrawal succeeds
    let result = ctx.client.delegated_withdraw(
        &stream_id,
        &relayer,
        &public_key_bytes,
        &nonce,
        &deadline,
        &expected_minimum,
        &signature_bytes,
    );
    assert!(result.is_ok());

    // Prepare second withdrawal (same ledger)
    let nonce2 = ctx.client.get_delegated_nonce(&ctx.recipient);
    let mut msg_bytes2 = Vec::new();
    msg_bytes2.extend_from_slice(&stream_id.to_be_bytes());
    msg_bytes2.extend_from_slice(&nonce2.to_be_bytes());
    msg_bytes2.extend_from_slice(&deadline.to_be_bytes());
    msg_bytes2.extend_from_slice(&expected_minimum.to_be_bytes());
    let signature2 = signing_key.sign(&msg_bytes2);
    let signature_bytes2 = soroban_sdk::Bytes::from_slice(&ctx.env, &signature2.to_bytes());

    // Second delegated withdrawal at same ledger should fail
    let result = ctx.client.try_delegated_withdraw(
        &stream_id,
        &relayer,
        &public_key_bytes,
        &nonce2,
        &deadline,
        &expected_minimum,
        &signature_bytes2,
    );
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));
}

#[test]
fn test_batch_withdraw_enforces_rate_limit_per_stream() {
    let ctx = TestContext::setup();

    // Create two streams
    let stream_id1 = ctx.create_stream();
    let stream_id2 = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First batch withdrawal succeeds for both streams
    let stream_ids = soroban_sdk::vec![&ctx.env, stream_id1, stream_id2];
    let result = ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
    assert!(result.is_ok());

    // Advance by 16 ledgers (not enough)
    ctx.advance_ledger(16);

    // Second batch withdrawal should fail (rate limit on first stream)
    let result = ctx.client.try_batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));

    // Advance by 1 more ledger (total 17)
    ctx.advance_ledger(1);

    // Second batch withdrawal should now succeed
    let result = ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
    assert!(result.is_ok());
}

#[test]
fn test_batch_withdraw_fails_if_any_stream_violates_rate_limit() {
    let ctx = TestContext::setup();

    // Create two streams
    let stream_id1 = ctx.create_stream();
    let stream_id2 = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // Withdraw from stream1 only
    ctx.client.withdraw(&stream_id1).unwrap();

    // Advance by 17 ledgers (stream1 can withdraw again, stream2 never withdrawn)
    ctx.advance_ledger(17);

    // Withdraw from stream1 again
    ctx.client.withdraw(&stream_id1).unwrap();

    // Now try batch withdraw with both streams
    // stream1 just withdrew (0 ledgers ago), stream2 never withdrew (should succeed)
    let stream_ids = soroban_sdk::vec![&ctx.env, stream_id1, stream_id2];
    let result = ctx.client.try_batch_withdraw(&ctx.recipient, &stream_ids);

    // Should fail because stream1 violates rate limit
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));
}

#[test]
fn test_initial_state_first_withdrawal_always_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance to a high ledger number
    ctx.advance_ledger(1000);

    // First withdrawal should succeed regardless of current ledger sequence
    // because last_withdraw_ledger is initialized to 0
    let result = ctx.client.withdraw(&stream_id);
    assert!(result.is_ok());
    assert!(result.unwrap() > 0);
}

#[test]
fn test_no_state_mutation_on_rate_limit_error() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First withdrawal succeeds
    let first_amount = ctx.client.withdraw(&stream_id).unwrap();

    // Get stream state after first withdrawal
    let stream_after_first = ctx.client.get_stream_state(&stream_id);
    let withdrawn_after_first = stream_after_first.withdrawn_amount;
    let balance_after_first = ctx.token.balance(&ctx.recipient);

    // Attempt second withdrawal at same ledger (should fail)
    let result = ctx.client.try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));

    // Verify no state mutation occurred
    let stream_after_failed = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream_after_failed.withdrawn_amount, withdrawn_after_first);

    let balance_after_failed = ctx.token.balance(&ctx.recipient);
    assert_eq!(balance_after_failed, balance_after_first);
}

#[test]
fn test_underflow_safety_invariant() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time and perform multiple withdrawals
    for _ in 0..5 {
        ctx.advance_ledger(20);
        ctx.client.withdraw(&stream_id).unwrap();

        // After each withdrawal, verify invariant: last_withdraw_ledger <= current_ledger
        let stream = ctx.client.get_stream_state(&stream_id);
        let current_ledger = ctx.env.ledger().sequence();

        // This assertion verifies the invariant holds
        // If last_withdraw_ledger > current_ledger, the subtraction would underflow
        assert!(stream.last_withdraw_ledger <= current_ledger);
    }
}

#[test]
fn test_zero_withdrawable_does_not_update_last_withdraw_ledger() {
    let ctx = TestContext::setup();

    // Create stream with cliff time in the future
    let stream_id = ctx
        .client
        .create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000,
            &1,
            &0,
            &500, // cliff at 500 seconds
            &1000,
            &0,
            &None,
        )
        .unwrap();

    // Advance ledgers but not past cliff
    ctx.advance_ledger(50);

    // Attempt withdrawal before cliff (returns 0, no state change)
    let result = ctx.client.withdraw(&stream_id).unwrap();
    assert_eq!(result, 0);

    // Verify last_withdraw_ledger is still 0 (not updated)
    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.last_withdraw_ledger, 0);

    // Advance past cliff
    ctx.advance_ledger(100);

    // Now withdrawal should succeed and update last_withdraw_ledger
    let result = ctx.client.withdraw(&stream_id).unwrap();
    assert!(result > 0);

    let stream = ctx.client.get_stream_state(&stream_id);
    assert!(stream.last_withdraw_ledger > 0);
}

#[test]
fn test_rate_limit_applies_across_different_withdrawal_methods() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // First withdrawal via regular withdraw
    ctx.client.withdraw(&stream_id).unwrap();

    // Attempt batch_withdraw at same ledger (should fail)
    let stream_ids = soroban_sdk::vec![&ctx.env, stream_id];
    let result = ctx.client.try_batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));

    // Advance by 17 ledgers
    ctx.advance_ledger(17);

    // Now batch_withdraw should succeed
    let result = ctx.client.batch_withdraw(&ctx.recipient, &stream_ids);
    assert!(result.is_ok());
}

#[test]
fn test_multiple_streams_independent_rate_limits() {
    let ctx = TestContext::setup();

    // Create two streams
    let stream_id1 = ctx.create_stream();
    let stream_id2 = ctx.create_stream();

    // Advance time to accrue tokens
    ctx.advance_ledger(100);

    // Withdraw from stream1
    ctx.client.withdraw(&stream_id1).unwrap();

    // Advance by 10 ledgers
    ctx.advance_ledger(10);

    // Withdraw from stream2 (should succeed, independent rate limit)
    let result = ctx.client.withdraw(&stream_id2);
    assert!(result.is_ok());

    // Attempt to withdraw from stream1 again (should fail, only 10 ledgers elapsed)
    let result = ctx.client.try_withdraw(&stream_id1);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)));

    // Advance by 7 more ledgers (total 17 from stream1's last withdrawal)
    ctx.advance_ledger(7);

    // Now stream1 withdrawal should succeed
    let result = ctx.client.withdraw(&stream_id1);
    assert!(result.is_ok());
}

// ─── Backward timestamp skew test (issue #940) ─────────────────────────────

/// After a withdrawal records `last_withdraw_ledger`, set the ledger to a
/// timestamp with a sequence number earlier than the recorded value.
/// This simulates a validator clock anomaly.
///
/// Without `saturating_sub`, `current_ledger - last_withdraw_ledger` would
/// underflow to a huge u32 value, bypassing the frequency limiter.
/// With `saturating_sub`, the elapsed time is clamped to 0, and the
/// withdrawal is correctly rejected.
#[test]
fn test_backward_timestamp_skew_cannot_bypass_rate_limit() {
    use soroban_sdk::testutils::{Ledger, LedgerInfo};

    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_id = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    client.init(&token_id, &admin);

    // Create a stream: deposit=1000, rate=1/s, no cliff, duration=1000
    let stream_id = client
        .create_stream(&sender, &recipient, &1000, &1, &0, &0, &1000, &0, &None)
        .unwrap();

    // Advance to ledger 100 to accrue tokens
    let ledger_100 = LedgerInfo {
        timestamp: 500,
        protocol_version: 20,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 16,
        max_entry_ttl: 6312000,
    };
    env.ledger().set(ledger_100);

    // First withdrawal — succeeds, records last_withdraw_ledger = 100
    let result = client.withdraw(&stream_id).unwrap();
    assert!(result > 0, "first withdrawal should accrue tokens");

    // ── ATTACK: set ledger backward ──
    // Simulate a validator clock anomaly: sequence number jumps BACK
    // from 100 to 50 (earlier than the recorded last_withdraw_ledger).
    let ledger_backward = LedgerInfo {
        timestamp: 250, // earlier timestamp too
        protocol_version: 20,
        sequence_number: 50, // < last_withdraw_ledger (100)
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 16,
        max_entry_ttl: 6312000,
    };
    env.ledger().set(ledger_backward);

    // With `saturating_sub`: elapsed = 50 - 100 = 0 (clamped)
    // 0 < MIN_WITHDRAW_INTERVAL_LEDGERS → withdrawal REJECTED
    // Without the fix: 50 - 100 would underflow to 4294967246,
    // which is >> MIN_WITHDRAW_INTERVAL_LEDGERS → withdrawal ALLOWED (vuln)
    let result = client.try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::WithdrawalTooFrequent)),
        "backward timestamp skew must NOT bypass rate limit");
}
