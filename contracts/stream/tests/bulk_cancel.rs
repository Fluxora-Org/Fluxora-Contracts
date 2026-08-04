#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Symbol, TryFromVal,
};

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus,
};

// ── Test helpers ───────────────────────────────────────────────────────────

fn setup_env() -> (Env, FluxoraStreamClient<'static>, Address, Address, Address) {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.mock_all_auths();
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &1_000_000_000_i128);
    let token = TokenClient::new(&env, &token_id);
    token.approve(&sender, &contract_id, &i128::MAX, &200_000);

    (env, client, admin, sender, recipient)
}

fn create_test_stream(
    env: &Env,
    client: &FluxoraStreamClient,
    sender: &Address,
    recipient: &Address,
    deposit: i128,
    rate: i128,
    start: u64,
    cliff: u64,
    end: u64,
) -> u64 {
    env.mock_all_auths();
    client.create_stream(
        sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: rate,
            start_time: start,
            cliff_time: cliff,
            end_time: end,
            withdraw_dust_threshold: Some(0i128),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    )
}

fn advance_time(env: &Env, seconds: u64) {
    env.ledger()
        .set_timestamp(env.ledger().timestamp() + seconds);
}

// ── bulk_cancel_streams tests ──────────────────────────────────────────────

#[test]
fn test_bulk_cancel_single_stream() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    advance_time(&env, 500);

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert!(stream.cancelled_at.is_some());
}

#[test]
fn test_bulk_cancel_multiple_streams_full_refund() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let s1 = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 100, 100, 1100);
    let s2 = create_test_stream(&env, &client, &sender, &recipient, 2000, 2, 100, 100, 1100);
    let s3 = create_test_stream(&env, &client, &sender, &recipient, 3000, 3, 100, 100, 1100);

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, s1, s2, s3]);

    for id in [s1, s2, s3] {
        let stream = client.get_stream_state(&id);
        assert_eq!(stream.status, StreamStatus::Cancelled);
    }
}

#[test]
fn test_bulk_cancel_multiple_streams_partial_refund() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let s1 = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    let s2 = create_test_stream(&env, &client, &sender, &recipient, 2000, 2, 0, 0, 1000);

    advance_time(&env, 500);

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, s1, s2]);

    let stream1 = client.get_stream_state(&s1);
    let stream2 = client.get_stream_state(&s2);
    assert_eq!(stream1.status, StreamStatus::Cancelled);
    assert_eq!(stream2.status, StreamStatus::Cancelled);
}

#[test]
fn test_bulk_cancel_pays_recipient_before_refund() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    advance_time(&env, 600);

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.withdrawn_amount, 600);
}

#[test]
fn test_bulk_cancel_emits_events_per_stream() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let s1 = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    let s2 = create_test_stream(&env, &client, &sender, &recipient, 2000, 2, 0, 0, 1000);

    advance_time(&env, 100);
    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, s1, s2]);

    let events = env.events().all();
    let cancelled_events: Vec<_> = events
        .iter()
        .filter(|e| {
            let topics = e.1.clone();
            topics.len() > 0
                && topics
                    .get(0)
                    .and_then(|t| Symbol::try_from_val(&env, &t).ok())
                    .map(|s| s == Symbol::new(&env, "cancelled"))
                    .unwrap_or(false)
        })
        .collect();

    assert_eq!(cancelled_events.len(), 2);
}

#[test]
fn test_bulk_cancel_empty_vec_is_noop() {
    let (env, client, _admin, sender, _recipient) = setup_env();
    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env]);
}

#[test]
fn test_bulk_cancel_rejects_duplicate_ids() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let s1 = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);

    env.mock_all_auths();
    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, s1, s1]);
    assert!(result.is_err());
    assert_eq!(result, Err(Ok(ContractError::DuplicateStreamId)));
}

#[test]
fn test_bulk_cancel_rejects_nonexistent_stream() {
    let (env, client, _admin, sender, _recipient) = setup_env();
    env.mock_all_auths();
    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, 999u64]);
    assert!(result.is_err());
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

#[test]
fn test_bulk_cancel_rejects_unauthorized_sender() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    let attacker = Address::generate(&env);

    env.mock_all_auths();
    let result = client.try_bulk_cancel_streams(&attacker, &vec![&env, stream_id]);
    assert!(result.is_err());
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
}

#[test]
fn test_bulk_cancel_rejects_terminal_stream() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    env.mock_all_auths();
    client.cancel_stream(&stream_id);

    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, stream_id]);
    assert!(result.is_err());
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn test_bulk_cancel_rejects_completed_stream() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    advance_time(&env, 1001);
    env.mock_all_auths();
    client.withdraw(&stream_id, &None);

    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, stream_id]);
    assert!(result.is_err());
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn test_bulk_cancel_atomic_rollback_on_failure() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let s1 = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    let s2 = create_test_stream(&env, &client, &sender, &recipient, 2000, 2, 0, 0, 1000);

    env.mock_all_auths();
    client.cancel_stream(&s2);

    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, s1, s2]);
    assert!(result.is_err());
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    let stream1 = client.get_stream_state(&s1);
    assert_eq!(stream1.status, StreamStatus::Active);
}

#[test]
fn test_bulk_cancel_atomic_rollback_on_unauthorized_stream() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let sender2 = Address::generate(&env);
    let token_id = client.get_config().token;
    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender2, &1_000_000_000_i128);
    let token = TokenClient::new(&env, &token_id);
    env.mock_all_auths();
    token.approve(&sender2, &client.address, &i128::MAX, &200_000);

    let s1 = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    let s2 = create_test_stream(&env, &client, &sender2, &recipient, 2000, 2, 0, 0, 1000);

    env.mock_all_auths();

    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, s1, s2]);
    assert!(result.is_err());
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));

    let stream1 = client.get_stream_state(&s1);
    assert_eq!(stream1.status, StreamStatus::Active);

    let stream2 = client.get_stream_state(&s2);
    assert_eq!(stream2.status, StreamStatus::Active);
}

#[test]
fn test_bulk_cancel_with_paused_stream() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    env.mock_all_auths();
    // Advance the ledger sequence past the pause/resume cooldown window
    // (MIN_PAUSE_INTERVAL_LEDGERS); the test env's sequence number does not
    // advance on its own alongside the timestamp.
    env.ledger().with_mut(|l| l.sequence_number += 32);
    client.pause_stream(&stream_id, &PauseReason::Operational);

    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

#[test]
fn test_bulk_cancel_large_batch_up_to_max_page_size() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);
    // A 100-stream batch plus per-stream verification exceeds the default
    // test budget; this is a resource-accounting ceiling of the harness,
    // not a contract limitation, so lift it for this stress test.
    env.budget().reset_unlimited();

    let mut stream_ids = vec![&env];
    for _ in 0..100 {
        let id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
        stream_ids.push_back(id);
    }

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &stream_ids);

    for i in 0..100 {
        let id = stream_ids.get(i).unwrap();
        let stream = client.get_stream_state(&id);
        assert_eq!(stream.status, StreamStatus::Cancelled);
    }
}

#[test]
fn test_bulk_cancel_reduces_liabilities_correctly() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let deposit = 1000i128;
    let stream_id = create_test_stream(&env, &client, &sender, &recipient, deposit, 1, 0, 0, 1000);
    let initial_liabilities = client.get_total_liabilities();

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let final_liabilities = client.get_total_liabilities();
    assert_eq!(final_liabilities, initial_liabilities - deposit);
}

#[test]
fn test_bulk_cancel_recipient_gets_paid_before_sender_refund() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    advance_time(&env, 750);

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.withdrawn_amount, 750);
}

#[test]
fn test_bulk_cancel_with_zero_accrued_before_cliff() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 500, 1000);
    advance_time(&env, 300);

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.withdrawn_amount, 0);
}

#[test]
fn test_bulk_cancel_mixed_streams_some_fully_accrued() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let s1 = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    let s2 = create_test_stream(&env, &client, &sender, &recipient, 2000, 2, 0, 0, 1000);

    advance_time(&env, 1001);
    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, s1, s2]);

    let stream1 = client.get_stream_state(&s1);
    let stream2 = client.get_stream_state(&s2);
    assert_eq!(stream1.withdrawn_amount, 1000);
    assert_eq!(stream2.withdrawn_amount, 2000);
}

#[test]
fn test_bulk_cancel_rejects_global_pause() {
    let (env, client, admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    env.mock_all_auths();
    client.set_global_emergency_paused(&true);

    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, stream_id]);
    assert!(result.is_err());
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));
}

// ===========================================================================
// CliffOnly-specific bulk_cancel tests (issue #1193)
// ===========================================================================

/// CliffOnly stream bulk-cancelled before cliff: recipient gets 0,
/// sender gets full deposit refund. The binary nature of CliffOnly accrual
/// means accrued == 0 before cliff_time.
#[test]
fn test_bulk_cancel_cliff_only_before_cliff_full_refund() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 0, // CliffOnly requires rate=0
            start_time: 0,
            cliff_time: 500,
            end_time: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    );

    // t=200, before cliff=500
    advance_time(&env, 200);

    let token = TokenClient::new(&env, &client.get_config().token);
    let sender_before = token.balance(&sender);
    let recipient_before = token.balance(&recipient);

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let recipient_delta = token.balance(&recipient) - recipient_before;
    let sender_delta = token.balance(&sender) - sender_before;

    assert_eq!(recipient_delta, 0, "recipient gets 0 before cliff");
    assert_eq!(sender_delta, 1000, "sender gets full deposit refund");

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.withdrawn_amount, 0);
}

/// CliffOnly stream bulk-cancelled after cliff: recipient gets full
/// deposit, sender gets 0. The binary accrual means the full deposit is
/// accrued once cliff_time is reached.
#[test]
fn test_bulk_cancel_cliff_only_after_cliff_recipient_gets_all() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 0,
            start_time: 0,
            cliff_time: 500,
            end_time: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    );

    // t=700, after cliff=500
    advance_time(&env, 700);

    let token = TokenClient::new(&env, &client.get_config().token);
    let sender_before = token.balance(&sender);
    let recipient_before = token.balance(&recipient);
    let liabilities_before = client.get_total_liabilities();

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let recipient_delta = token.balance(&recipient) - recipient_before;
    let sender_delta = token.balance(&sender) - sender_before;

    assert_eq!(recipient_delta, 1000, "recipient gets full deposit after cliff");
    assert_eq!(sender_delta, 0, "sender gets 0 (fully accrued)");

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.withdrawn_amount, 1000);

    assert_eq!(
        client.get_total_liabilities(),
        liabilities_before - 1000,
        "TotalLiabilities must decrease by deposit"
    );
}

/// Multiple CliffOnly streams in a single bulk_cancel, mixed before/after cliff.
#[test]
fn test_bulk_cancel_cliff_only_mixed_cliff() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    // s1: cliff=1000, end=2000 → before cliff at t=500
    let s1 = client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 500,
            rate_per_second: 0,
            start_time: 0,
            cliff_time: 1000,
            end_time: 2000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    );

    // s2: cliff=400, end=1000 → after cliff at t=500
    let s2 = client.create_stream(
        &sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 0,
            start_time: 0,
            cliff_time: 400,
            end_time: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    );

    // t=500: before s1.cliff, after s2.cliff
    advance_time(&env, 500);

    let token = TokenClient::new(&env, &client.get_config().token);
    let sender_before = token.balance(&sender);
    let recipient_before = token.balance(&recipient);

    env.mock_all_auths();
    client.bulk_cancel_streams(&sender, &vec![&env, s1, s2]);

    // s1: full refund to sender (before cliff), s2: full payment to recipient (after cliff)
    let recipient_delta = token.balance(&recipient) - recipient_before;
    let sender_delta = token.balance(&sender) - sender_before;

    assert_eq!(recipient_delta, 1000, "only s2 (after cliff) pays recipient");
    assert_eq!(sender_delta, 500, "only s1 (before cliff) refunds sender");

    assert_eq!(
        client.get_stream_state(&s1).status,
        StreamStatus::Cancelled
    );
    assert_eq!(
        client.get_stream_state(&s2).status,
        StreamStatus::Cancelled
    );
}

#[test]
fn test_bulk_cancel_requires_sender_auth() {
    // mock_all_auths() is sticky in soroban-sdk 21.7.7 and cannot be undone
    // (the removed env.set_auths(&[]) API was the old escape hatch). Use
    // catch_unwind to verify the call panics when a non-authorized address
    // attempts bulk_cancel_streams.
    use std::panic::AssertUnwindSafe;

    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);
    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);

    let attacker = Address::generate(&env);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        client.bulk_cancel_streams(&attacker, &vec![&env, stream_id]);
    }));
    assert!(
        result.is_err(),
        "bulk_cancel_streams must reject unauthorized caller"
    );
}
