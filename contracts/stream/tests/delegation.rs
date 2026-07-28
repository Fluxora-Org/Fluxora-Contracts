#![cfg(test)]

use fluxora_stream::{ContractError, CreateStreamParams, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, Env,
};

fn create_stream(
    env: &Env,
    client: &FluxoraStreamClient,
    token_id: &Address,
    sender: &Address,
    recipient: &Address,
    rate_per_second: i128,
) -> u64 {
    let deposit = rate_per_second * 1000;
    let now = env.ledger().timestamp();
    let sac = soroban_sdk::token::StellarAssetClient::new(env, token_id);
    sac.mint(sender, &deposit);
    let token = soroban_sdk::token::Client::new(env, token_id);
    token.approve(sender, &client.address, &deposit, &100_000);

    client.create_stream(
        sender,
        &CreateStreamParams {
            recipient: recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: rate_per_second,
            start_time: now,
            cliff_time: now,
            end_time: (now + 1000),
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    )
}

#[test]
fn test_delegate_recipient_share_success() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence_number = 10;
    });

    let contract_id = env.register_contract(None, fluxora_stream::FluxoraStream {});
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(token_admin);

    client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let recipient1 = Address::generate(&env);
    let recipient2 = Address::generate(&env);

    let stream_id = create_stream(&env, &client, &token_id, &sender, &recipient1, 10000);

    let share_bps = 5000; // 50%
    let child_id =
        client.delegate_recipient_share(&stream_id, &recipient1, &share_bps, &recipient2);

    let parent_state = client.get_stream_state(&stream_id);
    let child_state = client.get_stream_state(&child_id);

    assert_eq!(parent_state.rate_per_second, 5000);
    assert_eq!(parent_state.delegation_depth, 0);

    assert_eq!(child_state.rate_per_second, 5000);
    assert_eq!(child_state.recipient, recipient2);
    assert_eq!(child_state.parent_stream_id, Some(stream_id));
    assert_eq!(child_state.delegation_depth, 1);

    let recipient2_streams = client.get_recipient_streams(&recipient2);
    assert!(recipient2_streams.iter().any(|id| id == child_id));

    let sender_portfolio = client.get_sender_portfolio_health(&sender, &0, &100);
    assert!(sender_portfolio.stream_ids.iter().any(|id| id == stream_id));
    assert!(sender_portfolio.stream_ids.iter().any(|id| id == child_id));
}

#[test]
fn test_delegate_recipient_share_preserves_checkpoint_after_withdraw() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence_number = 10;
    });

    let contract_id = env.register_contract(None, fluxora_stream::FluxoraStream {});
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(token_admin);

    client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let recipient1 = Address::generate(&env);
    let recipient2 = Address::generate(&env);

    let stream_id = create_stream(&env, &client, &token_id, &sender, &recipient1, 10);
    let token = TokenClient::new(&env, &token_id);

    env.ledger().with_mut(|li| {
        li.timestamp = 1010;
        li.sequence_number = 20;
    });
    assert_eq!(client.withdraw(&stream_id), 100);
    assert_eq!(token.balance(&recipient1), 100);

    let child_id = client.delegate_recipient_share(&stream_id, &recipient1, &5000, &recipient2);
    let parent = client.get_stream_state(&stream_id);
    let child = client.get_stream_state(&child_id);

    assert_eq!(parent.checkpointed_at, 1010);
    assert_eq!(parent.checkpointed_amount, 100);
    assert_eq!(parent.withdrawn_amount, 100);
    assert_eq!(parent.rate_per_second, 5);
    assert_eq!(parent.deposit_amount, 5050);

    assert_eq!(child.start_time, 1010);
    assert_eq!(child.checkpointed_at, 1010);
    assert_eq!(child.checkpointed_amount, 0);
    assert_eq!(child.withdrawn_amount, 0);
    assert_eq!(child.rate_per_second, 5);
    assert_eq!(child.deposit_amount, 4950);

    assert_eq!(client.get_withdrawable(&stream_id), 0);
    assert_eq!(client.get_withdrawable(&child_id), 0);

    env.ledger().with_mut(|li| {
        li.timestamp = 1020;
        li.sequence_number = 30;
    });
    assert_eq!(client.get_withdrawable(&stream_id), 50);
    assert_eq!(client.get_withdrawable(&child_id), 50);
}

#[test]
fn test_delegate_recipient_share_depth_limit() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let contract_id = env.register_contract(None, fluxora_stream::FluxoraStream {});
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let token_id = env.register_stellar_asset_contract(Address::generate(&env));
    client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let r1 = Address::generate(&env);
    let r2 = Address::generate(&env);
    let r3 = Address::generate(&env);
    let r4 = Address::generate(&env);
    let r5 = Address::generate(&env);

    let id1 = create_stream(&env, &client, &token_id, &sender, &r1, 10000);

    // Depth 1
    let id2 = client.delegate_recipient_share(&id1, &r1, &5000, &r2);
    // Depth 2
    let id3 = client.delegate_recipient_share(&id2, &r2, &5000, &r3);
    // Depth 3
    let id4 = client.delegate_recipient_share(&id3, &r3, &5000, &r4);

    // Depth 4 should fail (MAX_DELEGATION_DEPTH is 3)
    let err = client
        .try_delegate_recipient_share(&id4, &r4, &5000, &r5)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::DelegationDepthExceeded);
}

#[test]
fn test_delegate_recipient_share_cyclic() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let contract_id = env.register_contract(None, fluxora_stream::FluxoraStream {});
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let token_id = env.register_stellar_asset_contract(Address::generate(&env));
    client.init(&token_id, &Address::generate(&env));

    let sender = Address::generate(&env);
    let r1 = Address::generate(&env);
    let r2 = Address::generate(&env);
    let r3 = Address::generate(&env);

    let id1 = create_stream(&env, &client, &token_id, &sender, &r1, 10000);
    let id2 = client.delegate_recipient_share(&id1, &r1, &5000, &r2);
    let id3 = client.delegate_recipient_share(&id2, &r2, &5000, &r3);

    // Cycle back to r1
    let err = client
        .try_delegate_recipient_share(&id3, &r3, &5000, &r1)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::CyclicDelegation);

    // Self-delegation
    let err2 = client
        .try_delegate_recipient_share(&id3, &r3, &5000, &r3)
        .unwrap_err()
        .unwrap();
    assert_eq!(err2, ContractError::CyclicDelegation);
}
