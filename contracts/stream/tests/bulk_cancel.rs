#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, IntoVal, Symbol, TryFromVal,
};

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, PauseReason, StreamStatus,
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
        recipient,
        &deposit,
        &rate,
        &start,
        &cliff,
        &end,
        &0i128,
        &None,
        &fluxora_stream::StreamKind::Linear,
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
    client.withdraw(&stream_id);

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

#[test]
fn test_bulk_cancel_requires_sender_auth() {
    // setup_env()/create_test_stream() call env.mock_all_auths(), which is
    // sticky and would make every subsequent require_auth() succeed
    // regardless of what's actually authorized — masking exactly the
    // behavior this test needs to observe. Build a minimal env here instead,
    // mocking only the create_stream call so the final bulk_cancel_streams
    // call genuinely has no authorization.
    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};

    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);
    env.set_auths(&[]);

    env.mock_auths(&[MockAuth {
        address: &sender,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "create_stream",
            args: (
                &sender,
                &recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
                0i128,
                Option::<soroban_sdk::Bytes>::None,
                fluxora_stream::StreamKind::Linear,
            )
                .into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let stream_id = client.create_stream(
        &sender,
        &recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0i128,
        &None,
        &fluxora_stream::StreamKind::Linear,
    );

    env.set_auths(&[]);
    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, stream_id]);
    assert!(result.is_err());
}
