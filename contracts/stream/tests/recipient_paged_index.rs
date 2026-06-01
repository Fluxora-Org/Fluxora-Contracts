extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient, MAX_RECIPIENT_PAGE_SIZE};
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct TestContext {
    env: Env,
    client: FluxoraStreamClient<'static>,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'static>,
}

impl TestContext {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract(token_admin.clone());
        let token = TokenClient::new(&env, &token_id);
        let token_asset = StellarAssetClient::new(&env, &token_id);
        token_asset.mint(&sender, &1_000_000_000);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        token.approve(&sender, &contract_id, &1_000_000_000, &100000);

        client.init(&token_id, &admin);

        Self {
            env,
            client,
            admin,
            sender,
            recipient,
            token,
        }
    }
}

#[test]
fn test_recipient_index_migration() {
    let ctx = TestContext::setup();

    // 1. Create 5 streams (flat list)
    for _ in 0..5 {
        ctx.client.create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000,
            &1,
            &0,
            &0,
            &1000,
            &0,
            &None,
            &fluxora_stream::StreamKind::Linear,
            );
    }

    let streams = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 5);

    // 2. Migrate
    ctx.client.migrate_recipient_index(&ctx.recipient);

    // 3. Verify streams still accessible
    let streams_after = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(streams_after.len(), 5);
    assert_eq!(streams, streams_after);

    // 4. Create another stream (should go to paged index)
    ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1,
        &0,
        &0,
        &1000,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let streams_final = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(streams_final.len(), 6);
}

#[test]
fn test_paged_index_pagination() {
    let ctx = TestContext::setup();
    ctx.env.budget().reset_unlimited();

    // Create many streams to force multiple pages
    let total_streams = (MAX_RECIPIENT_PAGE_SIZE * 2 + 5) as i32;
    for _ in 0..total_streams {
        ctx.client.create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100,
            &1,
            &0,
            &0,
            &100,
            &0,
            &None,
            &fluxora_stream::StreamKind::Linear,
            );
    }

    // Migrate to paged index
    ctx.client.migrate_recipient_index(&ctx.recipient);

    // Test pagination across page boundaries
    let limit = 10;

    // First page
    let page1 = ctx
        .client
        .get_recipient_streams_paginated(&ctx.recipient, &0, &limit);
    assert_eq!(page1.len(), 10);
    // Stream IDs start from 0 (NextStreamId is initialized to 0 in init)
    assert_eq!(page1.get(0).unwrap(), 0);

    // Boundary of page 0 and 1
    let cursor = (MAX_RECIPIENT_PAGE_SIZE - 5) as u64;
    let page_boundary = ctx
        .client
        .get_recipient_streams_paginated(&ctx.recipient, &cursor, &10);
    assert_eq!(page_boundary.len(), 10);

    // Verify results match full list
    let all = ctx.client.get_recipient_streams(&ctx.recipient);
    for i in 0..10 {
        assert_eq!(
            page_boundary.get(i as u32).unwrap(),
            all.get((cursor + i) as u32).unwrap()
        );
    }
}

#[test]
fn test_remove_from_paged_index() {
    let ctx = TestContext::setup();

    // Create 3 streams
    let id1 = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100,
        &1,
        &0,
        &0,
        &100,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );
    let id2 = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100,
        &1,
        &0,
        &0,
        &100,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );
    let id3 = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100,
        &1,
        &0,
        &0,
        &100,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    ctx.client.migrate_recipient_index(&ctx.recipient);

    // Cancel and close stream 2 (should remove from paged index)
    ctx.client.cancel_stream(&id2);
    ctx.client.close_completed_stream(&id2);

    let streams = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 2);
    assert!(streams.contains(id1));
    assert!(streams.contains(id3));
    assert!(!streams.contains(id2));
}
