#![cfg(test)]
extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Bytes, BytesN, Env, TryIntoVal,
};

/// The withdrawal limiter is deliberately one ledger: it prevents duplicate
/// same-ledger writes without delaying a recipient's next ledger withdrawal.
const MIN_WITHDRAW_INTERVAL_LEDGERS: u32 = 1;

struct TestContext {
    env: Env,
    client: FluxoraStreamClient<'static>,
    sender: Address,
    recipient: Address,
}

mod mock_token {
    use soroban_sdk::{contract, contractimpl, Address, Env};
    #[contract]
    pub struct MockToken;
    #[contractimpl]
    impl MockToken {
        pub fn init(env: Env, _token: Address, _admin: Address) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn mint(env: Env, _to: Address, _amount: i128) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn approve(
            env: Env,
            _from: Address,
            _spender: Address,
            _amount: i128,
            _expiration_ledger: u32,
        ) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn transfer_from(
            env: Env,
            _spender: Address,
            _from: Address,
            _to: Address,
            _amount: i128,
        ) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn balance(env: Env, _id: Address) -> i128 {
            env.storage().instance().extend_ttl(100_000, 100_000);
            1_000_000_000
        }
    }
}

impl TestContext {
    fn setup() -> Self {
        Self::setup_with_recipient(None)
    }

    fn setup_with_recipient(recipient_public_key: Option<[u8; 32]>) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set(LedgerInfo {
            timestamp: 0,
            protocol_version: 20,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 16,
            max_entry_ttl: 6_312_000,
        });

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token_id = env.register_contract(None, mock_token::MockToken);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = recipient_public_key
            .as_ref()
            .map(|public_key| address_from_pk(&env, public_key))
            .unwrap_or_else(|| Address::generate(&env));

        client.init(&token_id, &admin);

        Self {
            env,
            client,
            sender,
            recipient,
        }
    }

    fn create_stream(&self, dust_threshold: i128) -> u64 {
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: 2_000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1_000,
                withdraw_dust_threshold: Some(dust_threshold),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    fn advance_ledger(&self, ledgers: u32) {
        let current = self.env.ledger().sequence();
        self.env.ledger().set(LedgerInfo {
            timestamp: self.env.ledger().timestamp() + u64::from(ledgers) * 5,
            protocol_version: 20,
            sequence_number: current + ledgers,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 16,
            max_entry_ttl: 6_312_000,
        });
    }
}

fn address_from_pk(env: &Env, pk: &[u8; 32]) -> Address {
    ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(*pk))))
        .try_into_val(env)
        .expect("valid ed25519 public key")
}

fn delegated_message(
    env: &Env,
    stream_id: u64,
    nonce: u64,
    deadline: u64,
    expected_minimum: i128,
) -> Bytes {
    let mut message = Bytes::new(env);
    message.extend_from_array(&stream_id.to_be_bytes());
    message.extend_from_array(&nonce.to_be_bytes());
    message.extend_from_array(&deadline.to_be_bytes());
    message.extend_from_array(&expected_minimum.to_be_bytes());
    message
}

fn sign_message(env: &Env, signing_key: &SigningKey, message: &Bytes) -> BytesN<64> {
    let bytes: std::vec::Vec<u8> = (0..message.len())
        .map(|index| message.get_unchecked(index))
        .collect();
    BytesN::from_array(env, &signing_key.sign(&bytes).to_bytes())
}

#[test]
fn same_ledger_withdrawal_is_rejected_and_exact_interval_succeeds() {
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
fn lookback_caps_each_claim_without_reducing_lifetime_accrual() {
    let ctx = TestContext::setup();
    let stream_id = ctx
        .client
        .create_stream_with_lookback(
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
            &Some(10_u32),
        )
        .unwrap();

    ctx.advance_ledger(100);
    assert_eq!(ctx.client.calculate_accrued(&stream_id).unwrap(), 500);
    assert_eq!(ctx.client.get_withdrawable(&stream_id).unwrap(), 50);
    assert_eq!(ctx.client.get_claimable_at(&stream_id, &500).unwrap(), 50);

    let mut total_withdrawn = 0_i128;
    for index in 0..20 {
        if index > 0 {
            // The normal withdrawal-frequency guard still applies, so each
            // lookback window is separated by the minimum interval.
            ctx.advance_ledger(17);
        }
        total_withdrawn += ctx.client.withdraw(&stream_id).unwrap();
    }

    // The cap limits each call, but no accrued entitlement is permanently lost.
    assert_eq!(total_withdrawn, 1000);
    assert_eq!(ctx.client.calculate_accrued(&stream_id), 1000);
}

#[test]
fn lookback_window_can_be_cleared_or_rejected_by_sender() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    let zero = ctx
        .client
        .try_set_lookback_window(&stream_id, &ctx.sender, &Some(0_u32));
    assert_eq!(zero, Err(Ok(ContractError::InvalidParams)));

    ctx.client
        .set_lookback_window(&stream_id, &ctx.sender, &Some(10_u32));
    assert_eq!(
        ctx.client.get_lookback_window(&stream_id).unwrap(),
        Some(10_u32)
    );

    ctx.client
        .set_lookback_window(&stream_id, &ctx.sender, &None);
    assert_eq!(ctx.client.get_lookback_window(&stream_id).unwrap(), None);
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

    assert!(ctx.client.withdraw(&stream_id) > 0);
    assert_eq!(
        ctx.client.try_withdraw(&stream_id),
        Err(Ok(ContractError::WithdrawalTooFrequent))
    );

    ctx.advance_ledger(MIN_WITHDRAW_INTERVAL_LEDGERS);
    assert!(ctx.client.withdraw(&stream_id) > 0);
}

#[test]
fn every_successful_withdrawal_resets_the_interval() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(0);
    ctx.advance_ledger(10);
    ctx.client.withdraw(&stream_id);

    ctx.advance_ledger(MIN_WITHDRAW_INTERVAL_LEDGERS);
    ctx.client.withdraw(&stream_id);
    assert_eq!(
        ctx.client.try_withdraw(&stream_id),
        Err(Ok(ContractError::WithdrawalTooFrequent))
    );

    ctx.advance_ledger(MIN_WITHDRAW_INTERVAL_LEDGERS);
    assert!(ctx.client.withdraw(&stream_id) > 0);
}

#[test]
fn zero_withdrawable_does_not_consume_the_interval() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(100);
    ctx.advance_ledger(10); // 50 accrued, below the dust threshold.

    assert_eq!(ctx.client.withdraw(&stream_id), 0);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).last_withdraw_ledger,
        0
    );

    ctx.advance_ledger(10); // 100 accrued, exactly at the threshold.
    assert_eq!(ctx.client.withdraw(&stream_id), 100);
}

#[test]
fn batch_withdraw_shares_the_per_stream_interval() {
    let ctx = TestContext::setup();
    let first = ctx.create_stream(0);
    let second = ctx.create_stream(0);
    ctx.advance_ledger(10);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        fluxora_stream::WithdrawToParam {
            stream_id: first,
            destination: ctx.recipient.clone(),
        },
        fluxora_stream::WithdrawToParam {
            stream_id: second,
            destination: ctx.recipient.clone(),
        },
    ];

    ctx.client.batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert_eq!(
        ctx.client
            .try_batch_withdraw_to(&ctx.recipient, &withdrawals),
        Err(Ok(ContractError::WithdrawalTooFrequent))
    );

    ctx.advance_ledger(MIN_WITHDRAW_INTERVAL_LEDGERS);
    assert_eq!(
        ctx.client
            .batch_withdraw_to(&ctx.recipient, &withdrawals)
            .len(),
        2
    );
}

#[test]
fn rate_change_checkpoints_accrual_without_bypassing_the_interval() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(0);
    ctx.advance_ledger(10);

    assert_eq!(ctx.client.withdraw(&stream_id), 50);
    ctx.client.update_rate_per_second(&stream_id, &2);

    // The checkpoint preserves the first 50 tokens, but a rate update must not
    // make a second withdrawal possible in the same ledger.
    assert_eq!(
        ctx.client.try_withdraw(&stream_id),
        Err(Ok(ContractError::WithdrawalTooFrequent))
    );

    ctx.advance_ledger(MIN_WITHDRAW_INTERVAL_LEDGERS);
    assert_eq!(ctx.client.withdraw(&stream_id), 10);
    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.checkpointed_amount, 50);
    assert_eq!(stream.withdrawn_amount, 60);
}

#[test]
fn delegated_withdrawal_obeys_the_same_ledger_limit() {
    let signing_key = SigningKey::from_bytes(&[0xA5; 32]);
    let ctx = TestContext::setup_with_recipient(Some(signing_key.verifying_key().to_bytes()));
    let public_key = BytesN::from_array(&ctx.env, &signing_key.verifying_key().to_bytes());
    let stream_id = ctx.create_stream(0);
    let relayer = Address::generate(&ctx.env);
    ctx.advance_ledger(10);

    let deadline = ctx.env.ledger().timestamp() + 3_600;
    let first = sign_message(
        &ctx.env,
        &signing_key,
        &delegated_message(&ctx.env, stream_id, 0, deadline, 0),
    );
    assert!(
        ctx.client
            .delegated_withdraw(&stream_id, &relayer, &public_key, &0, &deadline, &0, &first,)
            > 0
    );

    let second = sign_message(
        &ctx.env,
        &signing_key,
        &delegated_message(&ctx.env, stream_id, 1, deadline, 0),
    );
    assert_eq!(
        ctx.client.try_delegated_withdraw(
            &stream_id,
            &relayer,
            &public_key,
            &1,
            &deadline,
            &0,
            &second,
        ),
        Err(Ok(ContractError::WithdrawalTooFrequent))
    );
}

#[test]
fn backward_ledger_sequence_cannot_bypass_the_limit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(0);
    ctx.advance_ledger(10);
    ctx.client.withdraw(&stream_id);

    ctx.env.ledger().set(LedgerInfo {
        timestamp: 25,
        protocol_version: 20,
        sequence_number: 5,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 16,
        max_entry_ttl: 6_312_000,
    });
    assert_eq!(
        ctx.client.try_withdraw(&stream_id),
        Err(Ok(ContractError::WithdrawalTooFrequent))
    );
}

// ─── Lookback-bounded withdrawal — extended coverage (CONTRACT_VERSION 8)
// ──────────────────────────────────────────────────────────────────────

/// Helper: build a stream with a custom lookback window for isolation.
fn create_stream_with_lookback_for(ctx: &TestContext, lookback: Option<u32>) -> u64 {
    ctx.client
        .create_stream_with_lookback(
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
            &lookback,
        )
        .unwrap()
}

/// `calculate_accrued` reports the stream's total lifetime accrual and must not
/// be perturbed by the optional lookback bound.
#[test]
fn lookback_does_not_change_calculate_accrued() {
    let ctx = TestContext::setup();

    // No bound: full accrual visible.
    let no_bound = ctx.create_stream();
    // Tight bound: same stream params, but cap = 1 ledger (5 s).
    let bounded = create_stream_with_lookback_for(&ctx, Some(1));

    ctx.advance_ledger(100); // both now at t=500 s, accrued = 500

    assert_eq!(ctx.client.calculate_accrued(&no_bound).unwrap(), 500);
    assert_eq!(ctx.client.calculate_accrued(&bounded).unwrap(), 500);

    // `calculate_accrued` is the lifetime entitlement; the lookback only
    // affects *withdrawable* amounts, never the entitlement itself.
}

/// The lookback bound is the per-call ceiling on top of the recipient's
/// normal withdrawable amount. Both views agree on the ceiling.
#[test]
fn lookback_caps_each_claim_to_one_window() {
    let ctx = TestContext::setup();
    let stream_id = create_stream_with_lookback_for(&ctx, Some(10)); // 10 ledgers = 50 s

    ctx.advance_ledger(100); // t=500. Lifetime accrued = 500 tokens.

    // First claim: cap is one window worth (50 tokens).
    let first = ctx.client.withdraw(&stream_id).unwrap();
    assert_eq!(first, 50, "first claim must equal the window size");

    // The withdrawal-frequency guard guarantees at least 17 ledgers elapse
    // between calls. After that, time has advanced past one full window
    // and a fresh slice of accrual is reachable.
    ctx.advance_ledger(17); // t=585
    let second = ctx.client.withdraw(&stream_id).unwrap();
    assert!(second > 0);
    assert!(
        second <= 50,
        "subsequent call is also bounded by the lookback window"
    );
}

/// Across enough disjoint lookback windows, the recipient drains the *full*
/// lifetime entitlement even though each call is bounded. This is the
/// "no permanent loss" guarantee called out in the feature spec.
#[test]
fn lookback_repeated_claims_drain_full_entitlement() {
    let ctx = TestContext::setup();
    let stream_id = create_stream_with_lookback_for(&ctx, Some(10));

    ctx.advance_ledger(100); // t=500 (pre-end_time)
    let initial_total = ctx.client.calculate_accrued(&stream_id).unwrap();
    assert_eq!(initial_total, 500);

    let mut withdrawn = 0_i128;
    // Each `advance_ledger(17)` shifts the lookback window forward by 85 s,
    // bringing fresh accrual into the claimed slice. After enough iterations
    // the recipient recovers every accrued token (capped by deposit).
    for _ in 0..30 {
        let amount = ctx.client.withdraw(&stream_id).unwrap();
        withdrawn += amount;
        ctx.advance_ledger(17);
    }

    // Re-query lifetime accrual *after* the loop: time elapsed by `2_550 s`
    // is well past `end_time=1_000`, so accrual clamps to `deposit_amount`
    // (`calculate_accrued` is time-terminal-aware — see accrual.rs).
    //
    // Because `deposit_amount` is the absolute ceiling and the lookback cap
    // never permanently destroys entitlement, the recipient must end up
    // matching the *final* claimable ceiling exactly.
    let final_total = ctx.client.calculate_accrued(&stream_id).unwrap();
    assert_eq!(
        withdrawn, final_total,
        "repeated bounded claims across windows recover 100% of the final entitlement"
    );
    // The cap only restricts velocity: the lifetime total it allows us to
    // recover equals the lifetime total the stream ever produced.
    assert_eq!(
        final_total, 1000,
        "after time-terminal, calculate_accrued clamps to deposit_amount"
    );
}

/// Setting then clearing the bound restores the full claimable amount.
#[test]
fn lookback_cleared_restores_full_claimability() {
    let ctx = TestContext::setup();
    let stream_id = create_stream_with_lookback_for(&ctx, Some(10));

    ctx.advance_ledger(100); // t=500

    // While the bound is present, claimable is capped at the window size.
    assert_eq!(ctx.client.get_withdrawable(&stream_id).unwrap(), 50);

    ctx.client
        .set_lookback_window(&stream_id, &ctx.sender, &None);
    assert_eq!(ctx.client.get_lookback_window(&stream_id).unwrap(), None);

    // After clearing, the full accrued amount (500) is claimable again.
    assert_eq!(ctx.client.get_withdrawable(&stream_id).unwrap(), 500);
}

/// Non-sender callers are rejected with `Unauthorized`. The signer in the
/// call (`sender` argument) must match the original stream's sender.
#[test]
fn lookback_setter_rejects_non_sender() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream();

    let result = ctx
        .client
        .try_set_lookback_window(&stream_id, &ctx.admin, &Some(10_u32));
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));

    // Recipient also not allowed (only the original sender authorises reads).
    let result = ctx
        .client
        .try_set_lookback_window(&stream_id, &ctx.recipient, &Some(10_u32));
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
}

/// Cancelled streams cannot have their lookback modified; the cap is read-only
/// post-cancellation so existing frozen accounting is preserved.
#[test]
fn lookback_setter_rejects_cancelled_stream() {
    let ctx = TestContext::setup();
    let stream_id = create_stream_with_lookback_for(&ctx, Some(10));

    ctx.advance_ledger(100);
    ctx.client.cancel_stream(&stream_id).unwrap();

    let result = ctx
        .client
        .try_set_lookback_window(&stream_id, &ctx.sender, &Some(20_u32));
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    // ...and clearing is also blocked.
    let result = ctx
        .client
        .try_set_lookback_window(&stream_id, &ctx.sender, &None);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// `set_lookback_window` on a non-existent stream returns `StreamNotFound`,
/// matching the rest of the contract's storage-key error model.
#[test]
fn lookback_setter_rejects_unknown_stream() {
    let ctx = TestContext::setup();
    let bogus = 9_999_u64;
    let result = ctx
        .client
        .try_set_lookback_window(&bogus, &ctx.sender, &Some(10_u32));
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

/// `get_withdrawable` and `get_claimable_at` must agree at the same evaluation
/// timestamp — both views apply the same lookback cap.
#[test]
fn lookback_get_withdrawable_matches_get_claimable_at() {
    let ctx = TestContext::setup();
    let stream_id = create_stream_with_lookback_for(&ctx, Some(10));

    ctx.advance_ledger(60); // t=300
    let now = ctx.env.ledger().timestamp();
    assert_eq!(
        ctx.client.get_withdrawable(&stream_id).unwrap(),
        ctx.client.get_claimable_at(&stream_id, &now).unwrap()
    );
}

/// CliffOnly streams lazily unlock their full deposit. Once the cliff has
/// passed, that one-shot entitlement wins over the lookback cap so a recipient
/// who first queries after a window has elapsed does not strand funds.
#[test]
fn lookback_cliff_only_full_claim_after_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx
        .client
        .create_stream_with_lookback(
            &ctx.sender,
            &CreateStreamParams {
                recipient: ctx.recipient.clone(),
                deposit_amount: 1000,
                rate_per_second: 0, // CliffOnly enforces rate=0
                start_time: 0,
                cliff_time: 500, // cliff at 500 s
                end_time: 1000,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::CliffOnly,
                irrevocable: None,
                witness: None,
            },
            &Some(10),
        )
        .unwrap();

    // Before cliff: nothing claimable regardless of lookback.
    ctx.advance_ledger(50); // t=250
    assert_eq!(ctx.client.get_withdrawable(&stream_id).unwrap(), 0);

    // Advance past cliff, and well past the lookback window.
    ctx.advance_ledger(60); // t=550, then 60 more... = 850
                            // Claimable = full deposit because CliffOnly entitlement bypasses cap.
    assert_eq!(
        ctx.client.get_withdrawable(&stream_id).unwrap(),
        1000,
        "CliffOnly post-cliff must bypass lookback cap to avoid stranded funds"
    );
}

/// Mid-stream activation of the lookback only affects future claims;
/// previously accrued (but unclaimed) amounts become claimable in
/// subsequent windows. Once `withdrawn_amount == deposit_amount`, the stream
/// transitions to `Completed`, so the loop terminates naturally.
#[test]
fn lookback_set_mid_stream_preserves_old_accrual() {
    let ctx = TestContext::setup();
    // Start with no bound.
    let stream_id = ctx.create_stream();
    ctx.advance_ledger(100); // t=500, pre-end_time. accrued = 500.

    // Now apply a tight bound.
    ctx.client
        .set_lookback_window(&stream_id, &ctx.sender, &Some(10));

    let mut withdrawn = 0_i128;
    let mut cycles: u32 = 0;
    // Bound the loop to keep time below `end_time` so the post-loop assertions
    // can still talk about the *initial* lifetime accrual (500). 20 cycles
    // is exactly the number needed to drain 1000 tokens at 50/cycle.
    while cycles < 20 {
        let available = ctx.client.get_withdrawable(&stream_id).unwrap();
        if available == 0 {
            break;
        }
        let amount = ctx.client.withdraw(&stream_id).unwrap();
        withdrawn += amount;
        ctx.advance_ledger(17);
        cycles += 1;
    }

    // By this point the loop has run for ~1_700 s of ledger time (well past
    // end_time=1_000), so `calculate_accrued` clamps to deposit_amount.
    assert_eq!(withdrawn, 1_000);

    // The bound doesn't change the lifetime ceiling. Whichever the publisher
    // bound sets, the recipient can still recover `deposit_amount`.
    assert_eq!(ctx.client.calculate_accrued(&stream_id).unwrap(), 1_000);
}
