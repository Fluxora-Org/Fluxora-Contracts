use fluxora_stream::{FluxoraStream, FluxoraStreamClient, CreateStreamParams, CreateStreamRelativeParams};
use soroban_sdk::{testutils::{Address as _, Events}, vec, Address, Env, token::Client as TokenClient};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
    fn setup(mock_auth: bool) -> Self {
        let env = Env::default();
        if mock_auth {
            env.mock_all_auths();
        }

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin).address();
        let token = TokenClient::new(&env, &token_id);
        
        let admin = Address::generate(&env);
        let sender = Address::generate(&env);

        client.init(&token_id, &admin);

        Self { env, client, sender, token }
    }
}

#[test]
fn test_create_streams_empty_batch_semantics() {
    let ctx = TestContext::setup(true);

    let balance_before = ctx.token.balance(&ctx.sender);
    let count_before = ctx.client.get_stream_count();
    let events_before = ctx.env.events().all().len();

    // Call with empty vector
    let result = ctx.client.create_streams(&ctx.sender, &vec![&ctx.env]);

    assert_eq!(result.len(), 0);
    assert_eq!(ctx.token.balance(&ctx.sender), balance_before);
    assert_eq!(ctx.client.get_stream_count(), count_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
fn test_create_streams_relative_empty_batch_semantics() {
    let ctx = TestContext::setup(true);

    let balance_before = ctx.token.balance(&ctx.sender);
    let count_before = ctx.client.get_stream_count();
    let events_before = ctx.env.events().all().len();

    // Call with empty vector
    let result = ctx.client.create_streams_relative(&ctx.sender, &vec![&ctx.env]);

    assert_eq!(result.len(), 0);
    assert_eq!(ctx.token.balance(&ctx.sender), balance_before);
    assert_eq!(ctx.client.get_stream_count(), count_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
#[should_panic]
fn test_create_streams_empty_batch_unauthorized() {
    let ctx = TestContext::setup(false);
    // This should panic because sender hasn't authorized the call
    ctx.client.create_streams(&ctx.sender, &vec![&ctx.env]);
}

#[test]
#[should_panic]
fn test_create_streams_relative_empty_batch_unauthorized() {
    let ctx = TestContext::setup(false);
    // This should panic because sender hasn't authorized the call
    ctx.client.create_streams_relative(&ctx.sender, &vec![&ctx.env]);
}

// ---------------------------------------------------------------------------
// Tests — Issue #517: sweep_excess admin recovery for trapped USDC deposits
// ---------------------------------------------------------------------------

/// Test sweep_excess when no excess exists (all funds are liabilities).
#[test]
fn sweep_excess_returns_zero_when_no_excess() {
    let ctx = TestContext::setup();
    
    // Create a stream with 1000 tokens
    let stream_id = ctx.create_default_stream();
    
    // Contract has 1000 tokens, all are liabilities
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
    
    // Try to sweep excess
    let sweep_recipient = Address::generate(&ctx.env);
    let swept = ctx.client().sweep_excess(&sweep_recipient);
    
    // Should return 0 since all funds are liabilities
    assert_eq!(swept, 0);
    assert_eq!(ctx.token.balance(&sweep_recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
}

/// Test sweep_excess after stream cancellation creates excess.
#[test]
fn sweep_excess_after_stream_cancellation() {
    let ctx = TestContext::setup();
    
    // Create stream: 1000 tokens over 1000 seconds
    let stream_id = ctx.create_default_stream();
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
    
    // Cancel at 50% completion (500 seconds)
    ctx.env.ledger().set_timestamp(500);
    ctx.client().cancel_stream(&stream_id);
    
    // After cancel: 500 refunded to sender, 500 remains for recipient
    // But if we manually send tokens back to contract to simulate trapped funds
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &500);
    
    // Now contract has 1000 tokens but only 500 liabilities
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
    
    // Sweep excess
    let sweep_recipient = Address::generate(&ctx.env);
    let swept = ctx.client().sweep_excess(&sweep_recipient);
    
    // Should sweep 500 excess tokens
    assert_eq!(swept, 500);
    assert_eq!(ctx.token.balance(&sweep_recipient), 500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 500);
}

/// Test sweep_excess after rate decrease creates excess.
#[test]
fn sweep_excess_after_rate_decrease() {
    let ctx = TestContext::setup();
    
    // Create stream: 1000 tokens, 10 tokens/sec, 100 seconds
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &10_i128,
        &0u64,
        &0u64,
        &100u64,
        &0,
        &None,
    );
    
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
    
    // Decrease rate at t=50 from 10/s to 5/s
    ctx.env.ledger().set_timestamp(50);
    ctx.client().decrease_rate_per_second(&stream_id, &5_i128);
    
    // After decrease: 500 accrued (50s * 10/s), 250 remaining (50s * 5/s)
    // Total needed: 750, so 250 should be refunded to sender
    // But let's manually add it back to simulate trapped funds
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &250);
    
    // Now contract has 1000 tokens but only 750 liabilities
    let sweep_recipient = Address::generate(&ctx.env);
    let swept = ctx.client().sweep_excess(&sweep_recipient);
    
    // Should sweep 250 excess tokens
    assert_eq!(swept, 250);
    assert_eq!(ctx.token.balance(&sweep_recipient), 250);
}

/// Test sweep_excess requires admin authorization.
#[test]
fn sweep_excess_requires_admin_auth() {
    let ctx = TestContext::setup_strict();
    
    // Create stream
    ctx.env.mock_all_auths();
    let stream_id = ctx.create_default_stream();
    
    // Manually add excess tokens
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &500);
    
    // Try to sweep as non-admin (should fail)
    let attacker = Address::generate(&ctx.env);
    let sweep_recipient = Address::generate(&ctx.env);
    
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &attacker,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "sweep_excess",
            args: (&sweep_recipient,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().sweep_excess(&sweep_recipient)
    }));
    
    assert!(result.is_err(), "sweep_excess must require admin auth");
}

/// Test sweep_excess with admin authorization succeeds.
#[test]
fn sweep_excess_with_admin_auth_succeeds() {
    let ctx = TestContext::setup_strict();
    
    // Create stream with mock_all_auths
    ctx.env.mock_all_auths();
    let stream_id = ctx.create_default_stream();
    
    // Manually add excess tokens
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &500);
    
    // Contract now has 1500 tokens, 1000 liabilities, 500 excess
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_500);
    
    let sweep_recipient = Address::generate(&ctx.env);
    
    // Sweep as admin
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &ctx.admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "sweep_excess",
            args: (&sweep_recipient,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    
    let swept = ctx.client().sweep_excess(&sweep_recipient);
    
    assert_eq!(swept, 500);
    assert_eq!(ctx.token.balance(&sweep_recipient), 500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
}

/// Test sweep_excess emits ExcessSwept event.
#[test]
fn sweep_excess_emits_event() {
    let ctx = TestContext::setup();
    
    // Create stream and add excess
    let stream_id = ctx.create_default_stream();
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &300);
    
    let sweep_recipient = Address::generate(&ctx.env);
    let events_before = ctx.env.events().all().len();
    
    let swept = ctx.client().sweep_excess(&sweep_recipient);
    
    assert_eq!(swept, 300);
    
    // Verify event was emitted
    let events = ctx.env.events().all();
    let mut found_event = false;
    
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        let topic0 = soroban_sdk::Symbol::from_val(&ctx.env, &event.1.get(0).unwrap());
        if topic0 == soroban_sdk::Symbol::new(&ctx.env, "ex_swept") {
            found_event = true;
            break;
        }
    }
    
    assert!(found_event, "ExcessSwept event should be emitted");
}

/// Test sweep_excess with multiple streams and partial withdrawals.
#[test]
fn sweep_excess_with_multiple_streams_complex_scenario() {
    let ctx = TestContext::setup();
    
    // Create first stream: 1000 tokens
    ctx.env.ledger().set_timestamp(0);
    let stream_id_1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
    );
    
    // Create second stream: 2000 tokens
    let recipient_2 = Address::generate(&ctx.env);
    let stream_id_2 = ctx.client().create_stream(
        &ctx.sender,
        &recipient_2,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
    );
    
    // Contract has 3000 tokens, 3000 liabilities
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);
    
    // Withdraw from first stream at t=500 (500 tokens)
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id_1);
    
    // Contract has 2500 tokens, 2500 liabilities (500 withdrawn, 500 + 2000 remaining)
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_500);
    
    // Cancel second stream at t=500 (1000 accrued, 1000 refunded)
    ctx.client().cancel_stream(&stream_id_2);
    
    // Contract has 1500 tokens, 1500 liabilities (500 from stream 1, 1000 from stream 2)
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_500);
    
    // Manually add trapped funds
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &400);
    
    // Contract has 1900 tokens, 1500 liabilities, 400 excess
    let sweep_recipient = Address::generate(&ctx.env);
    let swept = ctx.client().sweep_excess(&sweep_recipient);
    
    assert_eq!(swept, 400);
    assert_eq!(ctx.token.balance(&sweep_recipient), 400);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_500);
}

/// Test sweep_excess can be called multiple times.
#[test]
fn sweep_excess_can_be_called_multiple_times() {
    let ctx = TestContext::setup();
    
    // Create stream
    let stream_id = ctx.create_default_stream();
    
    // Add excess and sweep first time
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &200);
    let sweep_recipient = Address::generate(&ctx.env);
    let swept_1 = ctx.client().sweep_excess(&sweep_recipient);
    assert_eq!(swept_1, 200);
    
    // Add more excess and sweep again
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &150);
    let swept_2 = ctx.client().sweep_excess(&sweep_recipient);
    assert_eq!(swept_2, 150);
    
    // Total swept
    assert_eq!(ctx.token.balance(&sweep_recipient), 350);
}

/// Test sweep_excess protects recipient funds (doesn't sweep liabilities).
#[test]
fn sweep_excess_protects_recipient_funds() {
    let ctx = TestContext::setup();
    
    // Create stream: 1000 tokens
    let stream_id = ctx.create_default_stream();
    
    // Advance time to 500s (500 tokens accrued)
    ctx.env.ledger().set_timestamp(500);
    
    // Contract has 1000 tokens, 1000 liabilities (even though only 500 accrued)
    // because the full deposit is still owed until withdrawn or cancelled
    let sweep_recipient = Address::generate(&ctx.env);
    let swept = ctx.client().sweep_excess(&sweep_recipient);
    
    // Should not sweep anything - all funds are liabilities
    assert_eq!(swept, 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
    
    // Recipient can still withdraw their accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 500);
}

/// Test sweep_excess after stream completion and withdrawal.
#[test]
fn sweep_excess_after_stream_completion() {
    let ctx = TestContext::setup();
    
    // Create stream: 1000 tokens over 1000 seconds
    let stream_id = ctx.create_default_stream();
    
    // Complete stream and withdraw all
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    
    // Contract should have 0 tokens, 0 liabilities
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    
    // Manually add some tokens (simulating trapped funds)
    ctx.token.transfer(&ctx.sender, &ctx.contract_id, &100);
    
    // Now contract has 100 tokens, 0 liabilities, 100 excess
    let sweep_recipient = Address::generate(&ctx.env);
    let swept = ctx.client().sweep_excess(&sweep_recipient);
    
    assert_eq!(swept, 100);
    assert_eq!(ctx.token.balance(&sweep_recipient), 100);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}
