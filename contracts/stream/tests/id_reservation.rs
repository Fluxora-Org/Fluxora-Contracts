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
fn new_reservation_fails_if_active() {
    let ctx = Ctx::setup();
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None); // IDs 0..4
    let result = ctx.client.try_reserve_stream_ids(&ctx.sender, &5u32, &None);
    assert_eq!(result, Err(Ok(ContractError::ReservationAlreadyActive)));

    let res = ctx.client.get_id_reservation(&ctx.sender).unwrap();
    assert_eq!(res.start_id, 0);
    assert_eq!(res.consumed, 0);
}

#[test]
fn reserve_stream_ids_overwrites_unreleased_reservation_leaking_ids_regression() {
    // Regression test for #1021: Pin the current (leaky) behavior where a second 
    // reservation by the same caller overwrites their first unreleased reservation, 
    // leaking the first block of IDs permanently.
    //
    // Security Rationale / NatSpec:
    // @dev Currently `save_id_reservation` unconditionally overwrites the single
    // `DataKey::IdReservation(Address)` entry. The first reservation's ID range 
    // becomes permanently unreachable via any public entrypoint.
    // Expected to be updated in a future PR to assert `ContractError::ReservationAlreadyActive`.
    let ctx = Ctx::setup();
    
    // Caller reserves 10 IDs (0..9)
    ctx.client.reserve_stream_ids(&ctx.sender, &10u32, &None);
    
    // Caller reserves 5 more IDs without releasing the first (10..14)
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None);
    
    // Assert get_id_reservation(caller) returns only the second reservation
    let res = ctx.client.get_id_reservation(&ctx.sender).unwrap();
    assert_eq!(res.start_id, 10);
    assert_eq!(res.count, 5);
    assert_eq!(res.consumed, 0);
}

#[test]
fn next_stream_id_reflects_both_bumps_on_overwrite_regression() {
    // Companion test for #1021 proving that when a reservation is overwritten,
    // the global NextStreamId counter still correctly reflects both reservations' sizes.
    // This proves the leak is isolated to tracking, and the counter itself is safely advanced.
    let ctx = Ctx::setup();
    
    // Counter initially 0
    assert_eq!(ctx.client.get_stream_count(), 0);
    
    // Reserve 10 IDs -> counter advances to 10
    ctx.client.reserve_stream_ids(&ctx.sender, &10u32, &None);
    assert_eq!(ctx.client.get_stream_count(), 10);
    
    // Reserve 5 more IDs -> counter advances to 15
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None);
    
    // Assert counter reflects both bumps (10 + 5 = 15)
    assert_eq!(ctx.client.get_stream_count(), 15);
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

// ---------------------------------------------------------------------------
// release_id_reservation tip-adjacent reclamation (issue #1007)
// ---------------------------------------------------------------------------

/// Regression test for #1007: release_id_reservation now reclaims tip-adjacent
/// unused IDs, matching reclaim_expired_id_reservation behavior.
///
/// Security Rationale / NatSpec:
/// @dev Before #1007, release_id_reservation called remove_id_reservation directly
/// without invoking the release_reservation helper. This permanently orphaned
/// tip-adjacent ID ranges even when fully unconsumed. The fix routes through
/// release_reservation which checks reservation_end == current_count and rewinds
/// the counter when the reservation is at the tip and fully unconsumed.
#[test]
fn test_release_id_reservation_shrinks_next_stream_id_when_tip_adjacent() {
    let ctx = Ctx::setup();

    // NextStreamId is 0. Reserve 5. NextStreamId becomes 5.
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None);
    assert_eq!(ctx.client.get_stream_count(), 5);

    // Release immediately via release_id_reservation
    ctx.client.release_id_reservation(&ctx.sender);

    // Reservation is gone
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());

    // Counter rewinds. Next stream gets ID 0.
    let id = ctx.create_stream(&ctx.sender);
    assert_eq!(id, 0);
}

#[test]
fn test_reclaim_expired_id_reservation_shrinks_next_stream_id() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let expiry = now + 100;

    // NextStreamId is 0. Reserve 5. NextStreamId becomes 5.
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &Some(expiry));
    assert_eq!(ctx.client.get_stream_count(), 5);

    // Reclaim after advancing ledger past expiry
    ctx.env.ledger().set_timestamp(expiry + 1);
    ctx.client.reclaim_expired_id_reservation(&ctx.sender);

    // Reservation is gone
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());

    // Counter rewinds. Next stream gets ID 0.
    let id = ctx.create_stream(&ctx.sender);
    assert_eq!(id, 0);
}

// ---------------------------------------------------------------------------
// release_id_reservation edge cases
// ---------------------------------------------------------------------------

/// When a reservation is not tip-adjacent (IDs were consumed by later create_stream
/// calls that pushed the counter past reservation_end), release_id_reservation
/// must NOT rewind the counter. It should just remove the reservation record.
#[test]
fn test_release_id_reservation_no_rewind_when_not_tip_adjacent() {
    let ctx = Ctx::setup();

    // Reserve 3 IDs (0..2). NextStreamId = 3.
    ctx.client.reserve_stream_ids(&ctx.sender, &3u32, &None);
    assert_eq!(ctx.client.get_stream_count(), 3);

    // Consume all 3 reserved IDs.
    let id0 = ctx.create_stream(&ctx.sender);
    let id1 = ctx.create_stream(&ctx.sender);
    let id2 = ctx.create_stream(&ctx.sender);
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(ctx.client.get_stream_count(), 3);

    // Reservation is now fully consumed and removed by create_stream.
    // Trying to release should error since reservation is already gone.
    let result = ctx.client.try_release_id_reservation(&ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::ReservationNotFound)));
}

/// When a reservation is tip-adjacent but partially consumed, release_id_reservation
/// rewinds the counter to the first unconsumed ID. The `release_reservation` helper
/// checks reservation_end == current_count (tip-adjacent) and if unconsumed IDs
/// exist at the tip, rewinds to unconsumed_start. Since IDs 2..4 are unconsumed
/// and at the counter tip, rewinding to 2 is safe (no stream uses those IDs).
#[test]
fn test_release_id_reservation_no_rewind_when_partially_consumed() {
    let ctx = Ctx::setup();

    // Reserve 5 IDs (0..4). NextStreamId = 5.
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None);
    assert_eq!(ctx.client.get_stream_count(), 5);

    // Consume 2 of the 5 reserved IDs.
    let id0 = ctx.create_stream(&ctx.sender);
    let id1 = ctx.create_stream(&ctx.sender);
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);

    // Release via release_id_reservation
    ctx.client.release_id_reservation(&ctx.sender);

    // Reservation is gone
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());

    // Counter rewinds to unconsumed_start = 2. Next stream gets ID 2.
    let id = ctx.create_stream(&ctx.sender);
    assert_eq!(id, 2);
}

/// When a non-tip-adjacent reservation is released, the event is still emitted
/// with reclaimed = 0, maintaining consistent indexer accounting.
#[test]
fn test_release_id_reservation_emits_event_with_zero_reclaimed_when_not_tip_adjacent() {
    let ctx = Ctx::setup();

    // Reserve 3 IDs (0..2). NextStreamId = 3.
    ctx.client.reserve_stream_ids(&ctx.sender, &3u32, &None);

    // Consume all 3.
    ctx.create_stream(&ctx.sender);
    ctx.create_stream(&ctx.sender);
    ctx.create_stream(&ctx.sender);

    // Another caller reserves 3 more (3..5). Pushes counter to 6.
    let sender2 = Address::generate(&ctx.env);
    ctx.mint(&sender2);
    ctx.client.reserve_stream_ids(&sender2, &3u32, &None);
    assert_eq!(ctx.client.get_stream_count(), 6);

    // sender2's reservation is tip-adjacent (reservation_end=6, current_count=6).
    // sender2 releases: counter should rewind from 6 to 3.
    ctx.client.release_id_reservation(&sender2);
    assert!(ctx.client.get_id_reservation(&sender2).is_none());

    // Next stream gets ID 3 (rewound).
    let id = ctx.create_stream(&ctx.sender);
    assert_eq!(id, 3);
}

/// release_id_reservation errors when no reservation exists for the caller.
#[test]
fn test_release_id_reservation_nonexistent_errors() {
    let ctx = Ctx::setup();
    let result = ctx.client.try_release_id_reservation(&ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::ReservationNotFound)));
}

/// Double release_id_reservation errors on second call.
#[test]
fn test_release_id_reservation_double_release_errors() {
    let ctx = Ctx::setup();
    ctx.client.reserve_stream_ids(&ctx.sender, &3u32, &None);

    // First release succeeds
    ctx.client.release_id_reservation(&ctx.sender);
    assert!(ctx.client.get_id_reservation(&ctx.sender).is_none());

    // Second release errors
    let result = ctx.client.try_release_id_reservation(&ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::ReservationNotFound)));
}

/// After voluntary release with reclamation, the counter is correctly reused
/// by a subsequent reservation.
#[test]
fn test_release_id_reservation_counter_reused_by_subsequent_reservation() {
    let ctx = Ctx::setup();

    // Reserve 5. Counter = 5.
    ctx.client.reserve_stream_ids(&ctx.sender, &5u32, &None);
    assert_eq!(ctx.client.get_stream_count(), 5);

    // Release. Counter rewinds to 0.
    ctx.client.release_id_reservation(&ctx.sender);
    assert_eq!(ctx.client.get_stream_count(), 0);

    // Reserve 3. Counter advances to 3, starting from 0.
    let ids = ctx.client.reserve_stream_ids(&ctx.sender, &3u32, &None);
    assert_eq!(ids.get(0).unwrap(), 0);
    assert_eq!(ids.get(2).unwrap(), 2);
    assert_eq!(ctx.client.get_stream_count(), 3);
}

/// Verify that two independent callers releasing tip-adjacent reservations
/// each independently reclaim their ranges.
#[test]
fn test_two_callers_release_independently_reclaim() {
    let ctx = Ctx::setup();
    let sender2 = Address::generate(&ctx.env);
    ctx.mint(&sender2);

    // Caller 1 reserves 3 (0..2). Counter = 3.
    ctx.client.reserve_stream_ids(&ctx.sender, &3u32, &None);
    // Caller 2 reserves 3 (3..5). Counter = 6.
    ctx.client.reserve_stream_ids(&sender2, &3u32, &None);
    assert_eq!(ctx.client.get_stream_count(), 6);

    // Caller 2 releases first. Tip-adjacent: rewinds from 6 to 3.
    ctx.client.release_id_reservation(&sender2);
    assert_eq!(ctx.client.get_stream_count(), 3);

    // Caller 1 releases. Tip-adjacent: rewinds from 3 to 0.
    ctx.client.release_id_reservation(&ctx.sender);
    assert_eq!(ctx.client.get_stream_count(), 0);

    // Next stream gets ID 0.
    let id = ctx.create_stream(&ctx.sender);
    assert_eq!(id, 0);
}
