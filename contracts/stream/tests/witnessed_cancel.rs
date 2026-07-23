//! Integration tests for `witnessed_cancel_stream` — compliance attestation cancellation.

extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamEvent,
    StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    xdr::{AccountId, PublicKey, ScAddress, Uint256},
    Address, Bytes, BytesN, Env, TryFromVal, TryIntoVal,
};

const WITNESSED_CANCEL_DOMAIN: &[u8] = b"fluxora_witnessed_cancel";

fn address_from_pk(env: &Env, pk: &[u8; 32]) -> Address {
    ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(*pk))))
        .try_into_val(env)
        .expect("valid ed25519 key -> address")
}

fn build_witness_msg(env: &Env, stream_id: u64, deadline: u64) -> Bytes {
    let mut msg = Bytes::new(env);
    msg.extend_from_array(WITNESSED_CANCEL_DOMAIN);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg
}

fn sign_witness_msg(env: &Env, signing_key: &SigningKey, msg: &Bytes) -> BytesN<64> {
    let bytes: std::vec::Vec<u8> = (0..msg.len()).map(|i| msg.get_unchecked(i)).collect();
    BytesN::from_array(env, &signing_key.sign(&bytes).to_bytes())
}

struct WitnessCtx<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    witness_sk: SigningKey,
    witness_pk: BytesN<32>,
    witness_addr: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> WitnessCtx<'a> {
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
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let witness_sk = SigningKey::from_bytes(&[0xCDu8; 32]);
        let pk_arr = witness_sk.verifying_key().to_bytes();
        let witness_pk = BytesN::from_array(&env, &pk_arr);
        let witness_addr = address_from_pk(&env, &pk_arr);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);
        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        WitnessCtx {
            env,
            contract_id,
            sender,
            recipient,
            witness_sk,
            witness_pk,
            witness_addr,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn create_stream_with_witness(&self) -> u64 {
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
            &Some(self.witness_addr.clone()),
        )
    }

    fn create_stream_without_witness(&self) -> u64 {
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
            &None,
        )
    }

    fn sign_cancel(&self, stream_id: u64, deadline: u64) -> BytesN<64> {
        let msg = build_witness_msg(&self.env, stream_id, deadline);
        sign_witness_msg(&self.env, &self.witness_sk, &msg)
    }
}

#[test]
fn witnessed_cancel_valid_signature_cancels_stream() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_with_witness();

    ctx.env.ledger().set_timestamp(300);
    let sig = ctx.sign_cancel(stream_id, 9999);

    ctx.client()
        .witnessed_cancel_stream(&stream_id, &ctx.witness_pk, &9999, &sig);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.cancelled_at, Some(300));
}

#[test]
fn witnessed_cancel_refund_matches_sender_cancel() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_with_witness();

    ctx.env.ledger().set_timestamp(400);
    let sig = ctx.sign_cancel(stream_id, 9999);
    ctx.client()
        .witnessed_cancel_stream(&stream_id, &ctx.witness_pk, &9999, &sig);

    let sender_balance = ctx.token.balance(&ctx.sender);
    assert_eq!(sender_balance, 9600, "sender should receive 600 token refund at t=400");
}

#[test]
fn witnessed_cancel_emits_stream_cancelled_event() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_with_witness();

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 9999);
    ctx.client()
        .witnessed_cancel_stream(&stream_id, &ctx.witness_pk, &9999, &sig);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::StreamCancelled(stream_id)
    );
}

#[test]
fn witnessed_cancel_expired_deadline_rejected() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_with_witness();

    ctx.env.ledger().set_timestamp(5000);
    let sig = ctx.sign_cancel(stream_id, 100);

    let result = ctx.client().try_witnessed_cancel_stream(
        &stream_id,
        &ctx.witness_pk,
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
fn witnessed_cancel_no_witness_configured_rejected() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_without_witness();

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 9999);

    let result = ctx.client().try_witnessed_cancel_stream(
        &stream_id,
        &ctx.witness_pk,
        &9999,
        &sig,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "stream without witness must reject witnessed cancel"
    );
}

#[test]
fn witnessed_cancel_wrong_public_key_rejected() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_with_witness();

    let other_sk = SigningKey::from_bytes(&[0x01u8; 32]);
    let other_pk = BytesN::from_array(&ctx.env, &other_sk.verifying_key().to_bytes());
    let msg = build_witness_msg(&ctx.env, stream_id, 9999);
    let other_sig = sign_witness_msg(&ctx.env, &other_sk, &msg);

    ctx.env.ledger().set_timestamp(100);
    let result = ctx.client().try_witnessed_cancel_stream(
        &stream_id,
        &other_pk,
        &9999,
        &other_sig,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidSignature)),
        "wrong witness key must be rejected"
    );
}

#[test]
fn witnessed_cancel_delegated_withdraw_signature_not_replayable() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_with_witness();

    // Build a delegated-withdraw style message (no domain tag) — must not cancel.
    let mut delegated_msg = Bytes::new(&ctx.env);
    delegated_msg.extend_from_array(&stream_id.to_be_bytes());
    delegated_msg.extend_from_array(&0u64.to_be_bytes());
    delegated_msg.extend_from_array(&9999u64.to_be_bytes());
    delegated_msg.extend_from_array(&0i128.to_be_bytes());
    let delegated_sig = sign_witness_msg(&ctx.env, &ctx.witness_sk, &delegated_msg);

    ctx.env.ledger().set_timestamp(100);
    // Wrong key binding or bad signature — host trap or InvalidSignature.
    // With correct key but wrong payload the ed25519 verify traps in try_* path.
    let result = ctx.client().try_witnessed_cancel_stream(
        &stream_id,
        &ctx.witness_pk,
        &9999,
        &delegated_sig,
    );
    assert!(
        result.is_err(),
        "delegated-withdraw payload must not authorize witnessed cancel"
    );

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
}

#[test]
fn witnessed_cancel_from_paused_stream_succeeds() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_with_witness();

    ctx.client().pause_stream(&stream_id);
    ctx.env.ledger().set_timestamp(200);
    let sig = ctx.sign_cancel(stream_id, 9999);

    ctx.client()
        .witnessed_cancel_stream(&stream_id, &ctx.witness_pk, &9999, &sig);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

#[test]
fn witnessed_cancel_already_cancelled_rejected() {
    let ctx = WitnessCtx::setup();
    let stream_id = ctx.create_stream_with_witness();

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 9999);
    ctx.client()
        .witnessed_cancel_stream(&stream_id, &ctx.witness_pk, &9999, &sig);

    let replay = ctx.client().try_witnessed_cancel_stream(
        &stream_id,
        &ctx.witness_pk,
        &9999,
        &sig,
    );
    assert_eq!(replay, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn witnessed_cancel_stream_not_found() {
    let ctx = WitnessCtx::setup();
    let sig = ctx.sign_cancel(999, 9999);

    let result = ctx
        .client()
        .try_witnessed_cancel_stream(&999, &ctx.witness_pk, &9999, &sig);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

#[test]
fn create_streams_with_witness_persists_witness() {
    let ctx = WitnessCtx::setup();
    let now = ctx.env.ledger().timestamp();

    let streams = soroban_sdk::vec![
        &ctx.env,
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: now,
            cliff_time: now,
            end_time: now + 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            witness: Some(ctx.witness_addr.clone()),
        },
    ];

    let ids = ctx.client().create_streams(&ctx.sender, &streams);
    let stream = ctx.client().get_stream_state(&ids.get(0).unwrap());
    assert_eq!(stream.witness, Some(ctx.witness_addr));
}
