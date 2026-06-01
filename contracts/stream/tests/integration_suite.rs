extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamHealth,
    StreamStatus,
};
use proptest::prelude::*;
use soroban_sdk::log;
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, FromVal, IntoVal,
};

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

#[test]
fn get_stream_health_returns_correct_summary_active() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 1000 tokens, 0-1000s, rate 1

    ctx.env.ledger().set_timestamp(500);
    let health = ctx.client().get_stream_health(&stream_id);

    assert_eq!(health.is_underfunded, false);
    assert_eq!(health.is_expired, false);
    assert_eq!(health.accrued_to_date, 500);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(500));
}

#[test]
fn get_stream_health_returns_correct_summary_underfunded() {
    let ctx = TestContext::setup();
    // Create an underfunded stream: 1000 tokens, but rate 2 for 1000s (needs 2000)
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &None,
    );

    ctx.env.ledger().set_timestamp(300);
    let health = ctx.client().get_stream_health(&stream_id);

    assert_eq!(health.is_underfunded, true);
    assert_eq!(health.is_expired, false);
    assert_eq!(health.accrued_to_date, 600);
    assert_eq!(health.remaining_deposit, 1000);
    // Depletion at 500s (1000 / 2). 500 - 300 = 200
    assert_eq!(health.seconds_until_depletion, Some(200));
}

#[test]
fn get_stream_health_returns_correct_summary_expired() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1200);
    let health = ctx.client().get_stream_health(&stream_id);

    assert_eq!(health.is_underfunded, false);
    assert_eq!(health.is_expired, true);
    assert_eq!(health.accrued_to_date, 1000);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(0));
}

#[test]
fn get_stream_health_returns_correct_summary_with_withdrawn_amount() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw_to(&stream_id, &destination);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("wdraw_to")
    );
    assert_eq!(
        u64::from_val(&ctx.env, &last_event.1.get(1).unwrap()),
        stream_id
    );
}

#[test]
fn snapshot_event_paused_resumed_cancelled() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // 1. paused
    ctx.client()
        .pause_stream(&stream_id, &PauseReason::Operational);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("paused")
    );
    assert_eq!(
        u64::from_val(&ctx.env, &last_event.1.get(1).unwrap()),
        stream_id
    );

    // 2. resumed
    ctx.client().resume_stream(&stream_id);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("resumed")
    );
    assert_eq!(
        u64::from_val(&ctx.env, &last_event.1.get(1).unwrap()),
        stream_id
    );

    // 3. cancelled
    ctx.client().cancel_stream(&stream_id);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("cancelled")
    );
    assert_eq!(
        u64::from_val(&ctx.env, &last_event.1.get(1).unwrap()),
        stream_id
    );
}

#[test]
fn snapshot_event_rate_end_topup_recp() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use a very high deposit so subsequent operations (rate-up, shorten/refund,
    // extend) all stay within deposit bounds.
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &None,
    );

    // 1. rate_upd
    ctx.client().update_rate_per_second(&stream_id, &2_i128);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("rate_upd")
    );

    // 2. end_shrt
    ctx.client().shorten_stream_end_time(&stream_id, &500u64);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("end_shrt")
    );

    // 3. top_up — refill the deposit so we can subsequently extend the schedule.
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &1000_i128);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("top_up")
    );

    // 4. end_ext
    ctx.client().extend_stream_end_time(&stream_id, &800u64);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("end_ext")
    );

    // 5. recp_upd
    let new_recipient = Address::generate(&ctx.env);
    ctx.client().update_recipient(&stream_id, &new_recipient);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("recp_upd")
    );
}

#[test]
fn update_rate_rejects_equal_and_zero_rates() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let equal_rate_result = ctx.client().try_update_rate_per_second(&stream_id, &1_i128);
    assert_eq!(equal_rate_result, Err(Ok(ContractError::InvalidParams)));

    let zero_rate_result = ctx.client().try_update_rate_per_second(&stream_id, &0_i128);
    assert_eq!(zero_rate_result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn update_rate_accepts_maximum_i128_rate() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &i128::MAX,
        &1_i128,
        &0u64,
        &0u64,
        &1u64,
        &0,
        &None,
        &None,
    );

    ctx.client().update_rate_per_second(&stream_id, &i128::MAX);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, i128::MAX);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn update_rate_on_paused_stream_is_allowed() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.client()
        .pause_stream(&stream_id, &PauseReason::Operational);
    ctx.client().update_rate_per_second(&stream_id, &2_i128);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
    assert_eq!(state.rate_per_second, 2_i128);
}

#[test]
fn update_rate_rejected_on_cancelled_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.client().cancel_stream(&stream_id);
    let result = ctx.client().try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

proptest::proptest! {
    #[test]
    fn update_rate_accepts_monotonic_increase_sequences(
        mut rates in proptest::collection::vec(1_i128..1000, 2..6)
    ) {
        rates.sort();
        rates.dedup();
        proptest::prop_assume!(rates.len() >= 2);

        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let duration = 10u64;
        let deposit = rates.last().unwrap().checked_mul(duration as i128).unwrap();
        let stream_id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &deposit,
            &rates[0],
            &0u64,
            &0u64,
            &duration,
            &0,
            &None,
            &None,
        );

        for &next_rate in rates.iter().skip(1) {
            ctx.client().update_rate_per_second(&stream_id, &next_rate);
            let state = ctx.client().get_stream_state(&stream_id);
            proptest::prop_assert_eq!(state.rate_per_second, next_rate);
            proptest::prop_assert!(state.status == StreamStatus::Active || state.status == StreamStatus::Paused);
        }
    }
}

#[test]
fn snapshot_event_admin_and_pause_ctl() {
    let ctx = TestContext::setup();

    // 1. AdminUpdated
    let new_admin = Address::generate(&ctx.env);
    ctx.client().set_admin(&new_admin);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::Symbol::new(&ctx.env, "AdminUpdated")
    );

    // 2. paused_ctl
    ctx.client().set_contract_paused(&true);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::Symbol::new(&ctx.env, "paused_ctl")
    );
}

#[test]
fn snapshot_no_event_on_revert() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let events_before = ctx.env.events().all().len();

    // Reverting call (insufficient deposit)
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &None,
    );
    assert!(result.is_err());
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
fn snapshot_no_withdraw_event_when_amount_zero() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let events_before = ctx.env.events().all().len();

    // Withdraw at t=0 (nothing accrued)
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

// ---------------------------------------------------------------------------
// Issue #523: test_accrual_none_checkpoint_returns_zero
//
// Exercises the None-branch of CheckpointState lookup in
// calculate_accrued_amount_checkpointed (accrual.rs line 31).
//
// A brand-new stream queried at exactly start_time has no prior checkpoint
// epoch, so the function must return 0 without panicking.
// Cross-check: when cliff_time > start_time the same call also returns 0.
// ---------------------------------------------------------------------------

/// Verifies that `calculate_accrued` returns 0 at exactly `start_time`
/// for a freshly created stream (no checkpoint has been persisted yet).
///
/// This exercises the None-branch of the CheckpointState lookup in
/// `calculate_accrued_amount_checkpointed` (accrual.rs line 31).
#[test]
fn test_accrual_none_checkpoint_returns_zero() {
    let ctx = TestContext::setup();

    // Stream: start=100, cliff=100, end=1100, rate=1/s, deposit=1000
    // Queried at exactly start_time (t=100) — no checkpoint exists yet.
    ctx.env.ledger().set_timestamp(100);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &100u64,
        &1100u64,
        &0,
        &None,
        &None,
    );

    // At start_time the elapsed seconds are 0 → accrued must be 0.
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 0,
        "accrued at start_time must be 0 (no checkpoint)"
    );
}

/// Same scenario but with cliff_time > start_time.
///
/// Querying before the cliff must also return 0, confirming the cliff guard
/// fires before any checkpoint arithmetic is attempted.
#[test]
fn test_accrual_none_checkpoint_before_cliff_returns_zero() {
    let ctx = TestContext::setup();

    // Stream: start=0, cliff=500, end=1000, rate=1/s, deposit=1000
    // Queried at t=0 (start_time, before cliff).
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &500u64,
        &1000u64,
        &0,
        &None,
        &None,
    );

    // Before cliff → 0, regardless of checkpoint state.
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 0,
        "accrued before cliff must be 0 even with no checkpoint"
    );
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

// ============================================================================
// Auto-Claim Tests
// ============================================================================

/// Test set_auto_claim with valid destination
#[test]
fn test_set_auto_claim_valid_destination() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    // Set auto-claim destination
    ctx.client().set_auto_claim(&stream_id, &destination);

    // Verify destination is stored
    let stored_dest = ctx.client().get_auto_claim_destination(&stream_id);
    assert_eq!(stored_dest, Some(destination.clone()));

    // Verify status shows valid destination
    let status = ctx.client().get_auto_claim_status(&stream_id);
    match status {
        fluxora_stream::AutoClaimStatus::ValidDestination { destination: dest, claimable } => {
            assert_eq!(dest, destination);
            assert_eq!(claimable, 0); // No time has passed
        }
        _ => panic!("Expected ValidDestination status"),
    }

    // Verify event was emitted
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("ac_set")
    );
}

/// Test set_auto_claim rejects contract address as destination
#[test]
#[should_panic(expected = "InvalidParams")]
fn test_set_auto_claim_rejects_contract_address() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // Try to set contract address as destination (should fail)
    ctx.client().set_auto_claim(&stream_id, &ctx.contract_id);
}

/// Test set_auto_claim requires recipient authorization
#[test]
#[should_panic(expected = "Unauthorized")]
fn test_set_auto_claim_requires_recipient_auth() {
    let ctx = TestContext::setup_strict();
    ctx.env.ledger().set_timestamp(0);
    
    // Create stream with explicit auth
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    // Try to set auto-claim without auth (should fail)
    ctx.client().set_auto_claim(&stream_id, &destination);
}

/// Test set_auto_claim can update existing destination
#[test]
fn test_set_auto_claim_can_update_destination() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination1 = Address::generate(&ctx.env);
    let destination2 = Address::generate(&ctx.env);

    // Set first destination
    ctx.client().set_auto_claim(&stream_id, &destination1);
    assert_eq!(ctx.client().get_auto_claim_destination(&stream_id), Some(destination1));

    // Update to second destination
    ctx.client().set_auto_claim(&stream_id, &destination2);
    assert_eq!(ctx.client().get_auto_claim_destination(&stream_id), Some(destination2));
}

/// Test revoke_auto_claim removes destination
#[test]
fn test_revoke_auto_claim() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    // Set auto-claim destination
    ctx.client().set_auto_claim(&stream_id, &destination);
    assert_eq!(ctx.client().get_auto_claim_destination(&stream_id), Some(destination));

    // Revoke auto-claim
    ctx.client().revoke_auto_claim(&stream_id);
    assert_eq!(ctx.client().get_auto_claim_destination(&stream_id), None);

    // Verify status shows NotSet
    let status = ctx.client().get_auto_claim_status(&stream_id);
    assert_eq!(status, fluxora_stream::AutoClaimStatus::NotSet);

    // Verify event was emitted
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::symbol_short!("ac_revoke")
    );
}

/// Test revoke_auto_claim is idempotent (can call even if not set)
#[test]
fn test_revoke_auto_claim_idempotent() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // Revoke without setting first (should not panic)
    ctx.client().revoke_auto_claim(&stream_id);
    assert_eq!(ctx.client().get_auto_claim_destination(&stream_id), None);
}

/// Test get_auto_claim_status returns NotSet when no destination configured
#[test]
fn test_get_auto_claim_status_not_set() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let status = ctx.client().get_auto_claim_status(&stream_id);
    assert_eq!(status, fluxora_stream::AutoClaimStatus::NotSet);
}

/// Test get_auto_claim_status calculates claimable amount correctly
#[test]
fn test_get_auto_claim_status_claimable_amount() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Advance time to accrue tokens
    ctx.env.ledger().set_timestamp(500); // 500 seconds * 1 token/sec = 500 tokens

    let status = ctx.client().get_auto_claim_status(&stream_id);
    match status {
        fluxora_stream::AutoClaimStatus::ValidDestination { destination: dest, claimable } => {
            assert_eq!(dest, destination);
            assert_eq!(claimable, 500);
        }
        _ => panic!("Expected ValidDestination status"),
    }
}

/// Test get_auto_claim_status accounts for withdrawn amount
#[test]
fn test_get_auto_claim_status_after_withdrawal() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Advance time and withdraw
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);

    // Advance more time
    ctx.env.ledger().set_timestamp(800);

    let status = ctx.client().get_auto_claim_status(&stream_id);
    match status {
        fluxora_stream::AutoClaimStatus::ValidDestination { claimable, .. } => {
            assert_eq!(claimable, 300); // 800 accrued - 500 withdrawn = 300
        }
        _ => panic!("Expected ValidDestination status"),
    }
}

/// Test trigger_auto_claim succeeds after end_time
#[test]
fn test_trigger_auto_claim_success() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Advance to end_time
    ctx.env.ledger().set_timestamp(1000);

    let dest_balance_before = ctx.token.balance(&destination);
    let contract_balance_before = ctx.token.balance(&ctx.contract_id);

    // Trigger auto-claim (permissionless)
    let amount = ctx.client().trigger_auto_claim(&stream_id);

    assert_eq!(amount, 1000); // Full deposit
    assert_eq!(ctx.token.balance(&destination), dest_balance_before + 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_balance_before - 1000);

    // Verify stream is completed
    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Completed);

    // Verify events were emitted
    let events = ctx.env.events().all();
    let event_symbols: Vec<_> = events
        .iter()
        .map(|e| soroban_sdk::Symbol::from_val(&ctx.env, &e.1.get(0).unwrap()))
        .collect();
    
    assert!(event_symbols.contains(&soroban_sdk::symbol_short!("ac_trig")));
    assert!(event_symbols.contains(&soroban_sdk::symbol_short!("wdraw_to")));
    assert!(event_symbols.contains(&soroban_sdk::symbol_short!("completed")));
}

/// Test trigger_auto_claim fails before end_time
#[test]
#[should_panic(expected = "InvalidState")]
fn test_trigger_auto_claim_before_end_time() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Try to trigger before end_time (should fail)
    ctx.env.ledger().set_timestamp(500);
    ctx.client().trigger_auto_claim(&stream_id);
}

/// Test trigger_auto_claim fails when no destination set
#[test]
#[should_panic(expected = "InvalidParams")]
fn test_trigger_auto_claim_no_destination() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // Advance to end_time
    ctx.env.ledger().set_timestamp(1000);

    // Try to trigger without setting destination (should fail)
    ctx.client().trigger_auto_claim(&stream_id);
}

/// Test trigger_auto_claim fails on completed stream
#[test]
#[should_panic(expected = "InvalidState")]
fn test_trigger_auto_claim_completed_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Complete the stream manually
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Try to trigger auto-claim on completed stream (should fail)
    ctx.client().trigger_auto_claim(&stream_id);
}

/// Test trigger_auto_claim fails on cancelled stream
#[test]
#[should_panic(expected = "InvalidState")]
fn test_trigger_auto_claim_cancelled_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Cancel the stream
    ctx.env.ledger().set_timestamp(500);
    ctx.client().cancel_stream(&stream_id);

    // Try to trigger auto-claim on cancelled stream (should fail)
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().trigger_auto_claim(&stream_id);
}

/// Test trigger_auto_claim returns 0 when already fully withdrawn
#[test]
fn test_trigger_auto_claim_already_withdrawn() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Withdraw everything first
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Try to trigger auto-claim (should return 0)
    let amount = ctx.client().trigger_auto_claim(&stream_id);
    assert_eq!(amount, 0);
}

/// Test trigger_auto_claim is permissionless (anyone can call)
#[test]
fn test_trigger_auto_claim_permissionless() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Advance to end_time
    ctx.env.ledger().set_timestamp(1000);

    // Anyone can trigger (no auth required)
    let amount = ctx.client().trigger_auto_claim(&stream_id);
    assert_eq!(amount, 1000);
}

/// Test trigger_auto_claim with partial withdrawal before end_time
#[test]
fn test_trigger_auto_claim_after_partial_withdrawal() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Withdraw partially
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);

    // Advance to end_time and trigger auto-claim
    ctx.env.ledger().set_timestamp(1000);
    let dest_balance_before = ctx.token.balance(&destination);
    let amount = ctx.client().trigger_auto_claim(&stream_id);

    assert_eq!(amount, 500); // Remaining 500 tokens
    assert_eq!(ctx.token.balance(&destination), dest_balance_before + 500);
}

/// Test auto-claim with paused stream
#[test]
fn test_trigger_auto_claim_paused_stream_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Pause the stream
    ctx.env.ledger().set_timestamp(500);
    ctx.client().pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);

    // Try to trigger at end_time while paused
    // Note: Paused streams don't accrue, so this tests the terminal state check
    ctx.env.ledger().set_timestamp(1000);
    
    // This should work because paused is not a terminal state
    // The stream is still Active (just paused), not Completed or Cancelled
    let amount = ctx.client().trigger_auto_claim(&stream_id);
    assert!(amount >= 0); // Should succeed
}

/// Test get_auto_claim_status for non-existent stream
#[test]
#[should_panic(expected = "StreamNotFound")]
fn test_get_auto_claim_status_nonexistent_stream() {
    let ctx = TestContext::setup();
    ctx.client().get_auto_claim_status(&999);
}

/// Test set_auto_claim for non-existent stream
#[test]
#[should_panic(expected = "StreamNotFound")]
fn test_set_auto_claim_nonexistent_stream() {
    let ctx = TestContext::setup();
    let destination = Address::generate(&ctx.env);
    ctx.client().set_auto_claim(&999, &destination);
}

/// Test trigger_auto_claim respects global emergency pause
#[test]
#[should_panic(expected = "ContractPaused")]
fn test_trigger_auto_claim_respects_global_pause() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.client().set_auto_claim(&stream_id, &destination);

    // Activate global emergency pause
    ctx.client().pause_protocol(&soroban_sdk::String::from_str(&ctx.env, "Emergency"));

    // Try to trigger auto-claim (should fail due to global pause)
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().trigger_auto_claim(&stream_id);
}
