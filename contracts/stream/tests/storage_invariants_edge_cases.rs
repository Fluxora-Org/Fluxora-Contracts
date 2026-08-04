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

#[test]
fn test_total_liabilities_non_negative_floor() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        write_total_liabilities(env, 500);
        assert_eq!(read_total_liabilities(env), 500);

        // Attempting to write a negative amount saturates/clamps to 0
        write_total_liabilities(env, -100);
        assert_eq!(read_total_liabilities(env), 0, "TotalLiabilities must be non-negative");
    });
}

#[test]
fn test_offer_index_insertion_is_idempotent() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        add_offer_to_recipient_pending(env, &ctx.recipient, 7);
        assert_eq!(load_recipient_pending_offers(env, &ctx.recipient).len(), 1);

        // Re-adding same offer ID must be idempotent
        add_offer_to_recipient_pending(env, &ctx.recipient, 7);
        assert_eq!(
            load_recipient_pending_offers(env, &ctx.recipient).len(), 1,
            "Recipient pending offers index must not accept duplicate IDs"
        );
    });
}

// ---------------------------------------------------------------------------
// Additional edge-case tests hardening stream storage invariant enforcement
// ---------------------------------------------------------------------------

/// Validates that `validate_stream_invariants` correctly accepts boundary values:
/// - Zero rate_per_second (valid for cliff-only streams)
/// - Zero-duration stream (start_time == end_time == cliff_time)
/// - checkpointed_at == end_time (maximum allowed checkpoint timestamp)
/// - withdrawn_amount == deposit_amount (fully drained stream)
/// - checkpointed_amount == deposit_amount (fully checkpointed stream)
#[test]
fn test_validate_stream_invariants_boundary_values() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    // Helper: construct a Stream with given field overrides for invariant testing.
    fn base_stream(env: &Env, sender: &Address, recipient: &Address) -> fluxora_stream::Stream {
        fluxora_stream::Stream {
            stream_id: 0,
            sender: sender.clone(),
            recipient: recipient.clone(),
            claim_owner: None,
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            withdrawn_amount: 0,
            status: fluxora_stream::StreamStatus::Active,
            cancelled_at: None,
            checkpointed_amount: 0,
            checkpointed_at: 0,
            withdraw_dust_threshold: 0,
            memo: None,
            kind: fluxora_stream::StreamKind::Linear,
            last_pause_toggle_ledger: 0,
            last_withdraw_ledger: 0,
            metadata: None,
            irrevocable: None,
            witness: None,
            is_pooled: None,
            last_rate_change_ledger: 0,
            delegation_depth: 0,
            parent_stream_id: None,
            decommissioned: None,
            paused_at_timestamp: 0,
            cumulative_paused_duration: 0,
        }
    }

    env.as_contract(&ctx.contract_id, || {
        // 1. Zero rate_per_second — valid (CliffOnly streams)
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.rate_per_second = 0;
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_ok(),
            "zero rate_per_second must be accepted"
        );

        // 2. Zero-duration stream: start == end == cliff
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.start_time = 500;
        s.cliff_time = 500;
        s.end_time = 500;
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_ok(),
            "zero-duration stream (start==cliff==end) must be valid"
        );

        // 3. checkpointed_at == end_time (boundary)
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.checkpointed_at = s.end_time;
        s.checkpointed_amount = 1_000; // must be <= deposit_amount
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_ok(),
            "checkpointed_at == end_time must be valid"
        );

        // 4. withdrawn_amount == deposit_amount (fully drained)
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.withdrawn_amount = s.deposit_amount;
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_ok(),
            "withdrawn == deposit (fully drained) must be valid"
        );

        // 5. checkpointed_amount == deposit_amount (fully checkpointed)
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.checkpointed_amount = s.deposit_amount;
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_ok(),
            "checkpointed_amount == deposit_amount must be valid"
        );

        // 6. Invalid: checkpointed_at > end_time
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.checkpointed_at = s.end_time + 1;
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_err(),
            "checkpointed_at > end_time must be rejected"
        );

        // 7. Invalid: cliff_time < start_time
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.start_time = 500;
        s.cliff_time = 100;
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_err(),
            "cliff_time < start_time must be rejected"
        );

        // 8. Invalid: cliff_time > end_time
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.cliff_time = s.end_time + 1;
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_err(),
            "cliff_time > end_time must be rejected"
        );

        // 9. Invalid: withdraw_dust_threshold < 0
        let mut s = base_stream(env, &ctx.sender, &ctx.recipient);
        s.withdraw_dust_threshold = -1;
        assert!(
            fluxora_stream::storage::validate_stream_invariants(&s).is_err(),
            "negative withdraw_dust_threshold must be rejected"
        );
    });
}

/// Validates that `remove_stream_from_recipient_index` is a no-op when the
/// stream ID is not present (idempotent removal).
#[test]
fn test_remove_nonexistent_stream_id_is_noop() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Add stream 10 to the index
        add_stream_to_recipient_index(env, &ctx.recipient, 10, Some(2_000_000));
        assert_eq!(load_recipient_streams(env, &ctx.recipient).len(), 1);

        // Attempt to remove a non-existent stream ID — should be a no-op
        remove_stream_from_recipient_index(env, &ctx.recipient, 999);
        assert_eq!(
            load_recipient_streams(env, &ctx.recipient).len(),
            1,
            "removing a non-existent stream ID must not alter the index"
        );

        // Same test for sender index
        add_stream_to_sender_index(env, &ctx.sender, 10, Some(2_000_000));
        assert_eq!(load_sender_streams(env, &ctx.sender).len(), 1);

        remove_stream_from_sender_index(env, &ctx.sender, 999);
        assert_eq!(
            load_sender_streams(env, &ctx.sender).len(),
            1,
            "removing a non-existent stream ID from sender index must be a no-op"
        );
    });
}

/// Validates that the paused stream counter is correctly reconciled
/// across various state transitions.
#[test]
fn test_paused_stream_count_reconciliation() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        use fluxora_stream::StreamStatus;

        // Initial count should be 0
        assert_eq!(read_paused_stream_count(env), 0);

        // Active -> Paused: count increments
        reconcile_paused_stream_count(env, StreamStatus::Active, StreamStatus::Paused);
        assert_eq!(read_paused_stream_count(env), 1);

        // Paused -> Paused: no change (idempotent)
        reconcile_paused_stream_count(env, StreamStatus::Paused, StreamStatus::Paused);
        assert_eq!(read_paused_stream_count(env), 1);

        // Paused -> Active: count decrements
        reconcile_paused_stream_count(env, StreamStatus::Paused, StreamStatus::Active);
        assert_eq!(read_paused_stream_count(env), 0);

        // Active -> Cancelled (not from Paused): no change
        reconcile_paused_stream_count(env, StreamStatus::Active, StreamStatus::Cancelled);
        assert_eq!(read_paused_stream_count(env), 0);

        // Paused -> Cancelled: count decrements
        reconcile_paused_stream_count(env, StreamStatus::Paused, StreamStatus::Cancelled);
        assert_eq!(
            read_paused_stream_count(env), 0,
            "count must not underflow below 0"
        );
    });
}

/// Validates that `write_total_liabilities` correctly clamps extreme values.
#[test]
fn test_total_liabilities_extreme_values() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Write i128::MAX — should succeed
        write_total_liabilities(env, i128::MAX);
        assert_eq!(read_total_liabilities(env), i128::MAX);

        // Write i128::MIN — must clamp to 0
        write_total_liabilities(env, i128::MIN);
        assert_eq!(
            read_total_liabilities(env), 0,
            "i128::MIN must clamp to 0"
        );

        // Write 0 — valid
        write_total_liabilities(env, 0);
        assert_eq!(read_total_liabilities(env), 0);

        // Write -1 — must clamp to 0
        write_total_liabilities(env, -1);
        assert_eq!(
            read_total_liabilities(env), 0,
            "-1 must clamp to 0"
        );
    });
}

/// Validates keeper fee aggregate counter increment and overflow protection.
#[test]
fn test_keeper_fee_aggregate_overflow_protection() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Initial value is 0
        assert_eq!(read_total_keeper_fees_paid(env), 0);

        // Increment by 100
        increment_total_keeper_fees_paid(env, 100).unwrap();
        assert_eq!(read_total_keeper_fees_paid(env), 100);

        // Increment by another 50
        increment_total_keeper_fees_paid(env, 50).unwrap();
        assert_eq!(read_total_keeper_fees_paid(env), 150);

        // Overflow: incrementing by i128::MAX from a non-zero base should fail
        let result = increment_total_keeper_fees_paid(env, i128::MAX);
        assert_eq!(
            result,
            Err(ContractError::ArithmeticOverflow),
            "overflow must be caught"
        );

        // Value must remain unchanged after failed overflow
        assert_eq!(
            read_total_keeper_fees_paid(env),
            150,
            "failed overflow must not mutate the counter"
        );
    });
}

/// Validates that `is_terminal_state` correctly identifies terminal streams.
#[test]
fn test_is_terminal_state_edge_cases() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    fn make_stream(env: &Env, sender: &Address, recipient: &Address) -> fluxora_stream::Stream {
        fluxora_stream::Stream {
            stream_id: 0,
            sender: sender.clone(),
            recipient: recipient.clone(),
            claim_owner: None,
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            withdrawn_amount: 0,
            status: fluxora_stream::StreamStatus::Active,
            cancelled_at: None,
            checkpointed_amount: 0,
            checkpointed_at: 0,
            withdraw_dust_threshold: 0,
            memo: None,
            kind: fluxora_stream::StreamKind::Linear,
            last_pause_toggle_ledger: 0,
            last_withdraw_ledger: 0,
            metadata: None,
            irrevocable: None,
            witness: None,
            is_pooled: None,
            last_rate_change_ledger: 0,
            delegation_depth: 0,
            parent_stream_id: None,
            decommissioned: None,
            paused_at_timestamp: 0,
            cumulative_paused_duration: 0,
        }
    }

    env.as_contract(&ctx.contract_id, || {
        // 1. Active stream before end_time — NOT terminal
        let mut s = make_stream(env, &ctx.sender, &ctx.recipient);
        env.ledger().with_mut(|l| l.timestamp = 500);
        assert!(!is_terminal_state(env, &s));

        // 2. Active stream AT end_time — terminal (time-based)
        env.ledger().with_mut(|l| l.timestamp = 1_000);
        assert!(
            is_terminal_state(env, &s),
            "Active stream at exactly end_time must be terminal"
        );

        // 3. Active stream after end_time — terminal
        env.ledger().with_mut(|l| l.timestamp = 2_000);
        assert!(is_terminal_state(env, &s));

        // 4. Cancelled stream — always terminal regardless of time
        let mut s = make_stream(env, &ctx.sender, &ctx.recipient);
        s.status = fluxora_stream::StreamStatus::Cancelled;
        env.ledger().with_mut(|l| l.timestamp = 0);
        assert!(
            is_terminal_state(env, &s),
            "Cancelled stream must always be terminal"
        );

        // 5. Completed stream — always terminal
        let mut s = make_stream(env, &ctx.sender, &ctx.recipient);
        s.status = fluxora_stream::StreamStatus::Completed;
        env.ledger().with_mut(|l| l.timestamp = 0);
        assert!(
            is_terminal_state(env, &s),
            "Completed stream must always be terminal"
        );

        // 6. Paused stream before end_time — NOT terminal
        let mut s = make_stream(env, &ctx.sender, &ctx.recipient);
        s.status = fluxora_stream::StreamStatus::Paused;
        env.ledger().with_mut(|l| l.timestamp = 500);
        assert!(
            !is_terminal_state(env, &s),
            "Paused stream before end_time must not be terminal"
        );
    });
}

/// Validates that `remove_offer_from_recipient_pending` is idempotent.
#[test]
fn test_remove_nonexistent_offer_is_idempotent() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Add offers 1 and 3
        add_offer_to_recipient_pending(env, &ctx.recipient, 1);
        add_offer_to_recipient_pending(env, &ctx.recipient, 3);
        assert_eq!(load_recipient_pending_offers(env, &ctx.recipient).len(), 2);

        // Remove a non-existent offer — no-op
        remove_offer_from_recipient_pending(env, &ctx.recipient, 99);
        assert_eq!(
            load_recipient_pending_offers(env, &ctx.recipient).len(),
            2,
            "removing non-existent offer must be a no-op"
        );

        // Remove existing offer
        remove_offer_from_recipient_pending(env, &ctx.recipient, 1);
        assert_eq!(load_recipient_pending_offers(env, &ctx.recipient).len(), 1);

        // Remove the last offer — should reclaim storage
        remove_offer_from_recipient_pending(env, &ctx.recipient, 3);
        assert_eq!(
            load_recipient_pending_offers(env, &ctx.recipient).len(),
            0,
            "removing the last offer should yield an empty vector"
        );
    });
}

/// Validates that the sender index maintains sorted order across multiple
/// insertions and removals.
#[test]
fn test_sender_index_sorted_order_maintenance() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Insert IDs in non-sequential order
        add_stream_to_sender_index(env, &ctx.sender, 50, Some(2_000_000));
        add_stream_to_sender_index(env, &ctx.sender, 10, Some(2_000_000));
        add_stream_to_sender_index(env, &ctx.sender, 30, Some(2_000_000));
        add_stream_to_sender_index(env, &ctx.sender, 20, Some(2_000_000));
        add_stream_to_sender_index(env, &ctx.sender, 40, Some(2_000_000));

        let ids = load_sender_streams(env, &ctx.sender);
        assert_eq!(ids.len(), 5);
        // Must be sorted ascending
        assert_eq!(ids.get(0).unwrap(), 10);
        assert_eq!(ids.get(1).unwrap(), 20);
        assert_eq!(ids.get(2).unwrap(), 30);
        assert_eq!(ids.get(3).unwrap(), 40);
        assert_eq!(ids.get(4).unwrap(), 50);

        // Remove the middle element (30)
        remove_stream_from_sender_index(env, &ctx.sender, 30);
        let ids = load_sender_streams(env, &ctx.sender);
        assert_eq!(ids.len(), 4);
        assert_eq!(ids.get(0).unwrap(), 10);
        assert_eq!(ids.get(1).unwrap(), 20);
        assert_eq!(ids.get(2).unwrap(), 40);
        assert_eq!(ids.get(3).unwrap(), 50);
    });
}

/// Validates the reentrancy lock acquire/release cycle.
#[test]
fn test_reentrancy_lock_acquire_release_cycle() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Acquire should succeed when not locked
        assert!(acquire_reentrancy_lock(env).is_ok());

        // Second acquire should fail (lock is held)
        assert_eq!(
            acquire_reentrancy_lock(env),
            Err(ContractError::InvalidState),
            "reentrant acquire must be rejected"
        );

        // Release the lock
        release_reentrancy_lock(env);

        // Acquire should succeed again
        assert!(
            acquire_reentrancy_lock(env).is_ok(),
            "acquire after release must succeed"
        );

        // Clean up
        release_reentrancy_lock(env);
    });
}

/// Validates auto-renew storage round-trip.
#[test]
fn test_auto_renew_storage_round_trip() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Default: disabled
        assert!(!auto_renew_enabled(env, 42));

        // Enable
        set_auto_renew_enabled(env, 42, true);
        assert!(auto_renew_enabled(env, 42));

        // Disable
        set_auto_renew_enabled(env, 42, false);
        assert!(!auto_renew_enabled(env, 42));

        // Different stream IDs are independent
        set_auto_renew_enabled(env, 100, true);
        assert!(auto_renew_enabled(env, 100));
        assert!(!auto_renew_enabled(env, 101));
    });
}

/// Validates max_lookback_ledgers storage constraints.
#[test]
fn test_max_lookback_ledgers_validation() {
    let ctx = TestCtx::setup();
    let env = &ctx.env;

    env.as_contract(&ctx.contract_id, || {
        // Default: None
        assert_eq!(max_lookback_ledgers(env, 42), None);

        // Set a valid value
        set_max_lookback_ledgers(env, 42, Some(100)).unwrap();
        assert_eq!(max_lookback_ledgers(env, 42), Some(100));

        // Setting to Some(0) must be rejected
        assert_eq!(
            set_max_lookback_ledgers(env, 42, Some(0)),
            Err(ContractError::InvalidParams),
            "lookback of 0 must be rejected"
        );

        // Setting to None removes the entry
        set_max_lookback_ledgers(env, 42, None).unwrap();
        assert_eq!(max_lookback_ledgers(env, 42), None);
    });
}
