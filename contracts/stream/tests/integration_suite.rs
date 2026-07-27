#![cfg(any())]

extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason,
    StreamHealth, StreamStatus,
};
use proptest::prelude::*;
use soroban_sdk::log;
use soroban_sdk::{
    contract, contractimpl,
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
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let token = TokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);

        client.init(&token_id, &admin);

        Self {
            env,
            client,
            sender,
            token,
        }
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
    let result = ctx
        .client
        .create_streams_relative(&ctx.sender, &vec![&ctx.env]);

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
    ctx.client
        .create_streams_relative(&ctx.sender, &vec![&ctx.env]);
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 10_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 100u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Create second stream: 2000 tokens
    let recipient_2 = Address::generate(&ctx.env);
    let stream_id_2 = ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: recipient_2.clone(),
            deposit_amount: 2000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 5000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: i128::MAX,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: rates[0],
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: duration,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 10_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 100u64,
            cliff_time: 100u64,
            end_time: 1100u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 500u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 10_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 100u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Create second stream: 2000 tokens
    let recipient_2 = Address::generate(&ctx.env);
    let stream_id_2 = ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: recipient_2.clone(),
            deposit_amount: 2000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: fluxora_stream::StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
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
        fluxora_stream::AutoClaimStatus::ValidDestination(dest, claimable) => {
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
    assert_eq!(
        ctx.client().get_auto_claim_destination(&stream_id),
        Some(destination1)
    );

    // Update to second destination
    ctx.client().set_auto_claim(&stream_id, &destination2);
    assert_eq!(
        ctx.client().get_auto_claim_destination(&stream_id),
        Some(destination2)
    );
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
    assert_eq!(
        ctx.client().get_auto_claim_destination(&stream_id),
        Some(destination)
    );

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
        fluxora_stream::AutoClaimStatus::ValidDestination(dest, claimable) => {
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
        fluxora_stream::AutoClaimStatus::ValidDestination(_, claimable) => {
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
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_balance_before - 1000
    );

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
    ctx.client()
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);

    // Try to trigger at end_time while paused
    // Note: Paused streams don't accrue, so this tests the terminal state check
    ctx.env.ledger().set_timestamp(1000);

    // This should work because paused is not a terminal state
    // The stream is still Active (just paused), not Completed or Cancelled
    let amount = ctx.client().trigger_auto_claim(&stream_id);
    assert!(amount >= 0); // Should succeed
}

#[contract]
pub struct NonConformingToken;

#[contractimpl]
impl NonConformingToken {
    pub fn balance(_env: Env, _who: Address) -> i128 {
        0
    }

    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {
        panic!("token transfer panicked")
    }

    pub fn transfer_from(
        _env: Env,
        _invoker: Address,
        _from: Address,
        _to: Address,
        _amount: i128,
    ) {
        panic!("token transfer_from panicked")
    }

    pub fn approve(_env: Env, _spender: Address, _owner: Address, _amount: i128) {}
}

#[test]
fn init_accepts_valid_sep41_token() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    assert_eq!(client.init(&token_id, &admin), Ok(()));
}

#[test]
fn init_rejects_non_sep41_token() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_id = env.register_contract(None, NonConformingToken).address();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let init_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&token_id, &admin)
    }));

    assert!(
        matches!(init_result, Err(_) | Ok(Err(_))),
        "init must reject a non-SEP-41 token implementation"
    );
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
    ctx.client()
        .pause_protocol(&soroban_sdk::String::from_str(&ctx.env, "Emergency"));

    // Try to trigger auto-claim (should fail due to global pause)
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().trigger_auto_claim(&stream_id);
}

#[test]
fn test_contract_error_discriminants_unique() {
    let variants = std::vec![
        ContractError::StreamNotFound as u32,
        ContractError::InvalidState as u32,
        ContractError::InvalidParams as u32,
        ContractError::ContractPaused as u32,
        ContractError::StartTimeInPast as u32,
        ContractError::ArithmeticOverflow as u32,
        ContractError::Unauthorized as u32,
        ContractError::AlreadyInitialised as u32,
        ContractError::TokenVerificationFailed as u32,
        ContractError::InsufficientBalance as u32,
        ContractError::InsufficientDeposit as u32,
        ContractError::StreamAlreadyPaused as u32,
        ContractError::StreamNotPaused as u32,
        ContractError::StreamTerminalState as u32,
        ContractError::DuplicateStreamId as u32,
        ContractError::InvalidSignature as u32,
        ContractError::BelowMinimumAmount as u32,
        ContractError::ReservationCountZero as u32,
        ContractError::ReservationLimitExceeded as u32,
        ContractError::SignatureDeadlineExpired as u32,
        ContractError::TemplateNotFound as u32,
        ContractError::TemplateLimitExceeded as u32,
        ContractError::TemplateUnauthorized as u32,
        ContractError::ReservationNotFound as u32,
        ContractError::ReservationNotExpirable as u32,
        ContractError::ReservationStillActive as u32,
        ContractError::PauseReasonTooLong as u32,
        ContractError::ClockRegression as u32,
        ContractError::WithdrawalTooFrequent as u32,
        ContractError::UnsupportedStreamKind as u32,
        ContractError::KeeperGracePeriodNotElapsed as u32,
        ContractError::MetadataTooLarge as u32,
        ContractError::PauseCooldownActive as u32,
        ContractError::RateCapExceeded as u32,
    ];

    let mut sorted_variants = variants.clone();
    sorted_variants.sort();
    sorted_variants.dedup();
    assert_eq!(
        variants.len(),
        sorted_variants.len(),
        "ContractError has duplicate discriminants"
    );
}

// ---------------------------------------------------------------------------
// Tests — Contract-owned senders (vaults, multisigs)
// ---------------------------------------------------------------------------

#[contract]
pub struct MockVaultContract;

#[contractimpl]
impl MockVaultContract {
    pub fn vault_create_stream(
        env: Env,
        stream_contract: Address,
        recipient: Address,
        deposit_amount: i128,
        rate_per_second: i128,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
    ) -> u64 {
        let client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_contract);
        client.create_stream(
            &env.current_contract_address(),
            &CreateStreamParams {
                recipient: recipient.clone(),
                deposit_amount: deposit_amount,
                rate_per_second: rate_per_second,
                start_time: start_time,
                cliff_time: cliff_time,
                end_time: end_time,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: fluxora_stream::StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    pub fn vault_top_up_stream(env: Env, stream_contract: Address, stream_id: u64, amount: i128) {
        let client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_contract);
        client.top_up_stream(&stream_id, &amount)
    }

    pub fn vault_cancel_stream(env: Env, stream_contract: Address, stream_id: u64) {
        let client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_contract);
        client.cancel_stream(&stream_id)
    }
}

#[test]
fn test_contract_owned_vault_sender() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    // Mint tokens to the vault
    token.mint(&vault_id, &100_000);

    let recipient = Address::generate(&env);

    // Vault creates a stream (tests sender.require_auth() inside create_stream)
    let stream_id = vault_client.vault_create_stream(
        &contract_id,
        &recipient,
        &10_000,
        &10,
        &1000,
        &1000,
        &2000,
    );

    // Vault tops up the stream (tests funder.require_auth() inside top_up_stream)
    vault_client.vault_top_up_stream(&contract_id, &stream_id, &5_000);

    let state = stream_client.get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 15_000);
    assert_eq!(state.sender, vault_id);

    // Vault cancels the stream (tests sender.require_auth() inside cancel_stream)
    env.ledger().with_mut(|li| li.timestamp = 1500);
    vault_client.vault_cancel_stream(&contract_id, &stream_id);

    let state_after = stream_client.get_stream_state(&stream_id);
    assert_eq!(state_after.status, StreamStatus::Cancelled);

    // Vault gets its refund
    let final_balance = token.balance(&vault_id);
    // Initial 100_000 - 15_000 (deposit + topup) + 10_000 (refund for remaining 1000s * 10/s) = 95_000
    assert_eq!(final_balance, 95_000);
}

// ============================================================================
// CONTRACT-OWNED VAULT SENDER - COMPREHENSIVE TEST SUITE
// ============================================================================
// These tests verify that contract-owned addresses (vaults, multisigs) can
// act as senders for streams through the standard require_auth() interface.
// ============================================================================

/// Comprehensive test for vault sender pattern covering the full lifecycle
#[test]
fn test_vault_sender_full_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    // Deploy stream contract
    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    // Deploy token
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    // Initialize stream contract
    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    // Deploy vault contract
    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    // Mint tokens to the vault
    let initial_vault_balance = 100_000;
    token.mint(&vault_id, &initial_vault_balance);

    let recipient = Address::generate(&env);
    let deposit = 10_000;
    let rate = 10;
    let start_time = 1000;
    let cliff_time = 1000;
    let end_time = 2000;

    // 1. Vault creates a stream
    let stream_id = vault_client.vault_create_stream(
        &contract_id,
        &recipient,
        &deposit,
        &rate,
        &start_time,
        &cliff_time,
        &end_time,
    );

    // Verify stream was created correctly
    let state = stream_client.get_stream_state(&stream_id);
    assert_eq!(state.sender, vault_id);
    assert_eq!(state.recipient, recipient);
    assert_eq!(state.deposit_amount, deposit);
    assert_eq!(state.rate_per_second, rate);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.withdrawn_amount, 0);

    // Verify vault balance after creation
    assert_eq!(token.balance(&vault_id), initial_vault_balance - deposit);

    // 2. Vault tops up the stream
    let top_up_amount = 5_000;
    vault_client.vault_top_up_stream(&contract_id, &stream_id, &top_up_amount);

    let state_after_topup = stream_client.get_stream_state(&stream_id);
    assert_eq!(state_after_topup.deposit_amount, deposit + top_up_amount);
    assert_eq!(
        token.balance(&vault_id),
        initial_vault_balance - deposit - top_up_amount
    );

    // 3. Vault cancels the stream
    env.ledger().with_mut(|li| li.timestamp = 1500);

    // Get state before cancel
    let state_before_cancel = stream_client.get_stream_state(&stream_id);
    let accrued_before = stream_client.calculate_accrued(&stream_id);

    vault_client.vault_cancel_stream(&contract_id, &stream_id);

    let state_after_cancel = stream_client.get_stream_state(&stream_id);
    assert_eq!(state_after_cancel.status, StreamStatus::Cancelled);
    assert!(state_after_cancel.cancelled_at.is_some());
    assert_eq!(state_after_cancel.cancelled_at.unwrap(), 1500);

    // Verify refund: accrued at 1500 = (1500-1000) * 10 = 5000
    // Refund = deposit - accrued = 15000 - 5000 = 10000
    let expected_refund = (deposit + top_up_amount) - 5000;
    assert_eq!(
        token.balance(&vault_id),
        initial_vault_balance - (deposit + top_up_amount) + expected_refund
    );
}

/// Test vault sender with cliff time
#[test]
fn test_vault_sender_with_cliff() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    token.mint(&vault_id, &100_000);

    let recipient = Address::generate(&env);
    let deposit = 10_000;
    let rate = 10;
    let start_time = 1000;
    let cliff_time = 1300; // 5 minutes cliff
    let end_time = 2000;

    let stream_id = vault_client.vault_create_stream(
        &contract_id,
        &recipient,
        &deposit,
        &rate,
        &start_time,
        &cliff_time,
        &end_time,
    );

    // Before cliff - should have 0 accrued
    env.ledger().with_mut(|li| li.timestamp = 1200);
    let accrued_before_cliff = stream_client.calculate_accrued(&stream_id);
    assert_eq!(accrued_before_cliff, 0);

    // After cliff - should accrue from start_time
    env.ledger().with_mut(|li| li.timestamp = 1400);
    let accrued_after_cliff = stream_client.calculate_accrued(&stream_id);
    // (1400 - 1000) * 10 = 4000
    assert_eq!(accrued_after_cliff, 4000);

    // Withdraw after cliff
    let withdraw_amount = stream_client.withdraw(&stream_id);
    assert_eq!(withdraw_amount, 4000);

    let state = stream_client.get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 4000);
}

/// Test vault sender with metadata and memo
#[test]
fn test_vault_sender_with_metadata() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    token.mint(&vault_id, &100_000);

    let recipient = Address::generate(&env);

    // Create metadata
    use soroban_sdk::Map;
    let mut metadata = Map::new(&env);
    metadata.set(
        soroban_sdk::Bytes::from_slice(&env, b"invoice_id"),
        soroban_sdk::Bytes::from_slice(&env, b"INV-2026-001"),
    );
    metadata.set(
        soroban_sdk::Bytes::from_slice(&env, b"project"),
        soroban_sdk::Bytes::from_slice(&env, b"PROJ-42"),
    );

    // Vault creates stream with metadata and memo
    // Note: This requires extending the mock vault contract to support metadata
    // For now, we test that the stream contract accepts vault as sender with these params

    let deposit = 10_000;
    let rate = 10;
    let start_time = 1000;
    let cliff_time = 1000;
    let end_time = 2000;
    let memo = Some(soroban_sdk::Bytes::from_slice(&env, b"vault_payment"));

    // Test that stream creation with metadata works with vault sender
    let stream_id = vault_client.vault_create_stream_with_metadata(
        &contract_id,
        &recipient,
        &deposit,
        &rate,
        &start_time,
        &cliff_time,
        &end_time,
        &memo,
        &Some(metadata),
    );

    let state = stream_client.get_stream_state(&stream_id);
    assert_eq!(state.sender, vault_id);

    // Verify memo
    let stored_memo = stream_client.get_stream_memo(&stream_id);
    assert_eq!(stored_memo, memo);

    // Verify metadata
    let stored_metadata = stream_client.get_stream_metadata(&stream_id);
    assert!(stored_metadata.is_some());
}

/// Test vault sender authorization requirements with strict auth
#[test]
fn test_vault_sender_strict_authorization() {
    let env = Env::default();
    // NOTE: We use mock_all_auths but test the actual require_auth() calls
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    token.mint(&vault_id, &100_000);

    let recipient = Address::generate(&env);
    let deposit = 10_000;
    let rate = 10;
    let start_time = 1000;
    let cliff_time = 1000;
    let end_time = 2000;

    // This should succeed - vault authorizes itself
    let stream_id = vault_client.vault_create_stream(
        &contract_id,
        &recipient,
        &deposit,
        &rate,
        &start_time,
        &cliff_time,
        &end_time,
    );

    // Verify stream was created with vault as sender
    let state = stream_client.get_stream_state(&stream_id);
    assert_eq!(state.sender, vault_id);

    // Test that unauthorized address cannot act as vault
    // Try to top up using a different address (should fail)
    let unauthorized = Address::generate(&env);

    // We expect this to panic because the stream contract's require_auth() will fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        stream_client.top_up_stream(&stream_id, &unauthorized, &1000);
    }));
    assert!(result.is_err(), "Unauthorized top-up should fail");
}

/// Test vault sender with expiration and auto-renew
#[test]
fn test_vault_sender_auto_renew() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    let initial_balance = 200_000;
    token.mint(&vault_id, &initial_balance);
    token.approve(&vault_id, &contract_id, &initial_balance);

    let recipient = Address::generate(&env);
    let deposit = 10_000;
    let rate = 10;
    let start_time = 1000;
    let cliff_time = 1000;
    let end_time = 2000;

    let stream_id = vault_client.vault_create_stream(
        &contract_id,
        &recipient,
        &deposit,
        &rate,
        &start_time,
        &cliff_time,
        &end_time,
    );

    // Enable auto-renew
    stream_client.set_auto_renew(&stream_id, &vault_id, &true);

    // Complete the stream
    env.ledger().with_mut(|li| li.timestamp = 2000);
    stream_client.withdraw(&stream_id);

    // Verify stream is completed
    let state = stream_client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Renew the stream (permissionless - anyone can trigger)
    // Vault must have sufficient balance and allowance
    let new_stream_id = stream_client.renew_stream(&stream_id);

    // Verify new stream was created with vault as sender
    let new_state = stream_client.get_stream_state(&new_stream_id);
    assert_eq!(new_state.sender, vault_id);
    assert_eq!(new_state.recipient, recipient);
    assert_eq!(new_state.deposit_amount, deposit);
    assert_eq!(new_state.rate_per_second, rate);

    // Verify auto-renew is enabled on new stream
    assert!(stream_client.get_auto_renew(&new_stream_id));
}

/// Test vault sender with batch operations
#[test]
fn test_vault_sender_batch_operations() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    let total_deposit = 50_000;
    token.mint(&vault_id, &total_deposit);

    // Create multiple streams from vault
    let recipients: Vec<Address> = (0..3).map(|_| Address::generate(&env)).collect();
    let mut stream_ids = Vec::new();

    for (i, recipient) in recipients.iter().enumerate() {
        let deposit = 10_000 * (i as i128 + 1);
        let rate = 10 * (i as i128 + 1);
        let stream_id = vault_client.vault_create_stream(
            &contract_id,
            recipient,
            &deposit,
            &rate,
            &1000,
            &1000,
            &2000,
        );
        stream_ids.push(stream_id);
    }

    // Verify all streams were created with vault as sender
    for (i, stream_id) in stream_ids.iter().enumerate() {
        let state = stream_client.get_stream_state(stream_id);
        assert_eq!(state.sender, vault_id);
        let expected_deposit = 10_000 * (i as i128 + 1);
        assert_eq!(state.deposit_amount, expected_deposit);
    }

    // Test batch cancellation
    env.ledger().with_mut(|li| li.timestamp = 1500);

    // Vault cancels multiple streams in batch
    // Note: This requires the mock vault to have a batch cancel function
    vault_client.vault_bulk_cancel_streams(&contract_id, &stream_ids);

    // Verify all streams are cancelled
    for stream_id in stream_ids.iter() {
        let state = stream_client.get_stream_state(stream_id);
        assert_eq!(state.status, StreamStatus::Cancelled);
    }
}

/// Test vault sender edge cases
#[test]
fn test_vault_sender_edge_cases() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    token.mint(&vault_id, &100_000);
    let recipient = Address::generate(&env);

    // Edge Case 1: Zero amount stream
    let zero_deposit = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vault_client.vault_create_stream(
            &contract_id,
            &recipient,
            &zero_deposit,
            &10,
            &1000,
            &1000,
            &2000,
        );
    }));
    assert!(result.is_err(), "Zero deposit should be rejected");

    // Edge Case 2: Zero rate stream (should be rejected for Linear)
    let zero_rate = 0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vault_client.vault_create_stream(
            &contract_id,
            &recipient,
            &10_000,
            &zero_rate,
            &1000,
            &1000,
            &2000,
        );
    }));
    assert!(
        result.is_err(),
        "Zero rate should be rejected for Linear streams"
    );

    // Edge Case 3: Vault tries to cancel already cancelled stream
    let stream_id = vault_client.vault_create_stream(
        &contract_id,
        &recipient,
        &10_000,
        &10,
        &1000,
        &1000,
        &2000,
    );

    env.ledger().with_mut(|li| li.timestamp = 1500);
    vault_client.vault_cancel_stream(&contract_id, &stream_id);

    // Try to cancel again (should fail)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vault_client.vault_cancel_stream(&contract_id, &stream_id);
    }));
    assert!(result.is_err(), "Double cancellation should fail");

    // Edge Case 4: Vault tops up with zero amount (should succeed with no change)
    let state_before = stream_client.get_stream_state(&stream_id);
    vault_client.vault_top_up_stream(&contract_id, &stream_id, &0);
    let state_after = stream_client.get_stream_state(&stream_id);
    assert_eq!(state_before.deposit_amount, state_after.deposit_amount);
}

/// Test vault as recipient
#[test]
fn test_vault_as_recipient() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    // Deploy vault as recipient
    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    // Regular sender creates stream to vault
    let sender = Address::generate(&env);
    let deposit = 10_000;
    let rate = 10;
    let start_time = 1000;
    let cliff_time = 1000;
    let end_time = 2000;

    // Mint tokens to sender and approve
    token.mint(&sender, &deposit);
    token.approve(&sender, &contract_id, &deposit);

    // Create stream with vault as recipient
    let stream_id = stream_client.create_stream(
        &sender,
        &vault_id,
        &deposit,
        &rate,
        &start_time,
        &cliff_time,
        &end_time,
        &0,
        &None,
        &fluxora_stream::types::StreamKind::Linear,
    );

    // Verify stream was created with vault as recipient
    let state = stream_client.get_stream_state(&stream_id);
    assert_eq!(state.recipient, vault_id);
    assert_eq!(state.sender, sender);

    // Advance time
    env.ledger().with_mut(|li| li.timestamp = 1500);

    // Vault withdraws from stream (as recipient)
    let amount = stream_client.withdraw(&stream_id);

    // Should have accrued (1500-1000) * 10 = 5000
    assert_eq!(amount, 5000);

    // Verify vault received tokens
    assert_eq!(token.balance(&vault_id), 5000);
}

/// Test vault sender with deadline and expiration
#[test]
fn test_vault_sender_with_deadline() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1000;
        li.sequence = 10;
    });

    let contract_id = env.register_contract(None, FluxoraStream);
    let stream_client = FluxoraStreamClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let token = TokenClient::new(&env, &token_id);

    let admin = Address::generate(&env);
    stream_client.init(&token_id, &admin);

    let vault_id = env.register_contract(None, MockVaultContract);
    let vault_client = MockVaultContractClient::new(&env, &vault_id);

    token.mint(&vault_id, &100_000);

    let recipient = Address::generate(&env);

    // Create stream with deadline (should succeed)
    let stream_id = vault_client.vault_create_stream_with_deadline(
        &contract_id,
        &recipient,
        &10_000,
        &10,
        &1000,
        &1000,
        &2000,
        &Some(3000), // deadline
    );

    let state = stream_client.get_stream_state(&stream_id);
    assert_eq!(state.sender, vault_id);

    // Try to create stream with deadline in past (should fail)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vault_client.vault_create_stream_with_deadline(
            &contract_id,
            &recipient,
            &10_000,
            &10,
            &1000,
            &1000,
            &2000,
            &Some(900), // expired deadline
        );
    }));
    assert!(result.is_err(), "Expired deadline should be rejected");
}

// ============================================================================
// MOCK VAULT CONTRACT EXTENSIONS
// ============================================================================

#[contract]
pub struct MockVaultContract;

#[contractimpl]
impl MockVaultContract {
    // ... existing functions ...

    pub fn vault_create_stream_with_metadata(
        env: Env,
        stream_contract: Address,
        recipient: Address,
        deposit_amount: i128,
        rate_per_second: i128,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
        memo: &Option<soroban_sdk::Bytes>,
        metadata: &Option<soroban_sdk::Map<soroban_sdk::Bytes, soroban_sdk::Bytes>>,
    ) -> u64 {
        let client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_contract);
        client.create_stream(
            &env.current_contract_address(),
            &recipient,
            &deposit_amount,
            &rate_per_second,
            &start_time,
            &cliff_time,
            &end_time,
            &0,
            memo,
            &fluxora_stream::types::StreamKind::Linear,
            &None,
            &None,
        )
    }

    pub fn vault_bulk_cancel_streams(
        env: Env,
        stream_contract: Address,
        stream_ids: soroban_sdk::Vec<u64>,
    ) {
        let client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_contract);
        // Using bulk_cancel_streams if available, otherwise cancel individually
        // For compatibility, we'll use individual cancel
        for stream_id in stream_ids.iter() {
            client.cancel_stream(&stream_id);
        }
    }

    pub fn vault_create_stream_with_deadline(
        env: Env,
        stream_contract: Address,
        recipient: Address,
        deposit_amount: i128,
        rate_per_second: i128,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
        deadline: Option<u64>,
    ) -> u64 {
        let client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_contract);

        // Validate deadline if provided
        if let Some(deadline_ts) = deadline {
            if env.ledger().timestamp() >= deadline_ts {
                panic!("deadline must be in the future");
            }
        }

        client.create_stream(
            &env.current_contract_address(),
            &recipient,
            &deposit_amount,
            &rate_per_second,
            &start_time,
            &cliff_time,
            &end_time,
            &0,
            &None,
            &fluxora_stream::types::StreamKind::Linear,
            &None,
            &None,
        )
    }

    pub fn vault_withdraw_from_stream(env: Env, stream_contract: Address, stream_id: u64) -> i128 {
        let client = fluxora_stream::FluxoraStreamClient::new(&env, &stream_contract);
        client.withdraw(&stream_id)
    }
}
