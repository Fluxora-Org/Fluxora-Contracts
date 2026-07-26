#![cfg(test)]
extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token::{Client as TokenClient, StellarAssetClient},
    xdr::{AccountId, PublicKey, ScAddress, Uint256},
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
        let token_id = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        let token = TokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = recipient_public_key
            .as_ref()
            .map(|public_key| address_from_pk(&env, public_key))
            .unwrap_or_else(|| Address::generate(&env));
        client.init(&token_id, &admin);

        StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_000_000);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

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
            &self.recipient,
            &2_000,
            &1,
            &0,
            &0,
            &1_000,
            &dust_threshold,
            &None,
            &StreamKind::Linear,
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
    let stream_id = ctx.create_stream(0);
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
    assert_eq!(ctx.client.get_stream_state(&stream_id).last_withdraw_ledger, 0);

    ctx.advance_ledger(10); // 100 accrued, exactly at the threshold.
    assert_eq!(ctx.client.withdraw(&stream_id), 100);
}

#[test]
fn batch_withdraw_shares_the_per_stream_interval() {
    let ctx = TestContext::setup();
    let first = ctx.create_stream(0);
    let second = ctx.create_stream(0);
    ctx.advance_ledger(10);
    let streams = soroban_sdk::vec![&ctx.env, first, second];

    ctx.client.batch_withdraw(&ctx.recipient, &streams);
    assert_eq!(
        ctx.client.try_batch_withdraw(&ctx.recipient, &streams),
        Err(Ok(ContractError::WithdrawalTooFrequent))
    );

    ctx.advance_ledger(MIN_WITHDRAW_INTERVAL_LEDGERS);
    assert_eq!(ctx.client.batch_withdraw(&ctx.recipient, &streams).len(), 2);
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
    assert!(ctx.client.delegated_withdraw(
        &stream_id,
        &relayer,
        &public_key,
        &0,
        &deadline,
        &0,
        &first,
    ) > 0);

    let second = sign_message(
        &ctx.env,
        &signing_key,
        &delegated_message(&ctx.env, stream_id, 1, deadline, 0),
    );
    assert_eq!(
        ctx.client.try_delegated_withdraw(
            &stream_id, &relayer, &public_key, &1, &deadline, &0, &second,
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
