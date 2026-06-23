#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    vec, Address, Env, IntoVal, Symbol,
};

use crate::{
    accrual, Config, ContractError, DataKey, FluxoraStream, FluxoraStreamClient, StreamStatus,
};

// ── Test helpers ───────────────────────────────────────────────────────────

fn setup_env() -> (Env, FluxoraStreamClient<'static>, Address, Address, Address) {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.mock_all_auths();
    client.init(&token, &admin);

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
        sender, recipient, &deposit, &rate, &start, &cliff, &end, &0i128, &None,
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
            let topics: Vec<Symbol> = e.0.clone().try_into().unwrap_or_default();
            topics.len() > 0 && topics.get(0) == Some(Symbol::new(&env, "cancelled"))
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
    assert_eq!(result.err().unwrap(), ContractError::DuplicateStreamId);
}

#[test]
fn test_bulk_cancel_rejects_nonexistent_stream() {
    let (env, client, _admin, sender, _recipient) = setup_env();
    env.mock_all_auths();
    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, 999u64]);
    assert!(result.is_err());
    assert_eq!(result.err().unwrap(), ContractError::StreamNotFound);
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
    assert_eq!(result.err().unwrap(), ContractError::Unauthorized);
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
    assert_eq!(result.err().unwrap(), ContractError::InvalidState);
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
    assert_eq!(result.err().unwrap(), ContractError::InvalidState);
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
    assert_eq!(result.err().unwrap(), ContractError::InvalidState);

    let stream1 = client.get_stream_state(&s1);
    assert_eq!(stream1.status, StreamStatus::Active);
}

#[test]
fn test_bulk_cancel_with_paused_stream() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    env.mock_all_auths();
    client.pause_stream(&stream_id, &crate::PauseReason::Operational);

    client.bulk_cancel_streams(&sender, &vec![&env, stream_id]);

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

#[test]
fn test_bulk_cancel_large_batch_up_to_max_page_size() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

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
    assert_eq!(result.err().unwrap(), ContractError::ContractPaused);
}

#[test]
fn test_bulk_cancel_requires_sender_auth() {
    let (env, client, _admin, sender, recipient) = setup_env();
    env.ledger().set_timestamp(0);

    let stream_id = create_test_stream(&env, &client, &sender, &recipient, 1000, 1, 0, 0, 1000);
    let result = client.try_bulk_cancel_streams(&sender, &vec![&env, stream_id]);
    assert!(result.is_err());
}
