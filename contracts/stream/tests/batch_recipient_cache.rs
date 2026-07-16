//! Tests for issue #514: recipient stream index caching in `create_streams`.
//!
//! Verifies that batching multiple streams to the same recipient produces the
//! same index state as creating them one-by-one, and that the O(1)-per-recipient
//! flush path is correct for mixed-recipient batches.

use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{testutils::Address as _, token::Client as TokenClient, vec, Address, Env};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);

        // Mint enough tokens for tests
        stellar_asset.mint(&sender, &1_000_000_000);
        // The contract pulls deposits via `transfer_from`, which requires an
        // allowance from `sender`. Missing here previously (pre-existing bug,
        // predates this work): every `create_streams` call failed with
        // `ContractError::InsufficientBalance` since `sender` never granted
        // the contract an allowance.
        token.approve(&sender, &contract_id, &1_000_000_000, &100_000);

        client.init(&token_id, &admin);

        Self {
            env,
            client,
            sender,
            token,
        }
    }

    fn make_params(&self, recipient: &Address, deposit: i128, duration: u64) -> CreateStreamParams {
        let now = self.env.ledger().timestamp();
        CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: deposit / duration as i128,
            start_time: now,
            cliff_time: now,
            end_time: now + duration,
            withdraw_dust_threshold: None,
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
        }
    }
}

/// Batch with all streams going to the same recipient: index must contain all IDs in sorted order.
#[test]
fn test_batch_same_recipient_index_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    let params = vec![
        &ctx.env,
        ctx.make_params(&recipient, 1_000, 1_000),
        ctx.make_params(&recipient, 2_000, 2_000),
        ctx.make_params(&recipient, 3_000, 3_000),
    ];

    let ids = ctx.client.create_streams(&ctx.sender, &params);
    assert_eq!(ids.len(), 3);

    // All three IDs must appear in the recipient's index
    let index = ctx.client.get_recipient_streams(&recipient);
    assert_eq!(index.len(), 3);
    for id in ids.iter() {
        assert!(index.contains(id));
    }
}

/// Batch with distinct recipients: each recipient's index contains exactly their stream IDs.
#[test]
fn test_batch_distinct_recipients_index_correct() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);

    let params = vec![
        &ctx.env,
        ctx.make_params(&alice, 1_000, 1_000),
        ctx.make_params(&bob, 2_000, 2_000),
        ctx.make_params(&alice, 3_000, 3_000),
    ];

    let ids = ctx.client.create_streams(&ctx.sender, &params);
    assert_eq!(ids.len(), 3);

    let alice_index = ctx.client.get_recipient_streams(&alice);
    let bob_index = ctx.client.get_recipient_streams(&bob);

    assert_eq!(alice_index.len(), 2);
    assert_eq!(bob_index.len(), 1);

    // Alice gets stream 0 and 2, Bob gets stream 1
    assert!(alice_index.contains(ids.get(0).unwrap()));
    assert!(alice_index.contains(ids.get(2).unwrap()));
    assert!(bob_index.contains(ids.get(1).unwrap()));
}

/// Cached batch result matches sequential single-stream creation for the same recipient.
#[test]
fn test_batch_index_matches_sequential_creation() {
    // Sequential: create streams one by one
    let ctx1 = Ctx::setup();
    let recipient1 = Address::generate(&ctx1.env);
    let p1 = ctx1.make_params(&recipient1, 1_000, 1_000);
    let p2 = ctx1.make_params(&recipient1, 2_000, 2_000);
    ctx1.client.create_stream(
        &ctx1.sender,
        &p1.recipient,
        &p1.deposit_amount,
        &p1.rate_per_second,
        &p1.start_time,
        &p1.cliff_time,
        &p1.end_time,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
    );
    ctx1.client.create_stream(
        &ctx1.sender,
        &p2.recipient,
        &p2.deposit_amount,
        &p2.rate_per_second,
        &p2.start_time,
        &p2.cliff_time,
        &p2.end_time,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
    );
    let seq_index = ctx1.client.get_recipient_streams(&recipient1);

    // Batch: create both streams in one call
    let ctx2 = Ctx::setup();
    let recipient2 = Address::generate(&ctx2.env);
    let q1 = ctx2.make_params(&recipient2, 1_000, 1_000);
    let q2 = ctx2.make_params(&recipient2, 2_000, 2_000);
    ctx2.client
        .create_streams(&ctx2.sender, &vec![&ctx2.env, q1, q2]);
    let batch_index = ctx2.client.get_recipient_streams(&recipient2);

    // Both should have 2 streams
    assert_eq!(seq_index.len(), batch_index.len());
    assert_eq!(seq_index.len(), 2);
}

/// Empty batch returns empty vec and does not touch the index.
#[test]
fn test_batch_empty_no_index_change() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    let result = ctx.client.create_streams(&ctx.sender, &vec![&ctx.env]);
    assert_eq!(result.len(), 0);

    let index = ctx.client.get_recipient_streams(&recipient);
    assert_eq!(index.len(), 0);
}

/// Batch of 1 stream behaves identically to a single create_stream call.
#[test]
fn test_batch_single_entry_same_as_create_stream() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let params = ctx.make_params(&recipient, 5_000, 5_000);

    let ids = ctx
        .client
        .create_streams(&ctx.sender, &vec![&ctx.env, params]);
    assert_eq!(ids.len(), 1);

    let index = ctx.client.get_recipient_streams(&recipient);
    assert_eq!(index.len(), 1);
    assert_eq!(index.get(0).unwrap(), ids.get(0).unwrap());
}

/// Index is sorted after a batch with same recipient (IDs are monotonically increasing so order is preserved).
#[test]
fn test_batch_index_sorted_order() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    let params = vec![
        &ctx.env,
        ctx.make_params(&recipient, 1_000, 1_000),
        ctx.make_params(&recipient, 2_000, 2_000),
        ctx.make_params(&recipient, 3_000, 3_000),
        ctx.make_params(&recipient, 4_000, 4_000),
    ];

    let ids = ctx.client.create_streams(&ctx.sender, &params);
    let index = ctx.client.get_recipient_streams(&recipient);

    assert_eq!(index.len(), 4);
    // Verify sorted order
    for i in 0..index.len() - 1 {
        assert!(index.get(i).unwrap() < index.get(i + 1).unwrap());
    }
    // All created IDs present
    for id in ids.iter() {
        assert!(index.contains(id));
    }
}

/// Stream count is correct after a batch (no double-counting from cache flush).
#[test]
fn test_batch_stream_count_correct() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let count_before = ctx.client.get_stream_count();

    let params = vec![
        &ctx.env,
        ctx.make_params(&recipient, 1_000, 1_000),
        ctx.make_params(&recipient, 2_000, 2_000),
    ];

    ctx.client.create_streams(&ctx.sender, &params);
    assert_eq!(ctx.client.get_stream_count(), count_before + 2);
}
