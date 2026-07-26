//! Tests for issue #514: recipient stream index caching in `create_streams`.
//!
//! Verifies that batching multiple streams to the same recipient produces the
//! same index state as creating them one-by-one, and that the O(1)-per-recipient
//! flush path is correct for mixed-recipient batches.
//!
//! Also covers cross-stream isolation: the in-batch `Map<Address, Vec<u64>>`
//! must be keyed by recipient so interleaved multi-recipient batches never
//! attribute a stream ID to the wrong recipient index (fund-safety / discovery).

use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    vec, Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
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

/// Three distinct recipients interleaved (A,B,C,A,B,C) — exposes coarse cache keys.
///
/// If the in-batch cache were keyed only by position, last-seen recipient, or any
/// non-Address discriminator, stream IDs would land in the wrong recipient index.
#[test]
fn test_batch_three_recipients_interleaved_index_isolation() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);
    let carol = Address::generate(&ctx.env);

    // Interleave three recipients twice so each appears non-contiguously.
    let params = vec![
        &ctx.env,
        ctx.make_params(&alice, 1_000, 1_000),
        ctx.make_params(&bob, 2_000, 2_000),
        ctx.make_params(&carol, 3_000, 3_000),
        ctx.make_params(&alice, 4_000, 4_000),
        ctx.make_params(&bob, 5_000, 5_000),
        ctx.make_params(&carol, 6_000, 6_000),
    ];

    let ids = ctx.client.create_streams(&ctx.sender, &params);
    assert_eq!(ids.len(), 6);

    let id0 = ids.get(0).unwrap();
    let id1 = ids.get(1).unwrap();
    let id2 = ids.get(2).unwrap();
    let id3 = ids.get(3).unwrap();
    let id4 = ids.get(4).unwrap();
    let id5 = ids.get(5).unwrap();

    // Persisted stream.recipient must match the params entry (not a stale cache value).
    assert_eq!(ctx.client.get_stream_state(&id0).recipient, alice);
    assert_eq!(ctx.client.get_stream_state(&id1).recipient, bob);
    assert_eq!(ctx.client.get_stream_state(&id2).recipient, carol);
    assert_eq!(ctx.client.get_stream_state(&id3).recipient, alice);
    assert_eq!(ctx.client.get_stream_state(&id4).recipient, bob);
    assert_eq!(ctx.client.get_stream_state(&id5).recipient, carol);

    let alice_index = ctx.client.get_recipient_streams(&alice);
    let bob_index = ctx.client.get_recipient_streams(&bob);
    let carol_index = ctx.client.get_recipient_streams(&carol);

    assert_eq!(alice_index.len(), 2);
    assert_eq!(bob_index.len(), 2);
    assert_eq!(carol_index.len(), 2);

    assert!(alice_index.contains(id0));
    assert!(alice_index.contains(id3));
    assert!(!alice_index.contains(id1));
    assert!(!alice_index.contains(id2));
    assert!(!alice_index.contains(id4));
    assert!(!alice_index.contains(id5));

    assert!(bob_index.contains(id1));
    assert!(bob_index.contains(id4));
    assert!(!bob_index.contains(id0));
    assert!(!bob_index.contains(id2));
    assert!(!bob_index.contains(id3));
    assert!(!bob_index.contains(id5));

    assert!(carol_index.contains(id2));
    assert!(carol_index.contains(id5));
    assert!(!carol_index.contains(id0));
    assert!(!carol_index.contains(id1));
    assert!(!carol_index.contains(id3));
    assert!(!carol_index.contains(id4));
}

/// Same interleaved three-recipient batch: withdrawals must credit the owner of each stream.
///
/// Distinct deposit/rate pairs make a wrong-recipient credit detectable via balances.
#[test]
fn test_batch_three_recipients_interleaved_withdrawals_credited_correctly() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);
    let carol = Address::generate(&ctx.env);

    // rate = deposit / duration; duration=1000 → rate equals deposit/1000.
    // After +500s, each stream yields deposit/2.
    let params = vec![
        &ctx.env,
        ctx.make_params(&alice, 1_000, 1_000), // alice: 500
        ctx.make_params(&bob, 2_000, 1_000),   // bob: 1000
        ctx.make_params(&carol, 3_000, 1_000), // carol: 1500
        ctx.make_params(&alice, 4_000, 1_000), // alice: 2000
        ctx.make_params(&bob, 5_000, 1_000),   // bob: 2500
        ctx.make_params(&carol, 6_000, 1_000), // carol: 3000
    ];

    let ids = ctx.client.create_streams(&ctx.sender, &params);
    let id0 = ids.get(0).unwrap();
    let id1 = ids.get(1).unwrap();
    let id2 = ids.get(2).unwrap();
    let id3 = ids.get(3).unwrap();
    let id4 = ids.get(4).unwrap();
    let id5 = ids.get(5).unwrap();

    let start = ctx.env.ledger().timestamp();
    ctx.env.ledger().with_mut(|l| l.timestamp = start + 500);

    let alice_before = ctx.token.balance(&alice);
    let bob_before = ctx.token.balance(&bob);
    let carol_before = ctx.token.balance(&carol);

    let alice_results = ctx.client.batch_withdraw(&alice, &vec![&ctx.env, id0, id3]);
    let bob_results = ctx.client.batch_withdraw(&bob, &vec![&ctx.env, id1, id4]);
    let carol_results = ctx.client.batch_withdraw(&carol, &vec![&ctx.env, id2, id5]);

    assert_eq!(alice_results.get(0).unwrap().amount, 500);
    assert_eq!(alice_results.get(1).unwrap().amount, 2_000);
    assert_eq!(bob_results.get(0).unwrap().amount, 1_000);
    assert_eq!(bob_results.get(1).unwrap().amount, 2_500);
    assert_eq!(carol_results.get(0).unwrap().amount, 1_500);
    assert_eq!(carol_results.get(1).unwrap().amount, 3_000);

    assert_eq!(ctx.token.balance(&alice), alice_before + 500 + 2_000);
    assert_eq!(ctx.token.balance(&bob), bob_before + 1_000 + 2_500);
    assert_eq!(ctx.token.balance(&carol), carol_before + 1_500 + 3_000);

    // Cross-contamination check: each recipient must not be able to withdraw the others' streams.
    let steal = ctx.client.try_batch_withdraw(&alice, &vec![&ctx.env, id1]);
    assert!(steal.is_err());
}

/// Recipient rotation mid-sequence, then another interleaved batch.
///
/// `create_streams` cannot mutate a recipient inside a single call, so this exercises
/// the closest API path: accept a recipient update between two batches and confirm the
/// subsequent cache flush still scopes IDs to the correct recipient (no stale index).
#[test]
fn test_batch_after_recipient_update_no_stale_index() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);
    let carol = Address::generate(&ctx.env);

    let first = vec![
        &ctx.env,
        ctx.make_params(&alice, 1_000, 1_000),
        ctx.make_params(&bob, 2_000, 2_000),
        ctx.make_params(&carol, 3_000, 3_000),
    ];
    let first_ids = ctx.client.create_streams(&ctx.sender, &first);
    let alice_stream = first_ids.get(0).unwrap();
    let bob_stream = first_ids.get(1).unwrap();
    let carol_stream = first_ids.get(2).unwrap();

    // Move alice's stream to bob (propose + accept). Index must follow.
    ctx.client.update_recipient(&alice_stream, &bob);
    ctx.client.accept_recipient_update(&alice_stream);

    assert_eq!(ctx.client.get_stream_state(&alice_stream).recipient, bob);
    assert!(!ctx
        .client
        .get_recipient_streams(&alice)
        .contains(alice_stream));
    assert!(ctx
        .client
        .get_recipient_streams(&bob)
        .contains(alice_stream));
    assert!(ctx.client.get_recipient_streams(&bob).contains(bob_stream));

    // Second interleaved batch after the rotation — cache flush must not resurrect
    // alice_stream under alice or drop bob's existing IDs.
    let second = vec![
        &ctx.env,
        ctx.make_params(&carol, 4_000, 4_000),
        ctx.make_params(&alice, 5_000, 5_000),
        ctx.make_params(&bob, 6_000, 6_000),
        ctx.make_params(&carol, 7_000, 7_000),
        ctx.make_params(&alice, 8_000, 8_000),
        ctx.make_params(&bob, 9_000, 9_000),
    ];
    let second_ids = ctx.client.create_streams(&ctx.sender, &second);
    assert_eq!(second_ids.len(), 6);

    let s0 = second_ids.get(0).unwrap(); // carol
    let s1 = second_ids.get(1).unwrap(); // alice
    let s2 = second_ids.get(2).unwrap(); // bob
    let s3 = second_ids.get(3).unwrap(); // carol
    let s4 = second_ids.get(4).unwrap(); // alice
    let s5 = second_ids.get(5).unwrap(); // bob

    assert_eq!(ctx.client.get_stream_state(&s0).recipient, carol);
    assert_eq!(ctx.client.get_stream_state(&s1).recipient, alice);
    assert_eq!(ctx.client.get_stream_state(&s2).recipient, bob);
    assert_eq!(ctx.client.get_stream_state(&s3).recipient, carol);
    assert_eq!(ctx.client.get_stream_state(&s4).recipient, alice);
    assert_eq!(ctx.client.get_stream_state(&s5).recipient, bob);

    let alice_index = ctx.client.get_recipient_streams(&alice);
    let bob_index = ctx.client.get_recipient_streams(&bob);
    let carol_index = ctx.client.get_recipient_streams(&carol);

    // alice: only the two new streams (rotated stream left)
    assert_eq!(alice_index.len(), 2);
    assert!(alice_index.contains(s1));
    assert!(alice_index.contains(s4));
    assert!(!alice_index.contains(alice_stream));

    // bob: rotated stream + original bob stream + two new bob streams
    assert_eq!(bob_index.len(), 4);
    assert!(bob_index.contains(alice_stream));
    assert!(bob_index.contains(bob_stream));
    assert!(bob_index.contains(s2));
    assert!(bob_index.contains(s5));

    // carol: original + two new
    assert_eq!(carol_index.len(), 3);
    assert!(carol_index.contains(carol_stream));
    assert!(carol_index.contains(s0));
    assert!(carol_index.contains(s3));

    // Withdrawals after rotation: bob owns the rotated stream; alice does not.
    let start = ctx.env.ledger().timestamp();
    ctx.env.ledger().with_mut(|l| l.timestamp = start + 500);

    let bob_before = ctx.token.balance(&bob);
    let results = ctx
        .client
        .batch_withdraw(&bob, &vec![&ctx.env, alice_stream, bob_stream]);
    assert_eq!(results.get(0).unwrap().amount, 500); // 1000 deposit / 2
    assert_eq!(results.get(1).unwrap().amount, 500); // 2000 deposit over 2000s → rate 1 → 500
    assert_eq!(ctx.token.balance(&bob), bob_before + 1_000);

    let alice_steal = ctx
        .client
        .try_batch_withdraw(&alice, &vec![&ctx.env, alice_stream]);
    assert!(alice_steal.is_err());
}

/// Mixed-recipient batch where a stream partway through fails validation:
/// verifies atomic rollback where no stream is created and no recipient index is mutated.
#[test]
fn test_batch_mixed_recipient_partial_failure_rollback() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);

    // Initial state query: ensure both recipient indexes are empty before batch creation.
    let alice_index_before = ctx.client.get_recipient_streams(&alice);
    let bob_index_before = ctx.client.get_recipient_streams(&bob);
    assert_eq!(alice_index_before.len(), 0);
    assert_eq!(bob_index_before.len(), 0);

    // 5 streams across 2 recipients, with the 3rd stream (index 2, recipient Bob) deliberately invalid (rate_per_second = 0 for Linear stream).
    let mut invalid_params = ctx.make_params(&bob, 2_000, 2_000);
    invalid_params.rate_per_second = 0; // Invalid rate for Linear stream

    let params = vec![
        &ctx.env,
        ctx.make_params(&alice, 1_000, 1_000), // 1st: Alice (valid)
        ctx.make_params(&bob, 2_000, 2_000),   // 2nd: Bob (valid)
        invalid_params,                         // 3rd: Bob (invalid - rate below min/zero)
        ctx.make_params(&alice, 3_000, 3_000), // 4th: Alice (valid)
        ctx.make_params(&bob, 4_000, 4_000),   // 5th: Bob (valid)
    ];

    // Execution must revert with error / fail
    let res = ctx.client.try_create_streams(&ctx.sender, &params);
    assert!(res.is_err());

    // Assert zero mutation of recipient indexes for all involved recipients
    let alice_index_after = ctx.client.get_recipient_streams(&alice);
    let bob_index_after = ctx.client.get_recipient_streams(&bob);
    assert_eq!(alice_index_after.len(), 0);
    assert_eq!(bob_index_after.len(), 0);
}

/// Companion test confirming a fully-valid mixed-recipient batch produces correct, complete per-recipient index state.
#[test]
fn test_batch_mixed_recipient_fully_valid_success() {
    let ctx = Ctx::setup();
    let alice = Address::generate(&ctx.env);
    let bob = Address::generate(&ctx.env);

    let params = vec![
        &ctx.env,
        ctx.make_params(&alice, 1_000, 1_000), // 1st: Alice
        ctx.make_params(&bob, 2_000, 2_000),   // 2nd: Bob
        ctx.make_params(&bob, 2_000, 2_000),   // 3rd: Bob
        ctx.make_params(&alice, 3_000, 3_000), // 4th: Alice
        ctx.make_params(&bob, 4_000, 4_000),   // 5th: Bob
    ];

    let ids = ctx.client.create_streams(&ctx.sender, &params);
    assert_eq!(ids.len(), 5);

    let alice_index = ctx.client.get_recipient_streams(&alice);
    let bob_index = ctx.client.get_recipient_streams(&bob);

    assert_eq!(alice_index.len(), 2);
    assert_eq!(bob_index.len(), 3);

    assert!(alice_index.contains(ids.get(0).unwrap()));
    assert!(alice_index.contains(ids.get(3).unwrap()));

    assert!(bob_index.contains(ids.get(1).unwrap()));
    assert!(bob_index.contains(ids.get(2).unwrap()));
    assert!(bob_index.contains(ids.get(4).unwrap()));
}

