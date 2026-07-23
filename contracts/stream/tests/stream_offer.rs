//! Integration tests for the two-phase offer-then-accept stream creation flow.
//!
//! Covers: create_stream_offer, accept_stream_offer, reject_stream_offer,
//! cancel_stream_offer, get_stream_offer, get_recipient_pending_offers.

extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, StreamKind, StreamOffer, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    contract_id: Address,
    token: TokenClient<'a>,
    sender: Address,
    recipient: Address,
}

impl<'a> Ctx<'a> {
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
        client.init(&token_id, &admin);

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Mint tokens and grant allowance to the contract.
        StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_000_i128);
        token.approve(&sender, &contract_id, &i128::MAX, &999_999);

        // Start at a non-zero timestamp so start_time=now+delay is clearly in the future.
        env.ledger().set_timestamp(1_000_000);

        Ctx {
            env,
            client,
            contract_id,
            token,
            sender,
            recipient,
        }
    }

    /// Create a basic offer and return its offer_id.
    fn make_offer(&self) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream_offer(
            &self.sender,
            &self.recipient,
            &1_000_i128,
            &1_i128,
            &(now + 10),
            &(now + 10),
            &(now + 1_010),
            &0_i128,
            &None,
            &StreamKind::Linear,
            &None,
            &None,
        )
    }
}

// ---------------------------------------------------------------------------
// Happy-path: create → accept
// ---------------------------------------------------------------------------

#[test]
fn offer_accept_creates_active_stream() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    let offer_id = ctx.make_offer();

    // Deposit should have left the sender.
    assert_eq!(ctx.token.balance(&ctx.sender), 1_000_000 - 1_000);

    // Stream not yet in recipient index.
    assert_eq!(ctx.client.get_recipient_streams(&ctx.recipient).len(), 0);

    // Pending offer index has one entry.
    let pending = ctx.client.get_recipient_pending_offers(&ctx.recipient);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending.get(0).unwrap(), offer_id);

    // Accept the offer.
    ctx.env.ledger().set_timestamp(now + 5);
    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(stream_id, offer_id);

    // Stream is now Active.
    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Active);
    assert_eq!(stream.sender, ctx.sender);
    assert_eq!(stream.recipient, ctx.recipient);
    assert_eq!(stream.deposit_amount, 1_000);

    // Recipient stream index now has this stream.
    let ids = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(ids.len(), 1);
    assert_eq!(ids.get(0).unwrap(), stream_id);

    // Pending offers index is cleared.
    assert_eq!(
        ctx.client.get_recipient_pending_offers(&ctx.recipient).len(),
        0
    );

    // Offer record is gone.
    assert_eq!(
        ctx.client.try_get_stream_offer(&offer_id),
        Err(Ok(ContractError::OfferNotFound))
    );
}

// ---------------------------------------------------------------------------
// Happy-path: create → reject by recipient
// ---------------------------------------------------------------------------

#[test]
fn offer_reject_refunds_sender() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();

    let balance_before = ctx.token.balance(&ctx.sender);

    ctx.client.reject_stream_offer(&ctx.recipient, &offer_id);

    // Deposit returned to sender.
    assert_eq!(ctx.token.balance(&ctx.sender), balance_before + 1_000);

    // No stream created.
    assert_eq!(ctx.client.get_recipient_streams(&ctx.recipient).len(), 0);

    // Pending offers cleared.
    assert_eq!(
        ctx.client.get_recipient_pending_offers(&ctx.recipient).len(),
        0
    );

    // Offer is gone.
    assert_eq!(
        ctx.client.try_get_stream_offer(&offer_id),
        Err(Ok(ContractError::OfferNotFound))
    );
}

// ---------------------------------------------------------------------------
// Happy-path: create → cancel by sender
// ---------------------------------------------------------------------------

#[test]
fn offer_cancel_by_sender_refunds_deposit() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();

    let balance_before = ctx.token.balance(&ctx.sender);

    ctx.client.cancel_stream_offer(&ctx.sender, &offer_id);

    assert_eq!(ctx.token.balance(&ctx.sender), balance_before + 1_000);
    assert_eq!(ctx.client.get_recipient_streams(&ctx.recipient).len(), 0);
    assert_eq!(
        ctx.client.get_recipient_pending_offers(&ctx.recipient).len(),
        0
    );
    assert_eq!(
        ctx.client.try_get_stream_offer(&offer_id),
        Err(Ok(ContractError::OfferNotFound))
    );
}

// ---------------------------------------------------------------------------
// Expiry
// ---------------------------------------------------------------------------

#[test]
fn accept_after_expiry_returns_offer_expired() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    // Create offer with expiry 100 seconds from now.
    let expiry = now + 100;
    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 1_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &Some(expiry),
    );

    // Advance past expiry.
    ctx.env.ledger().set_timestamp(expiry + 1);

    let err = ctx.client.try_accept_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(err, Err(Ok(ContractError::OfferExpired)));

    // Offer still exists — sender can still cancel.
    assert!(ctx.client.try_get_stream_offer(&offer_id).is_ok());
}

#[test]
fn accept_at_expiry_boundary_still_valid() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let expiry = now + 100;

    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 1_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &Some(expiry),
    );

    // At exactly expiry_time (not strictly greater) — still valid.
    ctx.env.ledger().set_timestamp(expiry);
    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(stream_id, offer_id);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

#[test]
fn offer_with_no_expiry_never_expires() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &2_000_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 2_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None, // No expiry
    );

    // Fast-forward a long time.
    ctx.env.ledger().set_timestamp(now + 999_999);

    // Accept should still succeed.
    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

#[test]
fn create_offer_with_past_expiry_fails() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    let err = ctx.client.try_create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 1_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &Some(now), // expiry == now → invalid
    );
    assert_eq!(err, Err(Ok(ContractError::InvalidParams)));
}

// ---------------------------------------------------------------------------
// Wrong-caller / authorization
// ---------------------------------------------------------------------------

#[test]
fn accept_with_wrong_recipient_fails() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();

    let impostor = Address::generate(&ctx.env);
    let err = ctx
        .client
        .try_accept_stream_offer(&impostor, &offer_id);
    assert_eq!(err, Err(Ok(ContractError::OfferWrongRecipient)));

    // Offer still pending.
    assert!(ctx.client.try_get_stream_offer(&offer_id).is_ok());
}

#[test]
fn reject_with_wrong_recipient_fails() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();

    let impostor = Address::generate(&ctx.env);
    let err = ctx
        .client
        .try_reject_stream_offer(&impostor, &offer_id);
    assert_eq!(err, Err(Ok(ContractError::OfferWrongRecipient)));

    // Offer still pending.
    assert!(ctx.client.try_get_stream_offer(&offer_id).is_ok());
}

#[test]
fn cancel_with_wrong_sender_fails() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();

    let impostor = Address::generate(&ctx.env);
    let err = ctx
        .client
        .try_cancel_stream_offer(&impostor, &offer_id);
    assert_eq!(err, Err(Ok(ContractError::OfferWrongSender)));

    // Offer still pending.
    assert!(ctx.client.try_get_stream_offer(&offer_id).is_ok());
}

// ---------------------------------------------------------------------------
// Double-action idempotency guards
// ---------------------------------------------------------------------------

#[test]
fn double_accept_second_returns_not_found() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();

    ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);

    let err = ctx.client.try_accept_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(err, Err(Ok(ContractError::OfferNotFound)));
}

#[test]
fn double_cancel_second_returns_not_found() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();

    ctx.client.cancel_stream_offer(&ctx.sender, &offer_id);

    let err = ctx.client.try_cancel_stream_offer(&ctx.sender, &offer_id);
    assert_eq!(err, Err(Ok(ContractError::OfferNotFound)));
}

#[test]
fn double_reject_second_returns_not_found() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();

    ctx.client.reject_stream_offer(&ctx.recipient, &offer_id);

    let err = ctx
        .client
        .try_reject_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(err, Err(Ok(ContractError::OfferNotFound)));
}

#[test]
fn accept_nonexistent_offer_returns_not_found() {
    let ctx = Ctx::setup();
    let err = ctx.client.try_accept_stream_offer(&ctx.recipient, &999_u64);
    assert_eq!(err, Err(Ok(ContractError::OfferNotFound)));
}

// ---------------------------------------------------------------------------
// Start-time re-anchoring
// ---------------------------------------------------------------------------

#[test]
fn accept_re_anchors_start_time_when_past() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp(); // 1_000_000

    // Offer with start_time only 10s from now.
    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &(now + 10),   // start_time = now+10
        &(now + 10),   // cliff = now+10 (offset 0)
        &(now + 1_010),// end = now+1010  (duration = 1000)
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );

    // Advance well past the requested start_time.
    let accept_time = now + 500;
    ctx.env.ledger().set_timestamp(accept_time);

    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    let stream = ctx.client.get_stream_state(&stream_id);

    // start_time must be re-anchored to acceptance time (not original now+10).
    assert_eq!(stream.start_time, accept_time);
    // Duration preserved: 1000 seconds.
    assert_eq!(stream.end_time - stream.start_time, 1_000);
    // Cliff offset preserved: was 0 relative to start.
    assert_eq!(stream.cliff_time, stream.start_time);
}

#[test]
fn accept_preserves_cliff_offset_on_reanchor() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    // Offer: start=now+10, cliff=now+110 (100s offset), end=now+1010 (1000s duration).
    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &(now + 10),
        &(now + 110), // cliff offset = 100s
        &(now + 1_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );

    // Accept after the original start_time has passed.
    let accept_time = now + 200;
    ctx.env.ledger().set_timestamp(accept_time);

    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    let stream = ctx.client.get_stream_state(&stream_id);

    assert_eq!(stream.start_time, accept_time);
    // Cliff offset of 100s preserved.
    assert_eq!(stream.cliff_time, accept_time + 100);
    // Duration of 1000s preserved.
    assert_eq!(stream.end_time, accept_time + 1_000);
}

#[test]
fn accept_when_start_still_future_keeps_original_start() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &(now + 500), // start well in the future
        &(now + 500),
        &(now + 1_500),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );

    // Accept only 10s later — start is still in the future.
    ctx.env.ledger().set_timestamp(now + 10);
    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    let stream = ctx.client.get_stream_state(&stream_id);

    // Original start preserved.
    assert_eq!(stream.start_time, now + 500);
    assert_eq!(stream.end_time, now + 1_500);
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

#[test]
fn get_stream_offer_returns_full_offer() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let expiry = now + 3_600;

    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &5_000_i128,
        &5_i128,
        &(now + 60),
        &(now + 60),
        &(now + 1_060),
        &10_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &Some(expiry),
    );

    let offer: StreamOffer = ctx.client.get_stream_offer(&offer_id);
    assert_eq!(offer.offer_id, offer_id);
    assert_eq!(offer.sender, ctx.sender);
    assert_eq!(offer.recipient, ctx.recipient);
    assert_eq!(offer.deposit_amount, 5_000);
    assert_eq!(offer.rate_per_second, 5);
    assert_eq!(offer.withdraw_dust_threshold, 10);
    assert_eq!(offer.expiry_time, Some(expiry));
    assert_eq!(offer.created_at, now);
}

#[test]
fn get_recipient_pending_offers_lists_multiple() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    StellarAssetClient::new(&ctx.env, &ctx.token.address).mint(&ctx.sender, &10_000_i128);

    let id0 = ctx.make_offer();
    let id1 = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 510),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );

    let pending = ctx.client.get_recipient_pending_offers(&ctx.recipient);
    assert_eq!(pending.len(), 2);
    // Sorted ascending by offer_id.
    assert!(pending.get(0).unwrap() < pending.get(1).unwrap());
    assert!(pending.get(0).unwrap() == id0 || pending.get(0).unwrap() == id1);
}

// ---------------------------------------------------------------------------
// Liability and index isolation
// ---------------------------------------------------------------------------

#[test]
fn offer_does_not_appear_in_recipient_streams_index() {
    let ctx = Ctx::setup();
    ctx.make_offer();
    // RecipientStreams index must remain empty until accepted.
    assert_eq!(ctx.client.get_recipient_streams(&ctx.recipient).len(), 0);
}

#[test]
fn reject_does_not_pollute_recipient_streams_index() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();
    ctx.client.reject_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(ctx.client.get_recipient_streams(&ctx.recipient).len(), 0);
}

// ---------------------------------------------------------------------------
// Paused contract blocks offer creation
// ---------------------------------------------------------------------------

#[test]
fn create_offer_blocked_when_creation_paused() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let admin = Address::generate(&ctx.env);
    // Re-init so we know the admin address.
    let env2 = Env::default();
    env2.mock_all_auths();
    let cid2 = env2.register_contract(None, FluxoraStream);
    let c2 = FluxoraStreamClient::new(&env2, &cid2);
    let token_admin2 = Address::generate(&env2);
    let tok2 = env2.register_stellar_asset_contract_v2(token_admin2).address();
    let adm2 = Address::generate(&env2);
    let snd2 = Address::generate(&env2);
    let rcp2 = Address::generate(&env2);
    c2.init(&tok2, &adm2);
    StellarAssetClient::new(&env2, &tok2).mint(&snd2, &10_000_i128);
    TokenClient::new(&env2, &tok2).approve(&snd2, &cid2, &i128::MAX, &999_999);
    env2.ledger().set_timestamp(1_000_000);

    // Pause creation.
    c2.set_contract_paused(&adm2, &true);

    let err = c2.try_create_stream_offer(
        &snd2,
        &rcp2,
        &1_000_i128,
        &1_i128,
        &(1_000_010_u64),
        &(1_000_010_u64),
        &(1_001_010_u64),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );
    assert_eq!(err, Err(Ok(ContractError::ContractPaused)));
}

// ---------------------------------------------------------------------------
// Invalid parameter guards
// ---------------------------------------------------------------------------

#[test]
fn create_offer_rejects_zero_deposit() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx.client.try_create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &0_i128, // invalid
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 1_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );
    assert_eq!(err, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn create_offer_rejects_sender_equals_recipient() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx.client.try_create_stream_offer(
        &ctx.sender,
        &ctx.sender, // same as sender
        &1_000_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 1_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );
    assert_eq!(err, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn create_offer_rejects_insufficient_deposit() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    // rate=1, duration=1000 => needs 1000 tokens; only deposit 100.
    let err = ctx.client.try_create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 1_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );
    assert_eq!(err, Err(Ok(ContractError::InsufficientDeposit)));
}

#[test]
fn create_offer_rejects_negative_dust_threshold() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx.client.try_create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 1_010),
        &-1_i128, // invalid
        &None,
        &StreamKind::Linear,
        &None,
        &None,
    );
    assert_eq!(err, Err(Ok(ContractError::InvalidDustThreshold)));
}

// ---------------------------------------------------------------------------
// CliffOnly stream offer
// ---------------------------------------------------------------------------

#[test]
fn cliff_only_offer_accepted_as_cliff_only_stream() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &0_i128, // rate is 0 for CliffOnly
        &(now + 10),
        &(now + 510), // cliff at 500s
        &(now + 510), // end == cliff for CliffOnly
        &0_i128,
        &None,
        &StreamKind::CliffOnly,
        &None,
        &None,
    );

    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    let stream = ctx.client.get_stream_state(&stream_id);

    assert_eq!(stream.kind, StreamKind::CliffOnly);
    assert_eq!(stream.rate_per_second, 0);
    assert_eq!(stream.deposit_amount, 500);
    assert_eq!(stream.status, StreamStatus::Active);
}

// ---------------------------------------------------------------------------
// Deposit escrow: token balances
// ---------------------------------------------------------------------------

#[test]
fn deposit_held_in_contract_until_resolved() {
    let ctx = Ctx::setup();
    let contract_balance_before = ctx.token.balance(&ctx.contract_id);

    let offer_id = ctx.make_offer();

    // Deposit is now in the contract.
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_balance_before + 1_000
    );

    // After rejection, deposit leaves the contract.
    ctx.client.reject_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_balance_before
    );
}

#[test]
fn cancel_sender_after_expiry_still_refunds() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let expiry = now + 50;

    let offer_id = ctx.client.create_stream_offer(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &(now + 10),
        &(now + 10),
        &(now + 1_010),
        &0_i128,
        &None,
        &StreamKind::Linear,
        &None,
        &Some(expiry),
    );

    // Advance past expiry.
    ctx.env.ledger().set_timestamp(expiry + 100);

    let balance_before = ctx.token.balance(&ctx.sender);
    // Sender can always cancel even after expiry.
    ctx.client.cancel_stream_offer(&ctx.sender, &offer_id);
    assert_eq!(ctx.token.balance(&ctx.sender), balance_before + 1_000);
}

// ---------------------------------------------------------------------------
// Stream count increments correctly
// ---------------------------------------------------------------------------

#[test]
fn stream_count_increments_on_offer_creation() {
    let ctx = Ctx::setup();
    let count_before = ctx.client.get_stream_count();
    ctx.make_offer();
    assert_eq!(ctx.client.get_stream_count(), count_before + 1);
}

#[test]
fn accepted_offer_id_is_stable_stream_id() {
    let ctx = Ctx::setup();
    let offer_id = ctx.make_offer();
    let stream_id = ctx.client.accept_stream_offer(&ctx.recipient, &offer_id);
    assert_eq!(stream_id, offer_id);
    // Stream is readable by the same ID.
    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.stream_id, stream_id);
}
