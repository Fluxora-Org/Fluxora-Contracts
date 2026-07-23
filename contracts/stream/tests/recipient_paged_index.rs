//! Regression tests for the bounded `get_recipient_streams` entry-point.
//!
//! `get_recipient_streams` is hard-capped at `RECIPIENT_STREAMS_PAGE_LIMIT`
//! (= `MAX_RECIPIENT_PAGE_SIZE`) to prevent unbounded-read DoS.  These tests
//! verify:
//!
//! * Zero streams  → empty result.
//! * Exactly cap streams  → full result, all IDs present.
//! * cap + 1 streams (regression)  → exactly cap IDs returned, never more.
//! * Full enumeration via `get_recipient_streams_paginated`.

extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient, StreamKind, MAX_RECIPIENT_PAGE_SIZE};
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
// Edge-case: zero streams
// ---------------------------------------------------------------------------

#[test]
fn test_zero_streams_returns_empty() {
    let ctx = Ctx::setup();
    let ids = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(ids.len(), 0);
}

// ---------------------------------------------------------------------------
// Backward-compatible: small recipient (count < cap)
// ---------------------------------------------------------------------------

#[test]
fn test_small_recipient_returns_all() {
    let ctx = Ctx::setup();
    ctx.create_n(5);
    let ids = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(ids.len(), 5);
}

// ---------------------------------------------------------------------------
// Boundary: exactly cap streams → all returned
// ---------------------------------------------------------------------------

#[test]
fn test_exactly_cap_streams_returns_all() {
    let ctx = Ctx::setup();
    ctx.create_n(MAX_RECIPIENT_PAGE_SIZE);
    let ids = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(
        ids.len(),
        MAX_RECIPIENT_PAGE_SIZE,
        "expected full cap returned when count == cap"
    );
}

// ---------------------------------------------------------------------------
// Regression: cap + 1 streams must NOT exceed the cap
// ---------------------------------------------------------------------------

#[test]
fn test_cap_plus_one_is_bounded() {
    let ctx = Ctx::setup();
    ctx.create_n(MAX_RECIPIENT_PAGE_SIZE + 1);

    let ids = ctx.client.get_recipient_streams(&ctx.recipient);

    assert_eq!(
        ids.len(),
        MAX_RECIPIENT_PAGE_SIZE,
        "get_recipient_streams must never exceed RECIPIENT_STREAMS_PAGE_LIMIT"
    );
}

// ---------------------------------------------------------------------------
// Regression: high-volume recipient (2× cap + 5) stays bounded
// ---------------------------------------------------------------------------

#[test]
fn test_high_volume_recipient_is_bounded() {
    let ctx = Ctx::setup();
    let total = MAX_RECIPIENT_PAGE_SIZE * 2 + 5;
    ctx.create_n(total);

    let ids = ctx.client.get_recipient_streams(&ctx.recipient);

    assert!(
        ids.len() <= MAX_RECIPIENT_PAGE_SIZE,
        "returned {} IDs, expected at most {}",
        ids.len(),
        MAX_RECIPIENT_PAGE_SIZE,
    );
}

// ---------------------------------------------------------------------------
// Result is a prefix: IDs returned by the bounded call are the first page
// ---------------------------------------------------------------------------

#[test]
fn test_bounded_call_returns_first_page() {
    let ctx = Ctx::setup();
    ctx.create_n(MAX_RECIPIENT_PAGE_SIZE + 10);

    let bounded = ctx.client.get_recipient_streams(&ctx.recipient);
    let page =
        ctx.client
            .get_recipient_streams_paginated(&ctx.recipient, &0, &MAX_RECIPIENT_PAGE_SIZE);

    assert_eq!(bounded.len(), page.stream_ids.len());
    for i in 0..bounded.len() {
        assert_eq!(bounded.get(i).unwrap(), page.stream_ids.get(i).unwrap());
    }
}

// ---------------------------------------------------------------------------
// Full enumeration via pagination covers all streams
// ---------------------------------------------------------------------------

#[test]
fn test_paginated_covers_all_streams() {
    let ctx = Ctx::setup();
    let total = MAX_RECIPIENT_PAGE_SIZE + 15;
    ctx.create_n(total);

    let mut all_ids = soroban_sdk::Vec::new(&ctx.env);
    let mut cursor = 0u64;
    loop {
        let page = ctx.client.get_recipient_streams_paginated(
            &ctx.recipient,
            &cursor,
            &MAX_RECIPIENT_PAGE_SIZE,
        );
        for i in 0..page.stream_ids.len() {
            all_ids.push_back(page.stream_ids.get(i).unwrap());
        }
        cursor = page.next_cursor;
        if cursor == 0 {
            break;
        }
    }

    assert_eq!(
        all_ids.len(),
        total,
        "pagination must enumerate every stream"
    );
}

// ---------------------------------------------------------------------------
// IDs are sorted ascending in both interfaces
// ---------------------------------------------------------------------------

#[test]
fn test_bounded_ids_are_sorted() {
    let ctx = Ctx::setup();
    ctx.create_n(MAX_RECIPIENT_PAGE_SIZE + 3);

    let ids = ctx.client.get_recipient_streams(&ctx.recipient);
    for i in 1..ids.len() {
        assert!(
            ids.get(i - 1).unwrap() < ids.get(i).unwrap(),
            "IDs must be sorted ascending"
        );
    }
}
