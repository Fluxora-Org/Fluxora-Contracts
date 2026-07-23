//! Integration tests for `FluxoraStream::get_streams_by_id_range` edge cases.
//!
//! Enforces:
//! - Holey ranges containing closed or never-created IDs skip missing IDs.
//! - Limits above the `MAX_PAGE_SIZE` cap are clamped to the cap.
//! - Start IDs beyond the maximum/highest active ID return empty vectors.
//! - Zero limits return empty vectors.
//!
//! These tests are critical for ensuring stable and bounded results for operator migration.

extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient, StreamKind, MAX_PAGE_SIZE};
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct Ctx {
    env: Env,
    client: FluxoraStreamClient<'static>,
    sender: Address,
    recipient: Address,
}

impl Ctx {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.budget().reset_unlimited();

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        StellarAssetClient::new(&env, &token_id).mint(&Address::generate(&env), &0);

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_000_000_000);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        client.init(&token_id, &admin);

        TokenClient::new(&env, &token_id).approve(
            &sender,
            &contract_id,
            &1_000_000_000_000,
            &9_999_999,
        );

        // Safety: env lives as long as the returned Ctx; we only hold one Ctx at a time.
        let client: FluxoraStreamClient<'static> = unsafe { core::mem::transmute(client) };

        Ctx {
            env,
            client,
            sender,
            recipient,
        }
    }

    /// Create one minimal stream for `self.recipient` and return its ID.
    fn create_one(&self) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client
            .create_stream(
                &self.sender,
                &self.recipient,
                &100,
                &1,
                &now,
                &now,
                &(now + 100),
                &0,
                &None,
                &StreamKind::Linear,
            )
            .unwrap()
    }

    /// Create `n` streams for `self.recipient`.
    fn create_n(&self, n: u32) {
        for _ in 0..n {
            self.create_one();
        }
    }
}

// ---------------------------------------------------------------------------
// Edge Cases for get_streams_by_id_range
// ---------------------------------------------------------------------------

/// Edge Case 1: Holey ranges containing closed or never-created IDs must skip missing IDs.
///
/// Steps:
/// 1. Create 3 streams (IDs: 1, 2, 3).
/// 2. Fast-forward the ledger time so stream 2 finishes.
/// 3. Withdraw stream 2 fully and close it (this deletes the stream from storage, leaving a hole).
/// 4. Verify that querying range [1, 3] returns only streams 1 and 3, skipping stream 2.
#[test]
fn test_get_streams_by_id_range_holey_ranges() {
    let ctx = Ctx::setup();
    let id1 = ctx.create_one();
    let id2 = ctx.create_one();
    let id3 = ctx.create_one();

    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(id3, 3);

    // Fast-forward ledger time to let stream 2 complete (duration is 100 sec)
    let now = ctx.env.ledger().timestamp();
    ctx.env.ledger().set_timestamp(now + 101);

    // Withdraw stream 2 fully to make it complete-able
    ctx.client.withdraw(&id2);

    // Close the completed stream (removes it from storage)
    ctx.client.close_completed_stream(&id2);

    // Query range [1, 3] with limit 10
    // Result should only contain streams 1 and 3, skipping the hole at ID 2
    let streams = ctx.client.get_streams_by_id_range(&1, &3, &10);
    assert_eq!(streams.len(), 2, "Hole at ID 2 must be skipped");
    assert_eq!(streams.get(0).unwrap().stream_id, 1);
    assert_eq!(streams.get(1).unwrap().stream_id, 3);
}

/// Edge Case 2: Over-cap limits must be clamped.
///
/// Steps:
/// 1. Create MAX_PAGE_SIZE + 10 streams (110 streams total).
/// 2. Call `get_streams_by_id_range` with limit = MAX_PAGE_SIZE + 5 (105).
/// 3. Assert that the returned vector length is clamped exactly to MAX_PAGE_SIZE (100).
#[test]
fn test_get_streams_by_id_range_limit_clamping() {
    let ctx = Ctx::setup();
    ctx.create_n((MAX_PAGE_SIZE + 10) as u32);

    // Request limit 105, which is above MAX_PAGE_SIZE (100). It must be clamped.
    let over_cap_limit = MAX_PAGE_SIZE + 5;
    let streams = ctx
        .client
        .get_streams_by_id_range(&1, &(MAX_PAGE_SIZE + 10), &over_cap_limit);

    assert_eq!(
        streams.len(),
        MAX_PAGE_SIZE as u32,
        "Limit must be clamped to MAX_PAGE_SIZE (100)"
    );
}

/// Edge Case 3: Start beyond the max/highest stream ID returns empty.
///
/// Steps:
/// 1. Create 3 streams.
/// 2. Query range with start_id = 4, end_id = 10, limit = 5.
/// 3. Assert that the returned vector is empty.
#[test]
fn test_get_streams_by_id_range_start_beyond_max() {
    let ctx = Ctx::setup();
    ctx.create_n(3);

    // Stream IDs 1, 2, and 3 are created.
    // Query starting at 4 (beyond max) should return empty.
    let streams = ctx.client.get_streams_by_id_range(&4, &10, &5);
    assert_eq!(
        streams.len(),
        0,
        "Start ID beyond max stream ID must return an empty list"
    );
}

/// Edge Case 4: Zero limit returns empty.
///
/// Steps:
/// 1. Create 3 streams.
/// 2. Query range [1, 3] with limit = 0.
/// 3. Assert that the returned vector is empty.
#[test]
fn test_get_streams_by_id_range_zero_limit() {
    let ctx = Ctx::setup();
    ctx.create_n(3);

    let streams = ctx.client.get_streams_by_id_range(&1, &3, &0);
    assert_eq!(streams.len(), 0, "Limit of 0 must return an empty list");
}
