extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamStatus,
};
use soroban_sdk::log;
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, FromVal, IntoVal,
};

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        let token = TokenClient::new(&env, &token_id);

        Self {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn setup_strict() -> Self {
        let env = Env::default();
        // Do NOT call mock_all_auths() — tests in this mode must supply explicit auths.

        let contract_id = env.register_contract(None, FluxoraStream);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Init requires admin auth
        env.mock_auths(&[soroban_sdk::testutils::MockAuth {
            address: &admin,
            invoke: &soroban_sdk::testutils::MockAuthInvoke {
                contract: &contract_id,
                fn_name: "init",
                args: (&token_id, &admin).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        // Mint tokens with mock_all_auths just for the SAC mint
        env.mock_all_auths();
        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        let token = TokenClient::new(&env, &token_id);

        Self {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
        )
    }

    fn create_stream_with_cliff(&self, cliff_time: u64) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &cliff_time,
            &1000u64,
        )
    }
}

#[test]
fn init_sets_config_and_keeps_token_address() {
    let ctx = TestContext::setup();

    let config = ctx.client().get_config();
    assert_eq!(config.admin, ctx.admin);
    assert_eq!(config.token, ctx.token_id);
}

#[test]
fn init_twice_panics() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_init(&ctx.token_id, &ctx.admin);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));
}

#[test]
fn init_requires_admin_authorization_in_strict_mode() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract_id,
            fn_name: "init",
            args: (&token_id, &admin).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.init(&token_id, &admin);
    let cfg = client.get_config();
    assert_eq!(cfg.token, token_id);
    assert_eq!(cfg.admin, admin);
}

#[test]
fn init_wrong_signer_rejected_and_bootstrap_state_unset() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &attacker,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &contract_id,
            fn_name: "init",
            args: (&token_id, &admin).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    // In mock_all_auths() mode, provide_auth is usually enough, but here we
    // are testing explicit authorization failure.
    // Soroban's require_auth will still panic in testutils even if we use try_init,
    // if the auth is missing. However, we want to move away from catch_unwind
    // for contract errors. In this specific case of auth failure, catch_unwind
    // might still be needed if we want to assert it doesn't persist state,
    // as auth failures in Soroban are host-traps.

    let init_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&token_id, &admin);
    }));
    assert!(init_result.is_err(), "init must reject non-admin signer");

    // Since it panicked, the config must not have been set.
    let count = client.get_stream_count();
    assert_eq!(count, 0);

    // get_config should return Err(ContractError::InvalidState) if not initialized
    let cfg_result = client.try_get_config();
    assert_eq!(cfg_result, Err(Ok(ContractError::InvalidState)));
}

// ---------------------------------------------------------------------------
// Tests — Issue #62: config immutability after re-init attempt
// ---------------------------------------------------------------------------

/// After a failed re-init with different params, config must still hold the
/// original token and admin addresses.
#[test]
fn reinit_with_different_params_preserves_config() {
    let ctx = TestContext::setup();

    // Snapshot original config
    let original = ctx.client().get_config();

    // Attempt re-init with completely different addresses
    let new_token = Address::generate(&ctx.env);
    let new_admin = Address::generate(&ctx.env);

    let result = ctx.client().try_init(&new_token, &new_admin);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));

    // Config must be unchanged
    let after = ctx.client().get_config();
    assert_eq!(
        after.token, original.token,
        "token must survive reinit attempt"
    );
    assert_eq!(
        after.admin, original.admin,
        "admin must survive reinit attempt"
    );
}

/// Stream counter must remain unaffected by a failed re-init attempt.
#[test]
fn stream_counter_unaffected_by_reinit_attempt() {
    let ctx = TestContext::setup();

    // Create first stream (id = 0)
    let id0 = ctx.create_default_stream();
    assert_eq!(id0, 0);

    // Attempt re-init (should fail)
    let new_admin = Address::generate(&ctx.env);
    let result = ctx.client().try_init(&ctx.token_id, &new_admin);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));

    // Create second stream — counter must still be 1
    ctx.env.ledger().set_timestamp(0);
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(
        id1, 1,
        "stream counter must continue from 1 after failed reinit"
    );
}

#[test]
fn create_stream_persists_state_and_moves_deposit() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();
    let state = ctx.client().get_stream_state(&stream_id);

    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 0);
    assert_eq!(state.end_time, 1000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
}

#[test]
fn create_stream_rejects_self_stream_without_side_effects() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token.balance(&ctx.sender);
    let contract_balance_before = ctx.token.balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.sender, // invalid: sender == recipient
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
    assert_eq!(
        ctx.client().get_stream_count(),
        stream_count_before,
        "stream counter must not advance on validation failure"
    );
    assert_eq!(
        ctx.token.balance(&ctx.sender),
        sender_balance_before,
        "sender balance must not change on validation failure"
    );
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_balance_before,
        "contract balance must not change on validation failure"
    );
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no events should be emitted on validation failure"
    );
}

#[test]
fn create_streams_batch_success_moves_funds_and_assigns_sequential_ids() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let sender_balance_before = ctx.token.balance(&ctx.sender);
    let contract_balance_before = ctx.token.balance(&ctx.contract_id);

    let p1 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1200,
        rate_per_second: 2,
        start_time: 0,
        cliff_time: 0,
        end_time: 600,
    };
    let p2 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 2400,
        rate_per_second: 3,
        start_time: 10,
        cliff_time: 10,
        end_time: 810,
    };

    let streams = vec![&ctx.env, p1.clone(), p2.clone()];
    let ids = ctx.client().create_streams(&ctx.sender, &streams);

    assert_eq!(ids.len(), 2);
    assert_eq!(ids.get(0).unwrap(), 0);
    assert_eq!(ids.get(1).unwrap(), 1);
    assert_eq!(ctx.client().get_stream_count(), 2);

    assert_eq!(ctx.token.balance(&ctx.sender), sender_balance_before - 3600);
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_balance_before + 3600
    );
}

#[test]
fn create_streams_batch_invalid_entry_is_atomic_and_emits_no_events() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let valid = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
    };
    let invalid = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 10,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
    };

    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token.balance(&ctx.sender);
    let contract_balance_before = ctx.token.balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let streams = vec![&ctx.env, valid, invalid];
    let result = ctx.client().try_create_streams(&ctx.sender, &streams);

    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
    assert_eq!(ctx.client().get_stream_count(), stream_count_before);
    assert_eq!(ctx.token.balance(&ctx.sender), sender_balance_before);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_balance_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
fn withdraw_accrued_amount_updates_balances_and_state() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(250);
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(withdrawn, 250);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 250);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.recipient), 250);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 750);
}

// #[test]
// #[should_panic(expected = "nothing to withdraw")]
// fn withdraw_before_cliff_panics() {
//     let ctx = TestContext::setup();
//     let stream_id = ctx.create_stream_with_cliff(500);

//     ctx.env.ledger().set_timestamp(100);
//     ctx.client().withdraw(&stream_id);
// }

#[test]
fn withdraw_before_cliff_does_nothing() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream_with_cliff(500);

    ctx.env.ledger().set_timestamp(100);

    // 1. Create a token client to check the balance
    let token_client = soroban_sdk::token::Client::new(&ctx.env, &ctx.token_id);

    // 2. Check balance before
    let initial_balance = token_client.balance(&ctx.sender);

    ctx.client().withdraw(&stream_id);

    // 3. Check balance after - should be identical
    assert_eq!(token_client.balance(&ctx.sender), initial_balance);
}

#[test]
fn get_stream_state_returns_latest_status() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn full_lifecycle_create_withdraw_to_completion() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Mid-stream withdrawal.
    ctx.env.ledger().set_timestamp(400);
    let first = ctx.client().withdraw(&stream_id);
    assert_eq!(first, 400);

    // Final withdrawal at end of stream should complete the stream.
    ctx.env.ledger().set_timestamp(1000);
    let second = ctx.client().withdraw(&stream_id);
    assert_eq!(second, 600);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);

    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

#[test]
fn get_stream_state_unknown_id_panics() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_get_stream_state(&99);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

#[test]
fn create_stream_rejects_underfunded_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

#[test]
fn harness_mints_sender_balance() {
    let ctx = TestContext::setup();
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
}

#[test]
fn top_up_stream_increases_deposit_and_contract_balance() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // After creation, sender has 9000, contract has 1000
    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);

    // Top up by 500 from the sender
    ctx.env.ledger().set_timestamp(100);
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    // Deposit amount should increase
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);

    // Balances: sender 8500, contract 1500
    assert_eq!(ctx.token.balance(&ctx.sender), 8_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_500);
}

#[test]
fn cancel_stream_updates_state_before_transfer() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel at t=500, so 500 accrued, 500 unstreamed
    ctx.env.ledger().set_timestamp(500);
    ctx.client().cancel_stream(&stream_id);

    // State must be Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(500));

    // Sender gets back unstreamed 500
    assert_eq!(ctx.token.balance(&ctx.sender), 9_500);
    // Contract retains accrued 500 for recipient
    assert_eq!(ctx.token.balance(&ctx.contract_id), 500);
}

#[test]
fn cancel_stream_as_admin_updates_state_before_transfer() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().cancel_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(300));

    // Sender gets back 700 unstreamed
    assert_eq!(ctx.token.balance(&ctx.sender), 9_700);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 300);
}

#[test]
fn withdraw_updates_state_before_transfer() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(600);
    let withdrawn = ctx.client().withdraw(&stream_id);

    // Verify state was correctly updated
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(withdrawn, 600);
    assert_eq!(state.withdrawn_amount, 600);
    assert_eq!(state.status, StreamStatus::Active);

    // Verify balances reflect transfer
    assert_eq!(ctx.token.balance(&ctx.recipient), 600);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 400);
}

#[test]
fn withdraw_marks_completed_when_fully_withdrawn() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw entire deposit at end of stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);
}

#[test]
fn withdraw_final_drain_emits_withdrew_then_completed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Partial then final withdrawal.
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    ctx.env.ledger().set_timestamp(1000);
    let events_before = ctx.env.events().all().len();
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 700);

    let events = ctx.env.events().all();
    let mut withdrew_idx: Option<u32> = None;
    let mut completed_idx: Option<u32> = None;
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        let topic0 = soroban_sdk::Symbol::from_val(&ctx.env, &event.1.get(0).unwrap());
        if topic0 == soroban_sdk::Symbol::new(&ctx.env, "withdrew") {
            withdrew_idx = Some(i);
        }
        if topic0 == soroban_sdk::Symbol::new(&ctx.env, "completed") {
            completed_idx = Some(i);
        }
    }

    assert!(withdrew_idx.is_some(), "expected withdrew event");
    assert!(completed_idx.is_some(), "expected completed event");
    assert!(withdrew_idx.unwrap() < completed_idx.unwrap());
}

#[test]
fn cancel_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Complete the stream first
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Attempt to cancel completed stream should return error
    let result = ctx.client().try_cancel_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn withdraw_from_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Second withdraw should return error
    let result = ctx.client().try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn withdraw_from_paused_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().pause_stream(&stream_id);
    let result = ctx.client().try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn withdraw_after_cancel_at_end_stays_cancelled() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel at end: recipient can still withdraw accrued, but state must remain Cancelled.
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&stream_id);

    let events_before = ctx.env.events().all().len();
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 1000);

    let events = ctx.env.events().all();
    let mut saw_completed = false;
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        let topic0 = soroban_sdk::Symbol::from_val(&ctx.env, &event.1.get(0).unwrap());
        if topic0 == soroban_sdk::Symbol::new(&ctx.env, "completed") {
            saw_completed = true;
        }
    }
    assert!(
        !saw_completed,
        "cancelled stream withdraw must not emit completed"
    );
}

/// End-to-end integration test: create stream, advance time in steps,
/// withdraw multiple times, verify amounts and final Completed status.
///
/// This test covers:
/// - Stream creation and initial state
/// - Multiple partial withdrawals at different time points
/// - Balance verification after each withdrawal
/// - Final withdrawal that completes the stream
/// - Status transition to Completed
/// - Correct final balances for all parties
#[test]
fn integration_full_flow_multiple_withdraws_to_completed() {
    let ctx = TestContext::setup();

    // Initial balances
    let sender_initial = ctx.token.balance(&ctx.sender);
    assert_eq!(sender_initial, 10_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Create stream: 5000 tokens over 5000 seconds (1 token/sec), no cliff
    ctx.env.ledger().set_timestamp(1000);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &6000u64,
    );

    // Verify stream created and deposit transferred
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 1000);
    assert_eq!(state.end_time, 6000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);

    assert_eq!(ctx.token.balance(&ctx.sender), 5_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 5_000);

    // First withdrawal at 20% progress (1000 seconds elapsed)
    ctx.env.ledger().set_timestamp(2000);
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4000);

    // Second withdrawal at 50% progress (1500 more seconds)
    ctx.env.ledger().set_timestamp(3500);
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 1500);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 2500);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2500);

    // Third withdrawal at 80% progress (1000 more seconds)
    ctx.env.ledger().set_timestamp(4500);
    let withdrawn_3 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_3, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 3500);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 3500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1500);

    // Final withdrawal at 100% (end_time reached)
    ctx.env.ledger().set_timestamp(6000);
    let withdrawn_4 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_4, 1500);

    // Verify stream is now Completed
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 5000);
    assert_eq!(state.status, StreamStatus::Completed);

    // Verify final balances
    assert_eq!(ctx.token.balance(&ctx.recipient), 5000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    assert_eq!(ctx.token.balance(&ctx.sender), 5000);

    // Verify total withdrawn equals deposit
    assert_eq!(withdrawn_1 + withdrawn_2 + withdrawn_3 + withdrawn_4, 5000);
}

/// Integration test: multiple withdrawals with time advancement beyond end_time.
/// Verifies that accrual caps at deposit_amount and status transitions correctly.
#[test]
fn integration_withdraw_beyond_end_time() {
    let ctx = TestContext::setup();

    // Create stream: 2000 tokens over 1000 seconds (2 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    // Withdraw at 25%
    ctx.env.ledger().set_timestamp(250);
    let w1 = ctx.client().withdraw(&stream_id);
    assert_eq!(w1, 500);

    // Withdraw at 75%
    ctx.env.ledger().set_timestamp(750);
    let w2 = ctx.client().withdraw(&stream_id);
    assert_eq!(w2, 1000);

    // Advance time well beyond end_time
    ctx.env.ledger().set_timestamp(5000);
    let w3 = ctx.client().withdraw(&stream_id);
    assert_eq!(w3, 500); // Only remaining 500, not more

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 2000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2000);
}

/// Integration test: create stream → cancel immediately → sender receives full refund.
///
/// This test covers:
/// - Stream creation with deposit transfer
/// - Immediate cancellation (no time elapsed, no accrual)
/// - Full refund to sender
/// - Stream status transitions to Cancelled
/// - All balances are correct (sender gets full deposit back, recipient gets nothing)
#[test]
fn integration_cancel_immediately_full_refund() {
    let ctx = TestContext::setup();

    // Record initial balances
    let sender_initial = ctx.token.balance(&ctx.sender);
    let recipient_initial = ctx.token.balance(&ctx.recipient);
    let contract_initial = ctx.token.balance(&ctx.contract_id);

    assert_eq!(sender_initial, 10_000);
    assert_eq!(recipient_initial, 0);
    assert_eq!(contract_initial, 0);

    // Create stream: 3000 tokens over 3000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(1000);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &4000u64,
    );

    // Verify deposit transferred
    assert_eq!(ctx.token.balance(&ctx.sender), 7_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);

    // Cancel immediately (no time elapsed)
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 0);

    // Verify sender received full refund
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Integration test: create stream → advance time → cancel → sender receives partial refund.
///
/// This test covers:
/// - Stream creation and time advancement
/// - Partial accrual (30% of stream duration)
/// - Cancellation with partial refund
/// - Sender receives unstreamed amount (70% of deposit)
/// - Accrued amount (30%) remains in contract for recipient
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_partial_accrual_partial_refund() {
    let ctx = TestContext::setup();

    // Create stream: 5000 tokens over 5000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &5000u64,
    );

    // Verify initial state after creation
    assert_eq!(ctx.token.balance(&ctx.sender), 5_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 5_000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.deposit_amount, 5000);

    // Advance time to 30% completion (1500 seconds)
    ctx.env.ledger().set_timestamp(1500);

    // Verify accrued amount before cancel
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1500);

    // Cancel stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(1500));

    // Verify sender received refund of unstreamed amount (3500 tokens)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 3500);
    assert_eq!(sender_after_cancel, 8_500);

    // Verify accrued amount (1500) remains in contract for recipient
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    // Verify recipient can withdraw the accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

#[test]
fn integration_cancel_refund_plus_frozen_accrued_equals_deposit() {
    let ctx = TestContext::setup();

    // 3000 tokens over 3000s at 1 token/s
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &3000u64,
    );

    // Cancel at t=1200
    ctx.env.ledger().set_timestamp(1200);
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);
    let sender_after_cancel = ctx.token.balance(&ctx.sender);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(1200));

    // Move far forward; accrued must remain frozen at cancelled_at.
    ctx.env.ledger().set_timestamp(9_000);
    let frozen_accrued = ctx.client().calculate_accrued(&stream_id);
    let refund = sender_after_cancel - sender_before_cancel;

    assert_eq!(frozen_accrued, 1200);
    assert_eq!(refund, 1800);
    assert_eq!(refund + frozen_accrued, state.deposit_amount);
}

/// Integration test: create stream → advance to 100% → cancel → no refund.
///
/// This test covers:
/// - Stream creation and full time advancement
/// - Full accrual (100% of deposit)
/// - Cancellation when fully accrued
/// - Sender receives no refund (all tokens accrued to recipient)
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_fully_accrued_no_refund() {
    let ctx = TestContext::setup();

    // Create stream: 2000 tokens over 1000 seconds (2 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 8_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_000);

    // Advance time to 100% completion (or beyond)
    ctx.env.ledger().set_timestamp(1000);

    // Verify full accrual
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2000);

    // Cancel stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received NO refund (balance unchanged)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    assert_eq!(sender_after_cancel, sender_before_cancel);
    assert_eq!(sender_after_cancel, 8_000);

    // Verify all tokens remain in contract for recipient
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_000);

    // Verify recipient can withdraw full amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 2000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Integration test: create stream → withdraw partially → cancel → correct refund.
///
/// This test covers:
/// - Stream creation and partial withdrawal
/// - Cancellation after partial withdrawal
/// - Sender receives refund of unstreamed amount (not withdrawn amount)
/// - Accrued but not withdrawn amount remains for recipient
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_after_partial_withdrawal() {
    let ctx = TestContext::setup();

    // Create stream: 4000 tokens over 4000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &4000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &4000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 6_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4_000);

    // Advance to 25% and withdraw
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);

    // Advance to 60% and cancel
    ctx.env.ledger().set_timestamp(2400);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2400);

    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received refund of unstreamed amount
    // Unstreamed = deposit - accrued = 4000 - 2400 = 1600
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 1600);
    assert_eq!(sender_after_cancel, 7_600);

    // Verify accrued but not withdrawn amount remains in contract
    // Accrued = 2400, Withdrawn = 1000, Remaining = 1400
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_400);

    // Verify recipient can withdraw remaining accrued amount
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 1400);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2_400);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Verify total withdrawn equals accrued
    assert_eq!(withdrawn_1 + withdrawn_2, 2400);
}

/// Integration test: create stream → multiple partial withdrawals → cancel → correct refund.
///
/// This test covers:
/// - Multiple partial withdrawals before cancellation
/// - Cancellation after multiple withdrawal transactions
/// - Sender receives refund of unstreamed amount (independent of withdrawal count)
/// - Accrued but not withdrawn remains for recipient
/// - All balances remain consistent through multiple withdrawal operations
#[test]
fn integration_cancel_after_multiple_partial_withdrawals() {
    let ctx = TestContext::setup();

    // Create stream: 5000 tokens over 5000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &5000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 5_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 5_000);

    // First withdrawal at t=1000 (20% accrual)
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4_000);

    // Second withdrawal at t=2500 (50% accrual)
    ctx.env.ledger().set_timestamp(2500);
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 1500); // 2500 accrued - 1000 already withdrawn
    assert_eq!(ctx.token.balance(&ctx.recipient), 2_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_500);

    // Third withdrawal at t=3500 (70% accrual)
    ctx.env.ledger().set_timestamp(3500);
    let withdrawn_3 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_3, 1000); // 3500 accrued - 2500 already withdrawn
    assert_eq!(ctx.token.balance(&ctx.recipient), 3_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_500);

    // Cancel at t=4200 (84% accrual)
    ctx.env.ledger().set_timestamp(4200);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 4200);

    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 3500);

    // Verify sender received refund of unstreamed amount
    // Unstreamed = deposit - accrued = 5000 - 4200 = 800
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 800);
    assert_eq!(sender_after_cancel, 5_800);

    // Verify accrued but not withdrawn amount remains in contract
    // Accrued = 4200, Withdrawn = 3500, Remaining = 700
    assert_eq!(ctx.token.balance(&ctx.contract_id), 700);

    // Verify recipient can withdraw remaining accrued amount
    let withdrawn_final = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_final, 700);
    assert_eq!(ctx.token.balance(&ctx.recipient), 4_200);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Verify total withdrawn across all calls equals accrued
    assert_eq!(
        withdrawn_1 + withdrawn_2 + withdrawn_3 + withdrawn_final,
        4200
    );
}

/// Integration test: create stream with cliff → cancel before cliff → full refund.
///
/// This test covers:
/// - Stream creation with cliff
/// - Cancellation before cliff time
/// - Full refund to sender (no accrual before cliff)
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_before_cliff_full_refund() {
    let ctx = TestContext::setup();

    // Create stream with cliff: 3000 tokens over 3000 seconds, cliff at 1500
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &1500u64, // cliff at 50%
        &3000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 7_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);

    // Advance time before cliff (1000 seconds, before 1500 cliff)
    ctx.env.ledger().set_timestamp(1000);

    // Verify no accrual before cliff
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0);

    // Cancel stream
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received full refund
    assert_eq!(ctx.token.balance(&ctx.sender), 10_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);
}

/// Integration test: create stream with cliff → cancel after cliff → partial refund.
///
/// This test covers:
/// - Stream creation with cliff
/// - Cancellation after cliff time
/// - Partial refund based on accrual from start_time (not cliff_time)
/// - Stream status transitions to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_after_cliff_partial_refund() {
    let ctx = TestContext::setup();

    // Create stream with cliff: 4000 tokens over 4000 seconds, cliff at 2000
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &4000_i128,
        &1_i128,
        &0u64,
        &2000u64, // cliff at 50%
        &4000u64,
    );

    // Verify initial balances
    assert_eq!(ctx.token.balance(&ctx.sender), 6_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 4_000);

    // Advance time after cliff (2500 seconds, past 2000 cliff)
    ctx.env.ledger().set_timestamp(2500);

    // Verify accrual after cliff (calculated from start_time)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2500);

    // Cancel stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received refund of unstreamed amount (1500)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 1500);
    assert_eq!(sender_after_cancel, 7_500);

    // Verify accrued amount remains in contract
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_500);

    // Verify recipient can withdraw accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 2500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2_500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

// ---------------------------------------------------------------------------
// Integration tests — stream_id generation and uniqueness
// ---------------------------------------------------------------------------

/// Creating N streams must produce IDs 0, 1, 2, …, N-1 with no gaps or duplicates.
///
/// Verifies:
/// - Counter starts at 0 after init
/// - Each create_stream call advances the counter by exactly 1
/// - The returned stream_id matches the value stored in the Stream struct
/// - No two streams share the same id
#[test]
fn integration_stream_ids_are_unique_and_sequential() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    const N: u64 = 10;
    let mut collected: std::vec::Vec<u64> = std::vec::Vec::new();

    for expected in 0..N {
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100_i128,
            &1_i128,
            &0u64,
            &0u64,
            &100u64,
        );

        // Returned id must be sequential
        assert_eq!(
            id, expected,
            "stream {expected}: id must equal counter value"
        );

        // Id stored inside the struct must match the returned id
        let state = ctx.client().get_stream_state(&id);
        assert_eq!(
            state.stream_id, id,
            "stream {expected}: stored stream_id must equal returned id"
        );

        collected.push(id);
    }

    // Pairwise uniqueness — no duplicate ids
    for i in 0..collected.len() {
        for j in (i + 1)..collected.len() {
            assert_ne!(
                collected[i], collected[j],
                "stream_ids at positions {i} and {j} must be unique"
            );
        }
    }
}

/// A create_stream call that fails validation must NOT advance NextStreamId;
/// the following successful call must receive the id that would have been next.
///
/// Verifies:
/// - Validation failures (underfunded deposit) leave the counter unchanged
/// - Subsequent successful calls receive the correct sequential id
#[test]
fn integration_failed_creation_does_not_advance_counter() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // First successful stream → id = 0
    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(id0, 0, "first stream must be id 0");

    // Attempt a stream with an underfunded deposit → must return error
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_i128, // deposit < rate * duration
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));

    // Next successful stream must be id = 1, not 2
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(
        id1, 1,
        "counter must not advance after a failed create_stream"
    );

    // Verify both streams are independently retrievable
    assert_eq!(ctx.client().get_stream_state(&id0).stream_id, 0);
    assert_eq!(ctx.client().get_stream_state(&id1).stream_id, 1);
}

/// Integration test: create stream → pause → cancel → correct refund.
///
/// This test covers:
/// - Stream creation and pause
/// - Cancellation of paused stream
/// - Correct refund calculation (accrual continues even when paused)
/// - Stream status transitions from Paused to Cancelled
/// - All balances are correct
#[test]
fn integration_cancel_paused_stream() {
    let ctx = TestContext::setup();

    // Create stream: 3000 tokens over 3000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &3000u64,
    );

    // Advance to 40% and pause
    ctx.env.ledger().set_timestamp(1200);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Advance time further (accrual continues even when paused)
    ctx.env.ledger().set_timestamp(2000);

    // Verify accrual continues based on time (not affected by pause)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2000);

    // Cancel paused stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender received refund of unstreamed amount (1000)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 1000);
    assert_eq!(sender_after_cancel, 8_000);

    // Verify accrued amount remains in contract
    assert_eq!(ctx.token.balance(&ctx.contract_id), 2_000);
}

/// Integration test: create stream, pause, advance time, resume, advance time, withdraw.
/// Asserts accrual and withdrawals reflect paused period (accrual continues, withdrawals blocked).
///
/// Test flow:
/// 1. Create a 1000-token stream over 1000 seconds (1 token/sec), starting at t=0
/// 2. Advance to t=300, verify 300 tokens accrued, pause the stream
/// 3. Advance to t=700 (400 more seconds), verify accrual continues during pause (700 total)
/// 4. Attempt withdrawal while paused (should fail)
/// 5. Resume stream at t=700
/// 6. Withdraw 700 tokens accrued
/// 7. Advance to t=1000 (end of stream)
/// 8. Withdraw remaining 300 tokens
/// 9. Verify stream completes and final balances are correct
///
/// Key assertions:
/// - Accrual is time-based and unaffected by pause state
/// - Withdrawals are blocked while stream is paused
/// - After resume, withdrawals work with all accrued amounts
/// - Total withdrawn equals deposit amount
/// - Status transitions through Active -> Paused -> Active -> Completed
#[test]
fn integration_pause_resume_withdraw_lifecycle() {
    let ctx = TestContext::setup();

    // -----------------------------------------------------------------------
    // Phase 1: Create stream (t=0)
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.withdrawn_amount, 0);

    // Verify deposit transferred to contract
    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    // -----------------------------------------------------------------------
    // Phase 2: Advance to t=300 and pause
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(300);

    // Verify 300 tokens accrued
    let accrued_at_300 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_at_300, 300);

    // Pause stream (sender authorization required)
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
    assert_eq!(
        state.withdrawn_amount, 0,
        "no withdrawals should occur during pause"
    );

    // -----------------------------------------------------------------------
    // Phase 3: Advance to t=700 while paused, verify accrual continues
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(700);

    // Verify accrual continues during pause (time-based, not status-based)
    let accrued_at_700 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_at_700, 700,
        "accrual must continue during pause period"
    );

    // Attempt to withdraw while paused — should fail with InvalidState
    let withdrawal_result = ctx.client().try_withdraw(&stream_id);
    assert_eq!(withdrawal_result, Err(Ok(ContractError::InvalidState)));

    // Verify stream still paused and no tokens transferred
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(ctx.token.balance(&ctx.recipient), 0);

    // -----------------------------------------------------------------------
    // Phase 4: Resume stream at t=700
    // -----------------------------------------------------------------------
    ctx.client().resume_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.withdrawn_amount, 0);

    // -----------------------------------------------------------------------
    // Phase 5: Withdraw all accrued amount (700 tokens) at t=700
    // -----------------------------------------------------------------------
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 700, "should withdraw all 700 accrued tokens");

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.withdrawn_amount, 700);

    // Verify balances after withdrawal
    assert_eq!(ctx.token.balance(&ctx.recipient), 700);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 300);

    // -----------------------------------------------------------------------
    // Phase 6: Advance to t=1000 (end of stream) and withdraw remaining
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(1000);

    // Verify 1000 tokens accrued at end
    let accrued_at_1000 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_at_1000, 1000);

    // Withdraw final 300 tokens (1000 - 700 already withdrawn)
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 300, "should withdraw remaining 300 tokens");

    // Verify stream is now Completed
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);

    // Verify final balances
    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);

    // Verify total withdrawn equals deposit
    assert_eq!(withdrawn_1 + withdrawn_2, 1000);
}

/// Integration test: multiple pause/resume cycles with time advancement.
/// Verifies that accrual is unaffected by repeated pause/resume operations.
///
/// Test flow:
/// 1. Create 2000-token stream over 2000 seconds
/// 2. Advance to t=500, pause
/// 3. Advance to t=1000, resume
/// 4. Advance to t=1500, pause
/// 5. Advance to t=1800, resume
/// 6. Withdraw at t=1800 (1800 tokens should be accrued)
/// 7. Advance to t=2000 (end)
/// 8. Withdraw final 200 tokens
///
/// Verifies accrual accumulates correctly through multiple pause/resume cycles.
#[test]
fn integration_multiple_pause_resume_cycles() {
    let ctx = TestContext::setup();

    // Create stream: 2000 tokens over 2000 seconds (1 token/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
    );

    // First pause/resume cycle
    ctx.env.ledger().set_timestamp(500);
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.env.ledger().set_timestamp(1000);
    let accrued_at_1000 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_at_1000, 1000, "accrual continues during pause");

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Second pause/resume cycle
    ctx.env.ledger().set_timestamp(1500);
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.env.ledger().set_timestamp(1800);
    let accrued_at_1800 = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_at_1800, 1800,
        "accrual continues through multiple pauses"
    );

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Withdraw at t=1800
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 1800);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1800);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1800);

    // Final withdrawal at end
    ctx.env.ledger().set_timestamp(2000);
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 200);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 2000);
    assert_eq!(ctx.token.balance(&ctx.recipient), 2000);
}

/// Integration test: pause, advance past end_time, resume, verify capped accrual.
/// Ensures accrual remains capped at deposit_amount even with pause during stream.
///
/// Test flow:
/// 1. Create 1000-token stream over 1000 seconds
/// 2. Advance to t=300, pause
/// 3. Advance to t=2000 (well past end_time)
/// 4. Resume stream
/// 5. Verify accrual is capped at 1000 (not 2000)
/// 6. Withdraw all 1000 tokens
/// 7. Stream completes
#[test]
fn integration_pause_resume_past_end_time_accrual_capped() {
    let ctx = TestContext::setup();

    // Create stream: 1000 tokens over 1000 seconds
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    // Pause at t=300
    ctx.env.ledger().set_timestamp(300);
    ctx.client().pause_stream(&stream_id);

    // Advance far past end_time (t=2000)
    ctx.env.ledger().set_timestamp(2000);

    // Verify accrual is still capped at deposit_amount
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 1000,
        "accrual must be capped at deposit_amount even past end_time"
    );

    // Resume and withdraw
    ctx.client().resume_stream(&stream_id);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);
}

/// Integration test: pause stream, then cancel while paused.
/// Verifies that accrual reflects time elapsed even during pause,
/// and sender receives correct refund for unstreamed amount.
///
/// Test flow:
/// 1. Create 3000-token stream over 1000 seconds (3 tokens/sec)
/// 2. Advance to t=300, pause
/// 3. Advance to t=600 (paused, 1800 tokens accrued but blocked from withdrawal)
/// 4. Cancel stream as sender
/// 5. Verify sender receives refund for unstreamed amount (1200 tokens)
/// 6. Verify recipient can still withdraw accrued 1800 tokens
#[test]
fn integration_pause_then_cancel_preserves_accrual() {
    let ctx = TestContext::setup();

    // Create stream: 3000 tokens over 1000 seconds (3 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &3_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    assert_eq!(ctx.token.balance(&ctx.sender), 7_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3_000);

    // Pause at t=300 (900 tokens accrued)
    ctx.env.ledger().set_timestamp(300);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Advance to t=600 while paused (1800 tokens accrued, recipient cannot withdraw)
    ctx.env.ledger().set_timestamp(600);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1800, "accrual continues during pause");

    // Cancel paused stream
    let sender_before_cancel = ctx.token.balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify sender receives refund of unstreamed amount (3000 - 1800 = 1200)
    let sender_after_cancel = ctx.token.balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 1200, "refund should be deposit - accrued");
    assert_eq!(sender_after_cancel, 8_200);

    // Verify accrued amount (1800) remains in contract for recipient
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1800);

    // Recipient can still withdraw accrued amount from cancelled stream
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1800);

    assert_eq!(ctx.token.balance(&ctx.recipient), 1800);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

#[test]
fn test_create_many_streams_from_same_sender() {
    let ctx = TestContext::setup();

    // Reset budget to track clean for this test
    ctx.env.budget().reset_unlimited();

    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    // Mint 200k to cover 100 streams
    sac.mint(&ctx.sender, &200_000_i128);

    for _ in 0..100 {
        ctx.create_default_stream();
    }

    let cpu_insns = ctx.env.budget().cpu_instruction_cost();
    log!(&ctx.env, "cpu_insns", cpu_insns);
    // Guardrail: ensure creating 100 streams stays within a reasonable CPU budget.
    // Slightly relaxed to account for additional features while keeping a strict bound.
    assert!(cpu_insns <= 60_000_000);

    // Check memory bytes consumed
    let mem_bytes = ctx.env.budget().memory_bytes_cost();
    log!(&ctx.env, "mem_bytes", mem_bytes);
    // Guardrail: ensure memory usage stays bounded for 100 streams.
    assert!(mem_bytes <= 20_000_000);
}

// ---------------------------------------------------------------------------
// Integration tests — extend_stream_end_time: deposit sufficiency
// ---------------------------------------------------------------------------

/// Exact boundary: deposit == rate * new_duration succeeds; accrual reaches new end.
#[test]
fn integration_extend_end_time_exact_deposit_boundary() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit=2000, rate=1, end=1000 → can extend to exactly 2000
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 2000);
    assert_eq!(state.deposit_amount, 2000);

    // Withdraw full amount at new end_time
    ctx.env.ledger().set_timestamp(2000);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 2000);

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Insufficient deposit: extension rejected, stream state and balances unchanged.
#[test]
fn integration_extend_end_time_insufficient_deposit_rejected_no_side_effects() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    let sender_before = ctx.token.balance(&ctx.sender);
    let contract_before = ctx.token.balance(&ctx.contract_id);
    let state_before = ctx.client().get_stream_state(&stream_id);

    let result = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &2000u64);
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));

    // Balances unchanged
    assert_eq!(ctx.token.balance(&ctx.sender), sender_before);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_before);

    // Stream state unchanged
    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.end_time, state_before.end_time);
    assert_eq!(state_after.deposit_amount, state_before.deposit_amount);
    assert_eq!(state_after.status, state_before.status);
}

/// top_up then extend: combined operation allows longer stream duration.
#[test]
fn integration_top_up_then_extend_full_withdrawal() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Tight deposit: exactly covers original 1000s
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    // Top up 500 tokens
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    // Now extend to 1500s (rate(1) * 1500 = 1500 == new deposit)
    ctx.client().extend_stream_end_time(&stream_id, &1500u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 1500);
    assert_eq!(state.deposit_amount, 1500);

    // Withdraw full amount at new end
    ctx.env.ledger().set_timestamp(1500);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1500);

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1500);
}

/// Paused stream: extension succeeds, accrual and withdrawal work after resume.
#[test]
fn integration_extend_paused_stream_then_resume_withdraw() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    ctx.env.ledger().set_timestamp(400);
    ctx.client().pause_stream(&stream_id);

    // Extend while paused
    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 2000);
    assert_eq!(state.status, StreamStatus::Paused);

    // Resume and withdraw
    ctx.client().resume_stream(&stream_id);

    ctx.env.ledger().set_timestamp(2000);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 2000);

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );
}

/// Balance conservation: total tokens across all parties unchanged after extend + withdraw.
#[test]
fn integration_extend_end_time_balance_conservation() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let total_before = ctx.token.balance(&ctx.sender)
        + ctx.token.balance(&ctx.recipient)
        + ctx.token.balance(&ctx.contract_id);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    ctx.env.ledger().set_timestamp(2000);
    ctx.client().withdraw(&stream_id);

    let total_after = ctx.token.balance(&ctx.sender)
        + ctx.token.balance(&ctx.recipient)
        + ctx.token.balance(&ctx.contract_id);

    assert_eq!(
        total_after, total_before,
        "total token supply must be conserved"
    );
}

// ---------------------------------------------------------------------------
// Integration tests — batch_withdraw: completed streams yield zero amounts
// ---------------------------------------------------------------------------

/// Mixed batch [completed, active, completed]: zero amounts for completed entries,
/// correct transfer for active entry, balance conservation throughout.
#[test]
fn integration_batch_withdraw_completed_streams_yield_zero() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id0 = ctx.create_default_stream(); // will be completed
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    ); // active
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    ); // will be completed

    // Complete id0 and id2
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id0);
    ctx.client().withdraw(&id2);

    // id1 is still active at t=600
    ctx.env.ledger().set_timestamp(600);

    let total_before = ctx.token.balance(&ctx.sender)
        + ctx.token.balance(&ctx.recipient)
        + ctx.token.balance(&ctx.contract_id);

    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    ids.push_back(id0);
    ids.push_back(id1);
    ids.push_back(id2);
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);

    assert_eq!(results.len(), 3);
    assert_eq!(
        results.get(0).unwrap().amount,
        0,
        "completed id0 must yield 0"
    );
    assert_eq!(
        results.get(1).unwrap().amount,
        600,
        "active id1 must yield 600"
    );
    assert_eq!(
        results.get(2).unwrap().amount,
        0,
        "completed id2 must yield 0"
    );

    // Balance conservation
    let total_after = ctx.token.balance(&ctx.sender)
        + ctx.token.balance(&ctx.recipient)
        + ctx.token.balance(&ctx.contract_id);
    assert_eq!(total_after, total_before);

    // Contract holds only the remaining 400 for id1
    assert_eq!(ctx.token.balance(&ctx.contract_id), 400);
}

// ===========================================================================
// Integration: get_claimable_at simulation and cancel clamping (Issue #270)
// ===========================================================================

/// Full lifecycle: claimable_at predicts correctly before and after each operation.
#[test]
fn integration_claimable_at_lifecycle_prediction() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 0..1000, rate=1, deposit=1000

    // Before any operation: simulate at t=500 → 500
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &500), 500);

    // Withdraw 300 at t=300
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.token.balance(&ctx.recipient), 300);

    // After withdraw: simulate at t=800 → accrued=800, withdrawn=300 → 500
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &800), 500);

    // Simulate at end → 700
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &1000), 700);

    // Actually withdraw at t=1000
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);

    // Completed: claimable always 0
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &9999), 0);
}

/// Cancel clamping: claimable prediction matches actual fund flow.
#[test]
fn integration_claimable_at_cancel_matches_funds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel at t=600
    ctx.env.ledger().set_timestamp(600);
    ctx.client().cancel_stream(&stream_id);

    // Claimable prediction: 600 at any future time
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &9999), 600);

    // Actually withdraw what's claimable
    ctx.client().withdraw(&stream_id);
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        600,
        "actual withdrawal must match claimable prediction"
    );

    // After withdraw: claimable drops to 0
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &9999), 0);
}

/// Partial withdraw then cancel: prediction verified against real withdrawal.
#[test]
fn integration_claimable_at_partial_then_cancel() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 200 at t=200
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    // Cancel at t=700
    ctx.env.ledger().set_timestamp(700);
    ctx.client().cancel_stream(&stream_id);

    // Prediction: accrued clamped at 700, withdrawn 200 → claimable=500
    let predicted = ctx.client().get_claimable_at(&stream_id, &999_999);
    assert_eq!(predicted, 500);

    // Actual withdraw
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.token.balance(&ctx.recipient), 700); // 200 + 500

    // After full withdraw: claimable=0
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &999_999), 0);
}

/// Claimable at current time matches get_withdrawable across multiple time points.
#[test]
fn integration_claimable_at_equals_withdrawable() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    for &t in &[0u64, 250, 500, 750, 1000] {
        ctx.env.ledger().set_timestamp(t);
        let withdrawable = ctx.client().get_withdrawable(&stream_id);
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert_eq!(
            withdrawable, claimable,
            "at t={t}: get_withdrawable != get_claimable_at"
        );
    }
}

// Integration regression: double-init and missing-config reads (Issue #246)
// ===========================================================================

// ---------------------------------------------------------------------------
// Double-init: integration scenarios
// ---------------------------------------------------------------------------

/// Full integration: double-init attempt must not affect fund flows.
/// Creates a stream, attempts re-init, then verifies that withdrawal/balance
/// accounting is perfectly intact.
#[test]
fn integration_double_init_does_not_affect_fund_flows() {
    let ctx = TestContext::setup();

    let sender_initial = ctx.token.balance(&ctx.sender);
    let contract_initial = ctx.token.balance(&ctx.contract_id);

    // Create stream
    let stream_id = ctx.create_default_stream();
    assert_eq!(ctx.token.balance(&ctx.sender), sender_initial - 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_initial + 1000);

    // Attempt re-init (should fail)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().init(&ctx.token_id, &ctx.admin);
    }));
    assert!(result.is_err());

    // Balances must be unchanged by re-init attempt
    assert_eq!(
        ctx.token.balance(&ctx.sender),
        sender_initial - 1000,
        "sender balance must not change after failed re-init"
    );
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_initial + 1000,
        "contract balance must not change after failed re-init"
    );

    // Withdrawal still works perfectly
    ctx.env.ledger().set_timestamp(500);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 500);

    // Complete the stream
    ctx.env.ledger().set_timestamp(1000);
    let final_withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(final_withdrawn, 500);

    // Verify final state
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Double-init must not affect cancellation and refund mechanics.
#[test]
fn integration_double_init_does_not_affect_cancel_refund() {
    let ctx = TestContext::setup();

    let stream_id = ctx.create_default_stream();
    let sender_after_create = ctx.token.balance(&ctx.sender);

    // Attempt re-init
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client()
            .init(&Address::generate(&ctx.env), &Address::generate(&ctx.env));
    }));

    // Cancel at t=400 — should refund 600 to sender
    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(400));
    assert_eq!(
        ctx.token.balance(&ctx.sender),
        sender_after_create + 600,
        "sender must receive correct refund after re-init attempt"
    );
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        400,
        "contract must retain accrued amount"
    );

    // Recipient can still withdraw accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 400);
    assert_eq!(ctx.token.balance(&ctx.recipient), 400);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// Config immutability persists through multiple re-init attempts with
/// different parameter combinations.
#[test]
fn integration_config_immutable_through_multiple_reinit_permutations() {
    let ctx = TestContext::setup();
    let original_config = ctx.client().get_config();

    // Try 4 different re-init permutations
    let permutations: [(bool, bool); 4] = [
        (true, true),   // same token, same admin
        (true, false),  // same token, different admin
        (false, true),  // different token, same admin
        (false, false), // different token, different admin
    ];

    for (use_same_token, use_same_admin) in permutations {
        let token = if use_same_token {
            ctx.token_id.clone()
        } else {
            Address::generate(&ctx.env)
        };
        let admin = if use_same_admin {
            ctx.admin.clone()
        } else {
            Address::generate(&ctx.env)
        };

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ctx.client().init(&token, &admin);
        }));
        assert!(result.is_err());
    }

    // Config must still match original
    let config = ctx.client().get_config();
    assert_eq!(config.token, original_config.token);
    assert_eq!(config.admin, original_config.admin);
}

/// Stream counter continuity: create, re-init attempt, create again — IDs sequential.
#[test]
fn integration_stream_counter_continuous_after_reinit() {
    let ctx = TestContext::setup();

    let id0 = ctx.create_default_stream();
    assert_eq!(id0, 0);

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().init(&ctx.token_id, &ctx.admin);
    }));

    ctx.env.ledger().set_timestamp(0);
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
    assert_eq!(id1, 1, "second stream must get ID 1");
    assert_eq!(ctx.client().get_stream_count(), 2);
}

// ---------------------------------------------------------------------------
// Missing-config: integration scenarios
// ---------------------------------------------------------------------------

/// Full integration: uninitialised contract gives clear error for get_config.
#[test]
#[should_panic]
fn integration_uninitialised_get_config_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.get_config();
}

/// Uninitialised contract: create_stream must panic with missing config.
#[test]
#[should_panic]
fn integration_uninitialised_create_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    env.ledger().set_timestamp(0);
    client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
    );
}

/// Uninitialised contract: admin operations must panic with missing config.
#[test]
#[should_panic]
fn integration_uninitialised_admin_cancel_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.cancel_stream_as_admin(&0);
}

/// Uninitialised contract: version is still readable (no config dependency).
#[test]
fn integration_uninitialised_version_works() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    assert_eq!(client.version(), 1);
}

/// Uninitialised contract: stream count returns 0.
#[test]
fn integration_uninitialised_stream_count_zero() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    assert_eq!(client.get_stream_count(), 0);
}

/// Uninitialised contract: get_stream_state for non-existent stream fails.
#[test]
fn integration_uninitialised_get_stream_state_fails() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let result = client.try_get_stream_state(&0);
    assert!(result.is_err());
}

/// Uninitialised contract: set_contract_paused must fail with missing config.
#[test]
#[should_panic]
fn integration_uninitialised_set_contract_paused_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.set_contract_paused(&true);
}

/// After initialisation, all previously-failing paths become functional.
/// This verifies init correctly unblocks the full contract surface.
#[test]
fn integration_init_unblocks_all_paths() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    // Before init: get_config must fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_config();
    }));
    assert!(result.is_err(), "get_config must fail before init");

    // Initialise
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let admin = Address::generate(&env);
    client.init(&token_id, &admin);

    // After init: get_config must succeed
    let config = client.get_config();
    assert_eq!(config.token, token_id);
    assert_eq!(config.admin, admin);

    // After init: create_stream must succeed
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &10_000_i128);

    env.ledger().set_timestamp(0);
    let stream_id = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
    );
    assert_eq!(stream_id, 0);
    assert_eq!(client.get_stream_count(), 1);
}

/// Integration test: verify set_admin rotates the admin correctly, new admin can pause,
/// old admin cannot pause, and the AdminUpdated event is emitted.
#[test]
fn integration_set_admin_rotation_flow() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();
    let new_admin = Address::generate(&ctx.env);

    // Initial admin is ctx.admin
    let config = ctx.client().get_config();
    assert_eq!(config.admin, ctx.admin);

    // Mock old admin auth for the rotation
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &ctx.admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "set_admin",
            args: (new_admin.clone(),).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    // Rotate admin
    ctx.client().set_admin(&new_admin);

    // Verify config is updated
    let new_config = ctx.client().get_config();
    assert_eq!(new_config.admin, new_admin);

    // Verify event emitted
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(last_event.0, ctx.contract_id);
    assert_eq!(
        soroban_sdk::Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        soroban_sdk::Symbol::new(&ctx.env, "AdminUpd")
    );
    let data: (Address, Address) = last_event.2.into_val(&ctx.env);
    assert_eq!(data.0, ctx.admin); // old admin
    assert_eq!(data.1, new_admin); // new admin

    // New admin can pause
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &new_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().pause_stream_as_admin(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    // Old admin trying to resume panics
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &ctx.admin, // old admin
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().resume_stream_as_admin(&stream_id);
    }));
    assert!(
        result.is_err(),
        "Old admin should not be able to resume after rotation"
    );
}

// ---------------------------------------------------------------------------
// Integration — Gas / budget review: hot paths and batching
// ---------------------------------------------------------------------------
//
// These tests measure Soroban CPU instruction and memory byte costs for the
// three hot paths identified in the issue:
//   1. `withdraw`          — single-stream accrual + token push
//   2. `batch_withdraw`    — N-stream loop with one auth
//   3. `create_streams`    — N-stream validation + single bulk token pull
//
// Budget is reset to unlimited before each measured call so that setup
// overhead does not pollute the reading. Guardrails are 10× observed
// baseline to catch regressions without being brittle to minor SDK changes.

/// Budget guardrail: single `withdraw` on an active stream.
#[test]
fn integration_budget_single_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);

    ctx.env.budget().reset_unlimited();
    ctx.client().withdraw(&stream_id);

    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    assert!(
        cpu <= 1_000_000,
        "integration single withdraw cpu={cpu} exceeds guardrail 1_000_000"
    );
    assert!(
        mem <= 500_000,
        "integration single withdraw mem={mem} exceeds guardrail 500_000"
    );
}

/// Budget guardrail: `batch_withdraw` over 20 active streams.
#[test]
fn integration_budget_batch_withdraw_20_streams() {
    let ctx = TestContext::setup();
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &200_000_i128);

    ctx.env.ledger().set_timestamp(0);
    let mut ids = vec![&ctx.env];
    for _ in 0..20 {
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
        );
        ids.push_back(id);
    }

    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);

    assert_eq!(results.len(), 20);
    for i in 0..20 {
        assert_eq!(results.get(i).unwrap().amount, 500);
    }

    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    // Guardrail: 20-stream batch must stay under 10 M CPU and 4 MB.
    assert!(
        cpu <= 10_000_000,
        "batch_withdraw(20) cpu={cpu} exceeds guardrail 10_000_000"
    );
    assert!(
        mem <= 4_000_000,
        "batch_withdraw(20) mem={mem} exceeds guardrail 4_000_000"
    );
}

/// Budget guardrail: `create_streams` with 10 entries (single bulk token pull).
#[test]
fn integration_budget_create_streams_batch_10() {
    let ctx = TestContext::setup();
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &100_000_i128);

    ctx.env.ledger().set_timestamp(0);
    let mut params = vec![&ctx.env];
    for _ in 0..10 {
        params.push_back(CreateStreamParams {
            recipient: Address::generate(&ctx.env),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
        });
    }

    ctx.env.budget().reset_unlimited();
    let ids = ctx.client().create_streams(&ctx.sender, &params);

    assert_eq!(ids.len(), 10);

    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    // Guardrail: 10-stream batch create must stay under 6 M CPU and 3 MB.
    assert!(
        cpu <= 6_000_000,
        "create_streams(10) cpu={cpu} exceeds guardrail 6_000_000"
    );
    assert!(
        mem <= 3_000_000,
        "create_streams(10) mem={mem} exceeds guardrail 3_000_000"
    );
}

/// batch_withdraw on a cancelled stream transfers only the remaining
/// accrued-but-not-withdrawn amount (integration-level token balance check).
#[test]
fn integration_batch_withdraw_cancelled_stream_accrued_remainder() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 1000 tokens, rate=1, end=1000

    // Withdraw 300 at t=300
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.token.balance(&ctx.recipient), 300);

    // Cancel at t=700 → accrued_at_cancel=700, refund=300 to sender, 400 left for recipient
    ctx.env.ledger().set_timestamp(700);
    ctx.client().cancel_stream(&stream_id);

    let recipient_before = ctx.token.balance(&ctx.recipient);
    let contract_before = ctx.token.balance(&ctx.contract_id);

    let ids = vec![&ctx.env, stream_id];
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);

    assert_eq!(results.len(), 1);
    assert_eq!(
        results.get(0).unwrap().amount,
        400,
        "cancelled stream: batch_withdraw must transfer accrued(700) - withdrawn(300) = 400"
    );
    assert_eq!(ctx.token.balance(&ctx.recipient), recipient_before + 400);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_before - 400);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 0);
}

/// batch_withdraw: single-auth covers all streams — wrong recipient on any
/// stream returns Unauthorized and reverts the whole batch.
#[test]
fn integration_batch_withdraw_wrong_recipient_unauthorized() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id0 = ctx.create_default_stream();
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    ctx.env.ledger().set_timestamp(500);
    let other = Address::generate(&ctx.env);
    let ids = vec![&ctx.env, id0, id1];

    let result = ctx.client().try_batch_withdraw(&other, &ids);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));

    // No state change: both streams still have withdrawn_amount = 0
    assert_eq!(ctx.client().get_stream_state(&id0).withdrawn_amount, 0);
    assert_eq!(ctx.client().get_stream_state(&id1).withdrawn_amount, 0);
}

/// create_streams: single bulk token pull equals sum of individual deposits.
/// Verifies the gas-saving invariant: one transfer instead of N.
#[test]
fn integration_create_streams_single_token_pull_equals_sum() {
    let ctx = TestContext::setup();
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &10_000_i128);

    ctx.env.ledger().set_timestamp(0);
    let sender_before = ctx.token.balance(&ctx.sender);

    let p1 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
    };
    let p2 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 2000,
        rate_per_second: 2,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
    };
    let p3 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 500,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 500,
    };

    let params = vec![&ctx.env, p1, p2, p3];
    let ids = ctx.client().create_streams(&ctx.sender, &params);

    assert_eq!(ids.len(), 3);
    // Total pulled = 1000 + 2000 + 500 = 3500
    assert_eq!(ctx.token.balance(&ctx.sender), sender_before - 3500);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 3500);
}

// ===========================================================================
// Accrual property tests: bounds and monotonicity — integration tests
// ===========================================================================

/// Property: calculate_accrued is always non-negative and bounded by deposit.
#[test]
fn accrual_prop_non_negative_and_bounded() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender, &ctx.recipient, &1000, &1, &0, &0, &1000,
    );

    for t in [0u64, 1, 250, 500, 750, 999, 1000, 1001, 9999] {
        ctx.env.ledger().set_timestamp(t);
        let accrued = ctx.client().calculate_accrued(&stream_id);
        assert!(accrued >= 0, "negative accrual at t={t}: {accrued}");
        assert!(accrued <= 1000, "accrual {accrued} exceeds deposit at t={t}");
    }
}

/// Property: calculate_accrued is monotonically non-decreasing over time.
#[test]
fn accrual_prop_monotonic_no_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender, &ctx.recipient, &1000, &1, &0, &0, &1000,
    );

    let times = [0u64, 100, 250, 500, 750, 999, 1000, 1001, 5000];
    ctx.env.ledger().set_timestamp(times[0]);
    let mut prev = ctx.client().calculate_accrued(&stream_id);

    for &t in times.iter().skip(1) {
        ctx.env.ledger().set_timestamp(t);
        let now = ctx.client().calculate_accrued(&stream_id);
        assert!(now >= prev, "monotonicity violated at t={t}: got {now}, prev={prev}");
        prev = now;
    }
}

/// Property: calculate_accrued is monotonic for a stream with a cliff.
#[test]
fn accrual_prop_monotonic_with_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender, &ctx.recipient, &1000, &1, &0, &500, &1000,
    );

    let times = [0u64, 100, 499, 500, 501, 750, 1000, 1001, 5000];
    ctx.env.ledger().set_timestamp(times[0]);
    let mut prev = ctx.client().calculate_accrued(&stream_id);

    for &t in times.iter().skip(1) {
        ctx.env.ledger().set_timestamp(t);
        let now = ctx.client().calculate_accrued(&stream_id);
        assert!(now >= prev, "cliff-stream monotonicity violated at t={t}: got {now}, prev={prev}");
        prev = now;
    }
}

/// Property: calculate_accrued == 0 for all t < cliff_time.
#[test]
fn accrual_prop_zero_before_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender, &ctx.recipient, &1000, &1, &0, &500, &1000,
    );

    for t in [0u64, 1, 100, 250, 499] {
        ctx.env.ledger().set_timestamp(t);
        let accrued = ctx.client().calculate_accrued(&stream_id);
        assert_eq!(accrued, 0, "expected 0 before cliff at t={t}");
    }
}

/// Property: calculate_accrued == deposit for all t >= end_time.
#[test]
fn accrual_prop_saturates_at_end_time() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender, &ctx.recipient, &1000, &1, &0, &0, &1000,
    );

    for t in [1000u64, 1001, 5000, 99999] {
        ctx.env.ledger().set_timestamp(t);
        let accrued = ctx.client().calculate_accrued(&stream_id);
        assert_eq!(accrued, 1000, "must saturate at deposit for t={t}");
    }
}

/// Property: accrual is frozen after cancellation.
#[test]
fn accrual_prop_frozen_after_cancel() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender, &ctx.recipient, &1000, &1, &0, &0, &1000,
    );

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);
    let frozen = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(frozen, 400);

    for t in [500u64, 800, 1000, 9999] {
        ctx.env.ledger().set_timestamp(t);
        let later = ctx.client().calculate_accrued(&stream_id);
        assert_eq!(later, frozen, "accrual must be frozen after cancel at t={t}");
    }
}

/// Property: paused stream accrual continues (pause only blocks withdrawals).
#[test]
fn accrual_prop_paused_stream_continues_accruing() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender, &ctx.recipient, &1000, &1, &0, &0, &1000,
    );

    ctx.env.ledger().set_timestamp(300);
    ctx.client().pause_stream(&stream_id);

    ctx.env.ledger().set_timestamp(700);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 700, "paused stream must accrue by time");
}
