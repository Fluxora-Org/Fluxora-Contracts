//! Integration tests for `delegated_cancel` — sender-signed replay-protected cancellation.

extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    xdr::{AccountId, PublicKey, ScAddress, Uint256},
    Address, Bytes, BytesN, Env, TryIntoVal,
};

const DELEGATED_CANCEL_DOMAIN: &[u8; 24] = b"fluxora_delegated_cancel";

fn address_from_pk(env: &Env, pk: &[u8; 32]) -> Address {
    ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(*pk))))
        .try_into_val(env)
        .expect("valid ed25519 key -> address")
}

fn build_cancel_msg(env: &Env, stream_id: u64, nonce: u64, deadline: u64) -> Bytes {
    let mut msg = Bytes::new(env);
    msg.extend_from_array(DELEGATED_CANCEL_DOMAIN);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg
}

fn sign_cancel_msg(env: &Env, signing_key: &SigningKey, msg: &Bytes) -> BytesN<64> {
    let bytes: std::vec::Vec<u8> = (0..msg.len()).map(|i| msg.get_unchecked(i)).collect();
    BytesN::from_array(env, &signing_key.sign(&bytes).to_bytes())
}

struct DelegatedCancelCtx<'a> {
    env: Env,
    contract_id: Address,
    sender_sk: SigningKey,
    sender_pk: BytesN<32>,
    sender: Address,
    recipient: Address,
    relayer: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> DelegatedCancelCtx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(0);

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        let relayer = Address::generate(&env);

        let sender_sk = SigningKey::from_bytes(&[0x42u8; 32]);
        let pk_arr = sender_sk.verifying_key().to_bytes();
        let sender_pk = BytesN::from_array(&env, &pk_arr);
        let sender = address_from_pk(&env, &pk_arr);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);
        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        DelegatedCancelCtx {
            env,
            contract_id,
            sender_sk,
            sender_pk,
            sender,
            recipient,
            relayer,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn create_stream(&self) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: 1000u64,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    fn sign_cancel(&self, stream_id: u64, nonce: u64, deadline: u64) -> BytesN<64> {
        let msg = build_cancel_msg(&self.env, stream_id, nonce, deadline);
        sign_cancel_msg(&self.env, &self.sender_sk, &msg)
    }
}

#[test]
fn delegated_cancel_valid_signature_cancels_stream() {
    let ctx = DelegatedCancelCtx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(300);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);

    ctx.client().delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &sig,
    );

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.cancelled_at, Some(300));
}

#[test]
fn delegated_cancel_increments_nonce() {
    let ctx = DelegatedCancelCtx::setup();
    let stream_id1 = ctx.create_stream();
    let stream_id2 = ctx.create_stream();

    ctx.env.ledger().set_timestamp(300);

    // Cancel first stream with nonce 0
    let sig1 = ctx.sign_cancel(stream_id1, 0, 9999);
    ctx.client().delegated_cancel(
        &stream_id1,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &sig1,
    );

    // Attempting to cancel second stream with nonce 0 should fail
    let sig2_bad = ctx.sign_cancel(stream_id2, 0, 9999);
    let res = ctx.client().try_delegated_cancel(
        &stream_id2,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &sig2_bad,
    );
    assert_eq!(res, Err(Ok(ContractError::InvalidSignature)));

    // Canceling with nonce 1 should succeed
    let sig2_good = ctx.sign_cancel(stream_id2, 1, 9999);
    ctx.client().delegated_cancel(
        &stream_id2,
        &ctx.relayer,
        &ctx.sender_pk,
        &1,
        &9999,
        &sig2_good,
    );
    let stream = ctx.client().get_stream_state(&stream_id2);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

#[test]
fn delegated_cancel_expired_deadline_rejected() {
    let ctx = DelegatedCancelCtx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(5000);
    let sig = ctx.sign_cancel(stream_id, 0, 100);

    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &100,
        &sig,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::SignatureDeadlineExpired)),
        "expired deadline must return SignatureDeadlineExpired"
    );

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
}

#[test]
fn delegated_cancel_wrong_public_key_rejected() {
    let ctx = DelegatedCancelCtx::setup();
    let stream_id = ctx.create_stream();

    let other_sk = SigningKey::from_bytes(&[0x01u8; 32]);
    let other_pk = BytesN::from_array(&ctx.env, &other_sk.verifying_key().to_bytes());
    
    // Sign with correct key but pass wrong key
    let sig = ctx.sign_cancel(stream_id, 0, 9999);

    ctx.env.ledger().set_timestamp(100);
    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &other_pk,
        &0,
        &9999,
        &sig,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidSignature)),
        "wrong sender key must be rejected"
    );
}

#[test]
fn delegated_cancel_stream_not_found() {
    let ctx = DelegatedCancelCtx::setup();
    let sig = ctx.sign_cancel(999, 0, 9999);

    let result = ctx.client().try_delegated_cancel(
        &999,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &sig,
    );
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}
