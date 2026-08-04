//! Comprehensive integration tests for `delegated_cancel` —
//! sender-signed, nonce-and-deadline-protected stream cancellation.
//!
//! # Coverage map
//!
//! ## Happy-path
//! - `delegated_cancel_valid_signature_cancels_stream`
//! - `delegated_cancel_sets_cancelled_at_to_current_timestamp`
//! - `delegated_cancel_refund_amount_correct`
//! - `delegated_cancel_fully_accrued_zero_refund`
//! - `delegated_cancel_before_cliff_full_refund`
//! - `delegated_cancel_from_paused_stream`
//! - `delegated_cancel_recipient_can_withdraw_after_cancel`
//! - `delegated_cancel_emits_stream_cancelled_event`
//! - `delegated_cancel_get_delegated_cancel_nonce_view`
//!
//! ## Replay protection
//! - `delegated_cancel_increments_nonce`
//! - `delegated_cancel_replay_rejected`
//! - `delegated_cancel_nonce_is_per_sender`
//! - `delegated_cancel_failed_call_does_not_consume_nonce`
//!
//! ## Deadline
//! - `delegated_cancel_expired_deadline_rejected`
//! - `delegated_cancel_deadline_exactly_now_passes`
//! - `delegated_cancel_deadline_max_u64_passes`
//!
//! ## Signature binding
//! - `delegated_cancel_wrong_public_key_rejected`
//! - `delegated_cancel_wrong_stream_id_in_signature_rejected`
//! - `delegated_cancel_wrong_nonce_in_signature_rejected`
//! - `delegated_cancel_wrong_deadline_in_signature_rejected`
//! - `delegated_cancel_delegated_withdraw_payload_not_replayable`
//! - `delegated_cancel_witnessed_cancel_payload_not_replayable`
//!
//! ## Error paths
//! - `delegated_cancel_stream_not_found`
//! - `delegated_cancel_already_cancelled_stream_rejected`
//! - `delegated_cancel_completed_stream_rejected`
//! - `delegated_cancel_irrevocable_stream_rejected`
//!
//! ## Security invariants
//! - `delegated_cancel_nonce_scope_independent_of_withdraw_nonce`

extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    xdr::{AccountId, PublicKey, ScAddress, Uint256},
    Address, Bytes, BytesN, Env, FromVal, TryIntoVal,
};

// ---------------------------------------------------------------------------
// Domain tag — must match the constant in delegation.rs
// ---------------------------------------------------------------------------
const DELEGATED_CANCEL_DOMAIN: &[u8; 24] = b"fluxora_delegated_cancel";

// ---------------------------------------------------------------------------
// XDR helpers
// ---------------------------------------------------------------------------

/// Construct a Soroban `Address` from a raw ed25519 public-key byte array.
fn address_from_pk(env: &Env, pk: &[u8; 32]) -> Address {
    ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(*pk))))
        .try_into_val(env)
        .expect("valid ed25519 key -> address")
}

// ---------------------------------------------------------------------------
// Message-building helpers
// ---------------------------------------------------------------------------

/// Build the canonical delegated-cancel signed payload.
///
/// Layout:
/// `DELEGATED_CANCEL_DOMAIN` (24 bytes) | `stream_id` (8 bytes BE)
/// | `nonce` (8 bytes BE) | `deadline` (8 bytes BE)
fn build_cancel_msg(env: &Env, stream_id: u64, nonce: u64, deadline: u64) -> Bytes {
    let mut msg = Bytes::new(env);
    msg.extend_from_array(DELEGATED_CANCEL_DOMAIN);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg
}

/// Sign a `Bytes` payload with an ed25519 `SigningKey`.
fn sign_msg(env: &Env, signing_key: &SigningKey, msg: &Bytes) -> BytesN<64> {
    let bytes: std::vec::Vec<u8> = (0..msg.len()).map(|i| msg.get_unchecked(i)).collect();
    BytesN::from_array(env, &signing_key.sign(&bytes).to_bytes())
}

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

/// Everything a test needs, pre-wired.
///
/// The sender is an ed25519-key-derived Stellar account address.
/// We inject the required `AccountEntry` + `TrustLineEntry` into the host
/// ledger so the Stellar Asset Contract can mint tokens to and refund that
/// account (the same pattern used in `adversarial_auth.rs`).
struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    sender_sk: SigningKey,
    sender_pk: BytesN<32>,
    sender: Address,
    recipient: Address,
    relayer: Address,
    token_id: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        Self::setup_with_key(&[0x42u8; 32])
    }

    fn setup_with_key(raw_key: &[u8; 32]) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(0);

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        // Keep the full SAC object so we can extract the issuer for trustlines.
        let sac_contract = env.register_stellar_asset_contract_v2(token_admin);
        let token_id = sac_contract.address();
        let admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        let relayer = Address::generate(&env);

        let sender_sk = SigningKey::from_bytes(raw_key);
        let pk_arr = sender_sk.verifying_key().to_bytes();
        let sender_pk = BytesN::from_array(&env, &pk_arr);
        let sender_account_id = AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(pk_arr)));
        let sender = address_from_pk(&env, &pk_arr);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        // Inject AccountEntry + TrustLineEntry for the ed25519 sender so the
        // Stellar Asset Contract can credit and debit the account.
        {
            use soroban_env_host::budget::AsBudget;
            use soroban_sdk::xdr::{
                AccountEntry, AccountEntryExt, AlphaNum4, AssetCode4, LedgerEntry, LedgerEntryData,
                LedgerEntryExt, LedgerKey, LedgerKeyAccount, LedgerKeyTrustLine, SequenceNumber,
                Thresholds, TrustLineAsset, TrustLineEntry, TrustLineEntryExt, TrustLineFlags,
                VecM,
            };
            use std::rc::Rc;

            let issuer_addr = sac_contract.issuer().address();
            let issuer_xdr = match soroban_sdk::xdr::ScAddress::from(&issuer_addr) {
                soroban_sdk::xdr::ScAddress::Account(id) => id,
                other => panic!("expected Account, got {:?}", other),
            };
            let asset = TrustLineAsset::CreditAlphanum4(AlphaNum4 {
                asset_code: AssetCode4([b'a', b'a', b'a', 0]),
                issuer: issuer_xdr,
            });

            env.host()
                .with_mut_storage(|storage| {
                    let budget = env.host().as_budget();

                    // AccountEntry
                    let acct_key = Rc::new(LedgerKey::Account(LedgerKeyAccount {
                        account_id: sender_account_id.clone(),
                    }));
                    if !storage.has(&acct_key, budget)? {
                        storage.put(
                            &acct_key,
                            &Rc::new(LedgerEntry {
                                data: LedgerEntryData::Account(AccountEntry {
                                    account_id: sender_account_id.clone(),
                                    balance: 0,
                                    flags: 0,
                                    home_domain: Default::default(),
                                    inflation_dest: None,
                                    num_sub_entries: 0,
                                    seq_num: SequenceNumber(0),
                                    thresholds: Thresholds([1; 4]),
                                    signers: VecM::default(),
                                    ext: AccountEntryExt::V0,
                                }),
                                last_modified_ledger_seq: 0,
                                ext: LedgerEntryExt::V0,
                            }),
                            None,
                            budget,
                        )?;
                    }

                    // TrustLineEntry
                    let tl_key = Rc::new(LedgerKey::Trustline(LedgerKeyTrustLine {
                        account_id: sender_account_id.clone(),
                        asset: asset.clone(),
                    }));
                    if !storage.has(&tl_key, budget)? {
                        storage.put(
                            &tl_key,
                            &Rc::new(LedgerEntry {
                                data: LedgerEntryData::Trustline(TrustLineEntry {
                                    account_id: sender_account_id.clone(),
                                    asset,
                                    balance: 0,
                                    limit: i64::MAX,
                                    flags: TrustLineFlags::AuthorizedFlag as u32,
                                    ext: TrustLineEntryExt::V0,
                                }),
                                last_modified_ledger_seq: 0,
                                ext: LedgerEntryExt::V0,
                            }),
                            None,
                            budget,
                        )?;
                    }
                    Ok(())
                })
                .expect("trustline setup must succeed");
        }

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);
        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Ctx {
            env,
            contract_id,
            sender_sk,
            sender_pk,
            sender,
            recipient,
            relayer,
            token_id,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    /// Create a default 1000-token linear stream (rate 1/s, 0..1000 s, no cliff).
    fn create_stream(&self) -> u64 {
        self.create_stream_with(0, 0, 1000, 1)
    }

    /// Create a stream with explicit start/cliff/end and rate.
    fn create_stream_with(&self, start: u64, cliff: u64, end: u64, rate: i128) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: rate * (end - start) as i128,
                rate_per_second: rate,
                start_time: start,
                cliff_time: cliff,
                end_time: end,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    /// Create an irrevocable stream.
    fn create_irrevocable_stream(&self) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0,
                cliff_time: 0,
                end_time: 1000,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: Some(true),
                witness: None,
            },
        )
    }

    /// Sign the canonical cancel payload for (stream_id, nonce, deadline).
    fn sign_cancel(&self, stream_id: u64, nonce: u64, deadline: u64) -> BytesN<64> {
        let msg = build_cancel_msg(&self.env, stream_id, nonce, deadline);
        sign_msg(&self.env, &self.sender_sk, &msg)
    }

    /// Read the token balance of `addr` from the token contract.
    fn balance(&self, addr: &Address) -> i128 {
        self.token.balance(addr)
    }
}

// ===========================================================================
// Happy-path tests
// ===========================================================================

#[test]
fn delegated_cancel_valid_signature_cancels_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(300);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);

    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

#[test]
fn delegated_cancel_sets_cancelled_at_to_current_timestamp() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(450);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        stream.cancelled_at,
        Some(450),
        "cancelled_at must equal the ledger timestamp at cancellation"
    );
}

#[test]
fn delegated_cancel_refund_amount_correct() {
    // Stream: 1000 tokens, 1/s, 0..1000s. Cancel at t=300 → 300 accrued, 700 refunded.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    let sender_balance_before = ctx.balance(&ctx.sender);
    ctx.env.ledger().set_timestamp(300);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    let sender_balance_after = ctx.balance(&ctx.sender);
    assert_eq!(
        sender_balance_after - sender_balance_before,
        700,
        "sender must be refunded deposit_amount - accrued = 700"
    );
}

#[test]
fn delegated_cancel_fully_accrued_zero_refund() {
    // Cancel after end_time → accrued == deposit_amount → refund = 0.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    let sender_balance_before = ctx.balance(&ctx.sender);
    ctx.env.ledger().set_timestamp(1500); // past end_time=1000
    let sig = ctx.sign_cancel(stream_id, 0, 99999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &99999, &sig);

    let sender_balance_after = ctx.balance(&ctx.sender);
    assert_eq!(
        sender_balance_after - sender_balance_before,
        0,
        "fully-accrued stream must give sender zero refund"
    );
}

#[test]
fn delegated_cancel_before_cliff_full_refund() {
    // Cliff at t=500. Cancel at t=100 → 0 accrued → full refund.
    let ctx = Ctx::setup();
    // deposit = 1 * 1000 = 1000
    let stream_id = ctx.create_stream_with(0, 500, 1000, 1);

    let sender_balance_before = ctx.balance(&ctx.sender);
    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    let sender_balance_after = ctx.balance(&ctx.sender);
    assert_eq!(
        sender_balance_after - sender_balance_before,
        1000,
        "cancellation before cliff must refund full deposit"
    );
}

#[test]
fn delegated_cancel_from_paused_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    // Advance ledger sequence to clear the pause cooldown (cooldown = 17 ledgers).
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
    ctx.client()
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);
    ctx.env.ledger().set_timestamp(200);

    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        stream.status,
        StreamStatus::Cancelled,
        "delegated_cancel must work on Paused streams"
    );
}

#[test]
fn delegated_cancel_recipient_can_withdraw_after_cancel() {
    // After cancellation the accrued amount is frozen; recipient can still withdraw it.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(400);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    let recipient_balance_before = ctx.balance(&ctx.recipient);
    ctx.client().withdraw(&stream_id, &None);
    let recipient_balance_after = ctx.balance(&ctx.recipient);

    assert_eq!(
        recipient_balance_after - recipient_balance_before,
        400,
        "recipient must be able to withdraw the 400 tokens accrued before cancellation"
    );
}

#[test]
fn delegated_cancel_emits_stream_cancelled_event() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    let events = ctx.env.events().all();
    let last = events.last().unwrap();
    let payload = Option::<fluxora_stream::StreamEvent>::from_val(&ctx.env, &last.2).unwrap();
    assert_eq!(
        payload,
        fluxora_stream::StreamEvent::StreamCancelled(stream_id),
        "delegated_cancel must emit StreamCancelled event"
    );
}

#[test]
fn delegated_cancel_get_delegated_cancel_nonce_view() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    // Before any cancellation nonce is 0.
    assert_eq!(
        ctx.client().get_delegated_cancel_nonce(&ctx.sender),
        0,
        "nonce must start at 0"
    );

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    assert_eq!(
        ctx.client().get_delegated_cancel_nonce(&ctx.sender),
        1,
        "nonce must be 1 after one successful cancellation"
    );
}

// ===========================================================================
// Replay-protection tests
// ===========================================================================

#[test]
fn delegated_cancel_increments_nonce() {
    let ctx = Ctx::setup();
    let stream_id1 = ctx.create_stream();
    let stream_id2 = ctx.create_stream();

    ctx.env.ledger().set_timestamp(300);

    // Cancel first stream with nonce 0.
    let sig1 = ctx.sign_cancel(stream_id1, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id1, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig1);

    // Attempt to cancel second stream with stale nonce 0 — must fail.
    let sig2_stale = ctx.sign_cancel(stream_id2, 0, 9999);
    let res = ctx.client().try_delegated_cancel(
        &stream_id2,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &sig2_stale,
    );
    assert_eq!(
        res,
        Err(Ok(ContractError::InvalidSignature)),
        "stale nonce must be rejected after first cancellation"
    );

    // Cancel second stream with correct nonce 1.
    let sig2_ok = ctx.sign_cancel(stream_id2, 1, 9999);
    ctx.client().delegated_cancel(
        &stream_id2,
        &ctx.relayer,
        &ctx.sender_pk,
        &1,
        &9999,
        &sig2_ok,
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id2).status,
        StreamStatus::Cancelled
    );
}

#[test]
fn delegated_cancel_replay_rejected() {
    // Re-submitting the exact same signed payload after a successful cancel
    // must fail because the nonce was incremented.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    // The stream is now Cancelled; replay hits InvalidState (not InvalidSignature)
    // because the nonce check happens before status check inside the validator,
    // but the stream status check in cancel_stream_internal fires first.
    // Either InvalidSignature or InvalidState proves the replay was rejected.
    let replay = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &sig,
    );
    assert!(
        replay.is_err(),
        "replay of a consumed nonce must be rejected"
    );
}

#[test]
fn delegated_cancel_nonce_is_per_sender() {
    // Two different senders each have their own independent nonce counter.
    let ctx_a = Ctx::setup_with_key(&[0x11u8; 32]);
    let ctx_b = Ctx::setup_with_key(&[0x22u8; 32]);

    // Both start at 0.
    assert_eq!(ctx_a.client().get_delegated_cancel_nonce(&ctx_a.sender), 0);
    assert_eq!(ctx_b.client().get_delegated_cancel_nonce(&ctx_b.sender), 0);

    // Cancel a stream for sender A — sender B's nonce must remain 0.
    let stream_a = ctx_a.create_stream();
    ctx_a.env.ledger().set_timestamp(100);
    let sig_a = ctx_a.sign_cancel(stream_a, 0, 9999);
    ctx_a.client().delegated_cancel(
        &stream_a,
        &ctx_a.relayer,
        &ctx_a.sender_pk,
        &0,
        &9999,
        &sig_a,
    );
    assert_eq!(
        ctx_a.client().get_delegated_cancel_nonce(&ctx_a.sender),
        1,
        "sender A nonce must have incremented to 1"
    );

    // Sender B's nonce is still 0 in its own environment.
    assert_eq!(
        ctx_b.client().get_delegated_cancel_nonce(&ctx_b.sender),
        0,
        "sender B nonce must still be 0"
    );
}

#[test]
fn delegated_cancel_failed_call_does_not_consume_nonce() {
    // A call that fails (e.g. wrong nonce) must not advance the nonce counter.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(100);

    // Send wrong nonce 1 (stored is 0).
    let sig_bad = ctx.sign_cancel(stream_id, 1, 9999);
    let _ = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &1,
        &9999,
        &sig_bad,
    );

    // Correct nonce 0 must still work.
    let sig_ok = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig_ok);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
}

// ===========================================================================
// Deadline tests
// ===========================================================================

#[test]
fn delegated_cancel_expired_deadline_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(5000);
    let sig = ctx.sign_cancel(stream_id, 0, 100); // deadline 100 < now 5000

    let result =
        ctx.client()
            .try_delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &100, &sig);
    assert_eq!(
        result,
        Err(Ok(ContractError::SignatureDeadlineExpired)),
        "expired deadline must return SignatureDeadlineExpired"
    );
    // Stream must remain Active.
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

#[test]
fn delegated_cancel_deadline_exactly_now_passes() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(500);
    let sig = ctx.sign_cancel(stream_id, 0, 500); // deadline == now

    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &500, &sig);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
}

#[test]
fn delegated_cancel_deadline_max_u64_passes() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 0, u64::MAX);

    ctx.client().delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &u64::MAX,
        &sig,
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
}

// ===========================================================================
// Signature-binding tests
// ===========================================================================

#[test]
fn delegated_cancel_wrong_public_key_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    // A different keypair — public key does not match stream.sender.
    let other_sk = SigningKey::from_bytes(&[0x01u8; 32]);
    let other_pk = BytesN::from_array(&ctx.env, &other_sk.verifying_key().to_bytes());

    ctx.env.ledger().set_timestamp(100);
    // Sign with the *correct* key but submit the *wrong* public key.
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &other_pk, // wrong key
        &0,
        &9999,
        &sig,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidSignature)),
        "mismatched public key must be rejected"
    );
}

#[test]
fn delegated_cancel_wrong_stream_id_in_signature_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(100);
    // Sign with stream_id=999 but submit stream_id=0.
    let sig = ctx.sign_cancel(999, 0, 9999);
    // The ed25519_verify call will trap because the payload is wrong.
    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &sig,
    );
    assert!(
        result.is_err(),
        "signature over wrong stream_id must be rejected"
    );
}

#[test]
fn delegated_cancel_wrong_nonce_in_signature_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(100);
    // Submit nonce=1 to the function when stored nonce is 0.
    // validate_delegated_cancel_params checks nonce==stored before reaching ed25519_verify,
    // so this returns InvalidSignature cleanly without a host trap.
    let sig_for_nonce1 = ctx.sign_cancel(stream_id, 1, 9999);
    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &1, // wrong nonce passed to function — won't match stored 0
        &9999,
        &sig_for_nonce1,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidSignature)),
        "nonce mismatch must be rejected"
    );
}

#[test]
fn delegated_cancel_wrong_deadline_in_signature_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(100);
    // Sign with deadline=8888 but submit deadline=9999.
    // The signed payload includes the deadline, so ed25519_verify will fail.
    let sig = ctx.sign_cancel(stream_id, 0, 8888);
    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999, // different deadline than what was signed
        &sig,
    );
    assert!(
        result.is_err(),
        "signature over wrong deadline must be rejected"
    );
}

#[test]
fn delegated_cancel_delegated_withdraw_payload_not_replayable() {
    // A delegated_withdraw style payload (no domain tag, different fields)
    // must not authorize a delegated_cancel.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(100);

    // Build delegated_withdraw message format:
    // stream_id | nonce | deadline | expected_min | relayer_fee  (no domain tag)
    let mut withdraw_msg = Bytes::new(&ctx.env);
    withdraw_msg.extend_from_array(&stream_id.to_be_bytes());
    withdraw_msg.extend_from_array(&0u64.to_be_bytes());
    withdraw_msg.extend_from_array(&9999u64.to_be_bytes());
    withdraw_msg.extend_from_array(&0i128.to_be_bytes());
    withdraw_msg.extend_from_array(&0i128.to_be_bytes());
    let cross_sig = sign_msg(&ctx.env, &ctx.sender_sk, &withdraw_msg);

    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &cross_sig,
    );
    assert!(
        result.is_err(),
        "delegated_withdraw payload must not authorize delegated_cancel"
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

#[test]
fn delegated_cancel_witnessed_cancel_payload_not_replayable() {
    // A witnessed_cancel payload (different domain, no nonce field) must
    // not authorize a delegated_cancel.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(100);

    let mut witness_msg = Bytes::new(&ctx.env);
    witness_msg.extend_from_array(b"fluxora_witnessed_cancel");
    witness_msg.extend_from_array(&stream_id.to_be_bytes());
    witness_msg.extend_from_array(&9999u64.to_be_bytes()); // deadline, no nonce
    let cross_sig = sign_msg(&ctx.env, &ctx.sender_sk, &witness_msg);

    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &cross_sig,
    );
    assert!(
        result.is_err(),
        "witnessed_cancel payload must not authorize delegated_cancel"
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

// ===========================================================================
// Error-path tests
// ===========================================================================

#[test]
fn delegated_cancel_stream_not_found() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(100);

    let sig = ctx.sign_cancel(999, 0, 9999);
    let result =
        ctx.client()
            .try_delegated_cancel(&999, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);
    assert_eq!(
        result,
        Err(Ok(ContractError::StreamNotFound)),
        "non-existent stream_id must return StreamNotFound"
    );
}

#[test]
fn delegated_cancel_already_cancelled_stream_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(100);
    let sig0 = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig0);

    // Try again with nonce 1.
    let sig1 = ctx.sign_cancel(stream_id, 1, 9999);
    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &1,
        &9999,
        &sig1,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidState)),
        "cancelling an already-cancelled stream must return InvalidState"
    );
}

#[test]
fn delegated_cancel_completed_stream_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    // Advance to end and withdraw everything to reach Completed.
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id, &None);

    let sig = ctx.sign_cancel(stream_id, 0, 99999);
    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &99999,
        &sig,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidState)),
        "cancelling a Completed stream must return InvalidState"
    );
}

#[test]
fn delegated_cancel_irrevocable_stream_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_irrevocable_stream();

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    let result = ctx.client().try_delegated_cancel(
        &stream_id,
        &ctx.relayer,
        &ctx.sender_pk,
        &0,
        &9999,
        &sig,
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::Unauthorized)),
        "irrevocable stream must reject delegated_cancel with Unauthorized"
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

// ===========================================================================
// Security invariants
// ===========================================================================

#[test]
fn delegated_cancel_nonce_scope_independent_of_withdraw_nonce() {
    // The cancel nonce (DelegatedCancelNonce, keyed by sender) must be
    // completely independent of the withdraw nonce (DelegatedWithdrawNonce,
    // keyed by recipient).  Cancelling should not perturb the withdraw nonce
    // and vice versa.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    // Withdraw nonce starts at 0.
    assert_eq!(ctx.client().get_delegated_nonce(&ctx.recipient), 0);
    // Cancel nonce starts at 0.
    assert_eq!(ctx.client().get_delegated_cancel_nonce(&ctx.sender), 0);

    ctx.env.ledger().set_timestamp(100);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    // Cancel nonce incremented.
    assert_eq!(ctx.client().get_delegated_cancel_nonce(&ctx.sender), 1);
    // Withdraw nonce must be untouched.
    assert_eq!(
        ctx.client().get_delegated_nonce(&ctx.recipient),
        0,
        "cancel must not touch the recipient withdraw nonce"
    );
}

#[test]
fn delegated_cancel_accrued_and_refund_sum_to_deposit() {
    // Invariant: accrued_at_cancel + refund == deposit_amount for any cancel time.
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(); // deposit = 1000

    let sender_before = ctx.balance(&ctx.sender);
    ctx.env.ledger().set_timestamp(600);
    let sig = ctx.sign_cancel(stream_id, 0, 9999);
    ctx.client()
        .delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &9999, &sig);

    let refund = ctx.balance(&ctx.sender) - sender_before;
    let accrued = ctx.client().calculate_accrued(&stream_id);

    assert_eq!(
        refund + accrued,
        1000,
        "refund + frozen_accrued must equal deposit_amount"
    );
}

#[test]
fn delegated_cancel_state_change_atomic_no_partial_update() {
    // If the call fails (expired deadline), the stream must remain Active
    // and no state changes at all (nonce, cancelled_at, status untouched).
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(1000);
    let sig = ctx.sign_cancel(stream_id, 0, 50); // expired

    let _ =
        ctx.client()
            .try_delegated_cancel(&stream_id, &ctx.relayer, &ctx.sender_pk, &0, &50, &sig);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
    assert_eq!(stream.cancelled_at, None);
    assert_eq!(ctx.client().get_delegated_cancel_nonce(&ctx.sender), 0);
}
