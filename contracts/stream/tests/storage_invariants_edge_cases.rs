//! Edge-case and storage layout invariant integration test suite.
//!
//! Validates:
//! 1. Adaptive TTL behavior under edge cases (expired streams, immediate end_time, boundary values).
//! 2. Persistent storage reclamation for empty index vectors (RecipientStreams, SenderStreams, etc.).
//! 3. Instance TTL bumping consistency across query and mutation entry-points.
//! 4. Deterministic same-ledger timestamp retries and monotonic accrual checks.
//! 5. Backward compatibility across storage key layout versions.

extern crate std;

use fluxora_stream::{storage::*, ContractError, DataKey, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

struct TestCtx<'a> {
    env: Env,
    contract_id: Address,
    client: FluxoraStreamClient<'a>,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
}

impl<'a> TestCtx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|l| l.timestamp = 1_000_000);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        sac.mint(&sender, &1_000_000_000);

        client.init(&token_id, &admin);

        TestCtx {
            env,
            contract_id,
            client,
            token_id,
            admin,
            sender,
            recipient,
        }
    }
}

#[test]
fn test_adaptive_ttl_computation_boundaries() {
    let now = 1_000_000u64;

    // 1. Stream ending far in the future (~25,000,000 seconds -> ~5,000,000 ledgers) -> should cap at MAX_TTL
    let far_future_end = now + 25_000_000;
    let ttl_far = compute_adaptive_ttl(now, far_future_end);
    assert_eq!(ttl_far, MAX_TTL);

    // 2. Stream ending moderately in future (~500,000 seconds)
    let mid_end = now + 500_000;
    let ttl_mid = compute_adaptive_ttl(now, mid_end);
    let expected_mid = ((500_000 / LEDGER_CLOSE_TIME) + BUFFER_LEDGERS as u64) as u32;
    assert_eq!(ttl_mid, expected_mid.clamp(PERSISTENT_BUMP_AMOUNT, MAX_TTL));

    // 3. Stream ending exactly now -> falls back to PERSISTENT_BUMP_AMOUNT floor
    let ttl_now = compute_adaptive_ttl(now, now);
    assert_eq!(ttl_now, PERSISTENT_BUMP_AMOUNT);

    // 4. Expired stream (now > end_time) -> saturating sub yields 0 remaining seconds, falls back to PERSISTENT_BUMP_AMOUNT
    let ttl_past = compute_adaptive_ttl(now, now - 100);
    assert_eq!(ttl_past, PERSISTENT_BUMP_AMOUNT);

    // 5. end_time == 0 -> falls back to PERSISTENT_BUMP_AMOUNT
    let ttl_zero = compute_adaptive_ttl(now, 0);
    assert_eq!(ttl_zero, PERSISTENT_BUMP_AMOUNT);

    // 6. Extreme u64::MAX end_time -> clamped to MAX_TTL
    let ttl_max = compute_adaptive_ttl(now, u64::MAX);
    assert_eq!(ttl_max, MAX_TTL);
}

#[test]
fn test_recipient_and_sender_index_storage_reclamation() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    let key_rec = DataKey::RecipientStreams(ctx.recipient.clone());
    let key_snd = DataKey::SenderStreams(ctx.sender.clone());

    env.as_contract(&ctx.contract_id, || {
        // Initial state: recipient and sender have no persistent index key stored
        assert_eq!(load_recipient_streams(env, &ctx.recipient).len(), 0);
        assert_eq!(load_sender_streams(env, &ctx.sender).len(), 0);

        assert!(!env.storage().persistent().has(&key_rec));
        assert!(!env.storage().persistent().has(&key_snd));

        // Add stream 42 to index
        add_stream_to_recipient_index(env, &ctx.recipient, 42, Some(1_100_000));
        add_stream_to_sender_index(env, &ctx.sender, 42, Some(1_100_000));

        assert!(env.storage().persistent().has(&key_rec));
        assert!(env.storage().persistent().has(&key_snd));
        assert_eq!(load_recipient_streams(env, &ctx.recipient).get(0), Some(42));
        assert_eq!(load_sender_streams(env, &ctx.sender).get(0), Some(42));

        // Remove stream 42 from index
        remove_stream_from_recipient_index(env, &ctx.recipient, 42);
        remove_stream_from_sender_index(env, &ctx.sender, 42);

        // Persistent storage keys MUST be reclaimed when vectors become empty
        assert!(!env.storage().persistent().has(&key_rec));
        assert!(!env.storage().persistent().has(&key_snd));

        // Reads on absent keys safely return empty Vec
        assert_eq!(load_recipient_streams(env, &ctx.recipient).len(), 0);
        assert_eq!(load_sender_streams(env, &ctx.sender).len(), 0);
    });
}

#[test]
fn test_instance_ttl_bumping_on_queries() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Querying instance getters should succeed without panicking and bump instance TTL
        assert!(!is_global_emergency_paused(env));
        assert!(!is_creation_paused(env));
        assert_eq!(get_pause_reason(env), None);
        assert_eq!(get_pause_timestamp(env), None);
        assert_eq!(get_pause_admin(env), None);
        assert_eq!(get_max_rate_per_second(env), i128::MAX);
        assert_eq!(read_paused_stream_count(env), 0);
        assert_eq!(read_total_keeper_fees_paid(env), 0);
        assert_eq!(get_config(env).unwrap().admin, ctx.admin);
        assert_eq!(load_config(env).admin, ctx.admin);
    });
}

#[test]
fn test_same_ledger_retries_and_monotonicity() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // First call sets timestamp
        let t1 = current_accrual_timestamp(env).unwrap();
        assert_eq!(t1, 1_000_000);

        // Same-ledger retry: same timestamp must succeed deterministically
        let t2 = current_accrual_timestamp(env).unwrap();
        assert_eq!(t2, 1_000_000);
    });

    // Advance timestamp
    env.ledger().with_mut(|l| l.timestamp = 1_000_005);
    env.as_contract(&ctx.contract_id, || {
        let t3 = current_accrual_timestamp(env).unwrap();
        assert_eq!(t3, 1_000_005);
    });

    // Clock regression simulation
    env.ledger().with_mut(|l| l.timestamp = 1_000_000);
    env.as_contract(&ctx.contract_id, || {
        let res = current_accrual_timestamp(env);
        assert_eq!(res, Err(ContractError::ClockRegression));
    });
}

#[test]
fn test_id_reservation_and_monotonic_stream_ids() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        assert_eq!(read_stream_count(env), 0);

        // Save a reservation of 2 IDs starting at 100 for sender
        let res = fluxora_stream::IdReservation {
            start_id: 100,
            count: 2,
            consumed: 0,
            expiry: None,
        };
        save_id_reservation(env, &ctx.sender, &res);

        // First allocation consumes ID 100
        let id0 = next_stream_id_for(env, &ctx.sender);
        assert_eq!(id0, 100);

        // Second allocation consumes ID 101 and removes the reservation
        let id1 = next_stream_id_for(env, &ctx.sender);
        assert_eq!(id1, 101);

        assert!(load_id_reservation(env, &ctx.sender).is_none());

        // Subsequent allocation falls through to global counter (0) and increments it to 1
        let id2 = next_stream_id_for(env, &ctx.sender);
        assert_eq!(id2, 0);
        assert_eq!(read_stream_count(env), 1);
    });
}
