//! Tests for issue #584: reserve_stream_ids entrypoint.
//!
//! Covers: basic reservation, error cases, get_id_reservation view,
//! create_stream consuming reservations, and counter-gap semantics.

extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, StreamKind, MAX_ID_RESERVATION,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    contract_id: Address,
    sender: Address,
    token_id: Address,
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
        let stellar_asset = StellarAssetClient::new(&env, &token_id);
        let token = TokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        stellar_asset.mint(&sender, &1_000_000_000_000i128);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000u32);

        client.init(&token_id, &admin);

        Self {
            env,
            client,
            contract_id,
            sender,
            token_id,
        }
    }

    fn mint(&self, to: &Address) {
        StellarAssetClient::new(&self.env, &self.token_id).mint(to, &1_000_000_000_000i128);
        TokenClient::new(&self.env, &self.token_id).approve(
            to,
            &self.contract_id,
            &i128::MAX,
            &100_000u32,
        );
    }

    fn create_stream(&self, sender: &Address) -> u64 {
        let recipient = Address::generate(&self.env);
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            sender,
            &recipient,
            &1_000_000i128,
            &1i128,
            &(now + 1),
            &(now + 1),
            &(now + 1_000_001),
            &0i128,
            &None,
            &StreamKind::Linear,
        )
    }
}

// ---------------------------------------------------------------------------
// Basic reservation
// ---------------------------------------------------------------------------

#[test]
fn reserve_returns_correct_range_from_zero() {
    let ctx = Ctx::setup();
    let ids = ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None);
    assert_eq!(ids.len(), 5);
    for i in 0..5u32 {
        assert_eq!(ids.get(i).unwrap(), i as u64);
    }
    assert_eq!(ctx.client.get_stream_count(), 5);
}

#[test]
fn reserve_single_id() {
    let ctx = Ctx::setup();
    let ids = ctx.client.reserve_stream_ids(&ctx.sender, &1u32, &None);
    assert_eq!(ids.len(), 1);
    assert_eq!(ids.get(0).unwrap(), 0u64);
    assert_eq!(ctx.client.get_stream_count(), 1);
}

#[test]
fn reserve_max_ids() {
    let ctx = Ctx::setup();
    let ids = ctx
        .client
        .reserve_stream_ids(&ctx.sender, &MAX_ID_RESERVATION, &None);
    assert_eq!(ids.len(), MAX_ID_RESERVATION);
    assert_eq!(ids.get(0).unwrap(), 0u64);
    assert_eq!(
        ids.get(MAX_ID_RESERVATION - 1).unwrap(),
        (MAX_ID_RESERVATION - 1) as u64
    );
    assert_eq!(ctx.client.get_stream_count(), MAX_ID_RESERVATION as u64);
}

#[test]
fn sequential_reservations_are_non_overlapping() {
    let ctx = Ctx::setup();
    let sender2 = Address::generate(&ctx.env);

    let ids1 = ctx.client.reserve_stream_ids(&ctx.sender, &3u32, &None);
    let ids2 = ctx.client.reserve_stream_ids(&sender2, &3u32, &None);

    assert_eq!(ids1.get(0).unwrap(), 0u64);
    assert_eq!(ids2.get(0).unwrap(), 3u64);
    assert_eq!(ctx.client.get_stream_count(), 6);
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
fn reserve_zero_count_errors() {
    let ctx = Ctx::setup();
    let result = ctx.client.try_reserve_stream_ids(&ctx.sender, &0u32, &None);
    assert_eq!(result, Err(Ok(ContractError::ReservationCountZero)));
}

#[test]
fn reserve_over_max_errors() {
    let ctx = Ctx::setup();
    let result = ctx
        .client
        .try_reserve_stream_ids(&ctx.sender, &(MAX_ID_RESERVATION + 1), &None);
    assert_eq!(result, Err(Ok(ContractError::ReservationLimitExceeded)));
}

// ---------------------------------------------------------------------------
// get_id_reservation view
// ---------------------------------------------------------------------------

#[test]
fn get_id_reservation_none_before_reserve() {
    let ctx = Ctx::setup();
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());
}

#[test]
fn get_id_reservation_returns_active_reservation() {
    let ctx = Ctx::setup();
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None);
    let res = ctx.client.get_id_reservation(&ctx.sender).unwrap();
    assert_eq!(res.start_id, 0);
    assert_eq!(res.count, 5);
    assert_eq!(res.consumed, 0);
}

// ---------------------------------------------------------------------------
// create_stream consumes reservation
// ---------------------------------------------------------------------------

#[test]
fn create_stream_uses_reserved_id() {
    let ctx = Ctx::setup();
    ctx.client.reserve_stream_ids(&ctx.sender, &2u32, &None);

    let id0 = ctx.create_stream(&ctx.sender);
    assert_eq!(id0, 0u64);

    let res = ctx.client.get_id_reservation(&ctx.sender).unwrap();
    assert_eq!(res.consumed, 1);

    let id1 = ctx.create_stream(&ctx.sender);
    assert_eq!(id1, 1u64);

    // Fully consumed — reservation removed
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());
}

#[test]
fn create_stream_without_reservation_uses_live_counter() {
    let ctx = Ctx::setup();
    let id = ctx.create_stream(&ctx.sender);
    assert_eq!(id, 0u64);
    assert_eq!(ctx.client.get_stream_count(), 1);
}

#[test]
fn create_stream_after_reservation_exhausted_uses_live_counter() {
    let ctx = Ctx::setup();
    ctx.client.reserve_stream_ids(&ctx.sender, &1u32, &None);

    let id0 = ctx.create_stream(&ctx.sender);
    assert_eq!(id0, 0u64);
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());

    // Live counter is at 1 (reservation advanced it)
    let id1 = ctx.create_stream(&ctx.sender);
    assert_eq!(id1, 1u64);
}

#[test]
fn new_reservation_overwrites_existing() {
    let ctx = Ctx::setup();
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None); // IDs 0..4
    let ids2 = ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None); // IDs 5..9
    assert_eq!(ids2.get(0).unwrap(), 5u64);

    let res = ctx.client.get_id_reservation(&ctx.sender).unwrap();
    assert_eq!(res.start_id, 5);
    assert_eq!(res.consumed, 0);

    let id = ctx.create_stream(&ctx.sender);
    assert_eq!(id, 5u64);
}

#[test]
fn reservation_advances_stream_count_by_full_count() {
    let ctx = Ctx::setup();
    ctx.client.reserve_stream_ids(&ctx.sender, &10u32, &None);
    // Only consume 1
    let id = ctx.create_stream(&ctx.sender);
    assert_eq!(id, 0u64);
    // Counter was advanced by 10, not 1
    assert_eq!(ctx.client.get_stream_count(), 10);
}

#[test]
fn different_callers_get_independent_reservations() {
    let ctx = Ctx::setup();
    let sender2 = Address::generate(&ctx.env);
    ctx.mint(&sender2);

    ctx.client.reserve_stream_ids(&ctx.sender, &3u32, &None);
    ctx.client.reserve_stream_ids(&sender2, &3u32, &None);

    let id_s1 = ctx.create_stream(&ctx.sender);
    let id_s2 = ctx.create_stream(&sender2);

    assert_eq!(id_s1, 0u64);
    assert_eq!(id_s2, 3u64);
}

#[test]
fn reserve_after_existing_streams_starts_at_current_count() {
    let ctx = Ctx::setup();
    // Create 2 streams without reservation
    ctx.create_stream(&ctx.sender);
    ctx.create_stream(&ctx.sender);
    assert_eq!(ctx.client.get_stream_count(), 2);

    // Reserve 3 more — should start at 2
    let ids = ctx.client.reserve_stream_ids(&ctx.sender, &3u32, &None);
    assert_eq!(ids.get(0).unwrap(), 2u64);
    assert_eq!(ids.get(2).unwrap(), 4u64);
    assert_eq!(ctx.client.get_stream_count(), 5);
}

// ---------------------------------------------------------------------------
// reclaim_expired_id_reservation tests
// ---------------------------------------------------------------------------

/// Expiry Boundary Semantics:
/// - A reservation has an optional Time-To-Live (TTL) defined by the `expiry` timestamp.
/// - If `current_timestamp < expiry`, the reservation is active and cannot be reclaimed.
/// - If `current_timestamp >= expiry`, the reservation has expired, and anyone is permitted
///   to trigger reclamation of the reserved IDs to free storage slot and prevent counter blockage.
///
/// Why reclaim is only permitted after expiry:
/// - To protect the reservation holder's exclusive right to use their pre-allocated ID space.
/// - Preventing premature reclamation ensures that off-chain pre-computation pipelines are not
///   invalidated by third parties while the reservation is legally active.
///
/// Security Rationale:
/// - Pre-expiry rejection: Blocks denial-of-service (DoS) or front-running attacks where an attacker
///   reclaims a user's reservation before they can publish their streams.
/// - At-expiry & post-expiry success: Ensures that if a holder abandons or loses access to their
///   reservation, the counter space/storage is not permanently locked, maintaining contract liveness.
/// - Nonexistent reservation rejection: Prevents garbage state modifications or execution of release code paths
///   for addresses without a reservation.
/// - Double-reclaim prevention: After successful reclamation, the reservation is permanently deleted,
///   preventing replay or duplicate release operations.
#[test]
fn test_reclaim_before_expiry_errors() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let expiry = now + 100;

    // Reserve with expiry = Some(expiry)
    ctx.client
        .reserve_stream_ids(&ctx.sender, &5u32, &Some(expiry));

    // Attempt reclaim at now + 50 (pre-expiry)
    ctx.env.ledger().set_timestamp(now + 50);
    let result = ctx.client.try_reclaim_expired_id_reservation(&ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::ReservationStillActive)));
}

#[test]
fn test_reclaim_exactly_at_expiry_succeeds() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let expiry = now + 100;

    // Reserve with expiry = Some(expiry)
    ctx.client
        .reserve_stream_ids(&ctx.sender, &5u32, &Some(expiry));

    // Reclaim exactly at the expiry boundary
    ctx.env.ledger().set_timestamp(expiry);
    ctx.client.reclaim_expired_id_reservation(&ctx.sender);

    // Check that reservation is released
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());
}

#[test]
fn test_reclaim_after_expiry_succeeds() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let expiry = now + 100;

    // Reserve with expiry = Some(expiry)
    ctx.client
        .reserve_stream_ids(&ctx.sender, &5u32, &Some(expiry));

    // Reclaim after expiry
    ctx.env.ledger().set_timestamp(expiry + 1);
    ctx.client.reclaim_expired_id_reservation(&ctx.sender);

    // Check that reservation is released
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());
}

#[test]
fn test_reclaim_nonexistent_reservation_errors() {
    let ctx = Ctx::setup();
    let result = ctx.client.try_reclaim_expired_id_reservation(&ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::ReservationNotFound)));
}

#[test]
fn test_reclaim_twice_errors() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let expiry = now + 100;

    // Reserve with expiry = Some(expiry)
    ctx.client
        .reserve_stream_ids(&ctx.sender, &5u32, &Some(expiry));

    // Reclaim first time (succeeds)
    ctx.env.ledger().set_timestamp(expiry);
    ctx.client.reclaim_expired_id_reservation(&ctx.sender);

    // Reclaim second time (errors as it was already deleted)
    let result = ctx.client.try_reclaim_expired_id_reservation(&ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::ReservationNotFound)));
}
