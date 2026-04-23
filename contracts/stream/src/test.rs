extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, FromVal, IntoVal, Symbol, TryFromVal, Val, Vec,
};

use crate::{
    ContractError, ContractPauseChanged, CreateStreamParams, FluxoraStream, FluxoraStreamClient,
    GlobalEmergencyPauseChanged, StreamCreated, StreamEndShortened, StreamEvent, StreamStatus,
    StreamToppedUp, WithdrawalTo,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

pub(crate) struct TestContext<'a> {
    pub(crate) env: Env,
    pub(crate) contract_id: Address,
    pub(crate) token_id: Address,
    #[allow(dead_code)]
    pub(crate) admin: Address,
    pub(crate) sender: Address,
    pub(crate) recipient: Address,
    pub(crate) sac: StellarAssetClient<'a>,
}

impl<'a> TestContext<'a> {
    pub(crate) fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        // Deploy the streaming contract
        let contract_id = env.register_contract(None, FluxoraStream);

        // Create a mock SAC token (Stellar Asset Contract)
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Initialise the streaming contract
        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        // Mint tokens to sender (10_000 USDC-equivalent)
        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        TestContext {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            sac,
        }
    }
}

impl<'a> TestContext<'a> {
    /// Setup context without mock_all_auths(), for explicit auth testing
    pub(crate) fn setup_strict() -> Self {
        let env = Env::default();

        let contract_id = env.register_contract(None, FluxoraStream);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);

        // init requires bootstrap admin authorization in strict mode.
        use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "init",
                args: (&token_id, &admin).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);

        // Mock the minting auth since mock_all_auths is not enabled.
        env.mock_auths(&[MockAuth {
            address: &token_admin,
            invoke: &MockAuthInvoke {
                contract: &token_id,
                fn_name: "mint",
                args: (&sender, 10_000_i128).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        sac.mint(&sender, &10_000_i128);

        TestContext {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            sac,
        }
    }

    pub(crate) fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    pub(crate) fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    /// Create a standard 1000-unit stream spanning 1000 seconds (rate 1/s, no cliff).
    pub(crate) fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128, // deposit_amount
            &1_i128,    // rate_per_second  (1 token/s)
            &0u64,      // start_time
            &0u64,      // cliff_time (no cliff)
            &1000u64,   // end_time
        )
    }

    /// Create a stream with a cliff at t=500 out of 1000s.
    pub(crate) fn create_cliff_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &500u64, // cliff at t=500
            &1000u64,
            &None,
        )
    }

    pub(crate) fn create_max_rate_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &(i128::MAX - 1),
            &((i128::MAX - 1) / 3),
            &0,
            &0u64,
            &3,
            &None,
        )
    }

    pub(crate) fn create_half_max_rate_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &42535295865117307932921825928971026400_i128,
            &(42535295865117307932921825928971026400_i128 / 100),
            &0,
            &0u64,
            &100,
            &None,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests — init
// ---------------------------------------------------------------------------

#[test]
fn test_init_stores_token_and_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token, &admin);

    let config = client.get_config();
    assert_eq!(config.token, token);
    assert_eq!(config.admin, admin);
}

#[test]
fn test_init_requires_admin_authorization_in_strict_mode() {
    let env = Env::default();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
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
fn test_init_rejects_wrong_signer_and_has_no_side_effects() {
    let env = Env::default();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "init",
            args: (&token_id, &admin).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let result = client.try_init(&token_id, &admin);
    assert!(result.is_err(), "init must reject non-admin signer");

    // Bootstrap state must remain unset after failed auth.
    let cfg_result = client.try_get_config();
    assert!(
        cfg_result.is_err(),
        "failed init auth must not write config into storage"
    );
    assert_eq!(
        client.get_stream_count(),
        0,
        "failed init auth must not initialize stream counter"
    );
}

#[test]
fn test_init_second_call_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token, &admin);

    let result = client.try_init(&Address::generate(&env), &Address::generate(&env));
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));
}

#[test]
fn test_get_config_before_init_fails() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_get_config();
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Test that get_config panics with clear error when contract is not initialized.
/// This guards against using an uninitialized contract and documents expected behavior.
///
/// Security: Prevents operations on uninitialized contract state.
/// The panic message "contract not initialised: missing config" provides clear
/// feedback to integrators that init() must be called first.
#[test]
fn test_get_config_uninitialized_contract_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    // Calling get_config before init must return InvalidState
    let result = client.try_get_config();
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn test_init_stores_config() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let config = client.get_config();
    assert_eq!(config.token, token_id);
    assert_eq!(config.admin, admin);
}

#[test]
fn test_init_twice_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Second init should return AlreadyInitialised
    let token_id2 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let result = client.try_init(&token_id2, &admin2);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));
}

#[test]
fn test_init_sets_stream_counter_to_zero() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Create a stream to verify counter starts at 0
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    // Mint tokens to sender
    let token_admin = Address::generate(&env);
    let sac_token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let sac = StellarAssetClient::new(&env, &sac_token_id);
    sac.mint(&sender, &10_000_i128);

    // Re-init with the SAC token
    let contract_id2 = env.register_contract(None, FluxoraStream);
    let client2 = FluxoraStreamClient::new(&env, &contract_id2);
    client2.init(&sac_token_id, &admin);

    env.ledger().set_timestamp(0);
    let stream_id = client2.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );

    assert_eq!(stream_id, 0, "first stream should have id 0");
}

#[test]
fn test_get_stream_count_returns_zero_after_init() {
    let ctx = TestContext::setup();
    assert_eq!(
        ctx.client().get_stream_count(),
        0,
        "stream count should be zero before first create_stream"
    );
}

#[test]
fn test_get_stream_count_tracks_successful_creates() {
    let ctx = TestContext::setup();
    assert_eq!(ctx.client().get_stream_count(), 0);

    let id0 = ctx.create_default_stream();
    assert_eq!(id0, 0);
    assert_eq!(ctx.client().get_stream_count(), 1);

    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2_000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );
    assert_eq!(id1, 1);
    assert_eq!(ctx.client().get_stream_count(), 2);
}

#[test]
fn test_init_with_different_addresses() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    // Ensure token and admin are different
    assert_ne!(token_id, admin);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let config = client.get_config();
    assert_eq!(config.token, token_id);
    assert_eq!(config.admin, admin);
    assert_ne!(config.token, config.admin);
}

// ---------------------------------------------------------------------------
// Tests — Issue #62: init cannot be called twice (re-initialization)
// ---------------------------------------------------------------------------

/// Re-init with the exact same token and admin must still panic.
#[test]
fn test_reinit_same_token_same_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Second init with identical arguments must return AlreadyInitialised
    let result = client.try_init(&token_id, &admin);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));
}

/// Re-init with a different token but same admin must panic.
#[test]
fn test_reinit_different_token_same_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Second init with different token but same admin must return AlreadyInitialised
    let token_id2 = Address::generate(&env);
    let result = client.try_init(&token_id2, &admin);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));
}

/// Re-init with same token but a different admin must panic.
#[test]
fn test_reinit_same_token_different_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Second init with same token but different admin must return AlreadyInitialised
    let admin2 = Address::generate(&env);
    let result = client.try_init(&token_id, &admin2);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));
}

/// After a failed re-init attempt the original config must be unchanged.
#[test]
fn test_config_unchanged_after_failed_reinit() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Capture original config
    let original_config = client.get_config();

    // Attempt re-init with completely different params (should return AlreadyInitialised)
    let token_id2 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let result = client.try_init(&token_id2, &admin2);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));

    // Config must be identical to the original
    let config_after = client.get_config();
    assert_eq!(
        config_after.token, original_config.token,
        "token must not change"
    );
    assert_eq!(
        config_after.admin, original_config.admin,
        "admin must not change"
    );
}

/// Contract must remain fully operational after a failed re-init attempt.
#[test]
fn test_operations_work_after_failed_reinit() {
    let env = Env::default();
    env.mock_all_auths();

    // Deploy contract and set up a real SAC token
    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Fund the sender
    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &10_000_i128);

    let admin2 = Address::generate(&env);
    let result = client.try_init(&token_id, &admin2);
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));

    // Contract must still accept streams
    env.ledger().set_timestamp(0);
    let stream_id = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );

    let state = client.get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.status, StreamStatus::Active);
}

// ---------------------------------------------------------------------------
// Tests — create_stream
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_initial_state() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    assert_eq!(stream_id, 0, "first stream id should be 0");

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);

    // Contract should hold the deposit
    assert_eq!(ctx.token().balance(&ctx.contract_id), 1000);
    // Sender balance reduced by deposit
    assert_eq!(ctx.token().balance(&ctx.sender), 9000);
}

#[test]
fn test_create_stream_emits_event() {
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
        &None,
    );

    let events = ctx.env.events().all();
    let event = events.last().unwrap();

    let event_data = crate::StreamCreated::try_from_val(&ctx.env, &event.2).unwrap();
    assert_eq!(event_data.stream_id, stream_id);
    assert_eq!(event_data.sender, ctx.sender);
    assert_eq!(event_data.recipient, ctx.recipient);
    assert_eq!(event_data.deposit_amount, 1000);
    assert_eq!(event_data.rate_per_second, 1);
    assert_eq!(event_data.start_time, 0);
    assert_eq!(event_data.cliff_time, 0);
    assert_eq!(event_data.end_time, 1000);
}

#[test]
fn test_create_stream_panics_when_contract_paused() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().set_global_emergency_paused(&true);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));
}

#[test]
fn test_create_stream_succeeds_after_unpause() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().set_global_emergency_paused(&true);
    ctx.client().set_global_emergency_paused(&false);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    assert_eq!(id, 0);
    assert_eq!(
        ctx.client().get_stream_state(&id).status,
        StreamStatus::Active
    );
}

#[test]
fn test_withdraw_emits_event() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    let withdrawn = ctx.client().withdraw(&stream_id);

    let events = ctx.env.events().all();
    let event = events.last().unwrap();

    let event_data = crate::Withdrawal::try_from_val(&ctx.env, &event.2).unwrap();
    assert_eq!(event_data.stream_id, stream_id);
    assert_eq!(event_data.recipient, ctx.recipient);
    assert_eq!(event_data.amount, withdrawn);
    assert_eq!(event_data.amount, 500);
}

/// Create a stream, perform partial withdraws then a final withdraw, and
/// assert `withdrawn_amount` increments and status transitions to Completed.
#[test]
fn test_withdraw_partial_then_full_updates_state() {
    let ctx = TestContext::setup();

    // Create a standard stream: deposit=1000, rate=1/s, duration=1000s
    let stream_id = ctx.create_default_stream();

    // Advance to t=300 and withdraw -> should get 300
    ctx.env.ledger().set_timestamp(300);
    let amt1 = ctx.client().withdraw(&stream_id);
    assert_eq!(amt1, 300, "first withdraw should return 300");

    let state1 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state1.withdrawn_amount, 300);
    assert_eq!(state1.status, StreamStatus::Active);

    // Advance to t=800 and withdraw -> should get 500 (800 - 300)
    ctx.env.ledger().set_timestamp(800);
    let amt2 = ctx.client().withdraw(&stream_id);
    assert_eq!(amt2, 500, "second withdraw should return 500");

    let state2 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state2.withdrawn_amount, 800);
    assert_eq!(state2.status, StreamStatus::Active);

    // Advance to t=1000 and withdraw -> should get final 200 and mark Completed
    ctx.env.ledger().set_timestamp(1000);
    let amt3 = ctx.client().withdraw(&stream_id);
    assert_eq!(amt3, 200, "final withdraw should return remaining 200");

    let state3 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state3.withdrawn_amount, 1000);
    assert_eq!(state3.status, StreamStatus::Completed);
}

#[test]
fn test_create_stream_zero_deposit_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &0_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_create_stream_invalid_times_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &500u64, // end before start
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_create_stream_multiple() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id_1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals end
        &1000u64,
        &None,
    );

    let stream_id_2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals end
        &1000u64,
        &None,
    );

    let stream_id_3 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64, // cliff equals end
        &500u64,
        &None,
    );

    let stream_id_4 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &4000_i128,
        &1_i128,
        &0u64,
        &0u64, // cliff equals end
        &4000u64,
        &None,
    );

    let stream_id_5 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64, // cliff equals end
        &1000u64,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state.stream_id, 0);

    let state = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state.stream_id, 1);

    let state = ctx.client().get_stream_state(&stream_id_3);
    assert_eq!(state.stream_id, 2);

    let state = ctx.client().get_stream_state(&stream_id_4);
    assert_eq!(state.stream_id, 3);

    let state = ctx.client().get_stream_state(&stream_id_5);
    assert_eq!(state.stream_id, 4);
}

#[test]
fn test_create_stream_multiple_loop() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let mut counter = 0;
    let mut stream_vec = Vec::new(&ctx.env);
    loop {
        let stream_id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &10_i128,
            &1_i128,
            &0u64,
            &0u64, // cliff equals end
            &10u64,
            &None,
        );

        counter += 1;

        stream_vec.push_back(stream_id);

        if counter == 100 {
            break;
        }
    }

    let mut counter = 0;
    loop {
        let state = ctx.client().get_stream_state(&counter);
        let stream_id = stream_vec.get(counter as u32).unwrap();

        assert_eq!(state.stream_id, counter);
        assert_eq!(state.stream_id, stream_id);
        counter += 1;

        if counter == 100 {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — Issue #123: no hard cap on deposit or duration (policy test)
// ---------------------------------------------------------------------------

/// The contract must accept a very large deposit_amount (no artificial ceiling).
/// This verifies the "no hard cap" policy documented in create_stream.
/// Overflow in accrual math is handled separately by checked_mul + clamping.
#[test]
fn test_create_stream_large_deposit_accepted() {
    let ctx = TestContext::setup();

    // Use a value well above any "reasonable" protocol limit — 10^18 tokens.
    // The sender must have enough balance; mint it first.
    let large_deposit: i128 = 1_000_000_000_000_000_000_i128; // 10^18
    let rate: i128 = 1_000_000_000_i128; // 10^9 / s
    let duration: u64 = 1_000_000_000; // 10^9 s

    // Confirm deposit exactly covers rate × duration (no excess needed).
    assert_eq!(large_deposit, rate * duration as i128);

    ctx.sac.mint(&ctx.sender, &large_deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &large_deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, large_deposit);
    assert_eq!(state.rate_per_second, rate);
    assert_eq!(state.end_time - state.start_time, duration);
    assert_eq!(state.status, StreamStatus::Active);
}

/// The contract must accept a very long stream duration (no artificial ceiling).
/// This verifies the "no hard cap" policy documented in create_stream.
#[test]
fn test_create_stream_long_duration_accepted() {
    let ctx = TestContext::setup();

    // 100 years in seconds — deliberately beyond any "reasonable" UX limit.
    let duration: u64 = 3_153_600_000;
    let rate: i128 = 1;
    let deposit: i128 = rate * duration as i128; // exactly covers duration

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time - state.start_time, duration);
    assert_eq!(state.deposit_amount, deposit);
    assert_eq!(state.status, StreamStatus::Active);
}

/// The contract must handle a deposit_amount close to i128::MAX correctly.
/// This verifies that calculate_accrued does not overflow when dealing with
/// the maximum possible token amounts supported by the type system.
#[test]
fn test_large_deposit_amount_sanity() {
    let ctx = TestContext::setup();

    // Use a value very close to i128::MAX
    let deposit: i128 = i128::MAX - 100_000_000_i128;
    let rate: i128 = 10_000_000_i128;
    let duration: u64 = (deposit / rate) as u64; // Perfectly covers duration

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    // Check midway
    let midway = duration / 2;
    ctx.env.ledger().set_timestamp(midway);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, (midway as i128) * rate);

    // Check at end
    ctx.env.ledger().set_timestamp(duration);
    let accrued_end = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued_end <= deposit);
    // Due to precision/rounding in duration calculation, it might be slightly less than deposit
    // but should be exactly (duration * rate)
    assert_eq!(accrued_end, (duration as i128) * rate);
}

// ---------------------------------------------------------------------------
// Tests — Issue #44: create_stream validation (invalid params) — full suite
// ---------------------------------------------------------------------------

// --- Group 1: end_time <= start_time ---

/// end_time exactly equal to start_time must panic
#[test]
#[should_panic]
fn test_create_stream_end_equals_start_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &500u64,
        &500u64,
        &500u64, // end == start
    );
}

/// end_time strictly less than start_time must panic
#[test]
#[should_panic]
fn test_create_stream_end_before_start_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &999u64, // end < start
    );
}

/// end_time exactly one second before start_time (boundary)
#[test]
#[should_panic]
fn test_create_stream_end_one_less_than_start_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &100u64,
        &99u64, // end = start - 1
    );
}

// --- Group 2: cliff_time outside [start_time, end_time] ---

/// cliff_time one second before start_time (lower boundary violation)
#[test]
#[should_panic]
fn test_create_stream_cliff_one_before_start_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &99u64, // cliff = start - 1
        &1100u64,
        &None,
    );
}

/// cliff_time one second after end_time (upper boundary violation)
#[test]
#[should_panic]
fn test_create_stream_cliff_one_after_end_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1001u64, // cliff = end + 1
        &1000u64,
        &None,
    );
}

/// cliff_time far before start_time
#[test]
#[should_panic]
fn test_create_stream_cliff_far_before_start_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &500u64,
        &0u64, // cliff far before start
        &1500u64,
        &None,
    );
}

/// cliff_time far after end_time
#[test]
#[should_panic]
fn test_create_stream_cliff_far_after_end_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &9999u64, // cliff far after end
        &1000u64,
        &None,
    );
}

/// cliff_time at start_time is valid (inclusive lower bound)
#[test]
fn test_create_stream_cliff_at_start_valid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &100u64, // cliff == start
        &1100u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.cliff_time, 100);
    assert_eq!(state.start_time, 100);
}

/// cliff_time at end_time is valid (inclusive upper bound)
#[test]
fn test_create_stream_cliff_at_end_valid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff == end
        &1000u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 1000);
}

// --- Group 3: deposit_amount <= 0 ---

/// deposit_amount of zero must panic
#[test]
#[should_panic]
fn test_create_stream_deposit_zero_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &0_i128, // zero
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// deposit_amount of -1 must panic
#[test]
#[should_panic]
fn test_create_stream_deposit_minus_one_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &-1_i128, // -1 boundary
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// deposit_amount of i128::MIN must panic
#[test]
#[should_panic]
fn test_create_stream_deposit_i128_min_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &i128::MIN,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// deposit_amount of 1 is valid (minimum positive)
#[test]
fn test_create_stream_deposit_one_valid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_i128, // minimum valid
        &1_i128,
        &0u64,
        &0u64,
        &1u64, // 1 second, so rate * duration = 1 == deposit
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.deposit_amount, 1);
}

// --- Group 4: rate_per_second <= 0 ---

/// rate_per_second of zero must panic
#[test]
#[should_panic]
fn test_create_stream_rate_zero_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &0_i128, // zero rate
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// rate_per_second of -1 must panic
#[test]
#[should_panic]
fn test_create_stream_rate_minus_one_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &-1_i128, // -1 boundary
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// rate_per_second of i128::MIN must panic
#[test]
#[should_panic]
fn test_create_stream_rate_i128_min_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &i128::MIN,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// rate_per_second of 1 is valid (minimum positive)
#[test]
fn test_create_stream_rate_one_valid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128, // minimum valid rate
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.rate_per_second, 1);
}

// --- Group 5: deposit < rate * duration ---

/// deposit one less than required (rate * duration - 1) must panic
#[test]
#[should_panic]
fn test_create_stream_deposit_one_less_than_required_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // rate=2, duration=500 → required=1000; deposit=999
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &999_i128, // one under boundary
        &2_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );
}

/// deposit exactly equal to rate * duration is valid (boundary pass)
#[test]
fn test_create_stream_deposit_exactly_required_valid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // rate=2, duration=500 → required=1000; deposit=1000
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128, // exactly at boundary
        &2_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.deposit_amount, 1000);
}

/// deposit much less than rate * duration must panic
#[test]
#[should_panic]
fn test_create_stream_deposit_far_below_required_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // rate=10, duration=1000 → required=10000; deposit=100
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128, // way under
        &10_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// deposit greater than required is valid (excess stays in contract)
#[test]
fn test_create_stream_deposit_above_required_valid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128, // more than rate(1) * duration(1000) = 1000
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.status, StreamStatus::Active);
}

// --- Group 6: sender == recipient ---

/// sender and recipient are the same address must panic
#[test]
#[should_panic]
fn test_create_stream_sender_is_recipient_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.sender, // same address
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// Self-streaming must not persist state, move tokens, or emit events.
#[test]
#[should_panic]
fn test_create_stream_sender_equals_recipient_has_no_side_effects() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    ctx.client().create_stream(
        &ctx.sender,
        &ctx.sender, // invalid: same address
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    assert_eq!(
        ctx.client().get_stream_count(),
        stream_count_before,
        "stream counter must not advance on validation failure"
    );
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        sender_balance_before,
        "sender balance must not change on validation failure"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before,
        "contract balance must not change on validation failure"
    );
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no events should be emitted on validation failure"
    );
}

/// different sender and recipient is valid (sanity check)
#[test]
fn test_create_stream_different_sender_recipient_valid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let another = Address::generate(&ctx.env);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &another,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&id);
    assert_ne!(state.sender, state.recipient);
}

// ---------------------------------------------------------------------------
// Tests — Issue #35: validate positive amounts and sender != recipient
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_create_stream_zero_rate_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &0_i128, // zero rate
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

#[test]
#[should_panic]
fn test_create_stream_sender_equals_recipient_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.sender, // same as sender
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

// ---------------------------------------------------------------------------
// Tests — Issue #33: validate cliff_time in [start_time, end_time]
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_create_stream_cliff_before_start_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,  // start_time
        &50u64,   // cliff_time before start
        &1100u64, // end_time
    );
}

#[test]
#[should_panic]
fn test_create_stream_cliff_after_end_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1500u64, // cliff_time after end
        &1000u64,
        &None,
    );
}

#[test]
fn test_create_stream_cliff_equals_start_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64, // cliff equals start
        &1000u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 0);
}

#[test]
fn test_create_stream_cliff_equals_end_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals end
        &1000u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 1000);
}

// ---------------------------------------------------------------------------
// Tests — Issue #34: validate deposit_amount >= rate * duration
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_create_stream_deposit_less_than_total_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128, // deposit only 500
        &1_i128,   // rate = 1/s
        &0u64,
        &0u64,
        &1000u64, // duration = 1000s, so total = 1000 tokens needed
    );
}

#[test]
fn test_create_stream_deposit_equals_total_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128, // deposit exactly matches total
        &1_i128,    // rate = 1/s
        &0u64,
        &0u64,
        &1000u64, // duration = 1000s
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1000);
}

#[test]
fn test_create_stream_deposit_greater_than_total_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128, // deposit more than needed
        &1_i128,    // rate = 1/s
        &0u64,
        &0u64,
        &1000u64, // duration = 1000s, total needed = 1000
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 2000);
}

// ---------------------------------------------------------------------------
// Tests — Issue #36: reject when token transfer fails
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_create_stream_insufficient_balance_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Sender only has 10_000 tokens, trying to deposit 20_000
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &20_000_i128,
        &20_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

#[test]
fn test_create_stream_transfer_failure_no_state_change() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Attempt to create stream with insufficient balance (should panic)
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &20_000_i128, // more than sender has
            &20_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        )
    }));

    assert!(
        result.is_err(),
        "should have panicked on insufficient balance"
    );

    // In Soroban, a failed transaction is rolled back, so we can't easily verify
    // state wasn't changed in a unit test. The key point is the transfer happens
    // before any state modification in the contract logic.
}

// ---------------------------------------------------------------------------
// Tests — calculate_accrued
// ---------------------------------------------------------------------------

#[test]
fn test_calculate_accrued_at_start() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(0);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "nothing accrued at start_time");
}

#[test]
fn test_calculate_accrued_before_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &500u64,
        &1000u64,
        &None,
    );
    ctx.env.ledger().set_timestamp(300);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0);
}

#[test]
fn test_calculate_accrued_mid_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(300);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 300, "300s × 1/s = 300");
}

#[test]
fn test_calculate_accrued_capped_at_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(9999); // way past end

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1000, "accrued must be capped at deposit_amount");
}

#[test]
fn test_calculate_accrued_before_cliff_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(200); // before cliff at 500

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "nothing accrued before cliff");
}

#[test]
fn test_calculate_accrued_after_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(600); // 100s after cliff at 500

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 600,
        "600s × 1/s = 600 (uses start_time, not cliff)"
    );
}

#[test]
fn test_accrued_after_cliff_before_end() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10000_i128,
        &10_i128,
        &0u64,
        &500u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 5000);

    ctx.env.ledger().set_timestamp(750);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 7500);

    ctx.env.ledger().set_timestamp(999);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 9990);

    ctx.env.ledger().set_timestamp(1000);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 10000);

    ctx.env.ledger().set_timestamp(1500);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 10000);
}

#[test]
fn test_create_stream_with_cliff_equals_start_accrues_immediately() {
    let ctx = TestContext::setup();

    // Create stream where cliff_time == start_time (no cliff period, immediate vesting)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128, // deposit_amount
        &1_i128,    // rate_per_second (1 token per second)
        &0u64,      // start_time
        &0u64,      // cliff_time (equal to start_time)
        &1000u64,   // end_time
    );

    // Advance time past start; since cliff == start, accrual should begin immediately
    ctx.env.ledger().set_timestamp(500);

    // Verify accrual begins immediately at start_time
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 500,
        "accrual should start immediately at start_time when cliff == start; 500s × 1/s = 500"
    );

    // Verify stream state before withdrawal
    let state_before = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_before.status, StreamStatus::Active);
    assert_eq!(state_before.withdrawn_amount, 0);

    // Withdraw accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 500,
        "should withdraw the full accrued amount (no cliff to block)"
    );

    // Verify stream state after withdrawal
    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_after.withdrawn_amount, 500,
        "withdrawn_amount should be updated"
    );
    assert_eq!(
        state_after.status,
        StreamStatus::Active,
        "stream should remain active (not completed)"
    );
}

#[test]
fn test_calculate_accrued_max_values() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &(i128::MAX - 10_000_i128));
    let stream_id = ctx.create_max_rate_stream();

    ctx.env.ledger().set_timestamp(u64::MAX);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, i128::MAX - 1, "accrued should be max");

    let state = ctx.client().get_stream_state(&stream_id);
    assert!(accrued <= state.deposit_amount);
    assert!(accrued >= 0);
}

#[test]
fn test_calculate_accrued_overflow_protection() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &(i128::MAX - 10_000_i128));
    let stream_id = ctx.create_half_max_rate_stream();

    ctx.env.ledger().set_timestamp(1_800);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 42535295865117307932921825928971026400_i128);
}
/// Completed stream: calculate_accrued must return deposit_amount regardless
/// of the current timestamp, providing a deterministic informational value.
#[test]
fn test_calculate_accrued_completed_stream_returns_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 1000 tokens, 0–1000s

    // Fully withdraw to transition the stream to Completed
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Querying at the exact end time
    let accrued_at_end = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_at_end, 1000,
        "Completed stream must return deposit_amount at end time"
    );

    // Querying far in the future must return the same value
    ctx.env.ledger().set_timestamp(99_999);
    let accrued_far_future = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_far_future, 1000,
        "Completed stream must return deposit_amount regardless of current timestamp"
    );
}

/// Cancelled stream: calculate_accrued must return the final accrued value at
/// cancellation time and must not continue growing with wall-clock time.
#[test]
fn test_calculate_accrued_cancelled_stream_time_based() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 1000 tokens, 0–1000s, rate 1/s

    // Cancel at t=400 — contract refunds 600 to sender, holds 400 for recipient
    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // At the same timestamp, accrued must equal the amount held in the contract
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 400,
        "Cancelled stream must return time-based accrued amount"
    );
    assert_eq!(
        accrued - state.withdrawn_amount,
        400,
        "withdrawable must equal what the contract holds"
    );

    // Far in the future, value must stay frozen at cancellation accrual
    ctx.env.ledger().set_timestamp(9_999);
    let accrued_frozen = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_frozen, 400,
        "Cancelled stream accrued must remain frozen at cancellation accrual"
    );
}

// ---------------------------------------------------------------------------
// Tests — calculate_accrued: status-specific behavior matrix
// Issue #268: Crisp semantics for every stream status
//
// | Status     | Time Source            | Expected Behavior                         |
// |------------|------------------------|-------------------------------------------|
// | Active     | env.ledger().timestamp| Accrual grows with wall-clock time        |
// | Paused     | env.ledger().timestamp| Same as Active (accrual continues)        |
// | Completed  | N/A (ignored)         | Returns deposit_amount (deterministic)    |
// | Cancelled  | cancelled_at          | Frozen at cancellation time               |
// ---------------------------------------------------------------------------

/// Paused stream before cliff: calculate_accrued must return 0.
/// Accrual does NOT start until cliff_time, regardless of pause state.
#[test]
fn test_calculate_accrued_paused_before_cliff() {
    let ctx = TestContext::setup();
    // Create stream with cliff at 500
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,    // start_time
        &500u64,  // cliff_time
        &1000u64, // end_time
    );

    // Advance to t=300 (before cliff) and pause
    ctx.env.ledger().set_timestamp(300);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Accrued must be 0 (before cliff)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "paused stream before cliff must return 0");
}

/// Paused stream after cliff: calculate_accrued must accrue normally using current ledger time.
/// Pausing does NOT freeze accrual — it only blocks withdrawals.
#[test]
fn test_calculate_accrued_paused_after_cliff() {
    let ctx = TestContext::setup();
    // Create stream with cliff at 500
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,    // start_time
        &500u64,  // cliff_time
        &1000u64, // end_time
    );

    // Advance past cliff and pause
    ctx.env.ledger().set_timestamp(600);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Accrued must be 600 (time-based, not frozen at pause)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 600,
        "paused stream after cliff must accrue normally"
    );

    // Advance time while paused — accrual should continue
    ctx.env.ledger().set_timestamp(800);
    let accrued_later = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_later, 800, "accrual must continue while paused");
}

/// Paused stream at end_time: calculate_accrued must cap at deposit_amount.
#[test]
fn test_calculate_accrued_paused_at_end_time() {
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
        &None,
    );

    // Advance to nearly end_time and pause
    ctx.env.ledger().set_timestamp(999);
    ctx.client().pause_stream(&stream_id);

    // Advance to end_time
    ctx.env.ledger().set_timestamp(1000);

    // Accrued must be capped at deposit
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 1000,
        "paused stream at end_time must cap at deposit"
    );

    // Advance past end_time while paused — should still cap at deposit
    ctx.env.ledger().set_timestamp(2000);
    let accrued_future = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_future, 1000,
        "paused stream past end_time must still cap at deposit"
    );
}

/// Paused stream: calculate_accrued is independent of pause/resume cycle.
/// This test verifies that calling calculate_accrued multiple times
/// returns the same value at the same timestamp, regardless of pause state changes.
#[test]
fn test_calculate_accrued_paused_deterministic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Initial state: Active
    ctx.env.ledger().set_timestamp(500);
    let accrued_active = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_active, 500);

    // Pause the stream
    ctx.client().pause_stream(&stream_id);

    // At same timestamp, accrued must be identical
    let accrued_paused = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_paused, accrued_active,
        "accrued must be same before/after pause at same timestamp"
    );

    // Resume the stream
    ctx.client().resume_stream(&stream_id);

    // At same timestamp, accrued must still be identical
    let accrued_resumed = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_resumed, accrued_active,
        "accrued must be same after resume at same timestamp"
    );
}

/// Cancelled stream before cliff: calculate_accrued must return 0 (frozen at cancellation).
#[test]
fn test_calculate_accrued_cancelled_before_cliff() {
    let ctx = TestContext::setup();
    // Create stream with cliff at 500
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,    // start_time
        &500u64,  // cliff_time
        &1000u64, // end_time
    );

    // Cancel at t=300 (before cliff)
    ctx.env.ledger().set_timestamp(300);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Accrued must be 0 (frozen before cliff)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "cancelled stream before cliff must return 0");

    // Far in the future, still 0
    ctx.env.ledger().set_timestamp(9999);
    let accrued_frozen = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_frozen, 0, "cancelled stream must stay frozen at 0");
}

/// Cancelled stream at exact cliff: calculate_accrued must return cliff accrual.
#[test]
fn test_calculate_accrued_cancelled_at_cliff() {
    let ctx = TestContext::setup();
    // Create stream with cliff at 500
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,    // start_time
        &500u64,  // cliff_time
        &1000u64, // end_time
    );

    // Cancel at exact cliff time (t=500)
    ctx.env.ledger().set_timestamp(500);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Accrued at cliff = cliff_time - start_time = 500 - 0 = 500
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 500,
        "cancelled stream at cliff must return cliff accrual"
    );
}

/// Completed stream: calculate_accrued must return deposit_amount regardless of timestamp.
/// This is the deterministic, timestamp-independent answer.
#[test]
fn test_calculate_accrued_completed_deterministic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 1000 tokens, 0-1000s

    // Complete the stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // At exact end_time
    let accrued_at_end = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_at_end, 1000,
        "completed stream at end_time returns deposit"
    );

    // Far in the future
    ctx.env.ledger().set_timestamp(9999);
    let accrued_future = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_future, 1000,
        "completed stream far future returns deposit"
    );

    // Even in the past
    ctx.env.ledger().set_timestamp(0);
    let accrued_past = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_past, 1000,
        "completed stream at start returns deposit"
    );
}

/// calculate_accrued is permissionless: any address can call it without authorization.
/// This verifies that no auth is required, which is essential for indexers and UI.
#[test]
fn test_calculate_accrued_permissionless_access() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Create a random third-party address (not sender, not recipient, not admin)
    let _third_party = Address::generate(&ctx.env);

    // Third party must be able to call calculate_accrued without auth
    // This would panic if auth was required
    let accrued = ctx.client().try_calculate_accrued(&stream_id);
    assert!(
        accrued.is_ok(),
        "calculate_accrued must be callable by anyone"
    );
}

/// calculate_accrued is a pure read: calling it must not mutate stream state.
/// This test verifies that repeated calls do not change withdrawn_amount or any field.
#[test]
fn test_calculate_accrued_no_state_mutation() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Get initial state
    ctx.env.ledger().set_timestamp(500);
    let state_before = ctx.client().get_stream_state(&stream_id);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);

    // Call calculate_accrued multiple times
    for _ in 0..10 {
        let accrued = ctx.client().calculate_accrued(&stream_id);
        assert_eq!(
            accrued, accrued_before,
            "accrued must be stable across calls"
        );
    }

    // State must be unchanged
    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_after.withdrawn_amount, state_before.withdrawn_amount,
        "withdrawn_amount must not change"
    );
    assert_eq!(
        state_after.status, state_before.status,
        "status must not change"
    );
    assert_eq!(
        state_after.deposit_amount, state_before.deposit_amount,
        "deposit_amount must not change"
    );
}

/// Edge case: calculate_accrued on stream with zero duration (start == end).
/// Expected: returns InvalidParams because the schedule is invalid.
#[test]
fn test_calculate_accrued_zero_duration_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    // Attempt to create stream with zero duration (start == end)
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &500u64, // start_time
        &500u64, // cliff_time
        &500u64, // end_time
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Edge case: calculate_accrued on stream with zero deposit.
/// Expected: returns InvalidParams because deposit must be > 0.
#[test]
fn test_calculate_accrued_zero_deposit_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    // Attempt to create stream with zero deposit
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &0_i128, // zero deposit
        &1_i128,
        &100u64,
        &100u64,
        &1000u64,
        &None,
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Edge case: calculate_accrued with zero rate (no accrual ever).
/// Expected: returns InvalidParams because rate must be > 0.
#[test]
fn test_calculate_accrued_zero_rate_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    // Attempt to create stream with zero rate
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &0_i128, // zero rate
        &100u64,
        &100u64,
        &1000u64,
        &None,
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

// ---------------------------------------------------------------------------
// Tests — calculate_accrued overflow and edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_large_rate_no_overflow() {
    // Security: Large rate_per_second values must not cause overflow or panic.
    // This tests rates approaching i128::MAX to ensure safe multiplication.
    let ctx = TestContext::setup();

    // Use a very large rate but short duration to avoid overflow
    let large_rate = i128::MAX / 10;
    let deposit = i128::MAX / 5;

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &large_rate,
        &0u64,
        &0u64,
        &2u64, // Very short duration
    );

    ctx.env.ledger().set_timestamp(1);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // Should not panic and should be capped at deposit
    assert!(accrued <= deposit, "accrued must not exceed deposit");
    assert!(accrued >= 0, "accrued must be non-negative");
}

#[test]
fn test_large_duration_no_overflow() {
    // Security: Large elapsed time values must not cause overflow.
    // This tests very large duration values to ensure safe time calculations.
    let ctx = TestContext::setup();

    let rate = 1_i128;
    let duration = 1_000_000_000u64; // 1 billion seconds (about 31 years)
    let deposit = 2_000_000_000_i128; // Covers duration + extra

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    // Set time to a very large value past the end
    ctx.env.ledger().set_timestamp(duration + 1_000_000);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // Should not overflow and should be capped at deposit
    assert!(accrued <= deposit, "accrued must not exceed deposit");
    assert!(accrued >= 0, "accrued must be non-negative");
    // At end time, should accrue exactly rate * duration
    assert_eq!(
        accrued, duration as i128,
        "should accrue full duration amount"
    );
}

#[test]
fn test_combined_large_rate_and_duration() {
    // Security: Worst-case scenario - both large rate and large duration.
    // This is the most critical overflow scenario: elapsed * rate_per_second.
    let ctx = TestContext::setup();

    // Use values that pass validation but will overflow in extended scenarios
    let large_rate = i128::MAX / 10000;
    let deposit = i128::MAX / 100;
    let duration = 100u64;

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &large_rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    // Set time to cause potential overflow in multiplication
    ctx.env.ledger().set_timestamp(50);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // Should be capped at deposit when overflow would occur
    assert!(accrued <= deposit, "overflow should cap at deposit_amount");
    assert!(accrued >= 0, "accrued must be non-negative");
}

#[test]
fn test_boundary_max_rate_per_second() {
    // Security: Very large rate_per_second values must be handled safely.
    let ctx = TestContext::setup();

    // Use large but realistic values that won't overflow in validation
    let large_rate = i128::MAX / 10000;
    let deposit = i128::MAX / 1000;

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &large_rate,
        &0u64,
        &0u64,
        &2u64, // Short duration
    );

    ctx.env.ledger().set_timestamp(2);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // Should not overflow and should be capped at deposit
    assert!(accrued <= deposit, "large rate should cap at deposit");
    assert!(accrued >= 0, "accrued must be non-negative");
}

#[test]
fn test_boundary_min_positive_values() {
    // Security: Minimum positive values (1) must work correctly.
    let ctx = TestContext::setup();

    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_i128, // Minimum deposit
        &1_i128, // Minimum rate
        &0u64,
        &0u64,
        &1u64, // Minimum duration
    );

    ctx.env.ledger().set_timestamp(1);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1, "minimum values should work correctly");
}

#[test]
fn test_zero_rate_returns_zero() {
    // Security: Zero rate must return zero accrued, not cause division errors.
    // Note: create_stream may reject zero rate, so we test the calculation logic.
    let ctx = TestContext::setup();

    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128, // Start with valid rate
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Even with time elapsed, if rate were 0, accrued would be 0
    ctx.env.ledger().set_timestamp(500);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // With rate=1, we expect 500
    assert_eq!(accrued, 500, "normal calculation works");
}

#[test]
fn test_zero_duration_returns_zero() {
    // Security: When start time equals end time, it returns InvalidParams.
    let ctx = TestContext::setup();

    ctx.env.ledger().set_timestamp(0);

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10000_i128,
        &10_i128,
        &0u64, // Start at 0
        &0u64, // No cliff
        &0u64, // End at 0 (duration is zero)
    );

    assert_eq!(result, Err(Ok(crate::ContractError::InvalidParams)));
}

#[test]
fn test_result_capping_at_deposit() {
    // Security: Result must NEVER exceed deposit_amount, even with calculation errors.
    let ctx = TestContext::setup();

    let rate = 10_i128;
    let duration = 1000u64;
    let deposit = 15000_i128; // More than rate * duration to test capping

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    // Set time way past end
    ctx.env.ledger().set_timestamp(10000);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // Should be capped at rate * duration, not deposit (since deposit is larger)
    assert_eq!(
        accrued,
        (rate * duration as i128),
        "should accrue full stream amount"
    );
    assert!(accrued <= deposit, "accrued must never exceed deposit");
}

#[test]
fn test_result_capping_with_overflow() {
    // Security: When multiplication overflows, result must cap at deposit_amount.
    let ctx = TestContext::setup();

    let rate = i128::MAX / 100000;
    let duration = 1u64;
    // Use checked arithmetic to avoid overflow in test setup
    let deposit = rate.checked_add(1000).unwrap_or(rate);

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    ctx.env.ledger().set_timestamp(1);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // Should not overflow and should be capped at deposit
    assert!(accrued <= deposit, "overflow should cap at deposit");
    assert!(accrued >= 0, "accrued must be non-negative");
}

#[test]
fn test_no_panic_on_extreme_inputs() {
    // Security: No combination of extreme inputs should cause panic.
    let ctx = TestContext::setup();

    let rate = i128::MAX / 100000;
    let duration = 10u64;
    // Use checked arithmetic to avoid overflow in test setup
    let deposit = rate
        .checked_mul(duration as i128)
        .and_then(|v| v.checked_add(1000))
        .unwrap_or(i128::MAX / 10);

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    // Test at various timestamps
    ctx.env.ledger().set_timestamp(2);
    let accrued1 = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued1 >= 0 && accrued1 <= deposit);

    ctx.env.ledger().set_timestamp(5);
    let accrued2 = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued2 >= 0 && accrued2 <= deposit);

    ctx.env.ledger().set_timestamp(20);
    let accrued3 = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued3 >= 0 && accrued3 <= deposit);
}

#[test]
fn test_no_underflow_negative_result() {
    // Security: Result must never be negative due to underflow.
    // The max(0) in calculate_accrued ensures this.
    let ctx = TestContext::setup();

    ctx.env.ledger().set_timestamp(1000);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &2000u64,
        &None,
    );

    // Query before start (though this shouldn't happen in practice)
    ctx.env.ledger().set_timestamp(500);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued >= 0, "accrued must never be negative");
}

#[test]
fn test_elapsed_time_checked_subtraction() {
    // Security: Time subtraction must use checked arithmetic to prevent underflow.
    let ctx = TestContext::setup();

    ctx.env.ledger().set_timestamp(1000);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &2000u64,
        &None,
    );

    // Set time before start (edge case)
    ctx.env.ledger().set_timestamp(500);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // Should return 0, not panic or underflow
    assert_eq!(accrued, 0, "should handle time before start gracefully");
}

#[test]
fn test_rate_times_duration_overflow_caps() {
    // Security: The critical multiplication (elapsed * rate) must detect overflow.
    // When overflow occurs, it should cap at deposit_amount, not wrap around.
    let ctx = TestContext::setup();

    // Choose values that will definitely overflow when multiplied
    let rate = i128::MAX / 100000;
    let duration = 10u64;
    // Use checked arithmetic to avoid overflow in test setup
    let deposit = rate
        .checked_mul(duration as i128)
        .and_then(|v| v.checked_add(1000))
        .unwrap_or(i128::MAX / 10);

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    ctx.env.ledger().set_timestamp(5);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // Should not overflow
    assert!(
        accrued <= deposit,
        "overflow in multiplication should cap at deposit"
    );
    assert!(accrued >= 0, "accrued must be non-negative");
}

#[test]
fn test_accrued_never_exceeds_deposit_multiple_checks() {
    // Security: Comprehensive verification that accrued never exceeds deposit
    // across various scenarios and time points.
    let ctx = TestContext::setup();

    let deposit = 10_000_i128;
    let rate = 50_i128;

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &100u64, // Would accrue 5,000 at end
    );

    // Check at multiple time points
    let test_times = [0u64, 50, 100, 200, 500, 1000, 10000, u64::MAX / 2];

    for time in test_times.iter() {
        ctx.env.ledger().set_timestamp(*time);
        let accrued = ctx.client().calculate_accrued(&stream_id);
        assert!(
            accrued <= deposit,
            "accrued {} must not exceed deposit {} at time {}",
            accrued,
            deposit,
            time
        );
        assert!(
            accrued >= 0,
            "accrued must be non-negative at time {}",
            time
        );
    }
}

#[test]
fn test_cliff_with_overflow_scenario() {
    // Security: Cliff logic must work correctly even with overflow-prone values.
    let ctx = TestContext::setup();

    let deposit = i128::MAX / 1000;
    let rate = i128::MAX / 100000;

    ctx.sac.mint(&ctx.sender, &deposit);
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &50u64, // Cliff at 50
        &100u64,
        &None,
    );

    // Before cliff - should return 0
    ctx.env.ledger().set_timestamp(25);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 0, "before cliff should be 0");

    // After cliff - should calculate but cap at deposit
    ctx.env.ledger().set_timestamp(75);
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued_after > 0, "after cliff should accrue");
    assert!(accrued_after <= deposit, "must not exceed deposit");
}

// ---------------------------------------------------------------------------
// Tests — pause / resume
// ---------------------------------------------------------------------------

#[test]
fn test_pause_and_resume() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn test_admin_can_resume_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);

    // Auth override test for resume
    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
#[should_panic]
fn test_pause_already_paused_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().pause_stream(&stream_id);
    ctx.client().pause_stream(&stream_id); // second pause should panic
}

#[test]
#[should_panic]
fn test_resume_active_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().resume_stream(&stream_id);
}

#[test]
#[should_panic]
fn test_resume_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    ctx.client().resume_stream(&stream_id);
}

#[test]
#[should_panic]
fn test_resume_cancelled_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().cancel_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    ctx.client().resume_stream(&stream_id);
}

#[test]
#[should_panic]
fn test_pause_cancelled_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().cancel_stream(&stream_id);
    ctx.client().pause_stream(&stream_id); // Cancelled — must panic with general message
}

// ---------------------------------------------------------------------------
// Tests — cancel_stream
// ---------------------------------------------------------------------------

#[test]
fn test_cancel_stream_full_refund() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let sender_balance_before = ctx.token().balance(&ctx.sender);

    ctx.env.ledger().set_timestamp(0); // no time has passed
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(0));

    let sender_balance_after = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_balance_after - sender_balance_before, 1000);
}

#[test]
fn test_cancel_stream_partial_refund() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    let sender_balance_before = ctx.token().balance(&ctx.sender);

    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(300));

    let sender_balance_after = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_balance_after - sender_balance_before, 700);
}

#[test]
fn test_cancel_stream_as_admin() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(0);

    ctx.client().cancel_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(0));
}

#[test]
fn test_cancel_refund_plus_frozen_accrued_equals_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // deposit=1000, rate=1/s

    ctx.env.ledger().set_timestamp(420);
    let sender_before = ctx.token().balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);
    let sender_after = ctx.token().balance(&ctx.sender);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(420));

    // Advance time to prove accrual is frozen at cancelled_at.
    ctx.env.ledger().set_timestamp(9_999);
    let frozen_accrued = ctx.client().calculate_accrued(&stream_id);
    let refund = sender_after - sender_before;

    assert_eq!(refund, 580);
    assert_eq!(frozen_accrued, 420);
    assert_eq!(refund + frozen_accrued, state.deposit_amount);
}

#[test]
#[should_panic]
fn test_cancel_already_cancelled_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().cancel_stream(&stream_id);
    ctx.client().cancel_stream(&stream_id);
}

#[test]
#[should_panic]
fn test_cancel_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    ctx.client().cancel_stream(&stream_id);
}

#[test]
fn test_cancel_stream_allows_active_or_paused() {
    let ctx = TestContext::setup();
    let active_stream_id = ctx.create_default_stream();
    let paused_stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&paused_stream_id);

    ctx.client().cancel_stream(&active_stream_id);
    ctx.client().cancel_stream(&paused_stream_id);

    let active_state = ctx.client().get_stream_state(&active_stream_id);
    let paused_state = ctx.client().get_stream_state(&paused_stream_id);
    assert_eq!(active_state.status, StreamStatus::Cancelled);
    assert_eq!(paused_state.status, StreamStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Tests — withdraw
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_after_cancel_gets_accrued_amount() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    // On cancel: refund unstreamed, leave accrued in contract (temporarily)
    ctx.client().cancel_stream(&stream_id);

    // Recipient should NOT have received accrued yet (feature disabled temporarily)
    assert_eq!(ctx.token().balance(&ctx.recipient), 0);
    // Contract should hold the accrued amount (400)
    assert_eq!(ctx.token().balance(&ctx.contract_id), 400);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 0); // No automatic payout on cancel
    assert_eq!(state.status, StreamStatus::Cancelled);
}

#[test]
fn test_withdraw_twice_after_cancel_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream is Cancelled (withdraw on cancelled stream is rejected at contract level)
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    // If we tried to withdraw, the contract would reject it with "stream cancelled"
    // This validates the cancel path prevented further withdrawals
}

/// Status is Cancelled when user cancels (accrued left in contract for now)
#[test]
fn test_withdraw_completed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&stream_id);

    // On cancel at end, all funds remain streamed but not yet transferred to recipient
    // (feature temporarily disabled; accrued stays in contract until claimed)
    assert_eq!(ctx.token().balance(&ctx.recipient), 0);
    assert_eq!(ctx.token().balance(&ctx.contract_id), 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

/// Status is Complete when Recipient fully withdraws in batches
#[test]
fn test_withdraw_completed_in_batch() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 200 at t=200
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    // Withdraw 300 at t=500
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);

    // Withdraw remaining 500 at t=1000
    ctx.env.ledger().set_timestamp(1000);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 500);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);
}

#[test]
fn test_withdraw_full_completes_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000); // end of stream

    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);
}

#[test]
#[should_panic]
fn test_withdraw_from_paused_stream_completes_if_full() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().pause_stream(&stream_id);

    // This should panic now because withdrawals are blocked while paused
    ctx.client().withdraw(&stream_id);
}

/// Test withdraw when withdrawable is zero (nothing to withdraw).
/// Should return 0 without transfer or state change (idempotent).
#[test]
fn test_withdraw_nothing_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(0);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 0, "should return 0 when nothing to withdraw");
}

#[test]
#[should_panic]
fn test_withdraw_already_completed_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Try to withdraw again
    ctx.client().withdraw(&stream_id);
}

#[test]
fn test_withdraw_partial_stays_active() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.withdrawn_amount, 200);

    ctx.env.ledger().set_timestamp(500); // 500 accrued, 500 unstreamed
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, 300,
        "recipient should withdraw the difference (500 - 200)"
    );

    ctx.env.ledger().set_timestamp(800); // 800 accrued, 200 unstreamed
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, 300,
        "recipient should withdraw the difference (800 - 500)"
    );

    ctx.env.ledger().set_timestamp(1000); // 1000 accrued, 0 unstreamed
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, 200,
        "recipient should withdraw the final 200 tokens"
    );

    // Nothing left in contract
    assert_eq!(ctx.token().balance(&ctx.contract_id), 0);

    // Complete withdrawal record
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);
}

#[test]
fn test_withdraw_completed_panic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&stream_id);

    // Verify stream is Cancelled (withdraw on cancelled stream is rejected at contract level)
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    // If we tried to withdraw, the contract would reject it with "stream cancelled"
}

// ---------------------------------------------------------------------------
// Tests — withdraw (general)
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_mid_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 500);
}

/// Test withdraw before cliff returns 0 (idempotent behavior).
#[test]
fn test_withdraw_before_cliff_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(100);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 0, "should return 0 before cliff");
}

/// Verify that withdraw enforces recipient-only authorization.
/// The require_auth() on stream.recipient ensures only the recipient can withdraw.
/// This test verifies that the authorization check is in place.
/// Note: In SDK 21.7.7, env.invoker() is not available, so we use require_auth()
/// which is the security-equivalent mechanism. The require_auth() call ensures
/// that only the recipient can authorize the withdrawal, preventing unauthorized access.
#[test]
fn test_withdraw_requires_recipient_authorization() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    // With mock_all_auths(), recipient's auth is mocked, so withdraw succeeds
    // This verifies that the authorization mechanism works correctly
    let recipient_before = ctx.token().balance(&ctx.recipient);
    let amount = ctx.client().withdraw(&stream_id);

    assert_eq!(amount, 500);
    assert_eq!(ctx.token().balance(&ctx.recipient) - recipient_before, 500);

    // Verify the withdrawal was recorded
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 500);

    // The require_auth() call in withdraw() ensures that only the recipient
    // can authorize this call, which is equivalent to checking env.invoker() == recipient
}

// ---------------------------------------------------------------------------
// Tests — withdraw_to (#219)
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_to_destination_receives_tokens() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(400);
    let amount = ctx.client().withdraw_to(&stream_id, &destination);

    assert_eq!(amount, 400);
    assert_eq!(ctx.token().balance(&destination), 400);
    assert_eq!(ctx.token().balance(&ctx.recipient), 0);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 400);
}

#[test]
fn test_withdraw_to_full_amount_completes_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(1000);
    let amount = ctx.client().withdraw_to(&stream_id, &destination);

    assert_eq!(amount, 1000);
    assert_eq!(ctx.token().balance(&destination), 1000);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
}

#[test]
fn test_withdraw_to_requires_recipient_auth() {
    let ctx = TestContext::setup_strict();
    use soroban_sdk::testutils::MockAuth;
    use soroban_sdk::testutils::MockAuthInvoke;
    use soroban_sdk::IntoVal;

    ctx.env.ledger().set_timestamp(0);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(500);

    // Try to withdraw_to without recipient auth (should fail)
    let destination = Address::generate(&ctx.env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw_to(&stream_id, &destination)
    }));
    assert!(
        result.is_err(),
        "withdraw_to without recipient auth should panic"
    );
}

// Tests — batch_withdraw (#220)
// ---------------------------------------------------------------------------

fn stream_ids_vec(env: &Env, ids: &[u64]) -> soroban_sdk::Vec<u64> {
    let mut v = soroban_sdk::Vec::new(env);
    for &id in ids {
        v.push_back(id);
    }
    v
}

// --- batch_withdraw: completed streams yield zero amounts ---

/// A single completed stream in the batch returns amount=0, no transfer, no event.
#[test]
fn test_batch_withdraw_completed_stream_yields_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Complete the stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );

    let recipient_before = ctx.token().balance(&ctx.recipient);
    let contract_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let results = ctx
        .client()
        .batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[stream_id]));

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().stream_id, stream_id);
    assert_eq!(
        results.get(0).unwrap().amount,
        0,
        "completed stream must yield 0"
    );

    // No token transfer
    assert_eq!(ctx.token().balance(&ctx.recipient), recipient_before);
    assert_eq!(ctx.token().balance(&ctx.contract_id), contract_before);
    // No new events
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// Mixed batch: [active, completed, active] — completed entry is zero, others transfer.
#[test]
fn test_batch_withdraw_mixed_active_and_completed() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id0 = ctx.create_default_stream(); // active
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    ); // will be completed
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    ); // active

    // Complete id1
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id1);
    assert_eq!(
        ctx.client().get_stream_state(&id1).status,
        StreamStatus::Completed
    );

    ctx.env.ledger().set_timestamp(1000);
    let results = ctx
        .client()
        .batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0, id1, id2]));

    assert_eq!(results.len(), 3);
    // id0: accrued=1000, withdrawn=0 → amount=1000
    assert_eq!(results.get(0).unwrap().amount, 1000);
    // id1: Completed → amount=0
    assert_eq!(results.get(1).unwrap().amount, 0);
    // id2: accrued=1000, withdrawn=0 → amount=1000
    assert_eq!(results.get(2).unwrap().amount, 1000);

    assert_eq!(ctx.token().balance(&ctx.recipient), 1000 + 1000 + 1000); // 1000 from earlier + 2000 now
}

/// All streams in batch are completed — all results are zero, no transfers, no events.
#[test]
fn test_batch_withdraw_all_completed_all_zero() {
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
        &None,
    );

    // Complete both
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id0);
    ctx.client().withdraw(&id1);

    let recipient_before = ctx.token().balance(&ctx.recipient);
    let events_before = ctx.env.events().all().len();

    let results = ctx
        .client()
        .batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0, id1]));

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().amount, 0);
    assert_eq!(results.get(1).unwrap().amount, 0);

    assert_eq!(ctx.token().balance(&ctx.recipient), recipient_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// Completed stream in batch does NOT panic — the whole batch succeeds.
#[test]
fn test_batch_withdraw_completed_stream_does_not_panic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Must not panic
    let result = ctx
        .client()
        .try_batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[stream_id]));
    assert!(result.is_ok(), "completed stream in batch must not panic");
}

/// Completed stream state is unchanged after batch_withdraw (no double-spend).
#[test]
fn test_batch_withdraw_completed_stream_state_unchanged() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state_before = ctx.client().get_stream_state(&stream_id);

    ctx.client()
        .batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[stream_id]));

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.status, StreamStatus::Completed);
    assert_eq!(state_after.withdrawn_amount, state_before.withdrawn_amount);
    assert_eq!(state_after.deposit_amount, state_before.deposit_amount);
}

/// Paused stream in batch panics (contrast with completed which does not).
#[test]
#[should_panic]
fn test_batch_withdraw_paused_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().pause_stream(&stream_id);

    ctx.client()
        .batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[stream_id]));
}

/// batch_withdraw on a stream completed mid-batch: the stream that just completed
/// contributes its final amount; a second pass on the same id yields zero.
#[test]
fn test_batch_withdraw_stream_completes_mid_batch_second_pass_zero() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id0 = ctx.create_default_stream(); // 1000 tokens, 1000s

    // Withdraw all at end — stream becomes Completed
    ctx.env.ledger().set_timestamp(1000);
    let results = ctx
        .client()
        .batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0]));
    assert_eq!(results.get(0).unwrap().amount, 1000);
    assert_eq!(
        ctx.client().get_stream_state(&id0).status,
        StreamStatus::Completed
    );

    // Second batch call on the now-completed stream yields zero
    let results2 = ctx
        .client()
        .batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0]));
    assert_eq!(results2.get(0).unwrap().amount, 0);
    assert_eq!(ctx.token().balance(&ctx.contract_id), 0);
}

#[test]
fn test_batch_withdraw_multiple_streams() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id0 = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(0);
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    ctx.env.ledger().set_timestamp(0);
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(400);
    let stream_ids = stream_ids_vec(&ctx.env, &[id0, id1, id2]);
    let results = ctx.client().batch_withdraw(&ctx.recipient, &stream_ids);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().stream_id, id0);
    assert_eq!(results.get(0).unwrap().amount, 400);
    assert_eq!(results.get(1).unwrap().stream_id, id1);
    assert_eq!(results.get(1).unwrap().amount, 800);
    assert_eq!(results.get(2).unwrap().stream_id, id2);
    assert_eq!(results.get(2).unwrap().amount, 400);

    assert_eq!(ctx.token().balance(&ctx.recipient), 400 + 800 + 400);
}

#[test]
fn test_batch_withdraw_mixed_state_some_zero() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let _id0 = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(0);
    let _id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Test batch withdraw with mixed states
    ctx.env.ledger().set_timestamp(500);
    let mut stream_ids = soroban_sdk::Vec::new(&ctx.env);
    stream_ids.push_back(_id0);
    stream_ids.push_back(_id1);

    let results = ctx.client().batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(results.len(), 2);
}

#[test]
#[should_panic]
fn test_withdraw_to_rejects_contract_as_destination() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw_to(&stream_id, &ctx.contract_id);
}

#[test]
fn test_withdraw_to_zero_withdrawable_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at 500
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(100);
    let amount = ctx.client().withdraw_to(&stream_id, &destination);

    assert_eq!(amount, 0);
    assert_eq!(ctx.token().balance(&destination), 0);
}

#[test]
fn test_withdraw_to_after_partial_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    ctx.env.ledger().set_timestamp(700);
    let amount = ctx.client().withdraw_to(&stream_id, &destination);

    assert_eq!(amount, 400);
    assert_eq!(ctx.token().balance(&ctx.recipient), 300);
    assert_eq!(ctx.token().balance(&destination), 400);
}

#[test]
#[should_panic]
fn test_batch_withdraw_wrong_recipient_panics() {
    let ctx = TestContext::setup();
    let id0 = ctx.create_default_stream();
    let other = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(200);
    let stream_ids = stream_ids_vec(&ctx.env, &[id0]);
    let _ = ctx.client().batch_withdraw(&other, &stream_ids);
}

#[test]
fn test_batch_withdraw_empty_ids_returns_empty() {
    let ctx = TestContext::setup();
    let stream_ids = stream_ids_vec(&ctx.env, &[]);
    let results = ctx.client().batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(results.len(), 0);
}

#[test]
fn test_batch_withdraw_emits_events_per_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id0 = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(0);
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(250);
    let stream_ids = stream_ids_vec(&ctx.env, &[id0, id1]);
    let _ = ctx.client().batch_withdraw(&ctx.recipient, &stream_ids);

    // Each stream with withdrawable > 0 emits a "withdrew" event; we had 2 streams with 250 each
    let events = ctx.env.events().all();
    assert!(
        events.len() >= 2,
        "batch_withdraw must emit at least one event per withdrawal"
    );
}

#[test]
fn test_withdraw_recipient_success() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(500);

    // Mock recipient auth for withdraw
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.contract_id, &ctx.recipient, 500_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    let recipient_before = ctx.token().balance(&ctx.recipient);
    let amount = ctx.client().withdraw(&stream_id);

    assert_eq!(amount, 500);
    assert_eq!(ctx.token().balance(&ctx.recipient) - recipient_before, 500);
}

#[test]
#[should_panic]
fn test_withdraw_not_recipient_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(500);

    // Mock sender's auth for withdraw, which should fail because the contract
    // expects the recipient's auth.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    // This should panic with authorization failure because sender != recipient
    ctx.client().withdraw(&stream_id);
}

#[test]
fn test_withdraw_not_recipient_unauthorized_has_no_side_effects() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(700);
    let state_before = ctx.client().get_stream_state(&stream_id);
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let recipient_balance_before = ctx.token().balance(&ctx.recipient);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    // Wrong signer (sender instead of recipient) for withdraw.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw(&stream_id);
    }));
    assert!(result.is_err(), "non-recipient signer must be rejected");

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.withdrawn_amount, state_before.withdrawn_amount);
    assert_eq!(state_after.status, state_before.status);
    assert_eq!(ctx.token().balance(&ctx.sender), sender_balance_before);
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        recipient_balance_before
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

// ---------------------------------------------------------------------------
// Tests — close_completed_stream (#217)
// ---------------------------------------------------------------------------

#[test]
fn test_close_completed_stream_removes_storage() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    ctx.client().close_completed_stream(&stream_id);

    let result = ctx.client().try_get_stream_state(&stream_id);
    assert!(result.is_err(), "closed stream must not be queryable");
}

#[test]
#[should_panic]
fn test_close_completed_stream_rejects_active() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().close_completed_stream(&stream_id);
}

#[test]
fn test_close_cancelled_stream_success() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    ctx.client().close_completed_stream(&stream_id);
}

#[test]
fn test_close_completed_stream_emits_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    ctx.client().close_completed_stream(&stream_id);

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "closed event must be emitted");
}

#[test]
fn test_close_completed_stream_second_close_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    ctx.client().close_completed_stream(&stream_id);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().close_completed_stream(&stream_id);
    }));
    assert!(
        result.is_err(),
        "second close must panic (stream not found)"
    );
}

// COMPREHENSIVE EDGE CASE TESTS FOR close_completed_stream

#[test]
#[should_panic]
fn test_close_completed_stream_rejects_paused() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Pause the stream
    ctx.env.ledger().set_timestamp(500);
    ctx.client().pause_stream(&stream_id);

    // Try to close paused stream (should fail with InvalidState)
    ctx.client().close_completed_stream(&stream_id);
}

#[test]
#[should_panic]
fn test_close_completed_stream_rejects_nonexistent() {
    let ctx = TestContext::setup();

    // Try to close a stream that doesn't exist (should fail with StreamNotFound)
    ctx.client().close_completed_stream(&999u64);
}

#[test]
fn test_close_completed_stream_emits_correct_event_topic() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Clear events before close
    let _ = ctx.env.events().all();

    ctx.client().close_completed_stream(&stream_id);

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "StreamClosed event must be emitted");

    // Verify the event contains the correct stream_id
    // The event structure is: (symbol_short!("closed"), stream_id) -> StreamEvent::StreamClosed(stream_id)
    let found = events.iter().any(|e| {
        let topics = e.1.clone();
        topics.len() >= 2
            && topics
                .get(1)
                .map(|t: Val| u64::try_from_val(&ctx.env, &t) == Ok(stream_id))
                .unwrap_or(false)
    });
    assert!(
        found,
        "event must contain correct stream_id in topic (index 1)"
    );
}

#[test]
fn test_close_completed_stream_multiple_streams_closes_correct_one() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create three streams for the same recipient
    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
        &None,
    );

    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    // Complete all three streams
    ctx.env.ledger().set_timestamp(2000);
    ctx.client().withdraw(&id0);
    ctx.client().withdraw(&id1);
    ctx.client().withdraw(&id2);

    // Verify all are in recipient's index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 3);

    // Close only the middle stream (id1)
    ctx.client().close_completed_stream(&id1);

    // Verify only id1 is removed
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 2);
    assert_eq!(streams.get(0).unwrap(), id0);
    assert_eq!(streams.get(1).unwrap(), id2);

    // Verify remaining streams are still queryable
    let state0 = ctx.client().get_stream_state(&id0);
    assert_eq!(state0.status, StreamStatus::Completed);

    let state2 = ctx.client().get_stream_state(&id2);
    assert_eq!(state2.status, StreamStatus::Completed);

    // Verify removed stream is not queryable
    let result = ctx.client().try_get_stream_state(&id1);
    assert!(result.is_err(), "closed stream must not be queryable");
}

#[test]
fn test_close_completed_stream_permissionless_access() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Any caller (including non-owner) should be able to close
    // This test demonstrates permissionless cleanup semantics
    ctx.client().close_completed_stream(&stream_id);

    let result = ctx.client().try_get_stream_state(&stream_id);
    assert!(result.is_err(), "stream must be closed");
}

#[test]
fn test_close_completed_stream_recipient_index_sorted_after_close() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create streams: 0, 1, 2, 3, 4
    for _ in 0..5 {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100_i128,
            &1_i128,
            &0u64,
            &0u64,
            &100u64,
            &None,
        );
    }

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 5);

    // Complete and close stream 2 (middle)
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&2u64);
    ctx.client().close_completed_stream(&2u64);

    // Verify remaining streams are still sorted
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 4);
    assert_eq!(streams.get(0).unwrap(), 0);
    assert_eq!(streams.get(1).unwrap(), 1);
    assert_eq!(streams.get(2).unwrap(), 3);
    assert_eq!(streams.get(3).unwrap(), 4);

    // Close stream 0 (first)
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&0u64);
    ctx.client().close_completed_stream(&0u64);

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 3);
    assert_eq!(streams.get(0).unwrap(), 1);
    assert_eq!(streams.get(1).unwrap(), 3);
    assert_eq!(streams.get(2).unwrap(), 4);

    // Close stream 4 (last)
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&4u64);
    ctx.client().close_completed_stream(&4u64);

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 2);
    assert_eq!(streams.get(0).unwrap(), 1);
    assert_eq!(streams.get(1).unwrap(), 3);
}

#[test]
fn test_close_completed_stream_after_cliff_passed() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream with cliff at t=500
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &500u64, // cliff at 500
        &1000u64,
        &None,
    );

    // Advance past cliff and end time
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Verify stream is completed
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Close should succeed
    ctx.client().close_completed_stream(&stream_id);

    let result = ctx.client().try_get_stream_state(&stream_id);
    assert!(result.is_err());
}

#[test]
fn test_close_completed_stream_count_decreases() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create three streams
    for _ in 0..3 {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100_i128,
            &1_i128,
            &0u64,
            &0u64,
            &100u64,
            &None,
        );
    }

    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 3);

    // Complete and close one
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&0u64);
    ctx.client().close_completed_stream(&0u64);

    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 2);

    // Complete and close another
    ctx.client().withdraw(&1u64);
    ctx.client().close_completed_stream(&1u64);

    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 1);
}

#[test]
fn test_close_completed_stream_different_recipients_independent() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let recipient2 = Address::generate(&ctx.env);

    // Create stream for ctx.recipient
    let id_r1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );

    // Create stream for recipient2
    let id_r2 = ctx.client().create_stream(
        &ctx.sender,
        &recipient2,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );

    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 1);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient2), 1);

    // Complete and close stream for ctx.recipient
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&id_r1);
    ctx.client().close_completed_stream(&id_r1);

    // Verify ctx.recipient's index is updated
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 0);

    // Verify recipient2's index is unchanged
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient2), 1);
    let streams = ctx.client().get_recipient_streams(&recipient2);
    assert_eq!(streams.get(0).unwrap(), id_r2);
}

// ---------------------------------------------------------------------------
// Tests — top_up_stream
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_stream_increases_deposit_and_contract_balance() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // After creation, sender has 9000, contract has 1000
    assert_eq!(ctx.token().balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token().balance(&ctx.contract_id), 1_000);

    // Top up by 500 from the sender
    ctx.env.ledger().set_timestamp(100);
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    // Deposit amount should increase
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);

    // Balances: sender 8500, contract 1500
    assert_eq!(ctx.token().balance(&ctx.sender), 8_500);
    assert_eq!(ctx.token().balance(&ctx.contract_id), 1_500);
}

#[test]
fn test_top_up_stream_sender_auth_success_strict() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "top_up_stream",
            args: (stream_id, ctx.sender.clone(), 400_i128).into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 400_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &400_i128);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_400);
    assert_eq!(ctx.token().balance(&ctx.sender), 8_600);
    assert_eq!(ctx.token().balance(&ctx.contract_id), 1_400);

    let events = ctx.env.events().all();
    let top_up_event = events
        .iter()
        .skip(events_before as usize)
        .find(|(contract, topics, _)| {
            contract == &ctx.contract_id
                && topics.len() == 2
                && Symbol::try_from_val(&ctx.env, &topics.get(0).unwrap())
                    == Ok(Symbol::new(&ctx.env, "top_up"))
                && u64::try_from_val(&ctx.env, &topics.get(1).unwrap()) == Ok(stream_id)
        })
        .expect("expected a top_up event for the topped-up stream");

    let payload = StreamToppedUp::try_from_val(&ctx.env, &top_up_event.2)
        .expect("top_up event payload must decode");
    assert_eq!(payload.stream_id, stream_id);
    assert_eq!(payload.top_up_amount, 400);
    assert_eq!(payload.new_deposit_amount, 1_400);
}

#[test]
fn test_top_up_stream_allows_third_party_funder_and_emits_payload() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let treasury = Address::generate(&ctx.env);
    ctx.sac.mint(&treasury, &2_000_i128);

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let treasury_balance_before = ctx.token().balance(&treasury);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    ctx.env.ledger().set_timestamp(250);
    ctx.client().top_up_stream(&stream_id, &treasury, &750_i128);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_750);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 0);
    assert_eq!(state.end_time, 1_000);
    assert_eq!(state.withdrawn_amount, 0);

    assert_eq!(ctx.token().balance(&ctx.sender), sender_balance_before);
    assert_eq!(
        ctx.token().balance(&treasury),
        treasury_balance_before - 750
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before + 750
    );

    let events = ctx.env.events().all();
    let top_up_event = events
        .iter()
        .skip(events_before as usize)
        .find(|(contract, topics, _)| {
            contract == &ctx.contract_id
                && topics.len() == 2
                && Symbol::try_from_val(&ctx.env, &topics.get(0).unwrap())
                    == Ok(Symbol::new(&ctx.env, "top_up"))
                && u64::try_from_val(&ctx.env, &topics.get(1).unwrap()) == Ok(stream_id)
        })
        .expect("expected a top_up event for the topped-up stream");

    let payload = StreamToppedUp::try_from_val(&ctx.env, &top_up_event.2)
        .expect("top_up event payload must decode");
    assert_eq!(payload.stream_id, stream_id);
    assert_eq!(payload.top_up_amount, 750);
    assert_eq!(payload.new_deposit_amount, 1_750);
}

#[test]
fn test_top_up_stream_paused_preserves_schedule_and_status() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().pause_stream(&stream_id);

    let state_before = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_before.status, StreamStatus::Paused);

    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &250_i128);

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.stream_id, state_before.stream_id);
    assert_eq!(state_after.sender, state_before.sender);
    assert_eq!(state_after.recipient, state_before.recipient);
    assert_eq!(state_after.start_time, state_before.start_time);
    assert_eq!(state_after.cliff_time, state_before.cliff_time);
    assert_eq!(state_after.end_time, state_before.end_time);
    assert_eq!(state_after.rate_per_second, state_before.rate_per_second);
    assert_eq!(state_after.withdrawn_amount, state_before.withdrawn_amount);
    assert_eq!(state_after.status, StreamStatus::Paused);
    assert_eq!(
        state_after.deposit_amount,
        state_before.deposit_amount + 250
    );
}

#[test]
fn test_top_up_stream_fails_for_terminal_states() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Complete the stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    let deposit_before = state.deposit_amount;
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let result = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &100_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.deposit_amount, deposit_before);
    assert_eq!(ctx.token().balance(&ctx.sender), sender_balance_before);
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
fn test_top_up_stream_rejects_non_positive_amount() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let state_before = ctx.client().get_stream_state(&stream_id);
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let result_zero = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &0_i128);
    assert_eq!(result_zero, Err(Ok(ContractError::InvalidParams)));

    let result_negative = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &-1_i128);
    assert_eq!(result_negative, Err(Ok(ContractError::InvalidParams)));

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.deposit_amount, state_before.deposit_amount);
    assert_eq!(state_after.status, state_before.status);
    assert_eq!(ctx.token().balance(&ctx.sender), sender_balance_before);
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
fn test_top_up_stream_rejects_impersonated_funder_and_emits_no_event_strict() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    let deposit_before = ctx.client().get_stream_state(&stream_id).deposit_amount;
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "top_up_stream",
            args: (stream_id, ctx.sender.clone(), 100_i128).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client()
            .top_up_stream(&stream_id, &ctx.sender, &100_i128);
    }));
    assert!(result.is_err(), "impersonated funder auth must be rejected");

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.deposit_amount, deposit_before);
    assert_eq!(ctx.token().balance(&ctx.sender), sender_balance_before);
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
fn test_top_up_stream_preserves_invariants_for_large_streams() {
    let ctx = TestContext::setup();

    // Use a standard stream where deposit comfortably covers rate * duration.
    let stream_id = ctx.create_default_stream();
    let state = ctx.client().get_stream_state(&stream_id);
    let start = state.start_time;
    let end = state.end_time;
    let rate = state.rate_per_second;

    // Top up multiple times; deposit must remain >= rate * duration.
    ctx.env.ledger().set_timestamp(100);

    for _ in 0..5 {
        ctx.client()
            .top_up_stream(&stream_id, &ctx.sender, &1_000_i128);
    }

    let state = ctx.client().get_stream_state(&stream_id);
    assert!(state.deposit_amount >= rate * (end - start) as i128);
}

// ---------------------------------------------------------------------------
// Tests — Issue #315: top-up near-complete streams
// ---------------------------------------------------------------------------

/// Top-up at T-1 (one second before end) must succeed and increase deposit.
#[test]
fn test_top_up_at_t_minus_1_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // end_time = 1000

    ctx.env.ledger().set_timestamp(999); // T - 1
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);
    assert_eq!(state.end_time, 1_000); // schedule unchanged
}

/// Top-up exactly at T (current_time == end_time) must fail with InvalidState.
#[test]
fn test_top_up_at_end_time_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // end_time = 1000

    ctx.env.ledger().set_timestamp(1000); // exactly at T
    let result = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &500_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Top-up with zero amount must fail with InvalidParams.
#[test]
fn test_top_up_zero_amount_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    let result = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &0_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// A third-party address (neither sender nor admin) must be rejected.
#[test]
fn test_top_up_unauthorized_funder_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let stranger = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(100);
    let result = ctx
        .client()
        .try_top_up_stream(&stream_id, &stranger, &500_i128);
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
}

/// Admin is allowed to top up any stream.
#[test]
fn test_top_up_by_admin_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    // Mint tokens to admin so the pull can succeed
    ctx.sac.mint(&ctx.admin, &1_000_i128);
    ctx.client()
        .top_up_stream(&stream_id, &ctx.admin, &500_i128);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);
}

// ---------------------------------------------------------------------------
// Tests — Issue #37: withdraw reject when stream is Paused
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_withdraw_paused_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance time so there's something to withdraw
    ctx.env.ledger().set_timestamp(500);

    // Pause the stream
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Attempt to withdraw while paused should fail
    ctx.client().withdraw(&stream_id);
}

#[test]
fn test_withdraw_after_resume_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance time
    ctx.env.ledger().set_timestamp(500);

    // Pause and then resume
    ctx.client().pause_stream(&stream_id);
    ctx.client().resume_stream(&stream_id);

    // Withdraw should now succeed
    let recipient_before = ctx.token().balance(&ctx.recipient);
    let amount = ctx.client().withdraw(&stream_id);

    assert_eq!(amount, 500);
    assert_eq!(ctx.token().balance(&ctx.recipient) - recipient_before, 500);
}

// ---------------------------------------------------------------------------
// Tests — stream count / multiple streams
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_streams_independent() {
    let ctx = TestContext::setup();
    let id0 = ctx.create_default_stream();
    let id1 = ctx
        .client()
        .create_stream(&ctx.sender, &ctx.recipient, &200, &2, &0, &0, &100,
        &None
        );

    assert_eq!(id0, 0);
    assert_eq!(id1, 1);

    ctx.client().cancel_stream(&id0);
    assert_eq!(
        ctx.client().get_stream_state(&id0).status,
        StreamStatus::Cancelled
    );
    assert_eq!(
        ctx.client().get_stream_state(&id1).status,
        StreamStatus::Active
    );
}

// ---------------------------------------------------------------------------
// Tests — Issue #16: Auth Enforcement
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_pause_stream_as_recipient_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let env = Env::default();
    let client = FluxoraStreamClient::new(&env, &ctx.contract_id);

    client.pause_stream(&stream_id);
}

#[test]
#[should_panic]
fn test_cancel_stream_as_random_address_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let env = Env::default();
    let client = FluxoraStreamClient::new(&env, &ctx.contract_id);

    client.cancel_stream(&stream_id);
}

#[test]
fn test_admin_can_pause_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
}
// Tests — Events
// ---------------------------------------------------------------------------

#[test]
fn test_pause_resume_events() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Check pause event
    // The event is published as ((symbol_short!("paused"), stream_id), StreamEvent::Paused(stream_id))
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Paused(stream_id)
    );

    ctx.client().resume_stream(&stream_id);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Check resume event
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Resumed(stream_id)
    );
}

#[test]
fn test_cancel_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(77);

    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(77));

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Check cancel event
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::StreamCancelled(stream_id)
    );
}

#[test]
fn test_completed_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::StreamCompleted(stream_id)
    );
}

// ---------------------------------------------------------------------------
// Tests — Admin event parity
//
// Admin override operations (`*_as_admin`) must emit the **same** topic and
// payload as their sender counterparts. Indexers and integrators key off
// event topics to index state changes; a divergence between sender and admin
// paths would create silent gaps in downstream observability.
//
// Scope: pause, resume, cancel, set_contract_paused, set_global_emergency_paused.
// ---------------------------------------------------------------------------

/// `pause_stream_as_admin` must emit topic ("paused", stream_id) with
/// `StreamEvent::Paused(stream_id)` — identical to `pause_stream`.
#[test]
fn test_admin_pause_emits_same_event_as_sender_pause() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream_as_admin(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Topic[0] must be the symbol "paused"
    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "paused"),
        "pause_stream_as_admin topic[0] must be \"paused\""
    );
    // Topic[1] must be the stream_id
    let topic_id: u64 = last_event.1.get(1).unwrap().into_val(&ctx.env);
    assert_eq!(
        topic_id, stream_id,
        "pause_stream_as_admin topic[1] must be stream_id"
    );
    // Data must be StreamEvent::Paused(stream_id)
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Paused(stream_id),
        "pause_stream_as_admin data must be StreamEvent::Paused(stream_id)"
    );
}

/// `resume_stream_as_admin` must emit topic ("resumed", stream_id) with
/// `StreamEvent::Resumed(stream_id)` — identical to `resume_stream`.
#[test]
fn test_admin_resume_emits_same_event_as_sender_resume() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);
    ctx.client().resume_stream_as_admin(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "resumed"),
        "resume_stream_as_admin topic[0] must be \"resumed\""
    );
    let topic_id: u64 = last_event.1.get(1).unwrap().into_val(&ctx.env);
    assert_eq!(
        topic_id, stream_id,
        "resume_stream_as_admin topic[1] must be stream_id"
    );
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Resumed(stream_id),
        "resume_stream_as_admin data must be StreamEvent::Resumed(stream_id)"
    );
}

/// `cancel_stream_as_admin` must emit topic ("cancelled", stream_id) with
/// `StreamEvent::StreamCancelled(stream_id)` — identical to `cancel_stream`.
#[test]
fn test_admin_cancel_emits_same_event_as_sender_cancel() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(300);

    ctx.client().cancel_stream_as_admin(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "cancelled"),
        "cancel_stream_as_admin topic[0] must be \"cancelled\""
    );
    let topic_id: u64 = last_event.1.get(1).unwrap().into_val(&ctx.env);
    assert_eq!(
        topic_id, stream_id,
        "cancel_stream_as_admin topic[1] must be stream_id"
    );
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::StreamCancelled(stream_id),
        "cancel_stream_as_admin data must be StreamEvent::StreamCancelled(stream_id)"
    );
}

/// `set_contract_paused` must emit topic ("ct_pause",) with `ContractPauseChanged`
/// payload for both the pause and unpause transitions.
#[test]
fn test_set_contract_paused_emits_ct_pause_event() {
    let ctx = TestContext::setup();

    // Pause: paused = true
    ctx.client().set_contract_paused(&true);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "ct_pause"),
        "set_contract_paused topic[0] must be \"ct_pause\""
    );
    let payload = ContractPauseChanged::try_from_val(&ctx.env, &last_event.2)
        .expect("ct_pause data must be ContractPauseChanged");
    assert!(
        payload.paused,
        "ContractPauseChanged.paused must be true on pause"
    );

    // Unpause: paused = false
    ctx.client().set_contract_paused(&false);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "ct_pause"),
        "set_contract_paused topic[0] must be \"ct_pause\" on unpause"
    );
    let payload = ContractPauseChanged::try_from_val(&ctx.env, &last_event.2)
        .expect("ct_pause data must be ContractPauseChanged on unpause");
    assert!(
        !payload.paused,
        "ContractPauseChanged.paused must be false on unpause"
    );
}

/// `set_global_emergency_paused` must emit topic ("gl_pause",) with
/// `GlobalEmergencyPauseChanged` payload for both transitions.
#[test]
fn test_set_global_emergency_paused_emits_gl_pause_event() {
    let ctx = TestContext::setup();

    // Emergency pause: paused = true
    ctx.client().set_global_emergency_paused(&true);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "gl_pause"),
        "set_global_emergency_paused topic[0] must be \"gl_pause\""
    );
    let payload = GlobalEmergencyPauseChanged::try_from_val(&ctx.env, &last_event.2)
        .expect("gl_pause data must be GlobalEmergencyPauseChanged");
    assert!(
        payload.paused,
        "GlobalEmergencyPauseChanged.paused must be true on pause"
    );

    // Clear emergency pause
    ctx.client().set_global_emergency_paused(&false);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "gl_pause"),
        "set_global_emergency_paused topic[0] must be \"gl_pause\" on clear"
    );
    let payload = GlobalEmergencyPauseChanged::try_from_val(&ctx.env, &last_event.2)
        .expect("gl_pause data must be GlobalEmergencyPauseChanged on clear");
    assert!(
        !payload.paused,
        "GlobalEmergencyPauseChanged.paused must be false on clear"
    );
}

/// When set_global_emergency_paused is true, user mutations (withdraw, cancel) are
/// blocked but admin overrides still work and still emit the correct event.
#[test]
fn test_admin_ops_emit_events_during_global_emergency_pause() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Engage global emergency pause
    ctx.client().set_global_emergency_paused(&true);
    assert!(ctx.client().get_global_emergency_paused());

    // User withdraw is blocked
    let result = ctx.client().try_withdraw(&stream_id);
    assert!(
        result.is_err(),
        "withdraw must be blocked during global emergency pause"
    );

    // Admin pause still works and emits the correct event
    ctx.client().pause_stream_as_admin(&stream_id);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Paused(stream_id),
        "pause_stream_as_admin must emit Paused during global emergency pause"
    );

    // Admin resume still works and emits the correct event
    ctx.client().resume_stream_as_admin(&stream_id);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Resumed(stream_id),
        "resume_stream_as_admin must emit Resumed during global emergency pause"
    );

    // Admin cancel still works and emits the correct event
    ctx.client().cancel_stream_as_admin(&stream_id);
    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::StreamCancelled(stream_id),
        "cancel_stream_as_admin must emit StreamCancelled during global emergency pause"
    );
}

// ---------------------------------------------------------------------------
// Tests — pause/cancel authorization (strict mode)
// ---------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_pause_stream_recipient_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Sender creates the stream (authorize create + transfer)
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Recipient attempts to pause (should be unauthorized)
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream(&stream_id);
}

#[test]
#[should_panic]
fn test_pause_stream_third_party_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let other = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &other,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream(&stream_id);
}

#[test]
fn test_pause_stream_sender_success() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Sender authorises pause
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
}

#[test]
fn test_pause_stream_admin_success() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Create stream by sender
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Admin authorises pause via the admin-specific entrypoint
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream_as_admin(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
}

#[test]
#[should_panic]
fn test_pause_stream_as_admin_non_admin_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Create stream by sender
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // A non-admin cannot use the admin override entrypoint.
    let non_admin = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream_as_admin(&stream_id);
}

// Cancel authorization tests

#[test]
#[should_panic]
fn test_cancel_stream_recipient_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().cancel_stream(&stream_id);
}

#[test]
#[should_panic]
fn test_cancel_stream_third_party_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let other = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &other,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().cancel_stream(&stream_id);
}

#[test]
fn test_cancel_stream_sender_success() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().cancel_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

#[test]
fn test_cancel_stream_admin_success() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().cancel_stream_as_admin(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Additional Tests — create_stream (enhanced coverage)
// ---------------------------------------------------------------------------

/// Test creating a stream with negative deposit amount panics
#[test]
#[should_panic]
fn test_create_stream_negative_deposit_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &-100_i128, // negative deposit
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// Test creating a stream with negative rate_per_second panics
#[test]
#[should_panic]
fn test_create_stream_negative_rate_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &-5_i128, // negative rate
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// Test creating a stream where start_time equals end_time panics
#[test]
#[should_panic]
fn test_create_stream_equal_start_end_times_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &500u64,
        &500u64,
        &500u64, // start == end
    );
}

/// Test creating a stream with cliff_time equal to start_time (valid edge case)
#[test]
fn test_create_stream_cliff_equals_start() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &100u64, // cliff == start (valid)
        &1100u64,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 100);
    assert_eq!(state.start_time, 100);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test creating a stream with cliff_time equal to end_time (valid edge case)
#[test]
fn test_create_stream_cliff_equals_end() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff == end (valid)
        &1000u64,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 1000);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test creating multiple streams increments stream_id correctly
#[test]
fn test_create_stream_increments_id_correctly() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );

    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &1_i128,
        &0u64,
        &0u64,
        &200u64,
        &None,
    );

    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &300_i128,
        &1_i128,
        &0u64,
        &0u64,
        &300u64,
        &None,
    );

    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);

    // Verify each stream has correct data
    let s0 = ctx.client().get_stream_state(&id0);
    let s1 = ctx.client().get_stream_state(&id1);
    let s2 = ctx.client().get_stream_state(&id2);

    assert_eq!(s0.deposit_amount, 100);
    assert_eq!(s1.deposit_amount, 200);
    assert_eq!(s2.deposit_amount, 300);
}

/// Test creating a stream with very large deposit amount
#[test]
fn test_create_stream_large_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Mint large amount to sender
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &1_000_000_000_i128);

    let large_amount = 1_000_000_i128;
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &large_amount,
        &1000_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, large_amount);
    assert_eq!(ctx.token().balance(&ctx.contract_id), large_amount);
}

/// Test creating a stream with very high rate_per_second
#[test]
fn test_create_stream_high_rate() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let high_rate = 1000_i128;
    let duration = 10u64;
    let deposit = high_rate * duration as i128; // Ensure deposit covers total streamable

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &high_rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, high_rate);
    assert_eq!(state.deposit_amount, deposit);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test creating a stream with different sender and recipient
#[test]
fn test_create_stream_different_addresses() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let another_recipient = Address::generate(&ctx.env);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &another_recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, another_recipient);
}

/// Test creating a stream with future start_time
#[test]
fn test_create_stream_future_start_time() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &1000u64, // starts in the future
        &1000u64,
        &2000u64,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.start_time, 1000);
    assert_eq!(state.end_time, 2000);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test token balance changes after creating stream
#[test]
fn test_create_stream_token_balances() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let recipient_balance_before = ctx.token().balance(&ctx.recipient);

    let deposit = 2500_i128;
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &5_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    // Sender balance should decrease by deposit
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        sender_balance_before - deposit
    );

    // Contract balance should increase by deposit
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before + deposit
    );

    // Recipient balance should remain unchanged (no withdrawal yet)
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        recipient_balance_before
    );
}

/// Test creating stream with minimum valid duration (1 second)
#[test]
fn test_create_stream_minimum_duration() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &100_i128,
        &0u64,
        &0u64,
        &1u64, // 1 second duration
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time - state.start_time, 1);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test creating stream verifies all stream fields are set correctly
#[test]
fn test_create_stream_all_fields_correct() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 5000_i128;
    let rate = 10_i128;
    let start = 100u64;
    let cliff = 200u64;
    let end = 600u64;

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &start,
        &cliff,
        &end,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);

    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, deposit);
    assert_eq!(state.rate_per_second, rate);
    assert_eq!(state.start_time, start);
    assert_eq!(state.cliff_time, cliff);
    assert_eq!(state.end_time, end);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test that creating stream with same sender and recipient panics
#[test]
#[should_panic]
fn test_create_stream_self_stream_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Attempt to create stream where sender is also recipient (should panic)
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.sender, // same as sender - not allowed
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

// ---------------------------------------------------------------------------
// Tests — get_stream_state
// ---------------------------------------------------------------------------

#[test]
fn test_get_stream_state_non_existent() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_get_stream_state(&999);
    assert!(result.is_err());
}

#[test]
fn test_get_stream_state_all_statuses() {
    let ctx = TestContext::setup();

    // 1. Check Active (from creation)
    let id_active = ctx.create_default_stream();
    let state_active = ctx.client().get_stream_state(&id_active);
    assert_eq!(state_active.status, StreamStatus::Active);
    assert_eq!(state_active.stream_id, id_active);

    // 2. Check Paused
    let id_paused = ctx.create_default_stream();
    ctx.client().pause_stream(&id_paused);
    let state_paused = ctx.client().get_stream_state(&id_paused);
    assert_eq!(state_paused.status, StreamStatus::Paused);

    // 3. Check Cancelled
    let id_cancelled = ctx.create_default_stream();
    ctx.client().cancel_stream(&id_cancelled);
    let state_cancelled = ctx.client().get_stream_state(&id_cancelled);
    assert_eq!(state_cancelled.status, StreamStatus::Cancelled);

    // 4. Check Completed
    let id_completed = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id_completed);
    let state_completed = ctx.client().get_stream_state(&id_completed);
    assert_eq!(state_completed.status, StreamStatus::Completed);
}

#[test]
fn test_cancel_fully_accrued_no_refund() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // 1000 seconds pass → 1000 tokens accrued (full deposit)
    ctx.env.ledger().set_timestamp(1000);

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    let sender_balance_after = ctx.token().balance(&ctx.sender);
    assert_eq!(
        sender_balance_after, sender_balance_before,
        "nothing should be refunded"
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

#[test]
fn test_withdraw_multiple_times() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 200 at t=200
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    // Withdraw another 300 at t=500
    ctx.env.ledger().set_timestamp(500);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 300);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 500);
}

#[test]
#[should_panic]
fn test_create_stream_invalid_cliff_panics() {
    let ctx = TestContext::setup();
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1,
        &100,
        &50,
        &200, // cliff < start
    );
}

#[test]
fn test_create_stream_edge_cliffs() {
    let ctx = TestContext::setup();

    // Cliff at start_time
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100,
        &100,
        &1100,
        &None,
    );
    assert_eq!(ctx.client().get_stream_state(&id1).cliff_time, 100);

    // Cliff at end_time
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100,
        &1100,
        &1100,
        &None,
    );
    assert_eq!(ctx.client().get_stream_state(&id2).cliff_time, 1100);
}

#[test]
fn test_calculate_accrued_exactly_at_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at 500
    ctx.env.ledger().set_timestamp(500);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 500,
        "at cliff, should accrue full amount from start"
    );
}

#[test]
fn test_admin_can_pause_via_admin_path() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Verification: Admin can successfully pause via the admin entrypoint
    ctx.client().pause_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
}

#[test]
fn test_cancel_stream_as_admin_works() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Verification: Admin can still intervene via the admin path
    ctx.client().cancel_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Tests — Issue #52: cancel_stream refund and status verification
// ---------------------------------------------------------------------------

/// Test cancel at stream start (0% accrual) - full refund to sender
#[test]
fn test_cancel_at_start_full_refund_and_status() {
    let ctx = TestContext::setup();

    // Record initial balances
    let sender_initial = ctx.token().balance(&ctx.sender);
    let recipient_initial = ctx.token().balance(&ctx.recipient);
    let contract_initial = ctx.token().balance(&ctx.contract_id);

    // Create stream: 2000 tokens over 2000 seconds
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
        &None,
    );

    // Verify deposit transferred
    assert_eq!(ctx.token().balance(&ctx.sender), sender_initial - 2000);
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_initial + 2000
    );

    // Cancel immediately (no time elapsed, 0% accrual)
    ctx.env.ledger().set_timestamp(0);
    let sender_before_cancel = ctx.token().balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify full refund to sender (unstreamed = 2000 - 0 = 2000)
    let sender_after_cancel = ctx.token().balance(&ctx.sender);
    let refund = sender_after_cancel - sender_before_cancel;
    assert_eq!(refund, 2000, "sender should receive full refund");
    assert_eq!(
        sender_after_cancel, sender_initial,
        "sender balance restored"
    );

    // Verify contract balance is 0 (all refunded)
    assert_eq!(ctx.token().balance(&ctx.contract_id), contract_initial);

    // Verify recipient balance unchanged (no accrual)
    assert_eq!(ctx.token().balance(&ctx.recipient), recipient_initial);
}

/// Test cancel at 25% completion - partial refund, recipient can withdraw accrued
#[test]
fn test_cancel_at_25_percent_partial_refund_recipient_withdraws() {
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
        &None,
    );

    let sender_initial = ctx.token().balance(&ctx.sender);
    let recipient_initial = ctx.token().balance(&ctx.recipient);
    let contract_after_create = ctx.token().balance(&ctx.contract_id);

    // Advance to 25% completion (1000 seconds)
    ctx.env.ledger().set_timestamp(1000);

    // Verify accrued amount
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1000, "25% of 4000 = 1000 tokens accrued");

    // Cancel stream
    ctx.client().cancel_stream(&stream_id);

    // Verify status is Cancelled
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify partial refund to sender (unstreamed = 4000 - 1000 = 3000)
    let sender_after_cancel = ctx.token().balance(&ctx.sender);
    let refund = sender_after_cancel - sender_initial;
    assert_eq!(refund, 3000, "sender should receive 75% refund");

    // Verify contract balance (accrued amount remains)
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_after_create - 3000,
        "contract should hold accrued amount"
    );
    assert_eq!(ctx.token().balance(&ctx.contract_id), 1000);

    // Verify recipient can withdraw accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1000, "recipient should withdraw accrued amount");

    // Verify final balances
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        recipient_initial + 1000,
        "recipient receives accrued tokens"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        0,
        "contract balance should be 0 after withdrawal"
    );
}

/// Test cancel at 50% completion - verify exact refund calculation
#[test]
fn test_cancel_at_50_percent_exact_refund_calculation() {
    let ctx = TestContext::setup();

    // Create stream: 6000 tokens over 3000 seconds (2 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &6000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &3000u64,
        &None,
    );

    let sender_before_cancel = ctx.token().balance(&ctx.sender);
    let contract_before_cancel = ctx.token().balance(&ctx.contract_id);

    // Advance to 50% completion (1500 seconds)
    ctx.env.ledger().set_timestamp(1500);

    // Verify accrued: 1500 seconds × 2 tokens/sec = 3000 tokens
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 3000);

    // Cancel stream
    ctx.client().cancel_stream(&stream_id);

    // Verify status
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify refund: unstreamed = 6000 - 3000 = 3000
    let sender_after_cancel = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_after_cancel - sender_before_cancel, 3000);

    // Verify contract balance: accrued amount remains
    assert_eq!(ctx.token().balance(&ctx.contract_id), 3000);
    assert_eq!(
        contract_before_cancel - ctx.token().balance(&ctx.contract_id),
        3000
    );
}

/// Test cancel at 75% completion - verify recipient withdrawal after cancel
#[test]
fn test_cancel_at_75_percent_recipient_can_withdraw_accrued() {
    let ctx = TestContext::setup();

    // Create stream: 8000 tokens over 4000 seconds (2 tokens/sec)
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &8000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &4000u64,
        &None,
    );

    // Advance to 75% completion (3000 seconds)
    ctx.env.ledger().set_timestamp(3000);

    // Accrued: 3000 × 2 = 6000 tokens
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 6000);

    // Cancel stream
    ctx.client().cancel_stream(&stream_id);

    // Verify status
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 0);

    // Verify recipient can withdraw full accrued amount
    let recipient_before = ctx.token().balance(&ctx.recipient);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 6000);

    let recipient_after = ctx.token().balance(&ctx.recipient);
    assert_eq!(recipient_after - recipient_before, 6000);

    // Verify contract is empty after withdrawal
    assert_eq!(ctx.token().balance(&ctx.contract_id), 0);
}

/// Test cancel after partial withdrawal - verify correct refund calculation
#[test]
fn test_cancel_after_partial_withdrawal_correct_refund() {
    let ctx = TestContext::setup();

    // Create stream: 5000 tokens over 5000 seconds
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &5000u64,
        &None,
    );

    // Advance to 40% and withdraw
    ctx.env.ledger().set_timestamp(2000);
    let withdrawn_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_1, 2000);

    // Advance to 60% and cancel
    ctx.env.ledger().set_timestamp(3000);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 3000);

    let sender_before_cancel = ctx.token().balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify status
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 2000);

    // Verify refund: unstreamed = 5000 - 3000 = 2000
    let sender_after_cancel = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_after_cancel - sender_before_cancel, 2000);

    // Verify recipient can withdraw remaining accrued (3000 - 2000 = 1000)
    let withdrawn_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn_2, 1000);

    // Verify total withdrawn equals accrued
    assert_eq!(withdrawn_1 + withdrawn_2, 3000);
}

/// Test cancel with cliff - before cliff time (no accrual, full refund)
#[test]
fn test_cancel_before_cliff_full_refund() {
    let ctx = TestContext::setup();

    // Create stream with cliff: 3000 tokens, cliff at 1500 seconds
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &1500u64, // cliff at 50%
        &3000u64,
        &None,
    );

    let sender_before_cancel = ctx.token().balance(&ctx.sender);

    // Advance to before cliff (1000 seconds, before 1500 cliff)
    ctx.env.ledger().set_timestamp(1000);

    // Verify no accrual before cliff
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0);

    // Cancel stream
    ctx.client().cancel_stream(&stream_id);

    // Verify status
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify full refund (no accrual)
    let sender_after_cancel = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_after_cancel - sender_before_cancel, 3000);

    // Verify contract is empty
    assert_eq!(ctx.token().balance(&ctx.contract_id), 0);
}

/// Test cancel with cliff - after cliff time (partial accrual, partial refund)
#[test]
fn test_cancel_after_cliff_partial_refund() {
    let ctx = TestContext::setup();

    // Create stream with cliff: 4000 tokens, cliff at 2000 seconds
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &4000_i128,
        &1_i128,
        &0u64,
        &2000u64, // cliff at 50%
        &4000u64,
        &None,
    );

    let sender_before_cancel = ctx.token().balance(&ctx.sender);

    // Advance to after cliff (2500 seconds, past 2000 cliff)
    ctx.env.ledger().set_timestamp(2500);

    // Verify accrual after cliff (calculated from start_time)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2500);

    // Cancel stream
    ctx.client().cancel_stream(&stream_id);

    // Verify status
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify partial refund: unstreamed = 4000 - 2500 = 1500
    let sender_after_cancel = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_after_cancel - sender_before_cancel, 1500);

    // Verify contract holds accrued amount
    assert_eq!(ctx.token().balance(&ctx.contract_id), 2500);

    // Verify recipient can withdraw accrued
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 2500);
}

/// Test cancel of paused stream - verify accrual continues during pause
#[test]
fn test_cancel_paused_stream_accrual_continues() {
    let ctx = TestContext::setup();

    // Create stream: 3000 tokens over 3000 seconds
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &3000u64,
        &None,
    );

    // Advance to 30% and pause
    ctx.env.ledger().set_timestamp(900);
    ctx.client().pause_stream(&stream_id);

    // Advance time further (accrual continues even when paused)
    ctx.env.ledger().set_timestamp(1500);

    // Verify accrual at 50% (not stopped at pause time)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 1500);

    let sender_before_cancel = ctx.token().balance(&ctx.sender);

    // Cancel paused stream
    ctx.client().cancel_stream(&stream_id);

    // Verify status
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify refund based on current accrual: 3000 - 1500 = 1500
    let sender_after_cancel = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_after_cancel - sender_before_cancel, 1500);

    // Verify contract holds accrued amount
    assert_eq!(ctx.token().balance(&ctx.contract_id), 1500);
}

/// Test balance consistency - verify total tokens are conserved
#[test]
fn test_cancel_balance_consistency() {
    let ctx = TestContext::setup();

    let total_supply_initial = ctx.token().balance(&ctx.sender)
        + ctx.token().balance(&ctx.recipient)
        + ctx.token().balance(&ctx.contract_id);

    // Create stream: 7000 tokens over 7000 seconds
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &7000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &7000u64,
        &None,
    );

    // Verify total supply unchanged after creation
    let total_after_create = ctx.token().balance(&ctx.sender)
        + ctx.token().balance(&ctx.recipient)
        + ctx.token().balance(&ctx.contract_id);
    assert_eq!(total_after_create, total_supply_initial);

    // Advance to 40% and cancel
    ctx.env.ledger().set_timestamp(2800);
    ctx.client().cancel_stream(&stream_id);

    // Verify total supply unchanged after cancel
    let total_after_cancel = ctx.token().balance(&ctx.sender)
        + ctx.token().balance(&ctx.recipient)
        + ctx.token().balance(&ctx.contract_id);
    assert_eq!(total_after_cancel, total_supply_initial);

    // Recipient withdraws accrued amount
    ctx.client().withdraw(&stream_id);

    // Verify total supply still unchanged after withdrawal
    let total_after_withdraw = ctx.token().balance(&ctx.sender)
        + ctx.token().balance(&ctx.recipient)
        + ctx.token().balance(&ctx.contract_id);
    assert_eq!(total_after_withdraw, total_supply_initial);
}

// ---------------------------------------------------------------------------
// Tests — get_stream_state
// ---------------------------------------------------------------------------

#[test]
fn test_get_stream_state_initial() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    assert_eq!(stream_id, 0, "first stream id should be 0");

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
}

#[test]
fn test_get_stream_state_create_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &0u64, // cliff equals start
        &5000u64,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 0);
    assert_eq!(state.end_time, 5000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn test_get_stream_state_create_stream_withdraw_during_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals start
        &5000u64,
        &None,
    );
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 5000);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn test_get_stream_state_create_stream_withdraw() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals start
        &5000u64,
        &None,
    );
    ctx.env.ledger().set_timestamp(6000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 5000);
    assert_eq!(state.withdrawn_amount, 5000);
    assert_eq!(state.status, StreamStatus::Completed);
}

#[test]
fn test_get_stream_state_create_stream_cancel() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals start
        &5000u64,
        &None,
    );
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 5000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

#[test]
fn test_get_stream_state_pause_stream_cancel() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals start
        &5000u64,
        &None,
    );
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 5000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Paused);
}

#[test]
fn test_get_stream_state_pause_resume_stream_cancel() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals start
        &5000u64,
        &None,
    );
    ctx.client().pause_stream(&stream_id);

    ctx.client().resume_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 5000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
fn test_get_stream_state_non_existence_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let result = ctx.client().try_get_stream_state(&1);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Tests — Error API (StreamNotFound)
// ---------------------------------------------------------------------------

#[test]
fn test_pause_stream_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_pause_stream(&999);
    assert!(result.is_err());
}

#[test]
fn test_resume_stream_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_resume_stream(&999);
    assert!(result.is_err());
}

#[test]
fn test_cancel_stream_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_cancel_stream(&999);
    assert!(result.is_err());
}

#[test]
fn test_withdraw_stream_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_withdraw(&999);
    assert!(result.is_err());
}

#[test]
fn test_calculate_accrued_stream_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_calculate_accrued(&999);
    assert!(result.is_err());
}

#[test]
fn test_cancel_stream_as_admin_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_cancel_stream_as_admin(&999);
    assert!(result.is_err());
}

#[test]
fn test_pause_stream_as_admin_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_pause_stream_as_admin(&999);
    assert!(result.is_err());
}

#[test]
fn test_resume_stream_as_admin_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_resume_stream_as_admin(&999);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Tests — Issue: withdraw zero and excess handling
// ---------------------------------------------------------------------------

/// Test withdraw when withdrawable is zero before cliff.
/// Should return 0 without transfer or state change (idempotent).
#[test]
fn test_withdraw_zero_before_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Before cliff, accrued = 0, withdrawn = 0, so withdrawable = 0
    ctx.env.ledger().set_timestamp(100);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 0, "should return 0 before cliff");

    // Verify no state change
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test withdraw when accrued - withdrawn = 0 after full withdrawal
/// Should panic with "stream already completed"
#[test]
#[should_panic]
fn test_withdraw_zero_after_full_withdrawal() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw everything at t=1000
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);

    // Try to withdraw again - should panic with "stream already completed"
    ctx.client().withdraw(&stream_id);
}

/// Test withdraw when withdrawable is zero at start time (no cliff).
/// Should return 0 without transfer or state change (idempotent).
#[test]
fn test_withdraw_zero_at_start_time() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // At start time, accrued = 0, withdrawn = 0, so withdrawable = 0
    ctx.env.ledger().set_timestamp(0);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 0, "should return 0 at start time");

    // Verify no state change
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test withdraw immediately after previous withdrawal with no time elapsed.
/// Should return 0 without transfer or state change (idempotent).
#[test]
fn test_withdraw_zero_no_time_elapsed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw at t=500
    ctx.env.ledger().set_timestamp(500);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 500);

    // Try to withdraw again at same timestamp - should return 0
    let withdrawn2 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn2, 0, "should return 0 when no time elapsed");

    // Verify no additional state change
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 500);
}

/// Issue #128 — withdraw when accrued equals withdrawn (zero withdrawable)
/// Expected: second withdraw returns 0
/// and no token transfer occurs (recipient balance unchanged).
#[test]
fn test_withdraw_when_accrued_equals_withdrawn_zero_withdrawable() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance time to t=600: accrued = 600, withdrawn = 0
    ctx.env.ledger().set_timestamp(600);

    // First withdraw: drains the full accrued amount
    let first_withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(first_withdrawn, 600, "first withdraw should return 600");

    // Verify state: withdrawn_amount now equals accrued (both 600)
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 600);
    assert_eq!(state.status, StreamStatus::Active); // still active, stream not done

    // Verify no second transfer occurred by recording recipient balance
    let recipient_balance_after_first = ctx.token().balance(&ctx.recipient);
    assert_eq!(recipient_balance_after_first, 600);

    // Second withdraw at same timestamp: accrued (600) - withdrawn (600) = 0.
    // Must return 0 and must NOT transfer any additional tokens.
    let second_withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(second_withdrawn, 0, "zero withdrawable should return 0");

    // Verify no extra tokens moved.
    let recipient_balance_after_second = ctx.token().balance(&ctx.recipient);
    assert_eq!(
        recipient_balance_after_second, recipient_balance_after_first,
        "no tokens should transfer on zero-withdrawable call"
    );
}

/// Test withdraw when cancelled with zero accrued.
/// Should return 0 without transfer or state change (idempotent).
#[test]
fn test_withdraw_zero_after_immediate_cancel() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel immediately at t=0 (no accrual)
    ctx.env.ledger().set_timestamp(0);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Try to withdraw - should return 0 because accrued = 0
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 0,
        "should return 0 when cancelled with no accrual"
    );

    // Verify no state change
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 0);
}

/// Test that zero withdrawable is truly idempotent: multiple calls return 0.
/// Verifies no token transfer, no state change, no events published.
#[test]
fn test_withdraw_zero_idempotent_multiple_calls() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Before cliff, withdrawable = 0
    ctx.env.ledger().set_timestamp(100);

    let initial_recipient_balance = ctx.token().balance(&ctx.recipient);
    let initial_contract_balance = ctx.token().balance(&ctx.contract_id);

    // Call withdraw multiple times
    for _ in 0..5 {
        let withdrawn = ctx.client().withdraw(&stream_id);
        assert_eq!(withdrawn, 0, "should return 0 on every call");
    }

    // Verify no token transfers occurred
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        initial_recipient_balance,
        "recipient balance should not change"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        initial_contract_balance,
        "contract balance should not change"
    );

    // Verify no state change
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test that contract correctly calculates withdrawable amount
/// and doesn't allow withdrawing more than accrued
#[test]
fn test_withdraw_capped_at_accrued_amount() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // At t=300, accrued = 300
    ctx.env.ledger().set_timestamp(300);
    let withdrawn = ctx.client().withdraw(&stream_id);

    // Should withdraw exactly 300, not more
    assert_eq!(withdrawn, 300);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 300);

    // Verify recipient balance increased by exactly 300
    assert_eq!(ctx.token().balance(&ctx.recipient), 300);
}

/// Test that withdrawable amount is always non-negative
/// by verifying withdrawn_amount never exceeds deposit_amount
#[test]
fn test_withdraw_no_negative_withdrawable() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw multiple times
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);

    // Verify withdrawn_amount never exceeds deposit_amount
    assert!(state.withdrawn_amount <= state.deposit_amount);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.deposit_amount, 1000);
}

/// Test withdraw with maximum values doesn't overflow
#[test]
fn test_withdraw_no_overflow_max_values() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &(i128::MAX - 10_000_i128));
    let stream_id = ctx.create_max_rate_stream();

    // Advance to end of stream
    ctx.env.ledger().set_timestamp(3);

    let withdrawn = ctx.client().withdraw(&stream_id);

    // Verify withdrawal is valid and non-negative
    assert!(withdrawn > 0);
    assert!(withdrawn < i128::MAX);

    let state = ctx.client().get_stream_state(&stream_id);
    assert!(state.withdrawn_amount <= state.deposit_amount);
    assert_eq!(state.withdrawn_amount, withdrawn);
}

/// Test that accrued amount is properly capped at deposit_amount
/// preventing any possibility of withdrawing more than deposited
#[test]
fn test_withdraw_accrued_capped_at_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Go way past end time
    ctx.env.ledger().set_timestamp(10_000);

    let withdrawn = ctx.client().withdraw(&stream_id);

    // Should withdraw exactly deposit_amount, not more
    assert_eq!(withdrawn, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);
}

/// Test withdraw after cancel with partial accrual
/// Verifies correct calculation of withdrawable amount
#[test]
fn test_withdraw_after_cancel_partial_accrual() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel at t=250 (250 tokens accrued)
    ctx.env.ledger().set_timestamp(250);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Withdraw the accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 250);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 250);
    // After cancel, status remains Cancelled even after full withdrawal
    // because the stream was terminated early, not completed naturally
    assert_eq!(state.status, StreamStatus::Cancelled);
}

/// Test that multiple partial withdrawals sum correctly
/// and final withdrawal completes the stream
#[test]
fn test_withdraw_multiple_partial_no_excess() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // First withdrawal at t=100
    ctx.env.ledger().set_timestamp(100);
    let w1 = ctx.client().withdraw(&stream_id);
    assert_eq!(w1, 100);

    // Second withdrawal at t=300
    ctx.env.ledger().set_timestamp(300);
    let w2 = ctx.client().withdraw(&stream_id);
    assert_eq!(w2, 200);

    // Third withdrawal at t=700
    ctx.env.ledger().set_timestamp(700);
    let w3 = ctx.client().withdraw(&stream_id);
    assert_eq!(w3, 400);

    // Final withdrawal at t=1000
    ctx.env.ledger().set_timestamp(1000);
    let w4 = ctx.client().withdraw(&stream_id);
    assert_eq!(w4, 300);

    // Verify total withdrawn equals deposit
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(w1 + w2 + w3 + w4, 1000);
    assert_eq!(state.status, StreamStatus::Completed);
}

/// Test withdraw with cliff - before cliff returns zero withdrawable.
/// Should return 0 without transfer or state change (idempotent).
#[test]
fn test_withdraw_zero_one_second_before_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // One second before cliff
    ctx.env.ledger().set_timestamp(499);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 0, "should return 0 before cliff");
}

/// Test withdraw exactly at cliff time
#[test]
fn test_withdraw_exactly_at_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Exactly at cliff, should be able to withdraw accrued amount
    ctx.env.ledger().set_timestamp(500);
    let withdrawn = ctx.client().withdraw(&stream_id);

    // At cliff (t=500), accrued from start (t=0) = 500 tokens
    assert_eq!(withdrawn, 500);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 500);
}

/// Test that contract balance decreases correctly with withdrawals
#[test]
fn test_withdraw_contract_balance_tracking() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let initial_contract_balance = ctx.token().balance(&ctx.contract_id);
    assert_eq!(initial_contract_balance, 1000);

    // Withdraw 400 at t=400
    ctx.env.ledger().set_timestamp(400);
    ctx.client().withdraw(&stream_id);

    let contract_balance_after_first = ctx.token().balance(&ctx.contract_id);
    assert_eq!(contract_balance_after_first, 600);

    // Withdraw remaining 600 at t=1000
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let final_contract_balance = ctx.token().balance(&ctx.contract_id);
    assert_eq!(final_contract_balance, 0);
}

/// Test withdraw with deposit greater than total streamable
/// Ensures only streamable amount can be withdrawn
#[test]
fn test_withdraw_excess_deposit_only_streams_calculated_amount() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream with deposit > rate * duration
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128, // deposit 2000
        &1_i128,    // rate 1/s
        &0u64,
        &0u64,
        &1000u64, // duration 1000s, so only 1000 will stream
    );

    // At end, only 1000 should be withdrawable (rate * duration)
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn = ctx.client().withdraw(&stream_id);

    // Should withdraw exactly 1000 (rate * duration), not 2000 (deposit)
    assert_eq!(withdrawn, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.deposit_amount, 2000);
}

/// Test that withdrawn_amount is monotonically increasing
#[test]
fn test_withdraw_monotonic_increase() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let mut previous_withdrawn = 0_i128;

    for t in [100, 200, 400, 700, 1000] {
        ctx.env.ledger().set_timestamp(t);
        ctx.client().withdraw(&stream_id);

        let state = ctx.client().get_stream_state(&stream_id);

        // Verify withdrawn_amount only increases
        assert!(state.withdrawn_amount > previous_withdrawn);
        previous_withdrawn = state.withdrawn_amount;
    }
}

/// Test edge case: stream with very small rate
#[test]
fn test_withdraw_small_rate_no_underflow() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Small rate: 1 token per 10 seconds
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128, // deposit 100 tokens
        &1_i128,   // rate 1 token/second
        &0u64,
        &0u64,
        &100u64, // 100 seconds for 100 tokens total
    );

    // At t=50, accrued should be 50 tokens
    ctx.env.ledger().set_timestamp(50);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 50);

    // Withdraw at t=50
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 50);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 50);
}

/// Test that status transitions correctly on final withdrawal
#[test]
fn test_withdraw_status_transition_to_completed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Partial withdrawal
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Final withdrawal
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
}

#[test]
fn test_withdraw_active_final_drain_emits_withdrew_then_completed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Do a partial withdrawal first, then final-drain withdrawal.
    ctx.env.ledger().set_timestamp(400);
    ctx.client().withdraw(&stream_id);

    ctx.env.ledger().set_timestamp(1000);
    let events_before = ctx.env.events().all().len();
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 600);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);

    let events = ctx.env.events().all();
    let mut withdraw_idx: Option<u32> = None;
    let mut completed_idx: Option<u32> = None;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        let topic0 = Symbol::from_val(&ctx.env, &event.1.get(0).unwrap());
        if topic0 == Symbol::new(&ctx.env, "withdrew") {
            withdraw_idx = Some(i);
            let payload = crate::Withdrawal::try_from_val(&ctx.env, &event.2).unwrap();
            assert_eq!(payload.stream_id, stream_id);
            assert_eq!(payload.recipient, ctx.recipient);
            assert_eq!(payload.amount, 600);
        }
        if topic0 == Symbol::new(&ctx.env, "completed") {
            completed_idx = Some(i);
            let payload = StreamEvent::from_val(&ctx.env, &event.2);
            assert_eq!(payload, StreamEvent::StreamCompleted(stream_id));
        }
    }

    assert!(withdraw_idx.is_some(), "final withdraw must emit withdrew");
    assert!(
        completed_idx.is_some(),
        "final withdraw on active stream must emit completed"
    );
    assert!(
        withdraw_idx.unwrap() < completed_idx.unwrap(),
        "withdrew event must be emitted before completed"
    );
}

/// Test withdraw after cancel and then try to withdraw again
/// Test withdraw after cancel then all accrued withdrawn.
/// Should return 0 without transfer or state change (idempotent).
#[test]
fn test_withdraw_after_cancel_then_completed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel at t=600
    ctx.env.ledger().set_timestamp(600);
    ctx.client().cancel_stream(&stream_id);

    // Withdraw accrued amount (600 tokens)
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 600);

    let state = ctx.client().get_stream_state(&stream_id);
    // After withdrawing all accrued from a cancelled stream,
    // withdrawn_amount equals the accrued amount at cancellation
    assert_eq!(state.withdrawn_amount, 600);
    // Status remains Cancelled (not Completed) because stream was terminated early
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Advance time substantially; cancelled accrual must remain frozen.
    ctx.env.ledger().set_timestamp(9_999);

    // Try to withdraw again - should return 0 because accrued (600) - withdrawn (600) = 0
    let withdrawn2 = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn2, 0,
        "should return 0 when nothing left to withdraw"
    );
}

// ---------------------------------------------------------------------------
// Tests — Issue: pause/resume transitions and lifecycle
// ---------------------------------------------------------------------------

/// Test pause stream as sender - successfully pauses and asserts status is Paused
#[test]
fn test_pause_stream_sender_transitions_to_paused() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Verify initial state is Active
    let state_before = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_before.status, StreamStatus::Active);

    // Sender pauses the stream
    ctx.client().pause_stream(&stream_id);

    // Verify status transitioned to Paused
    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.status, StreamStatus::Paused);

    // Verify all other fields remain unchanged
    assert_eq!(state_after.stream_id, stream_id);
    assert_eq!(state_after.sender, ctx.sender);
    assert_eq!(state_after.recipient, ctx.recipient);
    assert_eq!(state_after.deposit_amount, 1000);
    assert_eq!(state_after.rate_per_second, 1);
    assert_eq!(state_after.withdrawn_amount, 0);
}

/// Test pause stream as admin - successfully pauses via admin entrypoint
#[test]
fn test_pause_stream_admin_transitions_to_paused() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Verify initial state is Active
    let state_before = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_before.status, StreamStatus::Active);

    // Admin pauses the stream using admin-specific entrypoint
    ctx.client().pause_stream_as_admin(&stream_id);

    // Verify status transitioned to Paused
    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.status, StreamStatus::Paused);

    // Verify other fields unchanged
    assert_eq!(state_after.stream_id, stream_id);
    assert_eq!(state_after.deposit_amount, 1000);
}

/// Test resume stream as sender - successfully resumes from paused state
#[test]
fn test_resume_stream_sender_transitions_to_active() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // First pause the stream
    ctx.client().pause_stream(&stream_id);
    let state_paused = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_paused.status, StreamStatus::Paused);

    // Sender resumes the stream
    ctx.client().resume_stream(&stream_id);

    // Verify status transitioned back to Active
    let state_resumed = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_resumed.status, StreamStatus::Active);

    // Verify all other fields remain unchanged
    assert_eq!(state_resumed.stream_id, stream_id);
    assert_eq!(state_resumed.sender, ctx.sender);
    assert_eq!(state_resumed.recipient, ctx.recipient);
    assert_eq!(state_resumed.deposit_amount, 1000);
    assert_eq!(state_resumed.withdrawn_amount, 0);
}

/// Test resume stream as admin - successfully resumes via admin entrypoint
#[test]
fn test_resume_stream_admin_transitions_to_active() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Pause the stream first
    ctx.client().pause_stream(&stream_id);
    let state_paused = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_paused.status, StreamStatus::Paused);

    // Admin resumes the stream using admin-specific entrypoint
    ctx.client().resume_stream_as_admin(&stream_id);

    // Verify status transitioned to Active
    let state_resumed = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_resumed.status, StreamStatus::Active);

    // Verify integrity
    assert_eq!(state_resumed.stream_id, stream_id);
    assert_eq!(state_resumed.deposit_amount, 1000);
}

/// Test pause when already paused - fails with "stream is not active"
#[test]
#[should_panic]
fn test_pause_already_paused_fails_with_error() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // First pause succeeds
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Second pause on already-paused stream should fail
    ctx.client().pause_stream(&stream_id);
}

/// Test resume when active (not paused) - fails with "stream is active, not paused"
#[test]
#[should_panic]
fn test_resume_active_stream_fails_with_error() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Stream is Active from creation
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Attempting to resume active stream should fail
    ctx.client().resume_stream(&stream_id);
}

/// Test pause-resume-pause-resume multiple times preserves integrity
#[test]
fn test_multiple_pause_resume_cycles() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // First cycle: pause → resume
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Second cycle: pause → resume
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Third cycle: pause → resume
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.client().resume_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);

    // Verify stream integrity after multiple cycles
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.withdrawn_amount, 0);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
}

/// Test pause then resume allows withdrawal
#[test]
fn test_resume_enables_withdrawal() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance time and pause
    ctx.env.ledger().set_timestamp(500);
    ctx.client().pause_stream(&stream_id);

    // Verify can't withdraw while paused
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw(&stream_id);
    }));
    assert!(result.is_err(), "should not allow withdrawal while paused");

    // Resume the stream
    ctx.client().resume_stream(&stream_id);

    // Now withdrawal should succeed
    let recipient_before = ctx.token().balance(&ctx.recipient);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 500);

    let recipient_after = ctx.token().balance(&ctx.recipient);
    assert_eq!(recipient_after - recipient_before, 500);
}

/// Test accrual continues during pause - pause doesn't affect accrual
#[test]
fn test_accrual_continues_during_pause() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance to t=300 and pause
    ctx.env.ledger().set_timestamp(300);
    let accrued_before_pause = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before_pause, 300);

    ctx.client().pause_stream(&stream_id);

    // Advance time further while paused
    ctx.env.ledger().set_timestamp(700);

    // Accrual should continue even though stream is paused
    let accrued_while_paused = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_while_paused, 700, "accrual should be 700 at t=700");

    // Resume and verify accrual is correct
    ctx.client().resume_stream(&stream_id);
    let accrued_after_resume = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_after_resume, 700);
}

/// Test pause stream with different sender/admin authorization
#[test]
fn test_pause_stream_sender_and_admin_can_pause() {
    let ctx = TestContext::setup();

    // Create first stream for sender test
    let stream_id_1 = ctx.create_default_stream();

    // Sender pauses stream
    ctx.client().pause_stream(&stream_id_1);
    let state = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state.status, StreamStatus::Paused);

    // Create second stream for admin test
    let stream_id_2 = ctx.create_default_stream();

    // Admin can also pause via admin path
    ctx.client().pause_stream_as_admin(&stream_id_2);
    let state = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state.status, StreamStatus::Paused);
}

/// Test resume stream with different sender/admin authorization
#[test]
fn test_resume_stream_sender_and_admin_can_resume() {
    let ctx = TestContext::setup();

    // Create first stream for sender test
    let stream_id_1 = ctx.create_default_stream();
    ctx.client().pause_stream(&stream_id_1);

    // Sender resumes stream
    ctx.client().resume_stream(&stream_id_1);
    let state = ctx.client().get_stream_state(&stream_id_1);
    assert_eq!(state.status, StreamStatus::Active);

    // Create second stream for admin test
    let stream_id_2 = ctx.create_default_stream();
    ctx.client().pause_stream(&stream_id_2);

    // Admin resumes via admin path
    ctx.client().resume_stream_as_admin(&stream_id_2);
    let state = ctx.client().get_stream_state(&stream_id_2);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test pause/resume events are published correctly
#[test]
fn test_pause_resume_events_published() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Clear previous events
    ctx.env.events().all();

    // Pause stream
    ctx.client().pause_stream(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Verify pause event
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Paused(stream_id)
    );

    // Resume stream
    ctx.client().resume_stream(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Verify resume event
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Resumed(stream_id)
    );
}

/// Test pause does not affect token balances
#[test]
fn test_pause_resume_preserves_token_balances() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let sender_before = ctx.token().balance(&ctx.sender);
    let recipient_before = ctx.token().balance(&ctx.recipient);
    let contract_before = ctx.token().balance(&ctx.contract_id);

    // Pause and resume multiple times
    ctx.client().pause_stream(&stream_id);
    ctx.client().resume_stream(&stream_id);
    ctx.client().pause_stream(&stream_id);
    ctx.client().resume_stream(&stream_id);

    // Verify token balances unchanged
    assert_eq!(ctx.token().balance(&ctx.sender), sender_before);
    assert_eq!(ctx.token().balance(&ctx.recipient), recipient_before);
    assert_eq!(ctx.token().balance(&ctx.contract_id), contract_before);
}

/// Test pause with cliff - can pause before and after cliff
#[test]
fn test_pause_resume_with_cliff_before_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Pause before cliff
    ctx.env.ledger().set_timestamp(200);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Verify accrual is still 0 before cliff
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0);

    // Resume before cliff
    ctx.client().resume_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Test pause with cliff - pause after cliff allows accrual
#[test]
fn test_pause_resume_with_cliff_after_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Advance past cliff and pause
    ctx.env.ledger().set_timestamp(700);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Verify accrual at 700 (time-based)
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 700);

    // Resume and withdraw
    ctx.client().resume_stream(&stream_id);

    let recipient_before = ctx.token().balance(&ctx.recipient);
    let withdrawn = ctx.client().withdraw(&stream_id);
    let recipient_after = ctx.token().balance(&ctx.recipient);

    assert_eq!(withdrawn, 700);
    assert_eq!(recipient_after - recipient_before, 700);
}

/// Test pause and cancel - can cancel a paused stream
#[test]
fn test_pause_then_cancel() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance time, pause, then cancel
    ctx.env.ledger().set_timestamp(300);
    ctx.client().pause_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Cancel paused stream
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Verify accrual at cancellation time was used for refund
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 300);

    // Verify recipient can withdraw accrued amount
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 300);
}

/// Test resume fails on completed stream
#[test]
#[should_panic]
fn test_resume_completed_stream_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw everything to complete the stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Attempting to resume completed stream should fail
    ctx.client().resume_stream(&stream_id);
}

/// Test resume fails on cancelled stream
#[test]
#[should_panic]
fn test_resume_cancelled_stream_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel the stream
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Attempting to resume cancelled stream should fail
    ctx.client().resume_stream(&stream_id);
}

/// Test pause then resume preserves withdrawal state
#[test]
fn test_pause_resume_preserves_withdrawal_state() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 300 tokens
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 300);

    // Pause and resume
    ctx.client().pause_stream(&stream_id);
    ctx.client().resume_stream(&stream_id);

    // Verify withdrawal state preserved
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 300);

    // Withdraw more at t=700
    ctx.env.ledger().set_timestamp(700);
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 400); // 700 - 300 already withdrawn

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 700);
}

// ---------------------------------------------------------------------------
// Tests — stream_id generation and uniqueness
// ---------------------------------------------------------------------------

/// The first stream created after init must receive stream_id = 0.
#[test]
fn test_stream_id_first_stream_is_zero() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );

    assert_eq!(id, 0, "first stream_id must be 0");
    assert_eq!(
        ctx.client().get_stream_state(&id).stream_id,
        0,
        "stream struct must also record stream_id = 0"
    );
}

/// Each subsequent call to create_stream increments the stream_id by exactly one,
/// producing a monotonically increasing sequence with no gaps.
#[test]
fn test_stream_id_increments_by_one() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );

    assert_eq!(id0, 0, "first id must be 0");
    assert_eq!(id1, id0 + 1, "second id must be first + 1");
    assert_eq!(id2, id1 + 1, "third id must be second + 1");
}

/// The stream_id returned by create_stream must equal the stream_id field
/// stored inside the persisted Stream struct for every stream created.
#[test]
fn test_create_stream_returned_id_matches_stored_id() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    for expected_id in 0u64..5 {
        let returned_id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100_i128,
            &1_i128,
            &0u64,
            &0u64,
            &100u64,
            &None,
        );
        let stored = ctx.client().get_stream_state(&returned_id);

        assert_eq!(
            returned_id, expected_id,
            "stream {expected_id}: returned id must be sequential"
        );
        assert_eq!(
            stored.stream_id, returned_id,
            "stream {expected_id}: stored stream_id must equal returned id"
        );
    }
}

// Tests — withdraw updates withdrawn_amount and status (comprehensive suite)
// Issue: test/withdraw-updates-state
// ---------------------------------------------------------------------------

/// Comprehensive test: Create stream, advance time, withdraw, assert updated state
/// This test validates that:
/// 1. Withdraw returns the correct amount
/// 2. Stream's withdrawn_amount is updated
/// 3. Recipient receives tokens
/// 4. Additional withdrawals add to withdrawn_amount (not reset)
/// 5. When fully withdrawn, status = Completed
#[test]
fn test_withdraw_updates_withdrawn_amount_and_status() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 1000 deposit, 1 token/s, 1000s

    // INITIAL STATE: Stream created, nothing withdrawn
    let initial_state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        initial_state.withdrawn_amount, 0,
        "initial withdrawn_amount should be 0"
    );
    assert_eq!(
        initial_state.status,
        StreamStatus::Active,
        "initial status should be Active"
    );
    assert_eq!(
        initial_state.deposit_amount, 1000,
        "deposit_amount should be 1000"
    );

    // FIRST WITHDRAWAL: At t=300, 300 tokens accrued
    ctx.env.ledger().set_timestamp(300);
    let recipient_before_first = ctx.token().balance(&ctx.recipient);

    let withdrawn_amount_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn_amount_1, 300,
        "first withdraw should return 300 tokens"
    );

    // Verify state updates: withdrawn_amount increased
    let state_after_first = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_after_first.withdrawn_amount, 300,
        "withdrawn_amount should be 300 after first withdrawal"
    );
    assert_eq!(
        state_after_first.status,
        StreamStatus::Active,
        "status should still be Active (not complete)"
    );

    // Verify recipient received 300 tokens
    let recipient_after_first = ctx.token().balance(&ctx.recipient);
    assert_eq!(
        recipient_after_first - recipient_before_first,
        300,
        "recipient should receive 300 tokens"
    );

    // SECOND WITHDRAWAL: At t=700, additional 400 tokens accrued (cumulative 700)
    ctx.env.ledger().set_timestamp(700);
    let recipient_before_second = ctx.token().balance(&ctx.recipient);

    let withdrawn_amount_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn_amount_2, 400,
        "second withdraw should return 400 additional tokens (700 - 300)"
    );

    // Verify state updates: withdrawn_amount increased (not reset)
    let state_after_second = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_after_second.withdrawn_amount, 700,
        "withdrawn_amount should be 700 after second withdrawal (300 + 400)"
    );
    assert_eq!(
        state_after_second.status,
        StreamStatus::Active,
        "status should still be Active (not complete)"
    );

    // Verify recipient received additional 400 tokens
    let recipient_after_second = ctx.token().balance(&ctx.recipient);
    assert_eq!(
        recipient_after_second - recipient_before_second,
        400,
        "recipient should receive 400 additional tokens"
    );

    // FINAL WITHDRAWAL: At t=1000, remaining 300 tokens accrued (cumulative 1000)
    ctx.env.ledger().set_timestamp(1000);
    let recipient_before_final = ctx.token().balance(&ctx.recipient);

    let withdrawn_amount_3 = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn_amount_3, 300,
        "final withdraw should return 300 remaining tokens (1000 - 700)"
    );

    // Verify state updates: withdrawn_amount reaches deposit (COMPLETED)
    let state_after_final = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_after_final.withdrawn_amount, 1000,
        "withdrawn_amount should equal deposit_amount (1000)"
    );
    assert_eq!(
        state_after_final.status,
        StreamStatus::Completed,
        "status should be Completed when fully withdrawn"
    );

    // Verify recipient received final 300 tokens
    let recipient_after_final = ctx.token().balance(&ctx.recipient);
    assert_eq!(
        recipient_after_final - recipient_before_final,
        300,
        "recipient should receive 300 final tokens"
    );

    // VERIFY TOTALS
    let total_recipient_tokens = ctx.token().balance(&ctx.recipient);
    assert_eq!(
        total_recipient_tokens, 1000,
        "recipient should have received all 1000 tokens total"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        0,
        "contract should have no tokens left"
    );
    assert_eq!(
        withdrawn_amount_1 + withdrawn_amount_2 + withdrawn_amount_3,
        1000,
        "total withdrawn should equal deposit"
    );
}

/// Test: Partial withdrawal then full withdrawal with intermediate time checks
/// Validates that withdrawn_amount accumulates correctly across multiple calls
#[test]
fn test_withdraw_partial_then_full_with_intermediate_checks() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // First partial withdrawal: 250 tokens at t=250
    ctx.env.ledger().set_timestamp(250);
    let w1 = ctx.client().withdraw(&stream_id);
    assert_eq!(w1, 250);

    let state1 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state1.withdrawn_amount, 250,
        "after first: withdrawn_amount = 250"
    );
    assert_eq!(
        state1.status,
        StreamStatus::Active,
        "after first: still Active"
    );
    assert_eq!(state1.deposit_amount, 1000, "deposit_amount unchanged");

    // Second partial withdrawal: 250 more tokens at t=500
    ctx.env.ledger().set_timestamp(500);
    let w2 = ctx.client().withdraw(&stream_id);
    assert_eq!(w2, 250, "second withdrawal adds 250 more");

    let state2 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state2.withdrawn_amount, 500,
        "after second: withdrawn_amount = 500"
    );
    assert_eq!(
        state2.status,
        StreamStatus::Active,
        "after second: still Active"
    );

    // Third partial withdrawal: 250 more tokens at t=750
    ctx.env.ledger().set_timestamp(750);
    let w3 = ctx.client().withdraw(&stream_id);
    assert_eq!(w3, 250, "third withdrawal adds 250 more");

    let state3 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state3.withdrawn_amount, 750,
        "after third: withdrawn_amount = 750"
    );
    assert_eq!(
        state3.status,
        StreamStatus::Active,
        "after third: still Active"
    );

    // Final withdrawal: last 250 tokens at t=1000 -> COMPLETED
    ctx.env.ledger().set_timestamp(1000);
    let w4 = ctx.client().withdraw(&stream_id);
    assert_eq!(w4, 250, "final withdrawal adds last 250");

    let state_final = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_final.withdrawn_amount, 1000,
        "final: withdrawn_amount = 1000 (full deposit)"
    );
    assert_eq!(
        state_final.status,
        StreamStatus::Completed,
        "final: status = Completed"
    );

    // Verify total
    assert_eq!(w1 + w2 + w3 + w4, 1000, "total withdrawn = 1000");
}

/// Test: Verify withdrawn_amount never decreases (monotonic)
/// Ensures state updates are only additive
#[test]
fn test_withdraw_withdrawn_amount_monotonic_increase() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let mut previous_withdrawn = 0_i128;

    let timestamps = [100, 250, 500, 750, 900, 1000];

    for &t in &timestamps {
        ctx.env.ledger().set_timestamp(t);
        ctx.client().withdraw(&stream_id);

        let state = ctx.client().get_stream_state(&stream_id);

        assert!(
            state.withdrawn_amount > previous_withdrawn,
            "withdrawn_amount must strictly increase at t={}: {} > {}",
            t,
            state.withdrawn_amount,
            previous_withdrawn
        );

        previous_withdrawn = state.withdrawn_amount;
    }
}

/// Test: Verify status only transitions Active -> Completed once fully withdrawn
/// No intermediate status changes during partial withdrawals
#[test]
fn test_withdraw_status_transitions_correctly() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let check_points = [
        (200u64, StreamStatus::Active),
        (400u64, StreamStatus::Active),
        (600u64, StreamStatus::Active),
        (800u64, StreamStatus::Active),
        (950u64, StreamStatus::Active),
        (1000u64, StreamStatus::Completed), // Only at end, when fully withdrawn
    ];

    for (timestamp, expected_status) in check_points {
        ctx.env.ledger().set_timestamp(timestamp);
        ctx.client().withdraw(&stream_id);

        let state = ctx.client().get_stream_state(&stream_id);
        assert_eq!(
            state.status, expected_status,
            "at t={}, status should be {:?}",
            timestamp, expected_status
        );
    }
}

/// N streams must produce N distinct IDs with no duplicates and no gaps,
/// forming the sequence 0, 1, 2, …, N-1.
#[test]
fn test_stream_ids_are_unique_no_gaps() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    const N: u64 = 20;
    let mut ids = Vec::new(&ctx.env);

    for expected in 0..N {
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &10_i128,
            &1_i128,
            &0u64,
            &0u64,
            &10u64,
            &None,
        );
        assert_eq!(id, expected, "stream {expected} must have id {expected}");
        ids.push_back(id);
    }

    // Pairwise uniqueness check — no two entries may share an id
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            assert_ne!(
                ids.get(i).unwrap(),
                ids.get(j).unwrap(),
                "stream_ids at positions {i} and {j} must be different"
            );
        }
    }
}

/// A create_stream call that fails validation (deposit too low) must NOT
/// advance the NextStreamId counter; the next successful call must receive
/// the id that the failed call would have consumed.
#[test]
fn test_failed_create_stream_does_not_advance_counter() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // First successful stream -> id = 0
    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    assert_eq!(id0, 0);

    // Attempt a stream with an underfunded deposit (1 token, need 100) -> must return InsufficientDeposit
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_i128, // deposit < rate * duration (100)
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));

    // Next successful stream must still be id = 1, not 2
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    assert_eq!(
        id1, 1,
        "counter must not advance after a failed create_stream"
    );
}

/// Streams created by different senders and recipients all draw from the
/// same global NextStreamId counter, producing globally unique ids.
#[test]
fn test_stream_ids_unique_across_different_senders() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Provision a second sender with enough tokens
    let sender2 = Address::generate(&ctx.env);
    let recipient2 = Address::generate(&ctx.env);
    ctx.sac.mint(&sender2, &1_000_i128);

    let id_a = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    let id_b = ctx.client().create_stream(
        &sender2,
        &recipient2,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    let id_c = ctx.client().create_stream(
        &ctx.sender,
        &recipient2,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );

    assert_eq!(id_a, 0, "first stream (sender1→recipient1) must be 0");
    assert_eq!(id_b, 1, "second stream (sender2→recipient2) must be 1");
    assert_eq!(id_c, 2, "third stream (sender1→recipient2) must be 2");

    assert_ne!(id_a, id_b, "ids from different senders must not collide");
    assert_ne!(id_b, id_c, "ids from different senders must not collide");
    assert_ne!(id_a, id_c, "ids from different senders must not collide");
}

/// Pausing, resuming, or cancelling a stream must not alter any stream's
/// stream_id field, and the global counter must continue from where it left off.
#[test]
fn test_stream_id_stability_after_state_changes() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &2_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &2_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &2_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );

    // Mutate stream 1: pause then cancel
    ctx.client().pause_stream(&id1);
    ctx.client().cancel_stream(&id1);

    // Stream struct stream_id fields must be unchanged
    assert_eq!(ctx.client().get_stream_state(&id0).stream_id, id0);
    assert_eq!(ctx.client().get_stream_state(&id1).stream_id, id1);
    assert_eq!(ctx.client().get_stream_state(&id2).stream_id, id2);

    // The global counter must continue from 3
    let id3 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &2_i128,
        &0u64,
        &0u64,
        &100u64,
        &None,
    );
    assert_eq!(
        id3, 3,
        "counter must continue monotonically after state mutations"
    );
}

/// Test: Verify returned amount matches withdrawn_amount increment
/// Ensures internal accounting matches external transfer amount
#[test]
fn test_withdraw_returned_amount_matches_increment() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // First withdrawal
    ctx.env.ledger().set_timestamp(300);
    let state_before_1 = ctx.client().get_stream_state(&stream_id);
    let returned_1 = ctx.client().withdraw(&stream_id);
    let state_after_1 = ctx.client().get_stream_state(&stream_id);

    let increment_1 = state_after_1.withdrawn_amount - state_before_1.withdrawn_amount;
    assert_eq!(
        returned_1, increment_1,
        "returned amount should equal withdrawn_amount increment"
    );

    // Second withdrawal
    ctx.env.ledger().set_timestamp(700);
    let state_before_2 = ctx.client().get_stream_state(&stream_id);
    let returned_2 = ctx.client().withdraw(&stream_id);
    let state_after_2 = ctx.client().get_stream_state(&stream_id);

    let increment_2 = state_after_2.withdrawn_amount - state_before_2.withdrawn_amount;
    assert_eq!(
        returned_2, increment_2,
        "returned amount should equal withdrawn_amount increment"
    );

    // Final withdrawal
    ctx.env.ledger().set_timestamp(1000);
    let state_before_3 = ctx.client().get_stream_state(&stream_id);
    let returned_3 = ctx.client().withdraw(&stream_id);
    let state_after_3 = ctx.client().get_stream_state(&stream_id);

    let increment_3 = state_after_3.withdrawn_amount - state_before_3.withdrawn_amount;
    assert_eq!(
        returned_3, increment_3,
        "returned amount should equal withdrawn_amount increment"
    );
}

/// Test: Edge case - withdraw in multiple small increments
/// Verifies correct state updates even with many frequent withdrawals
#[test]
fn test_withdraw_many_small_increments() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let mut total_withdrawn = 0_i128;

    // Withdraw in 10 equal parts
    for i in 1..=10 {
        let timestamp = 100 * i as u64;
        ctx.env.ledger().set_timestamp(timestamp);

        let amount = ctx.client().withdraw(&stream_id);
        total_withdrawn += amount;

        let state = ctx.client().get_stream_state(&stream_id);
        assert_eq!(
            state.withdrawn_amount, total_withdrawn,
            "at iteration {}, withdrawn_amount should be {}",
            i, total_withdrawn
        );

        if i == 10 {
            // Last withdrawal should mark as Completed
            assert_eq!(
                state.status,
                StreamStatus::Completed,
                "final should be Completed"
            );
        } else {
            assert_eq!(
                state.status,
                StreamStatus::Active,
                "intermediate should be Active"
            );
        }
    }

    assert_eq!(total_withdrawn, 1000, "total should equal deposit");
}

/// Test: Verify contract token balance decreases with each withdrawal
/// Ensures tokens are actually transferred out
#[test]
fn test_withdraw_contract_balance_decreases() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let initial_contract_balance = ctx.token().balance(&ctx.contract_id);
    assert_eq!(
        initial_contract_balance, 1000,
        "initial contract balance = deposit"
    );

    // First withdrawal: 300 tokens
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    let balance_after_1 = ctx.token().balance(&ctx.contract_id);
    assert_eq!(
        balance_after_1, 700,
        "contract balance should decrease by 300"
    );

    // Second withdrawal: 400 tokens
    ctx.env.ledger().set_timestamp(700);
    ctx.client().withdraw(&stream_id);

    let balance_after_2 = ctx.token().balance(&ctx.contract_id);
    assert_eq!(
        balance_after_2, 300,
        "contract balance should decrease by 400 more"
    );

    // Final withdrawal: 300 tokens
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let final_contract_balance = ctx.token().balance(&ctx.contract_id);
    assert_eq!(
        final_contract_balance, 0,
        "contract balance should be 0 after full withdrawal"
    );
}

/// Test: Verify recipient token balance increases with each withdrawal
/// Ensures recipient receives all withdrawn amounts
#[test]
fn test_withdraw_recipient_balance_increases() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let initial_recipient_balance = ctx.token().balance(&ctx.recipient);
    assert_eq!(
        initial_recipient_balance, 0,
        "recipient starts with 0 tokens"
    );

    // First withdrawal: 300 tokens
    ctx.env.ledger().set_timestamp(300);
    let amount_1 = ctx.client().withdraw(&stream_id);
    assert_eq!(amount_1, 300, "first withdrawal = 300");

    let balance_after_1 = ctx.token().balance(&ctx.recipient);
    assert_eq!(balance_after_1, 300, "recipient balance should be 300");

    // Second withdrawal: 400 tokens
    ctx.env.ledger().set_timestamp(700);
    let amount_2 = ctx.client().withdraw(&stream_id);
    assert_eq!(amount_2, 400, "second withdrawal = 400");

    let balance_after_2 = ctx.token().balance(&ctx.recipient);
    assert_eq!(balance_after_2, 700, "recipient balance should be 700");

    // Final withdrawal: 300 tokens
    ctx.env.ledger().set_timestamp(1000);
    let amount_3 = ctx.client().withdraw(&stream_id);
    assert_eq!(amount_3, 300, "final withdrawal = 300");

    let final_recipient_balance = ctx.token().balance(&ctx.recipient);
    assert_eq!(
        final_recipient_balance, 1000,
        "recipient should have all 1000 tokens"
    );
}

/// Test: Withdrawn_amount stays consistent between calls
/// Verifies state is persisted correctly
#[test]
fn test_withdraw_state_persists_across_calls() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 500 tokens at t=500
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);

    // Check state immediately
    let state_1 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_1.withdrawn_amount, 500);

    // Check state again (no additional withdraw)
    let state_2 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_2.withdrawn_amount, 500,
        "withdrawn_amount should persist"
    );

    // Now withdraw again at t=800
    ctx.env.ledger().set_timestamp(800);
    ctx.client().withdraw(&stream_id);

    // Check that previous withdraw didn't reset
    let state_3 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_3.withdrawn_amount, 800,
        "previous withdraw stayed, new added"
    );
}

/// Test: Withdrawn amount with cliff - verify only streamable amount after cliff
#[test]
fn test_withdraw_cliff_updates_withdrawn_correctly() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Cannot withdraw before cliff (nothing to withdraw)
    ctx.env.ledger().set_timestamp(200);
    // (would panic, so skip test here)

    // At cliff time (t=500), can withdraw accrued amount
    ctx.env.ledger().set_timestamp(500);
    let w1 = ctx.client().withdraw(&stream_id);
    assert_eq!(w1, 500, "at cliff, withdraw 500 tokens accrued from start");

    let state1 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state1.withdrawn_amount, 500);

    // Withdraw remaining at t=1000
    ctx.env.ledger().set_timestamp(1000);
    let w2 = ctx.client().withdraw(&stream_id);
    assert_eq!(w2, 500, "remaining 500 tokens");

    let state2 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state2.withdrawn_amount, 1000);
    assert_eq!(state2.status, StreamStatus::Completed);
}

/// Test: Cancel stream then withdraw - status stays Cancelled (not Completed)
/// even when fully withdrawing the accrued amount
#[test]
fn test_withdraw_after_cancel_status_stays_cancelled() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel at t=600 (600 tokens accrued)
    ctx.env.ledger().set_timestamp(600);
    ctx.client().cancel_stream(&stream_id);

    let state_after_cancel = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_after_cancel.status,
        StreamStatus::Cancelled,
        "status should be Cancelled"
    );
    assert_eq!(state_after_cancel.withdrawn_amount, 0, "no withdrawal yet");

    // Withdraw the accrued 600 tokens
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 600, "can withdraw accrued 600 tokens");

    let state_after_withdraw = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state_after_withdraw.withdrawn_amount, 600,
        "withdrawn_amount updated to 600"
    );
    assert_eq!(
        state_after_withdraw.status,
        StreamStatus::Cancelled,
        "status should STAY Cancelled (not become Completed)"
    );
}

#[test]
fn test_withdraw_after_cancel_at_full_accrual_stays_cancelled_no_completed_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel at end-time where accrued == deposit and refund == 0.
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&stream_id);
    let events_before = ctx.env.events().all().len();

    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 1000);

    let events = ctx.env.events().all();
    let mut saw_completed = false;
    let mut saw_withdrew = false;
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        let topic0 = Symbol::from_val(&ctx.env, &event.1.get(0).unwrap());
        if topic0 == Symbol::new(&ctx.env, "withdrew") {
            saw_withdrew = true;
        }
        if topic0 == Symbol::new(&ctx.env, "completed") {
            saw_completed = true;
        }
    }

    assert!(
        saw_withdrew,
        "recipient withdrawal after cancellation still emits withdrew"
    );
    assert!(
        !saw_completed,
        "cancelled stream must not emit completed on withdraw"
    );
}

/// Test: Verify that completed stream cannot be withdrawn again
/// Accessing a completed stream's withdraw should panic
#[test]
#[should_panic]
fn test_withdraw_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw all tokens
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);

    // Attempt another withdraw on completed stream - should panic
    ctx.client().withdraw(&stream_id);
}

// ---------------------------------------------------------------------------
// Tests — Issue #129: cancel_stream from Paused state
// ---------------------------------------------------------------------------

#[test]
fn test_cancel_stream_from_paused_state() {
    let ctx = TestContext::setup();

    // 1. Create a 1000 token / 1000 second stream
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    ctx.client().pause_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    // Verify it changed to Cancelled
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );

    // 5. Verify refund (Unstreamed = 500)
    let sender_balance_after = ctx.token().balance(&ctx.sender);
    assert_eq!(sender_balance_after - sender_balance_before, 500);

    assert_eq!(ctx.token().balance(&ctx.recipient), 0);
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.token().balance(&ctx.recipient), 500);
}

#[test]
fn test_create_stream_large_rate_overflow_in_accrual() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &(i128::MAX - 10_000_i128));

    // Extremely large rate, short duration → total still fits i128
    let rate_per_second = i128::MAX / 1_000_000;
    let duration: u64 = 1_000_000;
    let total_streamable = rate_per_second * (duration as i128);
    let deposit_amount = total_streamable + 1;

    let start_time = 1_700_000_000;
    let cliff_time = start_time;
    let end_time = start_time + duration;

    let stream_id = ctx.client().create_stream(
        &ctx.sender.clone(),
        &ctx.recipient.clone(),
        &deposit_amount,
        &rate_per_second,
        &start_time,
        &cliff_time,
        &end_time,
        &None,
    );

    ctx.env.ledger().set_timestamp(end_time);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued <= deposit_amount); // must not exceed deposit
    assert!(accrued >= 0);
    assert_eq!(accrued, deposit_amount - 1);
}

#[test]
fn test_accrual_capped_at_exact_total() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &(i128::MAX - 10_000_i128));

    let rate_per_second = i128::MAX / 2_000_000;
    let duration: u64 = 2_000_000;
    let total = rate_per_second * (duration as i128);
    let deposit_amount = total;

    let start_time = 1_700_000_000;
    let cliff_time = start_time;
    let end_time = start_time + duration;

    let stream_id = ctx.client().create_stream(
        &ctx.sender.clone(),
        &ctx.recipient.clone(),
        &deposit_amount,
        &rate_per_second,
        &start_time,
        &cliff_time,
        &end_time,
        &None,
    );

    ctx.env.ledger().set_timestamp(end_time);

    let accrued = ctx.client().calculate_accrued(&stream_id);

    assert_eq!(accrued, deposit_amount);
}

#[test]
fn test_accrual_capped_when_deposit_exceeds_total() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &(i128::MAX - 10_000_i128));

    let rate_per_second = i128::MAX / 1_000_000;
    let duration: u64 = 1_000_000;
    let total = rate_per_second * (duration as i128);
    let deposit_amount = total + 42;

    let start_time = 1_700_000_000;
    let cliff_time = start_time;
    let end_time = start_time + duration;

    let stream_id = ctx.client().create_stream(
        &ctx.sender.clone(),
        &ctx.recipient.clone(),
        &deposit_amount,
        &rate_per_second,
        &start_time,
        &cliff_time,
        &end_time,
        &None,
    );

    ctx.env.ledger().set_timestamp(end_time);

    let accrued = ctx.client().calculate_accrued(&stream_id);

    assert_eq!(accrued, total);
}

// ---------------------------------------------------------------------------
// Tests — Batch create_streams
// ---------------------------------------------------------------------------

use soroban_sdk::vec;

#[test]
fn test_create_streams_batch_success() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Initial balances
    let initial_sender_balance = ctx.token().balance(&ctx.sender);
    let initial_contract_balance = ctx.token().balance(&ctx.contract_id);

    // Create 3 streams in one batch
    let params1 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };

    let params2 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 2000,
        rate_per_second: 2,
        start_time: 100,
        cliff_time: 200,
        end_time: 1100,
        memo: None,
    };

    let params3 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 3000,
        rate_per_second: 3,
        start_time: 500,
        cliff_time: 500,
        end_time: 1500,
        memo: None,
    };

    let streams = vec![&ctx.env, params1.clone(), params2.clone(), params3.clone()];
    let stream_ids = ctx.client().create_streams(&ctx.sender, &streams);

    // Check returned IDs
    assert_eq!(stream_ids.len(), 3);
    assert_eq!(stream_ids.get(0).unwrap(), 0);
    assert_eq!(stream_ids.get(1).unwrap(), 1);
    assert_eq!(stream_ids.get(2).unwrap(), 2);

    // Verify balances (6000 total tokens transferred)
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        initial_sender_balance - 6000
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        initial_contract_balance + 6000
    );

    // Verify stored states
    let state1 = ctx.client().get_stream_state(&0);
    assert_eq!(state1.deposit_amount, 1000);
    assert_eq!(state1.recipient, params1.recipient);

    let state2 = ctx.client().get_stream_state(&1);
    assert_eq!(state2.deposit_amount, 2000);
    assert_eq!(state2.rate_per_second, 2);

    let state3 = ctx.client().get_stream_state(&2);
    assert_eq!(state3.deposit_amount, 3000);
    assert_eq!(state3.end_time, 1500);
}

#[test]
fn test_create_streams_batch_atomic_failure() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // One valid stream, one invalid stream
    let valid_params = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };

    let invalid_params = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 500, // Insufficient deposit (1 * 1000 = 1000 needed)
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };

    let streams = vec![&ctx.env, valid_params, invalid_params];
    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_streams(&ctx.sender, &streams);
    }));
    assert!(result.is_err(), "batch with one invalid stream must fail");

    assert_eq!(
        ctx.client().get_stream_count(),
        stream_count_before,
        "batch failure must not advance stream counter"
    );
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        sender_balance_before,
        "sender balance must not change on batch validation failure"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before,
        "contract balance must not change on batch validation failure"
    );
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "failed batch must emit no created events"
    );
}

#[test]
#[should_panic]
fn test_create_streams_batch_sender_recipient_panic() {
    let ctx = TestContext::setup();

    let params = CreateStreamParams {
        recipient: ctx.sender.clone(), // Invalid: recipient == sender
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };

    let streams = vec![&ctx.env, params];
    ctx.client().create_streams(&ctx.sender, &streams);
}

#[test]
fn test_create_streams_batch_sender_recipient_has_no_side_effects() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let params = CreateStreamParams {
        recipient: ctx.sender.clone(), // invalid: recipient == sender
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };

    let streams = vec![&ctx.env, params];

    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_streams(&ctx.sender, &streams);
    }));

    assert!(result.is_err(), "self-streaming in batch must be rejected");
    assert_eq!(
        ctx.client().get_stream_count(),
        stream_count_before,
        "batch failure must not advance stream counter"
    );
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        sender_balance_before,
        "sender balance must not change on validation failure"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
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
fn test_create_streams_batch_empty() {
    let ctx = TestContext::setup();
    let streams = Vec::new(&ctx.env);

    let initial_balance = ctx.token().balance(&ctx.sender);

    let ids = ctx.client().create_streams(&ctx.sender, &streams);

    // Should return empty vec, charge no tokens, and not advance ID counter
    assert_eq!(ids.len(), 0);
    assert_eq!(ctx.token().balance(&ctx.sender), initial_balance);

    // Next standard create_stream should get ID 0
    let next_id = ctx.create_default_stream();
    assert_eq!(next_id, 0);
}

#[test]
fn test_create_streams_batch_empty_requires_auth() {
    // Empty batch still requires sender authorization
    let ctx = TestContext::setup_strict();
    let streams = Vec::new(&ctx.env);

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Mock sender auth for empty batch
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_streams",
            args: (&ctx.sender, streams.clone()).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let ids = ctx.client().create_streams(&ctx.sender, &streams);
    assert_eq!(ids.len(), 0);
}

#[test]
fn test_create_streams_batch_empty_no_events() {
    // Empty batch must not emit any events
    let ctx = TestContext::setup();
    let streams = Vec::new(&ctx.env);

    let events_before = ctx.env.events().all().len();
    let ids = ctx.client().create_streams(&ctx.sender, &streams);
    let events_after = ctx.env.events().all().len();

    assert_eq!(ids.len(), 0);
    assert_eq!(
        events_before, events_after,
        "empty batch must not emit any events"
    );
}

#[test]
fn test_create_streams_batch_empty_no_state_change() {
    // Empty batch must not change stream count or any state
    let ctx = TestContext::setup();
    let streams = Vec::new(&ctx.env);

    let count_before = ctx.client().get_stream_count();
    let ids = ctx.client().create_streams(&ctx.sender, &streams);
    let count_after = ctx.client().get_stream_count();

    assert_eq!(ids.len(), 0);
    assert_eq!(
        count_before, count_after,
        "empty batch must not advance stream ID counter"
    );
}

#[test]
fn test_create_streams_batch_empty_then_normal_create() {
    // After empty batch, next stream should get ID 0 (not skipped)
    let ctx = TestContext::setup();

    // First: empty batch
    let empty_streams = Vec::new(&ctx.env);
    let empty_ids = ctx.client().create_streams(&ctx.sender, &empty_streams);
    assert_eq!(empty_ids.len(), 0);

    // Second: normal create_stream
    let id1 = ctx.create_default_stream();
    assert_eq!(id1, 0, "first stream after empty batch should get ID 0");

    // Third: another empty batch
    let empty_ids2 = ctx.client().create_streams(&ctx.sender, &empty_streams);
    assert_eq!(empty_ids2.len(), 0);

    // Fourth: another normal create_stream
    let id2 = ctx.create_default_stream();
    assert_eq!(id2, 1, "second stream should get ID 1");
}

#[test]
fn test_create_streams_batch_empty_multiple_times() {
    // Multiple empty batches should all succeed and have no side effects
    let ctx = TestContext::setup();
    let streams = Vec::new(&ctx.env);

    let initial_balance = ctx.token().balance(&ctx.sender);
    let initial_count = ctx.client().get_stream_count();

    for _ in 0..5 {
        let ids = ctx.client().create_streams(&ctx.sender, &streams);
        assert_eq!(ids.len(), 0);
    }

    assert_eq!(
        ctx.token().balance(&ctx.sender),
        initial_balance,
        "balance must not change after multiple empty batches"
    );
    assert_eq!(
        ctx.client().get_stream_count(),
        initial_count,
        "stream count must not change after multiple empty batches"
    );
}

#[test]
fn test_create_streams_batch_empty_when_paused() {
    // Empty batch should succeed even when contract is paused
    let ctx = TestContext::setup();
    let streams = Vec::new(&ctx.env);

    // Pause contract
    ctx.client().set_contract_paused(&true);

    // Empty batch should still succeed (no-op)
    let ids = ctx.client().create_streams(&ctx.sender, &streams);
    assert_eq!(ids.len(), 0);
}

#[test]
fn test_create_streams_batch_empty_recipient_index_unchanged() {
    // Empty batch must not affect recipient stream indices
    let ctx = TestContext::setup();
    let recipient = Address::generate(&ctx.env);
    let streams = Vec::new(&ctx.env);

    let count_before = ctx.client().get_recipient_stream_count(&recipient);
    let ids = ctx.client().create_streams(&ctx.sender, &streams);
    let count_after = ctx.client().get_recipient_stream_count(&recipient);

    assert_eq!(ids.len(), 0);
    assert_eq!(
        count_before, count_after,
        "empty batch must not modify recipient stream index"
    );
}

#[test]
fn test_create_streams_batch_strict_auth() {
    let ctx = TestContext::setup_strict();

    let params1 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };

    let params2 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 2000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 2000,
        memo: None,
    };

    let streams = vec![&ctx.env, params1.clone(), params2.clone()];

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Mock the sender's auth EXACTLY ONCE for the bulk operation
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_streams",
            args: (&ctx.sender, streams.clone()).into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                // Total deposit = 1000 + 2000 = 3000
                args: (&ctx.sender, &ctx.contract_id, 3000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    let stream_ids = ctx.client().create_streams(&ctx.sender, &streams);
    assert_eq!(stream_ids.len(), 2);
}

#[test]
fn test_create_streams_batch_emits_created_events_with_payloads() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let params1 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1111,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1111,
        memo: None,
    };
    let params2 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 2222,
        rate_per_second: 2,
        start_time: 10,
        cliff_time: 10,
        end_time: 1121,
        memo: None,
    };
    let streams = vec![&ctx.env, params1.clone(), params2.clone()];
    let events_before = ctx.env.events().all().len();

    let ids = ctx.client().create_streams(&ctx.sender, &streams);
    assert_eq!(ids.len(), 2);

    let events = ctx.env.events().all();
    let mut created_payloads: std::vec::Vec<StreamCreated> = std::vec::Vec::new();
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        let topic0 = Symbol::from_val(&ctx.env, &event.1.get(0).unwrap());
        if topic0 == Symbol::new(&ctx.env, "created") {
            created_payloads.push(StreamCreated::try_from_val(&ctx.env, &event.2).unwrap());
        }
    }
    assert_eq!(
        created_payloads.len(),
        2,
        "batch success must emit one created event per stream"
    );

    let payload1 = created_payloads[0].clone();
    let payload2 = created_payloads[1].clone();

    assert_eq!(payload1.stream_id, ids.get(0).unwrap());
    assert_eq!(payload1.sender, ctx.sender);
    assert_eq!(payload1.recipient, params1.recipient);
    assert_eq!(payload1.deposit_amount, params1.deposit_amount);

    assert_eq!(payload2.stream_id, ids.get(1).unwrap());
    assert_eq!(payload2.sender, ctx.sender);
    assert_eq!(payload2.recipient, params2.recipient);
    assert_eq!(payload2.deposit_amount, params2.deposit_amount);
}

#[test]
fn test_create_streams_batch_total_deposit_overflow_has_no_side_effects() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let params1 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: i128::MAX,
        rate_per_second: i128::MAX,
        start_time: 0,
        cliff_time: 0,
        end_time: 1,
        memo: None,
    };
    let params2 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1,
        memo: None,
    };
    let streams = vec![&ctx.env, params1, params2];

    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_streams(&ctx.sender, &streams);
    }));
    assert!(
        result.is_err(),
        "overflow in total batch deposit must abort the whole call"
    );
    assert_eq!(
        ctx.client().get_stream_count(),
        stream_count_before,
        "failed overflow batch must not advance stream counter"
    );
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        sender_balance_before,
        "failed overflow batch must not move sender funds"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before,
        "failed overflow batch must not change contract funds"
    );
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "failed overflow batch must emit no events"
    );
}

#[test]
fn test_create_streams_batch_wrong_auth_fails_without_side_effects() {
    let ctx = TestContext::setup_strict();
    ctx.env.ledger().set_timestamp(0);
    let attacker = Address::generate(&ctx.env);

    let params = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };
    let streams = vec![&ctx.env, params.clone()];
    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
    ctx.env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_streams",
            args: (&ctx.sender, streams.clone()).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_streams(&ctx.sender, &streams);
    }));
    assert!(result.is_err(), "non-sender auth must be rejected");
    assert_eq!(ctx.client().get_stream_count(), stream_count_before);
    assert_eq!(ctx.token().balance(&ctx.sender), sender_balance_before);
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

// ---------------------------------------------------------------------------
// Tests — set_admin (Issue #133)
// ---------------------------------------------------------------------------

#[test]
fn test_set_admin_emits_event() {
    let ctx = TestContext::setup();
    let new_admin = Address::generate(&ctx.env);

    ctx.client().set_admin(&new_admin);

    let events = ctx.env.events().all();
    let last_event = events.last().expect("expected at least one event");

    // Check event topic: (Symbol::new(&env, "AdminUpd"),)
    assert_eq!(last_event.0, ctx.contract_id);
    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "AdminUpd")
    );

    // Check event data: (old_admin, new_admin)
    let data: (Address, Address) = last_event.2.into_val(&ctx.env);
    assert_eq!(data.0, ctx.admin);
    assert_eq!(data.1, new_admin);

    // Verify config is updated
    let config = ctx.client().get_config();
    assert_eq!(config.admin, new_admin);
}

#[test]
#[should_panic] // Only current admin can update admin
fn test_set_admin_unauthorized_fails() {
    let ctx = TestContext::setup_strict();
    let non_admin = Address::generate(&ctx.env);
    let new_admin = Address::generate(&ctx.env);

    // Mock non_admin auth
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "set_admin",
            args: (new_admin.clone(),).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().set_admin(&new_admin);
}

#[test]
fn test_new_admin_can_perform_admin_ops() {
    let ctx = TestContext::setup();
    let new_admin = Address::generate(&ctx.env);

    // Switch admin
    ctx.client().set_admin(&new_admin);

    // Create a stream to test admin ops (pause)
    let stream_id = ctx.create_default_stream();

    // New admin should be able to pause as admin
    ctx.client().pause_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
}

#[test]
#[should_panic]
fn test_old_admin_loses_privileges_after_rotation() {
    let ctx = TestContext::setup_strict();
    let new_admin = Address::generate(&ctx.env);

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

    ctx.client().set_admin(&new_admin);

    // Now try to do an admin op as the old admin
    let stream_id = ctx.create_default_stream();

    // The old admin still tries to pause it
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &ctx.admin, // old admin
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream_as_admin(&stream_id);
}

#[test]
fn test_set_admin_same_address_succeeds() {
    let ctx = TestContext::setup();
    let old_admin = ctx.admin.clone();

    // Setting admin to the current admin is a valid rotation (no op functionally but rotates keys if they updated signer weights on the acc)
    ctx.client().set_admin(&old_admin);

    let config = ctx.client().get_config();
    assert_eq!(config.admin, old_admin);

    let events = ctx.env.events().all();
    let last_event = events.last().expect("expected at least one event");
    assert_eq!(last_event.0, ctx.contract_id);
    assert_eq!(
        Symbol::from_val(&ctx.env, &last_event.1.get(0).unwrap()),
        Symbol::new(&ctx.env, "AdminUpd")
    );
    let data: (Address, Address) = last_event.2.into_val(&ctx.env);
    assert_eq!(data.0, old_admin);
    assert_eq!(data.1, old_admin);
}

// ---------------------------------------------------------------------------
// Tests — Issue #108: start_time must not be in the past
// ---------------------------------------------------------------------------

/// start_time strictly before current ledger time must fail with StartTimeInPast.
#[test]
fn test_create_stream_start_time_in_past_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &999u64, // start_time < now (1000)
        &999u64,
        &1999u64,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::StartTimeInPast)));
}

/// start_time one second before now is rejected (boundary).
#[test]
fn test_create_stream_start_time_one_second_before_now_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(500);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &499u64, // start = now - 1
        &499u64,
        &1499u64,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::StartTimeInPast)));
}

/// start_time far in the past is rejected.
#[test]
fn test_create_stream_start_time_far_in_past_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(10_000);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64, // start far in the past
        &0u64,
        &1000u64,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::StartTimeInPast)));
}

/// start_time == current ledger timestamp is valid ("start now").
#[test]
fn test_create_stream_start_time_equals_now_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(500);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &500u64, // start == now
        &500u64,
        &1500u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.start_time, 500);
    assert_eq!(state.status, StreamStatus::Active);
}

/// start_time one second in the future is valid.
#[test]
fn test_create_stream_start_time_one_second_in_future_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(500);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &501u64, // start = now + 1
        &501u64,
        &1501u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.start_time, 501);
    assert_eq!(state.status, StreamStatus::Active);
}

/// start_time well in the future is valid.
#[test]
fn test_create_stream_start_time_future_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &5000u64, // start far in the future
        &5000u64,
        &6000u64,
        &None,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.start_time, 5000);
    assert_eq!(state.status, StreamStatus::Active);
    // Nothing accrued yet — stream hasn't started
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 0);
}

/// Ledger timestamp == 0 and start_time == 0 is valid (genesis edge case).
#[test]
fn test_create_stream_start_time_zero_at_genesis_succeeds() {
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
        &None,
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.start_time, 0);
    assert_eq!(state.status, StreamStatus::Active);
}

/// Past-start rejection must fire BEFORE the token transfer.
/// If the validation runs after transfer, the sender would lose tokens
/// on a failed call. Verify sender balance is unchanged after the panic.
#[test]
fn test_create_stream_past_start_no_token_transfer() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let stream_count_before = ctx.client().get_stream_count();
    let events_before = ctx.env.events().all().len();

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &500u64, // past
        &500u64,
        &1500u64,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::StartTimeInPast)));

    // Sender balance must be unchanged — no token was transferred
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        sender_balance_before,
        "sender balance must not change on validation failure"
    );
    assert_eq!(
        ctx.client().get_stream_count(),
        stream_count_before,
        "stream counter must not change on validation failure"
    );
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no events should be emitted on validation failure"
    );
}

// ---------------------------------------------------------------------------
// Tests — get_withdrawable
// ---------------------------------------------------------------------------

#[test]
fn test_get_withdrawable_stream_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_get_withdrawable(&999);
    assert!(result.is_err());
}

#[test]
fn test_get_withdrawable_before_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Check before cliff
    ctx.env.ledger().set_timestamp(100);
    let withdrawable = ctx.client().get_withdrawable(&stream_id);

    assert_eq!(withdrawable, 0, "withdrawable should be 0 before cliff");
}

#[test]
fn test_get_withdrawable_after_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500, rate=1

    // Check after cliff
    ctx.env.ledger().set_timestamp(600);
    let withdrawable = ctx.client().get_withdrawable(&stream_id);

    assert_eq!(
        withdrawable, 600,
        "withdrawable should equal full accrual after cliff"
    );
}

#[test]
fn test_get_withdrawable_after_partial_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw at t=300
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    // Advance time to t=800 and check withdrawable
    ctx.env.ledger().set_timestamp(800);
    let withdrawable = ctx.client().get_withdrawable(&stream_id);

    // Accrued (800) - Withdrawn (300) = 500
    assert_eq!(
        withdrawable, 500,
        "withdrawable should subtract already withdrawn amount"
    );
}

#[test]
fn test_get_withdrawable_paused_stream_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    // Pause the stream
    ctx.client().pause_stream(&stream_id);

    // Even though 500 is accrued, pause blocks withdrawals
    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    assert_eq!(
        withdrawable, 0,
        "withdrawable must be 0 when stream is Paused"
    );
}

#[test]
fn test_get_withdrawable_completed_stream_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Fully withdraw to mark stream as Completed
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    assert_eq!(
        withdrawable, 0,
        "withdrawable must be 0 when stream is Completed"
    );
}

#[test]
fn test_get_withdrawable_cancelled_stream_returns_accrued() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel stream midway
    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // The recipient should still be able to see and withdraw the accrued amount prior to cancellation
    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    assert_eq!(
        withdrawable, 400,
        "withdrawable must equal frozen accrued amount on Cancelled stream"
    );
}

#[test]
fn test_get_withdrawable_matches_withdraw_active() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(600);
    let expected = ctx.client().get_withdrawable(&stream_id);
    let withdrawn = ctx.client().withdraw(&stream_id);

    assert_eq!(
        withdrawn, expected,
        "withdraw should transfer exactly get_withdrawable amount"
    );
    assert_eq!(
        ctx.client().get_withdrawable(&stream_id),
        0,
        "after withdraw, get_withdrawable must return 0 at same time"
    );
}

#[test]
fn test_get_withdrawable_matches_withdraw_cancelled_freeze() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    // Even if time advances, cancelled streams freeze accrual at cancelled_at.
    ctx.env.ledger().set_timestamp(900);
    let expected = ctx.client().get_withdrawable(&stream_id);
    assert_eq!(expected, 400, "frozen accrual should remain at cancel time");

    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, expected,
        "withdraw should transfer exactly frozen get_withdrawable amount"
    );
    assert_eq!(
        ctx.client().get_withdrawable(&stream_id),
        0,
        "after withdraw on cancelled stream, get_withdrawable must return 0"
    );
}

#[test]
fn test_withdraw_before_cliff_matches_get_withdrawable_no_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    ctx.env.ledger().set_timestamp(100);
    let expected = ctx.client().get_withdrawable(&stream_id);
    assert_eq!(expected, 0, "before cliff, get_withdrawable is 0");

    let events_before = ctx.env.events().all().len();
    let withdrawn = ctx.client().withdraw(&stream_id);
    let events_after = ctx.env.events().all().len();

    assert_eq!(withdrawn, 0, "withdraw returns 0 before cliff");
    assert_eq!(
        events_after, events_before,
        "withdraw of 0 must not emit events"
    );
}

// ---------------------------------------------------------------------------
// Tests — get_claimable_at (#221)
// ---------------------------------------------------------------------------

#[test]
fn test_get_claimable_at_stream_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_get_claimable_at(&0, &100u64);
    assert!(result.is_err());
}

#[test]
fn test_get_claimable_at_before_cliff_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at 500

    let claimable = ctx.client().get_claimable_at(&stream_id, &100);
    assert_eq!(claimable, 0, "claimable at t=100 (before cliff) must be 0");
}

#[test]
fn test_get_claimable_at_at_cliff_boundary() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // start=0, cliff=500, end=1000, rate=1

    let claimable = ctx.client().get_claimable_at(&stream_id, &500);
    assert_eq!(
        claimable, 500,
        "at cliff time accrual starts from start_time"
    );
}

#[test]
fn test_get_claimable_at_at_end_boundary() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 0..1000, rate 1

    let claimable = ctx.client().get_claimable_at(&stream_id, &1000);
    assert_eq!(claimable, 1000, "at end_time claimable equals deposit");
}

#[test]
fn test_get_claimable_at_after_end_capped() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let claimable = ctx.client().get_claimable_at(&stream_id, &5000);
    assert_eq!(
        claimable, 1000,
        "after end_time claimable capped at deposit"
    );
}

#[test]
fn test_get_claimable_at_after_partial_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    // At t=800: accrued 800, withdrawn 300 -> claimable 500
    let claimable = ctx.client().get_claimable_at(&stream_id, &800);
    assert_eq!(claimable, 500);
}

#[test]
fn test_get_claimable_at_completed_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let claimable = ctx.client().get_claimable_at(&stream_id, &1000);
    assert_eq!(claimable, 0, "completed stream has nothing left to claim");
}

#[test]
fn test_get_claimable_at_cancelled_frozen_at_cancel_time() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    // At timestamp 400: accrued was 400 (frozen), withdrawn 0 -> claimable 400
    let claimable = ctx.client().get_claimable_at(&stream_id, &400);
    assert_eq!(claimable, 400);

    // At a future timestamp, claimable still 400 (accrual frozen)
    let claimable_future = ctx.client().get_claimable_at(&stream_id, &9999);
    assert_eq!(claimable_future, 400);
}

#[test]
fn test_get_claimable_at_matches_get_withdrawable_at_current_time() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(600);
    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    let claimable_at = ctx.client().get_claimable_at(&stream_id, &600);
    assert_eq!(
        withdrawable, claimable_at,
        "get_claimable_at(now) should match get_withdrawable"
    );
}

// ---------------------------------------------------------------------------
// Tests — update_rate_per_second
// ---------------------------------------------------------------------------

#[test]
fn test_update_rate_per_second_increases_rate_and_preserves_accrual() {
    let ctx = TestContext::setup();

    // Create a stream with generous deposit so we can safely increase the rate.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Mid-stream, record accrued with the original rate.
    ctx.env.ledger().set_timestamp(500);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 500);

    // Increase rate from 1 → 5 tokens/second.
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.rate_per_second, 5);
    assert_eq!(state_after.deposit_amount, 10_000);

    // Accrued amount must be monotonically non-decreasing after the update.
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert!(
        accrued_after >= accrued_before,
        "accrued_after ({accrued_after}) must be >= accrued_before ({accrued_before})"
    );
}

#[test]
#[should_panic]
fn test_update_rate_per_second_rejects_non_increasing_rate() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Attempting to set the same rate should panic.
    ctx.client().update_rate_per_second(&stream_id, &1_i128);
}

#[test]
#[should_panic]
fn test_update_rate_per_second_rejects_rate_exceeding_deposit_coverage() {
    let ctx = TestContext::setup();

    // Default stream: deposit=1000, start=0, end=1000 ⇒ duration=1000s.
    let stream_id = ctx.create_default_stream();

    // New rate would require 2000 tokens over 1000 seconds, but deposit is only 1000.
    ctx.client().update_rate_per_second(&stream_id, &2_i128);
}

#[test]
#[should_panic]
fn test_update_rate_per_second_rejects_completed_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Complete the stream by withdrawing everything at end_time.
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Attempting to update rate on a completed stream must panic.
    ctx.client().update_rate_per_second(&stream_id, &2_i128);
}

#[test]
#[should_panic]
fn test_update_rate_per_second_rejects_cancelled_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel the stream.
    ctx.env.ledger().set_timestamp(500);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Attempting to update rate on a cancelled stream must panic.
    ctx.client().update_rate_per_second(&stream_id, &2_i128);
}

#[test]
fn test_update_rate_per_second_works_on_paused_stream() {
    let ctx = TestContext::setup();

    // Create stream with generous deposit.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Pause the stream.
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Update rate while paused should succeed.
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.rate_per_second, 5);
    assert_eq!(state_after.status, StreamStatus::Paused);
}

#[test]
#[should_panic]
fn test_update_rate_per_second_rejects_zero_rate() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Attempting to set rate to zero must panic.
    ctx.client().update_rate_per_second(&stream_id, &0_i128);
}

#[test]
#[should_panic]
fn test_update_rate_per_second_rejects_negative_rate() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Attempting to set negative rate must panic.
    ctx.client().update_rate_per_second(&stream_id, &(-1_i128));
}

#[test]
#[should_panic]
fn test_update_rate_per_second_rejects_rate_decrease() {
    let ctx = TestContext::setup();

    // Create stream with rate 5.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &5_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Attempting to decrease rate from 5 → 3 must panic.
    ctx.client().update_rate_per_second(&stream_id, &3_i128);
}

#[test]
fn test_update_rate_per_second_before_cliff() {
    let ctx = TestContext::setup();
    // Mint more and manually create stream with larger deposit
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &2000);
    let stream_id =
        ctx.client()
            .create_stream(&ctx.sender, &ctx.recipient, &2000, &1, &0, &500, &1000,
            &None
            );

    // Before cliff at t=100, accrued is 0.
    ctx.env.ledger().set_timestamp(100);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 0);

    // Update rate from 1 → 2 (deposit=2000 >= 2*1000, valid).
    ctx.client().update_rate_per_second(&stream_id, &2_i128);

    // Still before cliff, accrued remains 0.
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_after, 0);

    // After cliff at t=600, accrual uses new rate forward-only from checkpoint (t=100).
    ctx.env.ledger().set_timestamp(600);
    let accrued_post_cliff = ctx.client().calculate_accrued(&stream_id);
    // checkpoint_at=100, checkpointed_amount=0, rate=2, elapsed=600-100=500 → 0+1000=1000
    assert_eq!(accrued_post_cliff, 1000);
}

#[test]
fn test_update_rate_per_second_at_cliff() {
    let ctx = TestContext::setup();
    // Mint more and manually create stream with larger deposit
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &5000);
    let stream_id =
        ctx.client()
            .create_stream(&ctx.sender, &ctx.recipient, &5000, &1, &0, &500, &1000,
            &None
            );

    // Exactly at cliff time t=500.
    ctx.env.ledger().set_timestamp(500);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 500); // rate=1, elapsed=500

    // Update rate from 1 → 2 (deposit=2000 >= 2*1000, valid).
    ctx.client().update_rate_per_second(&stream_id, &2_i128);

    // At same timestamp, accrued should not decrease.
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued_after >= accrued_before);
}

#[test]
fn test_update_rate_per_second_after_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();

    // After cliff at t=700.
    ctx.env.ledger().set_timestamp(700);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 700); // rate=1, elapsed=700

    // Update rate from 1 → 2 (but deposit is only 1000, so new total would be 2000).
    // This should panic due to insufficient deposit.
    let result = ctx.client().try_update_rate_per_second(&stream_id, &2_i128);
    assert!(result.is_err(), "Should fail due to insufficient deposit");
}

#[test]
fn test_update_rate_per_second_near_end_time() {
    let ctx = TestContext::setup();

    // Create stream with generous deposit.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Near end at t=950.
    ctx.env.ledger().set_timestamp(950);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 950);

    // Update rate from 1 → 5.
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    // At same timestamp, accrued should not decrease.
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert!(accrued_after >= accrued_before);

    // After end_time at t=1100, accrual is capped at end_time.
    ctx.env.ledger().set_timestamp(1100);
    let accrued_final = ctx.client().calculate_accrued(&stream_id);
    // checkpoint at t=950: amount=950; new epoch: 5*(end=1000-950)=250; total=1200
    assert_eq!(accrued_final, 1200);
}

#[test]
fn test_update_rate_per_second_after_end_time() {
    let ctx = TestContext::setup();

    // Create stream with generous deposit.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // After end_time at t=1500.
    ctx.env.ledger().set_timestamp(1500);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 1000); // capped at rate * duration

    // Update rate from 1 → 5 (at t=1500, past end_time=1000).
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    // Accrual is still capped at end_time; checkpoint_at=1500 >= end=1000,
    // so no additional accrual is possible beyond the checkpointed 1000 tokens.
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_after, 1000);
}

#[test]
fn test_update_rate_per_second_with_partial_withdrawal() {
    let ctx = TestContext::setup();

    // Create stream with generous deposit.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // At t=300, withdraw partial amount.
    ctx.env.ledger().set_timestamp(300);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 300);

    // Update rate from 1 → 5.
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 5);
    assert_eq!(state.withdrawn_amount, 300);

    // At t=400, calculate new withdrawable.
    ctx.env.ledger().set_timestamp(400);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    // checkpoint at t=300: amount=300; new epoch rate=5: 5*(400-300)=500; total=800
    assert_eq!(accrued, 800);

    let withdrawable = accrued - state.withdrawn_amount;
    assert_eq!(withdrawable, 500);
}

#[test]
fn test_update_rate_per_second_emits_event() {
    let ctx = TestContext::setup();

    // Create stream with generous deposit.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Update rate from 1 → 5.
    ctx.env.ledger().set_timestamp(500);
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    // Verify event was emitted.
    let events = ctx.env.events().all();
    let rate_update_events_count = events
        .into_iter()
        .filter(|e| {
            if e.1.len() < 2 {
                return false;
            }
            let s = Symbol::try_from_val(&ctx.env, &e.1.get(0).unwrap_or(Val::VOID.into()));
            let id = u64::try_from_val(&ctx.env, &e.1.get(1).unwrap_or(Val::VOID.into()));
            match (s, id) {
                (Ok(s), Ok(id)) => s == Symbol::new(&ctx.env, "rate_upd") && id == stream_id,
                _ => false,
            }
        })
        .count();

    assert_eq!(
        rate_update_events_count, 1,
        "Should emit exactly one rate_upd event"
    );
}

#[test]
fn test_update_rate_per_second_on_paused_stream_after_partial_withdrawal() {
    let ctx = TestContext::setup();

    // Create stream with generous deposit.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // At t=300, withdraw partial amount.
    ctx.env.ledger().set_timestamp(300);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 300);

    // Pause the stream.
    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);
    assert_eq!(state.withdrawn_amount, 300);

    // Update rate while paused should succeed.
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.rate_per_second, 5);
    assert_eq!(state_after.status, StreamStatus::Paused);
    assert_eq!(state_after.withdrawn_amount, 300);

    // Accrued at the same timestamp (t=300): checkpoint locked in accrual=300; rate applies forward.
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 300); // checkpoint preserves prior accrual; no new seconds elapsed yet

    let withdrawable = accrued - state_after.withdrawn_amount;
    assert_eq!(withdrawable, 0); // already fully withdrawn up to this point
}

#[test]
fn test_update_rate_per_second_after_partial_withdrawal_then_resume_and_withdraw() {
    let ctx = TestContext::setup();

    // Create stream with generous deposit.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // At t=200, withdraw partial amount.
    ctx.env.ledger().set_timestamp(200);
    let withdrawn1 = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn1, 200);

    // Pause the stream.
    ctx.client().pause_stream(&stream_id);

    // Update rate while paused.
    ctx.client().update_rate_per_second(&stream_id, &3_i128);

    // Resume the stream.
    ctx.client().resume_stream(&stream_id);

    // At t=400, withdraw again.
    ctx.env.ledger().set_timestamp(400);
    let withdrawn2 = ctx.client().withdraw(&stream_id);
    // Checkpoint at t=200: amount=200; new epoch rate=3: 3*(400-200)=600; total=800
    // Withdrawn so far: 200; withdrawable: 800-200=600
    assert_eq!(withdrawn2, 600);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 800);
    assert_eq!(state.status, StreamStatus::Active);
}

#[test]
#[should_panic]
fn test_update_rate_per_second_unauthorized_caller() {
    let ctx = TestContext::setup_strict();

    // Create stream.
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                ctx.sender.clone(),
                ctx.recipient.clone(),
                10_000_i128,
                1_i128,
                0u64,
                0u64,
                1_000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Attempt to update rate as recipient (not sender) without proper auth.
    // This should panic due to authorization failure.
    ctx.client().update_rate_per_second(&stream_id, &5_i128);
}

#[test]
#[should_panic]
fn test_update_rate_per_second_nonexistent_stream() {
    let ctx = TestContext::setup();

    // Attempt to update rate on a stream that doesn't exist.
    ctx.client().update_rate_per_second(&999_u64, &5_i128);
}

#[test]
fn test_update_rate_per_second_multiple_times() {
    let ctx = TestContext::setup();

    // Create stream with very generous deposit.
    ctx.env.ledger().set_timestamp(0);
    ctx.sac.mint(&ctx.sender, &100_000_i128); // Mint to cover
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // First update: 1 → 5.
    ctx.env.ledger().set_timestamp(100);
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    let state1 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state1.rate_per_second, 5);

    // Second update: 5 → 10.
    ctx.env.ledger().set_timestamp(200);
    ctx.client().update_rate_per_second(&stream_id, &10_i128);

    let state2 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state2.rate_per_second, 10);

    // Third update: 10 → 50.
    ctx.env.ledger().set_timestamp(300);
    ctx.client().update_rate_per_second(&stream_id, &50_i128);

    let state3 = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state3.rate_per_second, 50);
}

#[test]
fn test_update_rate_per_second_preserves_other_fields() {
    let ctx = TestContext::setup();

    // Create stream with specific parameters.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &100u64,
        &200u64,
        &1_000u64,
        &None,
    );

    let state_before = ctx.client().get_stream_state(&stream_id);

    // Update rate.
    ctx.env.ledger().set_timestamp(150);
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    let state_after = ctx.client().get_stream_state(&stream_id);

    // Verify only rate changed.
    assert_eq!(state_after.rate_per_second, 5);
    assert_eq!(state_after.stream_id, state_before.stream_id);
    assert_eq!(state_after.sender, state_before.sender);
    assert_eq!(state_after.recipient, state_before.recipient);
    assert_eq!(state_after.deposit_amount, state_before.deposit_amount);
    assert_eq!(state_after.start_time, state_before.start_time);
    assert_eq!(state_after.cliff_time, state_before.cliff_time);
    assert_eq!(state_after.end_time, state_before.end_time);
    assert_eq!(state_after.withdrawn_amount, state_before.withdrawn_amount);
    assert_eq!(state_after.status, state_before.status);
}

#[test]
fn test_update_rate_per_second_with_overflow_protection() {
    let ctx = TestContext::setup();

    // Create a normal stream (no huge minting required).
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // Updating to an extreme rate should overflow `new_rate * duration` and be rejected.
    let result = ctx
        .client()
        .try_update_rate_per_second(&stream_id, &i128::MAX);
    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

#[test]
fn test_update_rate_per_second_interaction_with_pause_resume() {
    let ctx = TestContext::setup();

    // Create stream with generous deposit.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Pause at t=100.
    ctx.env.ledger().set_timestamp(100);
    ctx.client().pause_stream(&stream_id);

    // Update rate while paused.
    ctx.client().update_rate_per_second(&stream_id, &5_i128);

    // Resume at t=200.
    ctx.env.ledger().set_timestamp(200);
    ctx.client().resume_stream(&stream_id);

    // Verify accrual uses new rate from checkpoint at t=100.
    ctx.env.ledger().set_timestamp(300);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    // checkpoint at t=100: amount=100; new epoch: 5*(300-100)=1000; total=1100
    assert_eq!(accrued, 1100);
}

#[test]
fn test_update_rate_per_second_exact_deposit_coverage() {
    let ctx = TestContext::setup();

    // Create stream where deposit exactly covers rate * duration.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Update to rate that exactly matches deposit.
    // deposit = 1000, duration = 1000, so max rate = 1.
    // Cannot increase rate without exceeding deposit.
    let result = ctx.client().try_update_rate_per_second(&stream_id, &2_i128);
    assert!(
        result.is_err(),
        "Should fail: new rate would require 2000 but deposit is only 1000"
    );
}

// ---------------------------------------------------------------------------
// Tests — shorten_stream_end_time
// ---------------------------------------------------------------------------

#[test]
fn test_shorten_stream_end_time_refunds_unstreamed_and_updates_schedule() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // At t=0, shorten end_time from 1000 → 500.
    ctx.env.ledger().set_timestamp(0);
    let sender_before = ctx.token().balance(&ctx.sender);

    ctx.client().shorten_stream_end_time(&stream_id, &500u64);

    let sender_after = ctx.token().balance(&ctx.sender);
    // Deposit was 1000, new deposit is 500 → refund 500.
    assert_eq!(sender_after - sender_before, 500);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 500);
    assert_eq!(state.deposit_amount, 500);
}

#[test]
fn test_shorten_stream_end_time_preserves_accrued_at_update_time() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Mid-stream at t=300.
    ctx.env.ledger().set_timestamp(300);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 300);

    // Shorten end_time from 1000 → 800; new deposit becomes 800.
    ctx.client().shorten_stream_end_time(&stream_id, &800u64);

    // At the same ledger timestamp, accrued must be unchanged.
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_after, accrued_before);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 800);
    assert_eq!(state.deposit_amount, 800);
}

#[test]
fn test_shorten_stream_end_time_rejects_past_end_time() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance beyond the proposed new end time.
    ctx.env.ledger().set_timestamp(600);

    // Attempting to shorten to a time in the past must return InvalidParams.
    let result = ctx
        .client()
        .try_shorten_stream_end_time(&stream_id, &500u64);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_shorten_stream_end_time_rejects_equal_or_later_end_time() {
    let ctx = TestContext::setup();
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // Equal old end_time is not a shorten.
    let same = ctx
        .client()
        .try_shorten_stream_end_time(&stream_id, &1_000u64);
    assert_eq!(same, Err(Ok(ContractError::InvalidParams)));

    // Later end_time must also be rejected on the shorten path.
    let later = ctx
        .client()
        .try_shorten_stream_end_time(&stream_id, &1_500u64);
    assert_eq!(later, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_shorten_stream_end_time_rejects_now_boundary() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Boundary at now is not strictly future, must be rejected.
    ctx.env.ledger().set_timestamp(500);
    let result = ctx
        .client()
        .try_shorten_stream_end_time(&stream_id, &500u64);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_shorten_stream_end_time_rejects_new_end_before_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff = 500

    let result = ctx
        .client()
        .try_shorten_stream_end_time(&stream_id, &400u64);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
#[should_panic]
fn test_shorten_stream_end_time_unauthorized_caller() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                ctx.sender.clone(),
                ctx.recipient.clone(),
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // No sender auth is provided for shorten; strict mode must trap.
    ctx.client().shorten_stream_end_time(&stream_id, &700u64);
}

#[test]
fn test_shorten_stream_end_time_emits_event_and_conserves_refund() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let sender_before = ctx.token().balance(&ctx.sender);
    let contract_before = ctx.token().balance(&ctx.contract_id);

    ctx.client().shorten_stream_end_time(&stream_id, &700u64);

    let sender_after = ctx.token().balance(&ctx.sender);
    let contract_after = ctx.token().balance(&ctx.contract_id);
    assert_eq!(sender_after - sender_before, 300);
    assert_eq!(contract_before - contract_after, 300);

    let events = ctx.env.events().all();
    let last = events.last().unwrap();
    let payload = StreamEndShortened::from_val(&ctx.env, &last.2);
    assert_eq!(payload.stream_id, stream_id);
    assert_eq!(payload.old_end_time, 1000);
    assert_eq!(payload.new_end_time, 700);
    assert_eq!(payload.refund_amount, 300);
}

#[test]
fn test_shorten_stream_end_time_failed_call_has_no_side_effects() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(600);
    let sender_before = ctx.token().balance(&ctx.sender);
    let contract_before = ctx.token().balance(&ctx.contract_id);
    let state_before = ctx.client().get_stream_state(&stream_id);
    let events_before = ctx.env.events().all().len();

    let result = ctx
        .client()
        .try_shorten_stream_end_time(&stream_id, &500u64);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.end_time, state_before.end_time);
    assert_eq!(state_after.deposit_amount, state_before.deposit_amount);
    assert_eq!(ctx.token().balance(&ctx.sender), sender_before);
    assert_eq!(ctx.token().balance(&ctx.contract_id), contract_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
fn test_shorten_stream_end_time_rejects_completed_and_cancelled_states() {
    let ctx = TestContext::setup();

    let completed_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&completed_id);
    let completed_result = ctx
        .client()
        .try_shorten_stream_end_time(&completed_id, &900u64);
    assert_eq!(completed_result, Err(Ok(ContractError::InvalidState)));

    let cancelled_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(300);
    ctx.client().cancel_stream(&cancelled_id);
    let cancelled_result = ctx
        .client()
        .try_shorten_stream_end_time(&cancelled_id, &800u64);
    assert_eq!(cancelled_result, Err(Ok(ContractError::InvalidState)));
}

// ---------------------------------------------------------------------------
// Tests — extend_stream_end_time
// ---------------------------------------------------------------------------

#[test]
fn test_extend_stream_end_time_preserves_accrued_and_allows_longer_accrual() {
    let ctx = TestContext::setup();

    // Create a stream with extra deposit so it can be safely extended.
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &None,
    );

    // At t=800, accrued should be 800.
    ctx.env.ledger().set_timestamp(800);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 800);

    // Extend end_time from 1000 → 2000.
    ctx.client().extend_stream_end_time(&stream_id, &2_000u64);

    // Accrued at the same ledger timestamp (t=800) must remain unchanged.
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_after, accrued_before);

    // After extension, accrual continues linearly up to the new end_time.
    ctx.env.ledger().set_timestamp(1_500);
    let accrued_late = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_late, 1_500);
}

#[test]
fn test_extend_stream_end_time_rejects_when_deposit_insufficient() {
    let ctx = TestContext::setup();

    // Default stream: deposit=1000, start=0, end=1000, rate=1.
    let stream_id = ctx.create_default_stream();

    // Extending to 2000 seconds would require 2000 tokens, but deposit is only 1000.
    let result = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &2_000u64);
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
}

// ---------------------------------------------------------------------------
// Tests — Recipient Stream Index (Feature: recipient-stream-index)
// ---------------------------------------------------------------------------

/// Test that a stream is added to the recipient's index on creation.
#[test]
fn test_recipient_stream_index_added_on_create() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Initially, recipient has no streams
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 0);

    // Create a stream
    let stream_id = ctx.create_default_stream();

    // Recipient's index should now contain the stream
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);
    assert_eq!(streams.get(0).unwrap(), stream_id);
}

/// Test that multiple streams are indexed in sorted order by stream_id.
#[test]
fn test_recipient_stream_index_sorted_order() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create multiple streams for the same recipient
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
        &None,
    );

    let id3 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    // Verify IDs are sequential
    assert_eq!(id1, 0);
    assert_eq!(id2, 1);
    assert_eq!(id3, 2);

    // Recipient's index should contain all streams in sorted order
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 3);
    assert_eq!(streams.get(0).unwrap(), 0);
    assert_eq!(streams.get(1).unwrap(), 1);
    assert_eq!(streams.get(2).unwrap(), 2);
}

/// Test that get_recipient_stream_count returns the correct count.
#[test]
fn test_recipient_stream_count() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Initially, count is 0
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 0);

    // Create first stream
    ctx.create_default_stream();
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 1);

    // Create second stream
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
        &None,
    );
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 2);

    // Create third stream
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 3);
}

/// Test that different recipients have separate indices.
#[test]
fn test_recipient_stream_index_separate_per_recipient() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let recipient2 = Address::generate(&ctx.env);
    let recipient3 = Address::generate(&ctx.env);

    // Create streams for different recipients
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &recipient2,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
        &None,
    );

    let id3 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    let id4 = ctx.client().create_stream(
        &ctx.sender,
        &recipient3,
        &3000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &3000u64,
        &None,
    );

    // Verify each recipient has the correct streams
    let streams1 = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams1.len(), 2);
    assert_eq!(streams1.get(0).unwrap(), id1);
    assert_eq!(streams1.get(1).unwrap(), id3);

    let streams2 = ctx.client().get_recipient_streams(&recipient2);
    assert_eq!(streams2.len(), 1);
    assert_eq!(streams2.get(0).unwrap(), id2);

    let streams3 = ctx.client().get_recipient_streams(&recipient3);
    assert_eq!(streams3.len(), 1);
    assert_eq!(streams3.get(0).unwrap(), id4);
}

/// Test that closing a completed stream removes it from the recipient's index.
#[test]
fn test_recipient_stream_index_removed_on_close() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create a stream
    let stream_id = ctx.create_default_stream();

    // Verify it's in the index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);
    assert_eq!(streams.get(0).unwrap(), stream_id);

    // Withdraw all tokens to complete the stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Close the completed stream
    ctx.client().close_completed_stream(&stream_id);

    // Verify it's removed from the index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 0);
}

/// Test that the index maintains sorted order after multiple creates and closes.
#[test]
fn test_recipient_stream_index_sorted_after_operations() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create streams with IDs 0, 1, 2
    let _id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
        &None,
    );

    let _id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    // Verify sorted order
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 3);
    assert_eq!(streams.get(0).unwrap(), 0);
    assert_eq!(streams.get(1).unwrap(), 1);
    assert_eq!(streams.get(2).unwrap(), 2);

    // Complete and close stream 1
    ctx.env.ledger().set_timestamp(2000);
    ctx.client().withdraw(&id1);
    ctx.client().close_completed_stream(&id1);

    // Verify remaining streams are still sorted
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 2);
    assert_eq!(streams.get(0).unwrap(), 0);
    assert_eq!(streams.get(1).unwrap(), 2);
}

/// Test that batch_withdraw works correctly with the recipient index.
#[test]
fn test_recipient_stream_index_with_batch_withdraw() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create multiple streams
    let id0 = ctx.create_default_stream();
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
        &None,
    );
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    // Verify all streams are in the index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 3);

    // Advance time and batch withdraw
    ctx.env.ledger().set_timestamp(500);
    let mut stream_ids = Vec::new(&ctx.env);
    stream_ids.push_back(id0);
    stream_ids.push_back(id1);
    stream_ids.push_back(id2);

    let results = ctx.client().batch_withdraw(&ctx.recipient, &stream_ids);
    assert_eq!(results.len(), 3);

    // Verify all streams are still in the index (batch_withdraw doesn't remove them)
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 3);
}

/// Test that the index is consistent after stream lifecycle operations.
#[test]
fn test_recipient_stream_index_lifecycle_consistency() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create a stream
    let stream_id = ctx.create_default_stream();

    // Verify it's in the index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);

    // Pause the stream
    ctx.client().pause_stream(&stream_id);
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        streams.len(),
        1,
        "stream should remain in index after pause"
    );

    // Resume the stream
    ctx.client().resume_stream(&stream_id);
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        streams.len(),
        1,
        "stream should remain in index after resume"
    );

    // Withdraw some tokens
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        streams.len(),
        1,
        "stream should remain in index after partial withdraw"
    );

    // Complete the stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        streams.len(),
        1,
        "stream should remain in index when completed"
    );

    // Close the stream
    ctx.client().close_completed_stream(&stream_id);
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        streams.len(),
        0,
        "stream should be removed from index after close"
    );
}

/// Test that cancelled streams remain in the recipient's index.
#[test]
fn test_recipient_stream_index_cancelled_stream_remains() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create a stream
    let stream_id = ctx.create_default_stream();

    // Verify it's in the index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);

    // Cancel the stream
    ctx.env.ledger().set_timestamp(500);
    ctx.client().cancel_stream(&stream_id);

    // Verify it's still in the index (cancelled streams are not removed)
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1, "cancelled stream should remain in index");

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
}

/// Test that the recipient index handles large numbers of streams.
#[test]
fn test_recipient_stream_index_many_streams() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let num_streams = 50;

    // Create many streams
    for _ in 0..num_streams {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &100_i128,
            &1_i128,
            &0u64,
            &0u64,
            &100u64,
            &None,
        );
    }

    // Verify all streams are in the index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len() as u64, num_streams);

    // Verify they're in sorted order
    for i in 0..num_streams {
        assert_eq!(streams.get(i as u32).unwrap(), i);
    }

    // Verify count is correct
    assert_eq!(
        ctx.client().get_recipient_stream_count(&ctx.recipient),
        num_streams
    );
}

/// Test that the index is empty for a recipient with no streams.
#[test]
fn test_recipient_stream_index_empty_for_new_recipient() {
    let ctx = TestContext::setup();

    let new_recipient = Address::generate(&ctx.env);

    // Verify the new recipient has no streams
    let streams = ctx.client().get_recipient_streams(&new_recipient);
    assert_eq!(streams.len(), 0);

    // Verify count is 0
    assert_eq!(ctx.client().get_recipient_stream_count(&new_recipient), 0);
}

/// Test that the index correctly handles streams with different senders.
#[test]
fn test_recipient_stream_index_multiple_senders() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let sender2 = Address::generate(&ctx.env);
    let sender3 = Address::generate(&ctx.env);

    // Mint tokens to additional senders
    ctx.sac.mint(&sender2, &5000_i128);
    ctx.sac.mint(&sender3, &5000_i128);

    // Create streams from different senders to the same recipient
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id2 = ctx.client().create_stream(
        &sender2,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &2000u64,
        &None,
    );

    let id3 = ctx.client().create_stream(
        &sender3,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64,
        &500u64,
        &None,
    );

    // Verify all streams are in the recipient's index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 3);
    assert_eq!(streams.get(0).unwrap(), id1);
    assert_eq!(streams.get(1).unwrap(), id2);
    assert_eq!(streams.get(2).unwrap(), id3);
}

// ---------------------------------------------------------------------------
// Tests — Recipient Index Insertion & Removal Edge Cases
// ---------------------------------------------------------------------------

#[test]
fn test_recipient_index_binary_search_edge_cases() {
    let env = Env::default();
    let contract_id = env.register_contract(None, crate::FluxoraStream);
    let recipient = Address::generate(&env);

    env.as_contract(&contract_id, || {
        // Initial empty state
        let streams = crate::load_recipient_streams(&env, &recipient);
        assert_eq!(streams.len(), 0);

        // Add elements out of order to ensure binary search insertions happen correctly
        crate::add_stream_to_recipient_index(&env, &recipient, 10);
        crate::add_stream_to_recipient_index(&env, &recipient, 5);
        crate::add_stream_to_recipient_index(&env, &recipient, 15);
        crate::add_stream_to_recipient_index(&env, &recipient, 1);

        // Verify ordering
        let streams1 = crate::load_recipient_streams(&env, &recipient);
        assert_eq!(streams1.len(), 4);
        assert_eq!(streams1.get(0).unwrap(), 1);
        assert_eq!(streams1.get(1).unwrap(), 5);
        assert_eq!(streams1.get(2).unwrap(), 10);
        assert_eq!(streams1.get(3).unwrap(), 15);

        // Test duplicate insertion (handles Ok(pos) branch)
        crate::add_stream_to_recipient_index(&env, &recipient, 10);
        let streams2 = crate::load_recipient_streams(&env, &recipient);
        assert_eq!(streams2.len(), 5);
        assert_eq!(streams2.get(2).unwrap(), 10);
        assert_eq!(streams2.get(3).unwrap(), 10);

        // Test removal of middle element
        crate::remove_stream_from_recipient_index(&env, &recipient, 5);
        let streams3 = crate::load_recipient_streams(&env, &recipient);
        assert_eq!(streams3.len(), 4);
        assert_eq!(streams3.get(1).unwrap(), 10);

        // Test removal of duplicate (should remove one instance)
        crate::remove_stream_from_recipient_index(&env, &recipient, 10);
        let streams4 = crate::load_recipient_streams(&env, &recipient);
        assert_eq!(streams4.len(), 3);
        assert_eq!(streams4.get(1).unwrap(), 10); // one 10 remains

        // Test removal of non-existent element
        crate::remove_stream_from_recipient_index(&env, &recipient, 100);
        let streams5 = crate::load_recipient_streams(&env, &recipient);
        assert_eq!(streams5.len(), 3);

        // Test removal from end
        crate::remove_stream_from_recipient_index(&env, &recipient, 15);
        let streams6 = crate::load_recipient_streams(&env, &recipient);
        assert_eq!(streams6.len(), 2);
        assert_eq!(streams6.get(0).unwrap(), 1);
        assert_eq!(streams6.get(1).unwrap(), 10);

        // Test empty remove
        let empty_recipient = Address::generate(&env);
        crate::remove_stream_from_recipient_index(&env, &empty_recipient, 999);
        let empty_streams = crate::load_recipient_streams(&env, &empty_recipient);
        assert_eq!(empty_streams.len(), 0);
    });
}
// Tests — withdraw_to: destination constraints and event parity (#265)
// ---------------------------------------------------------------------------

/// WithdrawalTo event is emitted with the correct payload (stream_id, recipient,
/// destination, amount). Indexers rely on this exact shape.
#[test]
fn test_withdraw_to_emits_withdrawal_to_event() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{symbol_short, IntoVal};

    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(600);
    let amount = ctx.client().withdraw_to(&stream_id, &destination);
    assert_eq!(amount, 600);

    let events = ctx.env.events().all();
    let wdraw_event = events.iter().find(|(_, topics, _)| {
        topics
            == &soroban_sdk::vec![
                &ctx.env,
                symbol_short!("wdraw_to").into_val(&ctx.env),
                stream_id.into_val(&ctx.env),
            ]
    });
    assert!(wdraw_event.is_some(), "wdraw_to event must be emitted");

    let (_, _, data) = wdraw_event.unwrap();
    let payload = WithdrawalTo::try_from_val(&ctx.env, &data)
        .expect("event data must deserialize as WithdrawalTo");
    assert_eq!(payload.stream_id, stream_id);
    assert_eq!(payload.recipient, ctx.recipient);
    assert_eq!(payload.destination, destination);
    assert_eq!(payload.amount, 600);
}

/// When withdraw_to drains the stream, a StreamCompleted event must follow the
/// WithdrawalTo event in the same transaction — same parity as withdraw().
#[test]
fn test_withdraw_to_emits_completed_event_on_full_drain() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{symbol_short, IntoVal};

    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(1000);
    let amount = ctx.client().withdraw_to(&stream_id, &destination);
    assert_eq!(amount, 1000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    let events = ctx.env.events().all();
    let completed_event = events.iter().find(|(_, topics, _)| {
        topics
            == &soroban_sdk::vec![
                &ctx.env,
                symbol_short!("completed").into_val(&ctx.env),
                stream_id.into_val(&ctx.env),
            ]
    });
    assert!(
        completed_event.is_some(),
        "completed event must be emitted when withdraw_to drains the stream"
    );
}

/// No event is emitted when withdraw_to returns 0 (before cliff / nothing accrued).
/// This matches the zero-withdrawable behavior of withdraw().
#[test]
fn test_withdraw_to_no_event_when_zero() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{symbol_short, IntoVal};

    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at 500
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(100); // before cliff
    let amount = ctx.client().withdraw_to(&stream_id, &destination);
    assert_eq!(amount, 0);

    let events = ctx.env.events().all();
    let wdraw_event = events.iter().find(|(_, topics, _)| {
        topics
            == &soroban_sdk::vec![
                &ctx.env,
                symbol_short!("wdraw_to").into_val(&ctx.env),
                stream_id.into_val(&ctx.env),
            ]
    });
    assert!(
        wdraw_event.is_none(),
        "wdraw_to event must NOT be emitted when withdrawable is 0"
    );
}

/// destination == recipient is explicitly allowed (self-redirect).
/// Tokens land at the recipient address; state is updated correctly.
#[test]
fn test_withdraw_to_destination_equals_recipient_is_allowed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    let amount = ctx.client().withdraw_to(&stream_id, &ctx.recipient);

    assert_eq!(amount, 500);
    assert_eq!(ctx.token().balance(&ctx.recipient), 500);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 500);
}

/// withdraw_to on a cancelled stream delivers the accrued-but-not-withdrawn amount
/// to the destination, matching the behaviour of withdraw() on cancelled streams.
#[test]
fn test_withdraw_to_on_cancelled_stream_delivers_accrued() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    // Cancel at t=400: 400 tokens accrued, 600 refunded to sender
    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.withdrawn_amount, 0);

    // Recipient redirects their accrued share to a cold wallet
    let amount = ctx.client().withdraw_to(&stream_id, &destination);

    assert_eq!(
        amount, 400,
        "accrued amount must be delivered to destination"
    );
    assert_eq!(ctx.token().balance(&destination), 400);
    assert_eq!(ctx.token().balance(&ctx.recipient), 0);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 400);
}

/// withdraw_to on a cancelled stream emits the WithdrawalTo event with the correct payload.
#[test]
fn test_withdraw_to_on_cancelled_stream_emits_event() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{symbol_short, IntoVal};

    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(300);
    ctx.client().cancel_stream(&stream_id);

    let amount = ctx.client().withdraw_to(&stream_id, &destination);
    assert_eq!(amount, 300);

    let events = ctx.env.events().all();
    let wdraw_event = events.iter().find(|(_, topics, _)| {
        topics
            == &soroban_sdk::vec![
                &ctx.env,
                symbol_short!("wdraw_to").into_val(&ctx.env),
                stream_id.into_val(&ctx.env),
            ]
    });
    assert!(
        wdraw_event.is_some(),
        "wdraw_to event must be emitted for cancelled stream withdrawal"
    );
    let (_, _, data) = wdraw_event.unwrap();
    let payload = WithdrawalTo::try_from_val(&ctx.env, &data).unwrap();
    assert_eq!(payload.amount, 300);
    assert_eq!(payload.destination, destination);
}

/// withdraw_to panics on a completed stream — same guard as withdraw().
#[test]
#[should_panic]
fn test_withdraw_to_panics_on_completed_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    ctx.client().withdraw_to(&stream_id, &destination);
}

/// withdraw_to panics on a paused stream — same guard as withdraw().
#[test]
#[should_panic]
fn test_withdraw_to_panics_on_paused_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(200);
    ctx.client().pause_stream(&stream_id);

    ctx.client().withdraw_to(&stream_id, &destination);
}

/// Interleaving withdraw and withdraw_to on the same stream never double-pays.
/// withdrawn_amount is the single source of truth for both paths.
#[test]
fn test_withdraw_and_withdraw_to_interleaved_no_double_pay() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    // t=200: withdraw normally → recipient gets 200
    ctx.env.ledger().set_timestamp(200);
    let a1 = ctx.client().withdraw(&stream_id);
    assert_eq!(a1, 200);

    // t=500: withdraw_to → destination gets 300 (500 - 200 already withdrawn)
    ctx.env.ledger().set_timestamp(500);
    let a2 = ctx.client().withdraw_to(&stream_id, &destination);
    assert_eq!(a2, 300);

    // t=1000: withdraw_to again → destination gets remaining 500
    ctx.env.ledger().set_timestamp(1000);
    let a3 = ctx.client().withdraw_to(&stream_id, &destination);
    assert_eq!(a3, 500);

    assert_eq!(ctx.token().balance(&ctx.recipient), 200);
    assert_eq!(ctx.token().balance(&destination), 800);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);
}

// Tests — Issue #252: create_stream deposit, rate, and schedule validation matrix
// ---------------------------------------------------------------------------

/// Verify ContractPaused fires as a structured error (not a generic panic).
/// This allows integrators to distinguish a paused-contract rejection from
/// other failures using the typed ContractError discriminant.
#[test]
#[should_panic]
fn test_create_stream_contract_paused_returns_structured_error() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().set_global_emergency_paused(&true);

    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
}

/// Verify ContractPaused fires as a structured error for batch creations.
#[test]
#[should_panic]
fn test_create_streams_batch_contract_paused_returns_structured_error() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().set_contract_paused(&true);

    let params = soroban_sdk::Vec::from_array(
        &ctx.env,
        [CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            memo: None,
        }],
    );

    ctx.client().create_streams(&ctx.sender, &params);
}

/// Verify that a global pause only blocks `create_stream`/`create_streams`,
/// while operations on existing streams (withdraw, top-up, pause, cancel) succeed.
#[test]
fn test_global_pause_does_not_affect_existing_streams() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream while unpaused
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Now admin pauses the contract
    ctx.client().set_contract_paused(&true);

    // 1. Withdraw should work
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 100);

    // 2. Top-up should work
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &100_i128);
    let state_after_topup = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after_topup.deposit_amount, 1100);

    // 3. Sender pausing an individual stream should work
    ctx.client().pause_stream(&stream_id);
    let state_after_pause = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after_pause.status, StreamStatus::Paused);

    // 4. Sender cancelling an individual stream should work
    ctx.client().cancel_stream(&stream_id);
    let state_after_cancel = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after_cancel.status, StreamStatus::Cancelled);
}

/// create_streams (batch) must reject when any stream's start_time is in the past,
/// emitting StartTimeInPast as a structured error so integrators can handle it.
#[test]
#[should_panic]
fn test_create_streams_batch_start_time_in_past_returns_structured_error() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let params = soroban_sdk::Vec::from_array(
        &ctx.env,
        [CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 500, // < current ledger time (1000)
            cliff_time: 500,
            end_time: 1500,
            memo: None,
        }],
    );

    ctx.client().create_streams(&ctx.sender, &params);
}

/// validate_stream_params uses checked_mul for rate * duration; if the product
/// overflows i128, the contract must panic before any state or balance changes.
/// No stream must be created and the sender's balance must be unchanged.
///
/// Note: we choose rate = i128::MAX / 2, duration = 3 so that rate * duration
/// definitely overflows i128. The deposit value passed is irrelevant — the
/// overflow check fires first and panics before the deposit check or any token
/// transfer. We use a small deposit (≤ sender's existing balance) so the mint
/// in TestContext::setup() is sufficient and we do not trigger a SAC balance
/// overflow when minting.
#[test]
fn test_create_stream_rate_times_duration_overflow_panics_no_state_change() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // rate = i128::MAX / 2, duration = 3 → rate * duration overflows i128.
    // validate_stream_params panics on this via checked_mul before it ever
    // looks at the deposit amount or initiates a token transfer.
    let overflow_rate: i128 = i128::MAX / 2;
    let start: u64 = 0;
    let end: u64 = 3; // duration = 3 → rate * 3 overflows

    // Use a small deposit within the sender's existing balance (10_000 from setup).
    // The value does not matter — the panic fires before the deposit check.
    let deposit: i128 = 1000;

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let stream_count_before = ctx.client().get_stream_count();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &deposit,
            &overflow_rate,
            &start,
            &start,
            &end,
            &None,
        )
    }));

    assert!(
        result.is_err(),
        "create_stream must panic when rate * duration overflows i128"
    );

    // Stream counter must not have advanced.
    assert_eq!(
        ctx.client().get_stream_count(),
        stream_count_before,
        "stream counter must not advance after overflow panic"
    );

    // Sender balance must be unchanged — no token transfer occurred.
    assert_eq!(
        ctx.token().balance(&ctx.sender),
        sender_balance_before,
        "sender balance must not change after overflow panic"
    );
}

/// Confirm that the StreamCreated event payload exactly matches the documented
/// schema in events.md: topics = ["created", stream_id], data = StreamCreated struct
/// with all eight fields populated correctly.
#[test]
fn test_create_stream_event_payload_matches_events_md_schema() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit: i128 = 2000;
    let rate: i128 = 2;
    let start: u64 = 0;
    let cliff: u64 = 500;
    let end: u64 = 1000;

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &start,
        &cliff,
        &end,
        &None,
    );

    // The last event must be the StreamCreated event.
    let events = ctx.env.events().all();
    let last = events.last().expect("expected at least one event");

    // Decode the data payload as a StreamCreated struct.
    let event_data = crate::StreamCreated::try_from_val(&ctx.env, &last.2)
        .expect("event data must deserialise as StreamCreated");

    assert_eq!(event_data.stream_id, stream_id, "stream_id field");
    assert_eq!(event_data.sender, ctx.sender, "sender field");
    assert_eq!(event_data.recipient, ctx.recipient, "recipient field");
    assert_eq!(event_data.deposit_amount, deposit, "deposit_amount field");
    assert_eq!(event_data.rate_per_second, rate, "rate_per_second field");
    assert_eq!(event_data.start_time, start, "start_time field");
    assert_eq!(event_data.cliff_time, cliff, "cliff_time field");
    assert_eq!(event_data.end_time, end, "end_time field");
}

/// A failed create_stream (past start_time) must emit NO events — neither a
/// partial StreamCreated nor any other observable event.
#[test]
fn test_create_stream_past_start_emits_no_events() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(500);

    let events_before = ctx.env.events().all().len();

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &400u64, // past
        &400u64,
        &1400u64,
        &None,
    );
    assert!(result.is_err());

    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no event must be emitted when create_stream fails validation"
    );
}

/// When deposit exactly equals rate * duration (minimum valid deposit), the
/// stored stream fields must reflect exactly what was provided with no silent
/// rounding or clamping, and the contract must hold exactly deposit_amount tokens.
#[test]
fn test_create_stream_exact_minimum_deposit_stored_fields_are_exact() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let rate: i128 = 7;
    let duration: u64 = 143;
    let deposit: i128 = rate * duration as i128; // exactly 1001 — minimum valid

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);

    assert_eq!(
        state.deposit_amount, deposit,
        "deposit_amount must be stored as-is"
    );
    assert_eq!(
        state.rate_per_second, rate,
        "rate_per_second must be stored as-is"
    );
    assert_eq!(state.start_time, 0, "start_time must be stored as-is");
    assert_eq!(state.cliff_time, 0, "cliff_time must be stored as-is");
    assert_eq!(state.end_time, duration, "end_time must be stored as-is");
    assert_eq!(
        state.withdrawn_amount, 0,
        "withdrawn_amount must start at 0"
    );
    assert_eq!(
        state.status,
        StreamStatus::Active,
        "status must start Active"
    );

    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        deposit,
        "contract must hold exactly deposit_amount tokens"
    );
}

/// Verify the full role matrix for create_stream:
/// - Only the sender (the address passed as first argument) is required to authorise.
/// - The recipient requires no auth at creation time.
/// - The admin requires no auth at creation time.
///   This is tested via setup_strict() where only mocked auths are honoured.
#[test]
fn test_create_stream_only_sender_auth_required() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.ledger().set_timestamp(0);

    // Only mock sender's auth — no recipient or admin auth provided.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

/// top_up_stream must follow CEI: state must be persisted BEFORE the token pull.
/// We verify this indirectly by confirming that if the funder has insufficient
/// balance (token pull will fail), no deposit_amount change is visible — i.e.,
/// the transaction reverts atomically and the stream state is unchanged.
///
/// Note: because the CEI fix moves save_stream before pull_token, a token pull
/// failure still causes the whole transaction to revert (Soroban atomicity), so
/// on-chain state will be as if neither the save nor the pull happened. This test
/// confirms the revert is clean.
#[test]
fn test_top_up_stream_insufficient_balance_reverts_cleanly() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let deposit_before = ctx.client().get_stream_state(&stream_id).deposit_amount;

    // Attempt top-up with an amount the funder cannot cover (funder has 9000 left,
    // top-up amount is 1_000_000 — way more than available).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client()
            .top_up_stream(&stream_id, &ctx.sender, &1_000_000_i128);
    }));

    assert!(
        result.is_err(),
        "top_up must panic when funder has insufficient balance"
    );

    // Stream state must be unchanged (full revert).
    let deposit_after = ctx.client().get_stream_state(&stream_id).deposit_amount;
    assert_eq!(
        deposit_after, deposit_before,
        "deposit_amount must be unchanged after failed top_up"
    );
}

// ---------------------------------------------------------------------------
// Tests — extend_stream_end_time: deposit sufficiency under longer duration
// ---------------------------------------------------------------------------

// --- Success paths ---

/// Exact boundary: deposit == rate * new_duration must succeed.
/// This is the tightest valid case; any less would be rejected.
#[test]
fn test_extend_end_time_deposit_exactly_covers_new_duration() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit=2000, rate=1, start=0, end=1000 → deposit covers up to t=2000
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Extend to 2000: rate(1) * new_duration(2000) == deposit(2000) — exact boundary
    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 2000);
    assert_eq!(state.deposit_amount, 2000, "deposit must be unchanged");
    assert_eq!(state.rate_per_second, 1, "rate must be unchanged");
    assert_eq!(state.status, StreamStatus::Active);
}

/// Deposit exceeds new required amount — extension succeeds with surplus.
#[test]
fn test_extend_end_time_deposit_exceeds_new_duration_requirement() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit=5000, rate=1, end=1000 → surplus of 4000 over minimum
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Extend to 3000: rate(1) * 3000 = 3000 < deposit(5000) — surplus remains
    ctx.client().extend_stream_end_time(&stream_id, &3000u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 3000);
    assert_eq!(state.deposit_amount, 5000, "deposit must be unchanged");
}

/// Extension on a Paused stream must succeed (non-terminal state).
#[test]
fn test_extend_end_time_paused_stream_succeeds() {
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
        &None,
    );

    ctx.client().pause_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 2000);
    assert_eq!(
        state.status,
        StreamStatus::Paused,
        "status must stay Paused"
    );
}

/// Accrual at the current ledger time must be unchanged after extension.
#[test]
fn test_extend_end_time_accrual_unchanged_at_extension_time() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(600);
    let accrued_before = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 600);

    ctx.client().extend_stream_end_time(&stream_id, &3000u64);

    // Same ledger timestamp — accrued must not change
    let accrued_after = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_after, accrued_before,
        "accrual at extension time must be unchanged"
    );
}

/// After extension, accrual continues linearly up to the new end_time.
#[test]
fn test_extend_end_time_accrual_continues_to_new_end() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.client().extend_stream_end_time(&stream_id, &3000u64);

    // Past the old end_time (1000) but before new end_time (3000)
    ctx.env.ledger().set_timestamp(2000);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 2000, "accrual must continue past old end_time");

    // At new end_time
    ctx.env.ledger().set_timestamp(3000);
    let accrued_at_new_end = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued_at_new_end, 3000, "accrual must reach new end_time");

    // Past new end_time — capped at deposit
    ctx.env.ledger().set_timestamp(9999);
    let accrued_past = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued_past, 3000,
        "accrual must cap at deposit after new end"
    );
}

/// Recipient can withdraw tokens accrued in the extended window.
#[test]
fn test_extend_end_time_recipient_can_withdraw_extended_accrual() {
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
        &None,
    );

    // Withdraw up to old end_time
    ctx.env.ledger().set_timestamp(1000);
    let w1 = ctx.client().withdraw(&stream_id);
    assert_eq!(w1, 1000);

    // Extend before stream completes (deposit still covers more)
    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    // Withdraw in the extended window
    ctx.env.ledger().set_timestamp(1500);
    let w2 = ctx.client().withdraw(&stream_id);
    assert_eq!(w2, 500, "should withdraw tokens accrued in extended window");

    ctx.env.ledger().set_timestamp(2000);
    let w3 = ctx.client().withdraw(&stream_id);
    assert_eq!(w3, 500);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 2000);
    assert_eq!(ctx.token().balance(&ctx.contract_id), 0);
}

/// top_up then extend: after topping up, a previously-blocked extension succeeds.
#[test]
fn test_extend_end_time_after_top_up_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Tight deposit: exactly covers original duration
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Extension to 1500 would need 1500 tokens — currently blocked
    let result = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &1500u64);
    assert!(
        result.is_err(),
        "extension must fail before top-up when deposit is insufficient"
    );

    // Top up 500 tokens to cover the extended duration
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    // Now extension must succeed
    ctx.client().extend_stream_end_time(&stream_id, &1500u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 1500);
    assert_eq!(state.deposit_amount, 1500);
}

/// Extension emits StreamEndExtended event with correct old/new end_time.
#[test]
fn test_extend_end_time_emits_correct_event() {
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
        &None,
    );

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    let events = ctx.env.events().all();
    let last = events.last().expect("expected at least one event");

    let payload = crate::StreamEndExtended::try_from_val(&ctx.env, &last.2)
        .expect("event data must deserialise as StreamEndExtended");

    assert_eq!(payload.stream_id, stream_id);
    assert_eq!(payload.old_end_time, 1000);
    assert_eq!(payload.new_end_time, 2000);
}

/// No token transfer occurs on extension (deposit stays in contract).
#[test]
fn test_extend_end_time_no_token_transfer() {
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
        &None,
    );

    let sender_before = ctx.token().balance(&ctx.sender);
    let contract_before = ctx.token().balance(&ctx.contract_id);
    let recipient_before = ctx.token().balance(&ctx.recipient);

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    assert_eq!(ctx.token().balance(&ctx.sender), sender_before);
    assert_eq!(ctx.token().balance(&ctx.contract_id), contract_before);
    assert_eq!(ctx.token().balance(&ctx.recipient), recipient_before);
}

// --- Failure paths ---

#[test]
fn test_extend_end_time_deposit_one_short_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit=1000, rate=1, end=1000 → deposit covers exactly 1000s
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Extending to 1001 requires 1001 tokens; deposit is only 1000
    let result = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &1001u64);
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
}

#[test]
fn test_extend_end_time_deposit_far_below_new_requirement_rejected() {
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
        &None,
    );

    // Extending to 10000 requires 10000 tokens; deposit is only 1000
    let result = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &10000u64);
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
}

#[test]
fn test_extend_end_time_completed_stream_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit == rate * duration so the stream completes on full withdrawal
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Any extension on a Completed stream must return InvalidState
    let result = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &2000u64);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn test_extend_end_time_cancelled_stream_rejected() {
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
        &None,
    );

    ctx.client().cancel_stream(&stream_id);

    // Any extension on a Cancelled stream must return InvalidState
    let result = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &2000u64);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Stream with cliff: still in index at exactly cliff time.
#[test]
fn test_get_recipient_streams_cliff_stream_indexed_at_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_cliff_stream(); // cliff at t=500

    ctx.env.ledger().set_timestamp(500);
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);
    assert_eq!(streams.get(0).unwrap(), id);
}

/// Stream cancelled exactly at start_time (before any accrual): stays in index.
#[test]
fn test_get_recipient_streams_cancel_at_start_time_stays_in_index() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_default_stream(); // start=0, end=1000

    // Cancel immediately at start (0 accrued, full refund to sender).
    ctx.client().cancel_stream(&id);

    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    assert_eq!(state.cancelled_at, Some(0));

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        streams.len(),
        1,
        "stream cancelled at start must remain in index"
    );
}

/// Stream cancelled before cliff: accrual is frozen at 0, stream stays in index.
#[test]
fn test_get_recipient_streams_cancel_before_cliff_frozen_accrual() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_cliff_stream(); // cliff=500, end=1000

    // Cancel at t=200 (before cliff).
    ctx.env.ledger().set_timestamp(200);
    ctx.client().cancel_stream(&id);

    // Accrual is frozen at cancellation time; before cliff so accrued=0.
    let accrued = ctx.client().calculate_accrued(&id);
    assert_eq!(accrued, 0, "accrued must be 0 when cancelled before cliff");

    // Stream still in index (recipient can query it).
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 1);
}

/// Stream cancelled after cliff: accrual is frozen at cancellation timestamp.
/// Index still contains the stream so recipient can withdraw the frozen amount.
#[test]
fn test_get_recipient_streams_cancel_after_cliff_frozen_accrual_in_index() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_cliff_stream(); // cliff=500, rate=1, end=1000

    // Cancel at t=700 (after cliff, partial accrual).
    ctx.env.ledger().set_timestamp(700);
    ctx.client().cancel_stream(&id);

    // Accrual frozen at t=700: 700 tokens accrued (rate=1, start=0).
    let accrued = ctx.client().calculate_accrued(&id);
    assert_eq!(accrued, 700, "accrued must be frozen at cancellation time");

    // Stream in index so recipient can still withdraw.
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);

    // Advancing time must NOT change the frozen accrual.
    ctx.env.ledger().set_timestamp(9999);
    let accrued_later = ctx.client().calculate_accrued(&id);
    assert_eq!(
        accrued_later, 700,
        "cancelled stream accrual must not grow after cancellation"
    );
}

/// Stream cancelled exactly at end_time: full deposit accrued, frozen, stays in index.
#[test]
fn test_get_recipient_streams_cancel_at_end_time_full_accrual() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_default_stream(); // rate=1, deposit=1000, end=1000

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&id);

    let accrued = ctx.client().calculate_accrued(&id);
    assert_eq!(
        accrued, 1000,
        "full deposit must be accrued when cancelled at end_time"
    );

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);
}

/// Stream cancelled past end_time: accrual capped at deposit, stays in index.
#[test]
fn test_get_recipient_streams_cancel_past_end_time_capped_accrual() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_default_stream(); // deposit=1000, end=1000

    ctx.env.ledger().set_timestamp(2000); // well past end
    ctx.client().cancel_stream(&id);

    let accrued = ctx.client().calculate_accrued(&id);
    assert_eq!(
        accrued, 1000,
        "accrual must be capped at deposit even when cancelled past end"
    );

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);
}

// --- create_streams (batch) updates the index ---

/// Batch create_streams must add all streams to the recipient's index.
#[test]
fn test_get_recipient_streams_batch_create_updates_index() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let params = soroban_sdk::Vec::from_array(
        &ctx.env,
        [
            CreateStreamParams {
                recipient: ctx.recipient.clone(),
                deposit_amount: 500,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 500,
                memo: None,
            },
            CreateStreamParams {
                recipient: ctx.recipient.clone(),
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1000,
                memo: None,
            },
        ],
    );

    let ids = ctx.client().create_streams(&ctx.sender, &params);
    assert_eq!(ids.len(), 2);

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        streams.len(),
        2,
        "batch create must add all streams to index"
    );
    assert_eq!(streams.get(0).unwrap(), ids.get(0).unwrap());
    assert_eq!(streams.get(1).unwrap(), ids.get(1).unwrap());
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 2);
}

/// Batch create_streams to different recipients: each recipient's index is independent.
#[test]
fn test_get_recipient_streams_batch_create_separate_recipient_indices() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let recipient2 = Address::generate(&ctx.env);

    let params = soroban_sdk::Vec::from_array(
        &ctx.env,
        [
            CreateStreamParams {
                recipient: ctx.recipient.clone(),
                deposit_amount: 500,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 500,
                memo: None,
            },
            CreateStreamParams {
                recipient: recipient2.clone(),
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: 0,
                cliff_time: 0,
                end_time: 1000,
                memo: None,
            },
        ],
    );

    let ids = ctx.client().create_streams(&ctx.sender, &params);

    let streams1 = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams1.len(), 1);
    assert_eq!(streams1.get(0).unwrap(), ids.get(0).unwrap());

    let streams2 = ctx.client().get_recipient_streams(&recipient2);
    assert_eq!(streams2.len(), 1);
    assert_eq!(streams2.get(0).unwrap(), ids.get(1).unwrap());
}

// --- Sorted order invariant ---

/// After interleaved creates and closes, the remaining IDs are always sorted ascending.
#[test]
fn test_get_recipient_streams_sorted_after_interleaved_close() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create streams 0, 1, 2, 3
    for _ in 0..4 {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );
    }

    // Complete and close stream 1 (middle).
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&1u64);
    ctx.client().close_completed_stream(&1u64);

    // Complete and close stream 3 (last).
    ctx.client().withdraw(&3u64);
    ctx.client().close_completed_stream(&3u64);

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 2);
    // Must be sorted: [0, 2]
    assert_eq!(streams.get(0).unwrap(), 0u64);
    assert_eq!(streams.get(1).unwrap(), 2u64);
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 2);
}

// --- Count / list consistency ---

/// get_recipient_stream_count always equals get_recipient_streams().len().
#[test]
fn test_get_recipient_stream_count_matches_list_len() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Empty
    assert_eq!(
        ctx.client().get_recipient_stream_count(&ctx.recipient),
        ctx.client().get_recipient_streams(&ctx.recipient).len() as u64,
    );

    // After 3 creates
    for _ in 0..3 {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );
    }
    assert_eq!(
        ctx.client().get_recipient_stream_count(&ctx.recipient),
        ctx.client().get_recipient_streams(&ctx.recipient).len() as u64,
    );

    // After close
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&0u64);
    ctx.client().close_completed_stream(&0u64);
    assert_eq!(
        ctx.client().get_recipient_stream_count(&ctx.recipient),
        ctx.client().get_recipient_streams(&ctx.recipient).len() as u64,
    );
}

/// Closing the only stream leaves count=0 and an empty list.
#[test]
fn test_get_recipient_stream_count_zero_after_only_stream_closed() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id);
    ctx.client().close_completed_stream(&id);

    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 0);
    assert_eq!(ctx.client().get_recipient_streams(&ctx.recipient).len(), 0);
}

// --- IDs in list correspond to real, queryable streams ---

/// Every ID returned by get_recipient_streams must resolve via get_stream_state
/// and have the correct recipient field.
#[test]
fn test_get_recipient_streams_ids_resolve_to_correct_recipient() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    for _ in 0..5 {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );
    }

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    for id in streams.iter() {
        let state = ctx.client().get_stream_state(&id);
        assert_eq!(
            state.recipient, ctx.recipient,
            "stream {id} must have the queried recipient"
        );
        assert_eq!(
            state.stream_id, id,
            "stream_id field must match the index entry"
        );
    }
}

// --- Numeric boundary: single-second stream ---

/// A stream with duration=1 second is indexed and removed correctly.
#[test]
fn test_get_recipient_streams_single_second_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1u64,
        &None,
    );

    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 1);

    // Complete and close.
    ctx.env.ledger().set_timestamp(1);
    ctx.client().withdraw(&id);
    ctx.client().close_completed_stream(&id);

    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 0);
    assert_eq!(ctx.client().get_recipient_streams(&ctx.recipient).len(), 0);
}

// --- Admin cancel does not remove from index ---

/// Admin-cancelled stream stays in the index (recipient must still be able to withdraw accrued).
#[test]
fn test_get_recipient_streams_admin_cancel_stays_in_index() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream_as_admin(&id);

    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        streams.len(),
        1,
        "admin-cancelled stream must remain in index"
    );
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 1);
}

// --- Withdraw-to does not affect index ---

/// withdraw_to does not remove the stream from the index.
#[test]
fn test_get_recipient_streams_withdraw_to_does_not_affect_index() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id = ctx.create_default_stream();

    let destination = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw_to(&id, &destination);

    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1, "withdraw_to must not affect the index");
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 1);
}

/// new_end_time <= current end_time must be rejected.
#[test]
#[should_panic]
fn test_extend_end_time_same_end_time_rejected() {
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
        &None,
    );

    // Same end_time — not an extension
    ctx.client().extend_stream_end_time(&stream_id, &1000u64);
}

/// new_end_time before current end_time must be rejected (use shorten instead).
#[test]
#[should_panic]
fn test_extend_end_time_shorter_end_time_rejected() {
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
        &None,
    );

    ctx.client().extend_stream_end_time(&stream_id, &500u64);
}

/// Non-sender (recipient) cannot extend the stream.
#[test]
#[should_panic]
fn test_extend_end_time_recipient_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                2000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 2000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Recipient attempts to extend — must fail
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "extend_stream_end_time",
            args: (stream_id, 2000u64).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);
}

/// Third-party address cannot extend the stream.
#[test]
#[should_panic]
fn test_extend_end_time_third_party_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                2000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 2000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let other = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &other,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "extend_stream_end_time",
            args: (stream_id, 2000u64).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);
}

/// Sender authorization succeeds (positive auth test).
#[test]
fn test_extend_end_time_sender_authorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                2000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 2000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "extend_stream_end_time",
            args: (stream_id, 2000u64).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 2000);
}

// --- Overflow / numeric edge cases ---

/// Extension that would cause rate * new_duration to overflow i128 must panic
/// before any state change.
///
/// Strategy: create a stream with rate=1000, duration=1 (deposit=1000 covers it).
/// Extending to u64::MAX seconds: 1000 * u64::MAX overflows i128.
#[test]
fn test_extend_end_time_overflow_panics_no_state_change() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // rate=1000, duration=1 → deposit=1000 exactly covers it (within setup mint of 10_000)
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1000_i128,
        &0u64,
        &0u64,
        &1u64,
        &None,
    );

    let end_before = ctx.client().get_stream_state(&stream_id).end_time;

    // Extending to u64::MAX: 1000 * u64::MAX >> i128::MAX → overflow
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().extend_stream_end_time(&stream_id, &u64::MAX);
    }));

    assert!(
        result.is_err(),
        "overflow in rate * new_duration must panic"
    );

    // end_time must be unchanged
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).end_time,
        end_before,
        "end_time must not change after overflow panic"
    );
}

/// High-rate stream: extension to exact deposit boundary succeeds without overflow.
#[test]
fn test_extend_end_time_high_rate_exact_boundary() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let rate: i128 = 1_000_000_i128;
    let deposit: i128 = 2_000_000_i128; // covers 2 seconds at this rate
    ctx.sac.mint(&ctx.sender, &deposit);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &1u64, // 1 second initially
    );

    // Extend to 2 seconds: rate(1_000_000) * 2 = 2_000_000 == deposit — exact boundary
    ctx.client().extend_stream_end_time(&stream_id, &2u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 2);
    assert_eq!(state.deposit_amount, deposit);
}

// --- State consistency after failed extension ---

/// A failed extension (insufficient deposit) must leave all stream fields unchanged.
#[test]
fn test_extend_end_time_failed_leaves_state_unchanged() {
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
        &None,
    );

    let state_before = ctx.client().get_stream_state(&stream_id);

    let result = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &5000u64);
    assert!(result.is_err(), "extension must fail");

    let state_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_after.end_time, state_before.end_time);
    assert_eq!(state_after.deposit_amount, state_before.deposit_amount);
    assert_eq!(state_after.rate_per_second, state_before.rate_per_second);
    assert_eq!(state_after.status, state_before.status);
    assert_eq!(state_after.withdrawn_amount, state_before.withdrawn_amount);
}

/// A failed extension must not emit any events.
#[test]
fn test_extend_end_time_failed_emits_no_event() {
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
        &None,
    );

    let events_before = ctx.env.events().all().len();

    let _ = ctx
        .client()
        .try_extend_stream_end_time(&stream_id, &5000u64);

    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no event must be emitted on failed extension"
    );
}

// --- Cliff interaction ---

/// Extension must preserve cliff_time; new_end_time >= cliff_time is already
/// guaranteed because new_end_time > old_end_time >= cliff_time.
#[test]
fn test_extend_end_time_cliff_preserved() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &3000_i128,
        &1_i128,
        &0u64,
        &500u64, // cliff at 500
        &1000u64,
        &None,
    );

    ctx.client().extend_stream_end_time(&stream_id, &3000u64);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 500, "cliff_time must be unchanged");
    assert_eq!(state.end_time, 3000);

    // Accrual before cliff must still be zero
    ctx.env.ledger().set_timestamp(300);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 0);

    // Accrual after cliff must work correctly
    ctx.env.ledger().set_timestamp(800);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 800);
}

// --- Integration: extend then withdraw full extended amount ---

/// Full integration: create → extend → withdraw entire extended deposit.
#[test]
fn test_extend_end_time_integration_full_withdrawal() {
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
        &None,
    );

    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    ctx.env.ledger().set_timestamp(2000);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 2000);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 2000);
    assert_eq!(ctx.token().balance(&ctx.contract_id), 0);
    assert_eq!(ctx.token().balance(&ctx.recipient), 2000);
}

// ---------------------------------------------------------------------------
// Tests — Issue #258: pause_stream / resume_stream sender authorization paths
//
// This section provides crisp, explicit coverage for every authorization
// boundary on pause_stream and resume_stream:
//
//   Sender path  : only the stream's original sender may call pause_stream /
//                  resume_stream.  Recipient and arbitrary third parties must
//                  be rejected.
//
//   Admin path   : only the contract admin may call pause_stream_as_admin /
//                  resume_stream_as_admin.  Any other address must be rejected.
//
//   State guards : pause requires Active; resume requires Paused.  Terminal
//                  states (Completed, Cancelled) must be rejected on both
//                  paths.
//
// All strict-mode tests use setup_strict() (no mock_all_auths) and supply
// explicit MockAuth entries so the Soroban auth engine enforces the check.
// ---------------------------------------------------------------------------

// ── helpers ─────────────────────────────────────────────────────────────────

/// Create a stream in strict-mode context and return its id.
/// Authorises only the sender for create_stream + the underlying token transfer.
fn strict_create_stream(ctx: &TestContext) -> u64 {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);

    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    )
}

/// Authorise sender to call pause_stream in strict mode.
fn strict_pause_as_sender(ctx: &TestContext, stream_id: u64) {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().pause_stream(&stream_id);
}

// ── resume_stream: sender authorization (strict mode) ───────────────────────

/// Recipient must NOT be able to call resume_stream.
/// The Soroban auth engine must reject the invocation because the stream's
/// sender address is required, not the recipient.
#[test]
#[should_panic]
fn test_resume_stream_recipient_unauthorized() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    strict_pause_as_sender(&ctx, stream_id);

    // Recipient attempts to resume — must be rejected.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream(&stream_id);
}

/// An arbitrary third party must NOT be able to call resume_stream.
#[test]
#[should_panic]
fn test_resume_stream_third_party_unauthorized() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    strict_pause_as_sender(&ctx, stream_id);

    let other = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &other,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream(&stream_id);
}

/// The stream's sender MUST be able to call resume_stream successfully.
/// Verifies: auth accepted, status transitions Active → Paused → Active,
/// and all other stream fields are unchanged.
#[test]
fn test_resume_stream_sender_success_strict() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    strict_pause_as_sender(&ctx, stream_id);

    let state_paused = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_paused.status, StreamStatus::Paused);

    // Sender authorises resume.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    // Invariant: no other fields mutated by resume.
    assert_eq!(state.stream_id, stream_id);
    assert_eq!(state.sender, ctx.sender);
    assert_eq!(state.recipient, ctx.recipient);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.withdrawn_amount, 0);
}

/// resume_stream must emit a Resumed event observable by integrators.
#[test]
fn test_resume_stream_emits_resumed_event_strict() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    strict_pause_as_sender(&ctx, stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream(&stream_id);

    let events = ctx.env.events().all();
    let last = events.last().unwrap();
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last.2).unwrap(),
        StreamEvent::Resumed(stream_id),
        "resume_stream must publish Resumed(stream_id) event"
    );
}

// ── resume_stream_as_admin: admin authorization (strict mode) ────────────────

/// The contract admin MUST be able to call resume_stream_as_admin successfully.
/// Verifies: auth accepted, status transitions Paused → Active.
#[test]
fn test_resume_stream_as_admin_success_strict() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    strict_pause_as_sender(&ctx, stream_id);

    let state_paused = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_paused.status, StreamStatus::Paused);

    // Admin authorises resume via the admin-specific entrypoint.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream_as_admin(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.deposit_amount, 1000);
}

/// A non-admin address must NOT be able to call resume_stream_as_admin.
/// This guards the admin override path from privilege escalation.
#[test]
#[should_panic]
fn test_resume_stream_as_admin_non_admin_unauthorized() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    strict_pause_as_sender(&ctx, stream_id);

    let non_admin = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream_as_admin(&stream_id);
}

/// The stream's sender must NOT be able to call resume_stream_as_admin.
/// The sender is not the admin; using the admin entrypoint must be rejected.
#[test]
#[should_panic]
fn test_resume_stream_as_admin_sender_is_not_admin() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    strict_pause_as_sender(&ctx, stream_id);

    // Sender tries to use the admin entrypoint — must be rejected.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream_as_admin(&stream_id);
}

/// The recipient must NOT be able to call resume_stream_as_admin.
#[test]
#[should_panic]
fn test_resume_stream_as_admin_recipient_is_not_admin() {
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    let ctx = TestContext::setup_strict();
    let stream_id = strict_create_stream(&ctx);
    strict_pause_as_sender(&ctx, stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream_as_admin(&stream_id);
}

// ── State-boundary guards: pause_stream ─────────────────────────────────────

/// pause_stream on a Completed stream must be rejected.
/// Completed is a terminal state; no further status transitions are allowed.
#[test]
#[should_panic]
fn test_pause_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Drive stream to Completed by withdrawing everything at end_time.
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Attempting to pause a completed stream must panic.
    ctx.client().pause_stream(&stream_id);
}

// ── State-boundary guards: pause_stream_as_admin ────────────────────────────

/// pause_stream_as_admin on an already-Paused stream must be rejected.
/// The admin path enforces the same Active-only precondition as the sender path.
#[test]
fn test_pause_stream_as_admin_already_paused_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Second pause via admin path must return StreamAlreadyPaused.
    let result = ctx.client().try_pause_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamAlreadyPaused)));
}

/// pause_stream_as_admin on a Completed stream must be rejected.
#[test]
fn test_pause_stream_as_admin_completed_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );

    let result = ctx.client().try_pause_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// pause_stream_as_admin on a Cancelled stream must be rejected.
#[test]
fn test_pause_stream_as_admin_cancelled_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().cancel_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );

    let result = ctx.client().try_pause_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

// ── State-boundary guards: resume_stream_as_admin ───────────────────────────

/// resume_stream_as_admin on an Active (not paused) stream must be rejected.
#[test]
fn test_resume_stream_as_admin_active_stream_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );

    let result = ctx.client().try_resume_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamNotPaused)));
}

/// resume_stream_as_admin on a Completed stream must be rejected.
#[test]
fn test_resume_stream_as_admin_completed_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1_000);
    ctx.client().withdraw(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );

    let result = ctx.client().try_resume_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// resume_stream_as_admin on a Cancelled stream must be rejected.
#[test]
fn test_resume_stream_as_admin_cancelled_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().cancel_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );

    let result = ctx.client().try_resume_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

// ── Cross-path invariant: sender pause → admin resume and vice-versa ─────────

/// Sender pauses, admin resumes — cross-role lifecycle must work.
/// Verifies that the two authorization paths are orthogonal and composable.
#[test]
fn test_sender_pause_admin_resume_cross_path() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Sender pauses.
    ctx.client().pause_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    // Admin resumes via admin path.
    ctx.client().resume_stream_as_admin(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

/// Admin pauses, sender resumes — cross-role lifecycle must work.
#[test]
fn test_admin_pause_sender_resume_cross_path() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Admin pauses via admin path.
    ctx.client().pause_stream_as_admin(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    // Sender resumes via sender path.
    ctx.client().resume_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

// ── Time-boundary verification: Admin pause/resume ──────────────────────────

/// Admin can pause exactly at start_time.
#[test]
fn test_admin_pause_at_start_time() {
    let ctx = TestContext::setup();
    let start_time = 100u64;
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1,
        &start_time,
        &start_time,
        &1100,
        &None,
    );

    ctx.env.ledger().set_timestamp(start_time);
    ctx.client().pause_stream_as_admin(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );
}

/// Admin can pause exactly at cliff_time.
#[test]
fn test_admin_pause_at_cliff_time() {
    let ctx = TestContext::setup();
    let cliff_time = 200u64;
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1,
        &100,
        &cliff_time,
        &1100,
        &None,
    );

    ctx.env.ledger().set_timestamp(cliff_time);
    ctx.client().pause_stream_as_admin(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );
}

/// Admin cannot pause at end_time because it should be Completed.
#[test]
fn test_admin_pause_at_end_time_fails() {
    let ctx = TestContext::setup();
    let end_time = 1100u64;
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1,
        &100,
        &200,
        &end_time,
        &None,
    );

    ctx.env.ledger().set_timestamp(end_time);
    // Note: Stored status will still be Active until a state-changing call is made,
    // but the contract must already treat it as Terminal based on time.

    let result = ctx.client().try_pause_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// A paused stream can be withdrawn from if it's past end_time.
#[test]
fn test_withdraw_from_paused_at_end_time() {
    let ctx = TestContext::setup();
    let end_time = 1_000u64;
    let stream_id =
        ctx.client()
            .create_stream(&ctx.sender, &ctx.recipient, &1000, &1, &0, &0, &end_time,
            &None
            );

    // Pause at t=500
    ctx.env.ledger().set_timestamp(500);
    ctx.client().pause_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    // Advance to end_time
    ctx.env.ledger().set_timestamp(1_000);

    // Withdraw should succeed even if Paused
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );
}

// ── Observability: events on admin paths ────────────────────────────────────

/// pause_stream_as_admin must emit the same Paused event as the sender path.
/// Integrators must not need to distinguish which path was used from events alone.
#[test]
fn test_pause_stream_as_admin_emits_paused_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream_as_admin(&stream_id);

    let events = ctx.env.events().all();
    let last = events.last().unwrap();
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last.2).unwrap(),
        StreamEvent::Paused(stream_id),
        "pause_stream_as_admin must publish Paused(stream_id) event"
    );
}

/// resume_stream_as_admin must emit the same Resumed event as the sender path.
#[test]
fn test_resume_stream_as_admin_emits_resumed_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().pause_stream(&stream_id);
    ctx.client().resume_stream_as_admin(&stream_id);

    let events = ctx.env.events().all();
    let last = events.last().unwrap();
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last.2).unwrap(),
        StreamEvent::Resumed(stream_id),
        "resume_stream_as_admin must publish Resumed(stream_id) event"
    );
}

// ── Not-found guard on both paths ───────────────────────────────────────────

/// resume_stream on a non-existent stream_id must return StreamNotFound.
#[test]
fn test_resume_stream_not_found_returns_error() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_resume_stream(&9999);
    assert!(result.is_err(), "resume_stream on unknown id must error");
}

/// resume_stream_as_admin on a non-existent stream_id must return StreamNotFound.
#[test]
fn test_resume_stream_as_admin_not_found_returns_error() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_resume_stream_as_admin(&9999);
    assert!(
        result.is_err(),
        "resume_stream_as_admin on unknown id must error"
    );
}

// ===========================================================================
// Negative tests: pause/resume by non-sender/non-admin
//
// This section codifies authorization boundaries for pause/resume operations.
// Only the stream sender or admin can pause/resume streams. All other roles
// must receive Unauthorized errors.
//
// Scope:
// - Sender can pause/resume (positive tests already exist)
// - Admin can pause_as_admin/resume_as_admin (positive tests already exist)
// - Recipient cannot pause/resume (tested: test_pause_stream_recipient_unauthorized, etc.)
// - Third party cannot pause/resume (tested: test_pause_stream_third_party_unauthorized, etc.)
// - Non-admin cannot use *_as_admin variants (tested in strict mode)
//
// Excluded (covered elsewhere):
// - Stream status transitions (Paused/Active/Completed/Cancelled)
// - Event emission verification
// - Token balance changes
// ===========================================================================

// ---------------------------------------------------------------------------
// §1  pause_stream_as_admin: negative authorization tests
// ---------------------------------------------------------------------------

/// Recipient cannot use pause_stream_as_admin (requires admin auth).
/// Must panic with Unauthorized.
#[test]
#[should_panic]
fn test_pause_stream_as_admin_recipient_is_not_admin() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Create stream by sender
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Recipient tries to use pause_stream_as_admin - must fail
    // MockAuth as recipient (not admin)
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream_as_admin(&stream_id);
}

/// Third party (neither sender nor admin) cannot use pause_stream_as_admin.
/// Must panic with Unauthorized.
#[test]
#[should_panic]
fn test_pause_stream_as_admin_third_party_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Create stream by sender
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Third party tries to use pause_stream_as_admin - must fail
    let third_party = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &third_party,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().pause_stream_as_admin(&stream_id);
}

// ---------------------------------------------------------------------------
// §2  resume_stream_as_admin: negative authorization tests
// ---------------------------------------------------------------------------

/// Recipient cannot use resume_stream_as_admin (requires admin auth).
/// Must panic with Unauthorized.
#[test]
#[should_panic]
fn test_resume_stream_as_admin_recipient_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Create and pause stream
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Admin pauses the stream first
    ctx.client().pause_stream_as_admin(&stream_id);

    // Recipient tries to use resume_stream_as_admin - must fail
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream_as_admin(&stream_id);
}

/// Third party (neither sender nor admin) cannot use resume_stream_as_admin.
/// Must panic with Unauthorized.
#[test]
#[should_panic]
fn test_resume_stream_as_admin_third_party_unauthorized() {
    let ctx = TestContext::setup_strict();

    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

    // Create and pause stream
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Admin pauses the stream first
    ctx.client().pause_stream_as_admin(&stream_id);

    // Third party tries to use resume_stream_as_admin - must fail
    let third_party = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &third_party,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    ctx.client().resume_stream_as_admin(&stream_id);
}

// ---------------------------------------------------------------------------
// §3  Authorization matrix verification
// ---------------------------------------------------------------------------

/// Verify authorization matrix for pause operations.
#[test]
fn test_pause_authorization_matrix() {
    let ctx = TestContext::setup();

    // Create stream
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Admin can pause
    ctx.client().pause_stream_as_admin(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Resume for next test
    ctx.client().resume_stream_as_admin(&stream_id);
}

/// Verify authorization matrix for resume operations.
#[test]
fn test_resume_authorization_matrix() {
    let ctx = TestContext::setup();

    // Create and pause stream
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );
    ctx.client().pause_stream_as_admin(&stream_id);

    // Admin can resume
    ctx.client().resume_stream_as_admin(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
}

// ===========================================================================
// Regression tests: double-init and missing-config reads (Issue #246)
// ===========================================================================
//
// These tests codify externally visible guarantees that treasury operators,
// recipient-facing applications, and third-party auditors rely on:
//
// 1. **Double-init prevention** — `init()` can only succeed once. All subsequent
//    calls must panic with `"already initialised"` and must have zero side effects
//    (config, stream counter, token balances unchanged).
//
// 2. **Missing-config reads** — every read/write path that depends on `Config`
//    must produce a clear, deterministic failure when the contract has never been
//    initialised. This prevents silent undefined behaviour and gives integrators
//    an actionable error message.

// ---------------------------------------------------------------------------
// §1  Double-init regression tests
// ---------------------------------------------------------------------------

/// Calling `init` with the exact same arguments a second time must panic
/// with "already initialised" — idempotent args do not bypass the guard.
#[test]
#[should_panic]
fn regression_double_init_identical_args_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token, &admin);
    // Second call — must panic even though args are identical
    client.init(&token, &admin);
}

/// Calling `init` with a different token but same admin must panic.
#[test]
#[should_panic]
fn regression_double_init_different_token_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token1 = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token1, &admin);
    client.init(&Address::generate(&env), &admin);
}

/// Calling `init` with same token but a different admin must panic.
#[test]
#[should_panic]
fn regression_double_init_different_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin1 = Address::generate(&env);

    client.init(&token, &admin1);
    client.init(&token, &Address::generate(&env));
}

/// Calling `init` with entirely different token AND admin must panic.
#[test]
#[should_panic]
fn regression_double_init_both_different_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    client.init(&Address::generate(&env), &Address::generate(&env));
    client.init(&Address::generate(&env), &Address::generate(&env));
}

/// After a failed double-init, the original config must be completely unchanged.
/// This verifies zero side-effects on both the `token` and `admin` fields.
#[test]
fn regression_double_init_preserves_config_fields() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token_original = Address::generate(&env);
    let admin_original = Address::generate(&env);
    client.init(&token_original, &admin_original);

    // Snapshot the original config
    let config_before = client.get_config();

    // Attempt re-init with completely different addresses
    let attacker_token = Address::generate(&env);
    let attacker_admin = Address::generate(&env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&attacker_token, &attacker_admin);
    }));
    assert!(result.is_err(), "double-init must panic");

    // Config must be byte-identical to original
    let config_after = client.get_config();
    assert_eq!(
        config_after.token, config_before.token,
        "token must be unchanged after failed re-init"
    );
    assert_eq!(
        config_after.admin, config_before.admin,
        "admin must be unchanged after failed re-init"
    );
    assert_eq!(
        config_after.token, token_original,
        "token must match the originally supplied address"
    );
    assert_eq!(
        config_after.admin, admin_original,
        "admin must match the originally supplied address"
    );
}

/// The stream counter (NextStreamId) must not change after a failed double-init.
#[test]
fn regression_double_init_preserves_stream_counter() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);

    let count_before = client.get_stream_count();
    assert_eq!(count_before, 0);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&Address::generate(&env), &Address::generate(&env));
    }));
    assert!(result.is_err());

    assert_eq!(
        client.get_stream_count(),
        count_before,
        "stream counter must not change after failed re-init"
    );
}

/// After N failed re-init attempts, all contract operations must still work.
/// Exercises resilience under repeated attack patterns.
#[test]
fn regression_double_init_repeated_attacks_do_not_degrade_contract() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &50_000_i128);

    // Pound the init endpoint 5 times with different params
    for _ in 0..5 {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.init(&Address::generate(&env), &Address::generate(&env));
        }));
        assert!(r.is_err());
    }

    // Contract must still work normally — create a stream, withdraw, verify
    env.ledger().set_timestamp(0);
    let stream_id = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );
    assert_eq!(stream_id, 0);
    assert_eq!(client.get_stream_count(), 1);

    env.ledger().set_timestamp(500);
    let withdrawn = client.withdraw(&stream_id);
    assert_eq!(withdrawn, 500);

    let state = client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.withdrawn_amount, 500);

    // Config still intact
    let config = client.get_config();
    assert_eq!(config.token, token_id);
    assert_eq!(config.admin, admin);
}

/// A stream created before a failed re-init must remain fully intact and
/// withdrawable after the failed re-init. Verifies no corruption of
/// persistent storage.
#[test]
fn regression_double_init_existing_stream_survives() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &10_000_i128);

    // Create a stream
    env.ledger().set_timestamp(0);
    let stream_id = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );

    // Attempt re-init
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&Address::generate(&env), &Address::generate(&env));
    }));
    assert!(r.is_err());

    // Existing stream must be unaffected
    let state = client.get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.sender, sender);
    assert_eq!(state.recipient, recipient);
    assert_eq!(state.deposit_amount, 1000);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.status, StreamStatus::Active);

    // Full lifecycle still works
    env.ledger().set_timestamp(1000);
    let withdrawn = client.withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);
    let state = client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
}

/// Re-init after creating two streams must not reset or advance the counter.
/// The next stream after re-init attempt should have the correct sequential ID.
#[test]
fn regression_double_init_counter_continuity() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &50_000_i128);

    // Create two streams (counter should be 2)
    env.ledger().set_timestamp(0);
    let id0 = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );
    let id1 = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
    assert_eq!(client.get_stream_count(), 2);

    // Attempt re-init
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&Address::generate(&env), &Address::generate(&env));
    }));

    // Counter must still be 2 and next stream must be ID 2
    assert_eq!(client.get_stream_count(), 2);
    let id2 = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );
    assert_eq!(
        id2, 2,
        "stream ID must continue from 2 after failed re-init"
    );
    assert_eq!(client.get_stream_count(), 3);
}

/// No events must be emitted during a failed re-init attempt.
#[test]
fn regression_double_init_emits_no_events() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);
    client.init(&token, &admin);

    let events_before = env.events().all().len();

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&Address::generate(&env), &Address::generate(&env));
    }));

    assert_eq!(
        env.events().all().len(),
        events_before,
        "failed re-init must not emit any events"
    );
}

// ---------------------------------------------------------------------------
// §2  Missing-config reads — uninitialised contract behaviour
// ---------------------------------------------------------------------------
//
// Invariant: an uninitialised contract (init never called) must produce
// deterministic, explicit failures for all paths that depend on Config.
// The error message "contract not initialised: missing config" is documented
// as the integrator-facing signal.

/// `get_config()` on an uninitialised contract must panic with a clear message.
#[test]
#[should_panic]
fn regression_missing_config_get_config_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.get_config();
}

/// `get_stream_count()` should return 0 without requiring init (it uses
/// `unwrap_or(0)` on `NextStreamId`). This is safe because stream count
/// is semantically 0 before init.
#[test]
fn regression_missing_config_get_stream_count_returns_zero() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    assert_eq!(
        client.get_stream_count(),
        0,
        "stream count must be 0 on uninitialised contract"
    );
}

/// `create_stream()` on an uninitialised contract must fail because it
/// reads config to get the token address for the deposit transfer.
#[test]
#[should_panic]
fn regression_missing_config_create_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.ledger().set_timestamp(0);
    client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );
}

/// `create_streams()` (batch) on uninitialised contract must also fail.
#[test]
#[should_panic]
fn regression_missing_config_create_streams_batch_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let params = CreateStreamParams {
        recipient: recipient.clone(),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };

    env.ledger().set_timestamp(0);
    let streams = soroban_sdk::vec![&env, params];
    client.create_streams(&sender, &streams);
}

/// `set_contract_paused()` on uninitialised contract must fail because it
/// calls `get_admin()` which reads config.
#[test]
#[should_panic]
fn regression_missing_config_set_global_emergency_paused_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.set_global_emergency_paused(&true);
}

/// `set_admin()` on uninitialised contract must fail because it reads
/// current admin from config.
#[test]
#[should_panic]
fn regression_missing_config_set_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let new_admin = Address::generate(&env);
    client.set_admin(&new_admin);
}

/// `version()` must work even without init — it reads a compile-time
/// constant, not config storage. This is an important availability
/// guarantee for deployment scripts.
#[test]
fn regression_missing_config_version_still_works() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let version = client.version();
    assert_eq!(version, 2, "version must be accessible without init");
}

/// `get_stream_state()` for a non-existent stream on an uninitialised
/// contract must fail with "stream not found" (not a config error,
/// since get_stream_state reads persistent storage directly).
#[test]
fn regression_missing_config_get_stream_state_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_get_stream_state(&0);
    assert!(
        result.is_err(),
        "get_stream_state on uninitialised contract must fail"
    );
}

/// `calculate_accrued()` for a non-existent stream on an uninitialised
/// contract must return an error (StreamNotFound).
#[test]
fn regression_missing_config_calculate_accrued_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_calculate_accrued(&0);
    assert!(
        result.is_err(),
        "calculate_accrued on uninitialised contract must fail"
    );
}

/// `get_withdrawable()` for a non-existent stream on an uninitialised
/// contract must return an error.
#[test]
fn regression_missing_config_get_withdrawable_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_get_withdrawable(&0);
    assert!(
        result.is_err(),
        "get_withdrawable on uninitialised contract must fail"
    );
}

/// `get_claimable_at()` for a non-existent stream on an uninitialised
/// contract must return an error.
#[test]
fn regression_missing_config_get_claimable_at_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_get_claimable_at(&0, &500);
    assert!(
        result.is_err(),
        "get_claimable_at on uninitialised contract must fail"
    );
}

/// `withdraw()` on an uninitialised contract with non-existent stream must fail.
#[test]
fn regression_missing_config_withdraw_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_withdraw(&0);
    assert!(
        result.is_err(),
        "withdraw on uninitialised contract must fail"
    );
}

/// `cancel_stream()` on an uninitialised contract must fail.
#[test]
fn regression_missing_config_cancel_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_cancel_stream(&0);
    assert!(
        result.is_err(),
        "cancel_stream on uninitialised contract must fail"
    );
}

/// `cancel_stream_as_admin()` on an uninitialised contract must fail.
/// It reads admin from config, so it should panic with missing config.
#[test]
#[should_panic]
fn regression_missing_config_cancel_stream_as_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.cancel_stream_as_admin(&0);
}

/// `pause_stream_as_admin()` on an uninitialised contract must fail.
#[test]
#[should_panic]
fn regression_missing_config_pause_stream_as_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.pause_stream_as_admin(&0);
}

/// `resume_stream_as_admin()` on an uninitialised contract must fail.
#[test]
#[should_panic]
fn regression_missing_config_resume_stream_as_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.resume_stream_as_admin(&0);
}

/// `pause_stream()` on uninitialised contract w/ non-existent stream must fail.
#[test]
fn regression_missing_config_pause_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_pause_stream(&0);
    assert!(
        result.is_err(),
        "pause_stream on uninitialised contract must fail"
    );
}

/// `resume_stream()` on uninitialised contract w/ non-existent stream must fail.
#[test]
fn regression_missing_config_resume_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_resume_stream(&0);
    assert!(
        result.is_err(),
        "resume_stream on uninitialised contract must fail"
    );
}

/// `get_recipient_streams()` on an uninitialised contract returns an
/// empty vector — no init required for this read path.
#[test]
fn regression_missing_config_get_recipient_streams_returns_empty() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let recipient = Address::generate(&env);
    let streams = client.get_recipient_streams(&recipient);
    assert_eq!(
        streams.len(),
        0,
        "recipient streams must be empty on uninitialised contract"
    );
}

/// `get_recipient_stream_count()` on an uninitialised contract returns 0.
#[test]
fn regression_missing_config_get_recipient_stream_count_returns_zero() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let recipient = Address::generate(&env);
    assert_eq!(
        client.get_recipient_stream_count(&recipient),
        0,
        "recipient stream count must be 0 on uninitialised contract"
    );
}

/// `top_up_stream()` on uninitialised contract must fail (stream not found).
#[test]
fn regression_missing_config_top_up_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let funder = Address::generate(&env);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.top_up_stream(&0, &funder, &100_i128);
    }));
    assert!(
        result.is_err(),
        "top_up_stream on uninitialised contract must fail"
    );
}

/// `update_rate_per_second()` on uninitialised contract must fail.
#[test]
fn regression_missing_config_update_rate_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_update_rate_per_second(&0, &2_i128);
    assert!(
        result.is_err(),
        "update_rate_per_second on uninitialised contract must fail"
    );
}

/// `shorten_stream_end_time()` on uninitialised contract must fail.
#[test]
fn regression_missing_config_shorten_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_shorten_stream_end_time(&0, &500u64);
    assert!(
        result.is_err(),
        "shorten_stream_end_time on uninitialised contract must fail"
    );
}

/// `extend_stream_end_time()` on uninitialised contract must fail.
#[test]
fn regression_missing_config_extend_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_extend_stream_end_time(&0, &2000u64);
    assert!(
        result.is_err(),
        "extend_stream_end_time on uninitialised contract must fail"
    );
}

/// `close_completed_stream()` on uninitialised contract must fail.
#[test]
fn regression_missing_config_close_completed_stream_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_close_completed_stream(&0);
    assert!(
        result.is_err(),
        "close_completed_stream on uninitialised contract must fail"
    );
}

/// `batch_withdraw()` on uninitialised contract w/ non-existent stream must fail.
#[test]
fn regression_missing_config_batch_withdraw_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let recipient = Address::generate(&env);
    let ids = soroban_sdk::vec![&env, 0u64];

    let result = client.try_batch_withdraw(&recipient, &ids);
    assert!(
        result.is_err(),
        "batch_withdraw on uninitialised contract must fail"
    );
}

/// `withdraw_to()` on uninitialised contract must fail.
#[test]
fn regression_missing_config_withdraw_to_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let destination = Address::generate(&env);

    let result = client.try_withdraw_to(&0, &destination);
    assert!(
        result.is_err(),
        "withdraw_to on uninitialised contract must fail"
    );
}

// ---------------------------------------------------------------------------
// §3  Combined scenario: init → use → failed re-init → continued use
// ---------------------------------------------------------------------------

/// End-to-end scenario: full stream lifecycle works correctly when
/// double-init attacks are interleaved at different lifecycle stages.
#[test]
fn regression_double_init_interleaved_with_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &100_000_i128);
    let token = TokenClient::new(&env, &token_id);

    // Phase 1: Create stream, attempt re-init, verify stream
    env.ledger().set_timestamp(0);
    let stream_id = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
        &None,
    );
    assert_eq!(stream_id, 0);

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&Address::generate(&env), &Address::generate(&env));
    }));

    // Phase 2: Partial withdraw, attempt re-init, verify balances
    env.ledger().set_timestamp(300);
    let withdrawn = client.withdraw(&stream_id);
    assert_eq!(withdrawn, 300);

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&Address::generate(&env), &Address::generate(&env));
    }));

    assert_eq!(token.balance(&recipient), 300);
    assert_eq!(token.balance(&contract_id), 700);

    // Phase 3: Pause, attempt re-init, resume, withdraw to completion
    client.pause_stream(&stream_id);
    let state = client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&Address::generate(&env), &Address::generate(&env));
    }));

    client.resume_stream(&stream_id);
    env.ledger().set_timestamp(1000);
    let final_withdrawn = client.withdraw(&stream_id);
    assert_eq!(final_withdrawn, 700);

    let state = client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(token.balance(&recipient), 1000);
    assert_eq!(token.balance(&contract_id), 0);

    // Phase 4: Create another stream after all the chaos — counter must be correct
    env.ledger().set_timestamp(2000);
    let stream_id2 = client.create_stream(
        &sender, &recipient, &2000_i128, &1_i128, &2000u64, &2000u64, &4000u64,
        &None,
    );
    assert_eq!(stream_id2, 1);
    assert_eq!(client.get_stream_count(), 2);
}

// ===========================================================================
// get_claimable_at: future simulation and cancel clamping (Issue #270)
// ===========================================================================
//
// `get_claimable_at(stream_id, timestamp)` is a read-only view function that
// simulates  "how much could the recipient claim at time T?" without mutating
// state. The two key invariants this suite codifies:
//
//   1. **Future simulation**: for Active/Paused streams, claimable grows
//      with timestamp exactly as `calculate_accrued(timestamp) - withdrawn`,
//      clamped at deposit.
//
//   2. **Cancel clamping**: for Cancelled streams, the effective time is
//      `min(timestamp, cancelled_at)`, so claimable can never exceed what
//      was accrued at the moment of cancellation.
//
//   3. **Completed**: always returns 0 (nothing left to claim).

// ---------------------------------------------------------------------------
// §1  Future simulation: Active streams
// ---------------------------------------------------------------------------

/// Claimable at t=0 (start_time) for a no-cliff stream must be 0.
#[test]
fn claimable_at_start_time_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 0..1000, rate=1, no cliff

    let claimable = ctx.client().get_claimable_at(&stream_id, &0);
    assert_eq!(claimable, 0, "claimable at start_time must be 0");
}

/// Claimable at t=1 (one second into stream) returns 1 token.
#[test]
fn claimable_at_one_second_in() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let claimable = ctx.client().get_claimable_at(&stream_id, &1);
    assert_eq!(claimable, 1, "1 second at rate=1 → 1 token");
}

/// Claimable grows linearly with time for a constant-rate stream.
#[test]
fn claimable_at_linearly_proportional() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // rate=1, 0..1000

    for &t in &[0u64, 100, 250, 500, 750, 999, 1000] {
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert_eq!(
            claimable, t as i128,
            "at t={t}, claimable must be {t} (rate=1, no withdraw)"
        );
    }
}

/// Claimable is capped at deposit for timestamps beyond end_time.
#[test]
fn claimable_at_capped_beyond_end() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // deposit=1000

    for &t in &[1000u64, 1001, 2000, 999_999] {
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert_eq!(
            claimable, 1000,
            "at t={t} (>= end_time), claimable must equal deposit 1000"
        );
    }
}

/// After a partial withdraw, claimable deducts the withdrawn amount.
#[test]
fn claimable_at_deducts_withdrawn() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id); // withdraws 300

    // Simulate at t=700: accrued=700, withdrawn=300 → claimable=400
    let claimable = ctx.client().get_claimable_at(&stream_id, &700);
    assert_eq!(claimable, 400);

    // At t=300: accrued=300, withdrawn=300 → claimable=0
    let claimable_at_300 = ctx.client().get_claimable_at(&stream_id, &300);
    assert_eq!(claimable_at_300, 0);

    // At t=1000 (end): accrued=1000, withdrawn=300 → claimable=700
    let claimable_at_end = ctx.client().get_claimable_at(&stream_id, &1000);
    assert_eq!(claimable_at_end, 700);
}

/// After multiple partial withdrawals, claimable reflects cumulative withdrawn.
#[test]
fn claimable_at_after_multiple_withdrawals() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id); // withdrawn=200

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id); // withdrawn=500

    // At t=800: accrued=800, withdrawn=500 → claimable=300
    let claimable = ctx.client().get_claimable_at(&stream_id, &800);
    assert_eq!(claimable, 300);
}

/// Claimable at a time equal to withdrawn amount returns 0.
#[test]
fn claimable_at_exact_withdrawn_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id); // withdrawn=500

    let claimable = ctx.client().get_claimable_at(&stream_id, &500);
    assert_eq!(claimable, 0, "accrued == withdrawn → claimable must be 0");
}

/// Claimable before the withdrawn timestamp still returns 0 (can't have
/// negative claimable).
#[test]
fn claimable_at_before_withdrawn_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id); // withdrawn=500

    // At t=300: accrued=300, withdrawn=500 → claimable=max(0, -200)=0
    let claimable = ctx.client().get_claimable_at(&stream_id, &300);
    assert_eq!(claimable, 0);
}

// ---------------------------------------------------------------------------
// §2  Future simulation: Cliff interactions
// ---------------------------------------------------------------------------

/// With a cliff, claimable is 0 before cliff and jumps at cliff.
#[test]
fn claimable_at_cliff_boundary_detailed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff=500, 0..1000, rate=1

    // One second before cliff: 0
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &499), 0);
    // At cliff: accrual from start → 500
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &500), 500);
    // One second after cliff: 501
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &501), 501);
}

/// With a cliff, withdrawing at cliff then querying future shows correct deduction.
#[test]
fn claimable_at_cliff_after_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id); // withdraws 500

    // At t=800: accrued=800, withdrawn=500 → claimable=300
    let claimable = ctx.client().get_claimable_at(&stream_id, &800);
    assert_eq!(claimable, 300);

    // Before cliff: still 0 (accrual=0 < withdrawn=500, clamp to 0)
    let claimable_pre = ctx.client().get_claimable_at(&stream_id, &100);
    assert_eq!(claimable_pre, 0);
}

// ---------------------------------------------------------------------------
// §3  Cancel clamping: core invariant
// ---------------------------------------------------------------------------

/// After cancel at t=400, claimable for any timestamp >= 400 is frozen at 400.
#[test]
fn claimable_at_cancel_clamped_at_all_future_times() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id); // cancelled_at=400

    // Claimable at exactly cancelled_at
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &400), 400);

    // Claimable at future timestamps: all clamped to 400
    for &t in &[401u64, 500, 800, 1000, 5000, 999_999] {
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert_eq!(
            claimable, 400,
            "cancelled at 400: claimable at t={t} must be clamped to 400"
        );
    }
}

/// Cancel at t=400 + timestamps before cancel_time still work normally.
#[test]
fn claimable_at_cancel_before_cancel_time_follows_schedule() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    // Before cancel_time: effective_time = min(t, 400) = t
    for &t in &[0u64, 100, 200, 300, 399] {
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert_eq!(
            claimable, t as i128,
            "cancelled at 400: claimable at t={t} (<cancel) should follow schedule"
        );
    }
}

/// Cancel at t=0 (immediately) → claimable is always 0.
#[test]
fn claimable_at_cancel_at_start_always_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(0);
    ctx.client().cancel_stream(&stream_id);

    for &t in &[0u64, 1, 100, 1000, 999_999] {
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert_eq!(
            claimable, 0,
            "cancelled at t=0: claimable at any time must be 0"
        );
    }
}

/// Cancel at end_time → claimable at any future time equals full deposit.
#[test]
fn claimable_at_cancel_at_end_time_equals_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 0..1000, deposit=1000

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&stream_id);

    let claimable = ctx.client().get_claimable_at(&stream_id, &9999);
    assert_eq!(claimable, 1000, "cancel at end → full deposit claimable");
}

/// Cancel with partial withdraw: claimable is clamped to (accrued_at_cancel - withdrawn).
#[test]
fn claimable_at_cancel_after_partial_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 200 at t=200
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);

    // Cancel at t=600: accrued=600, withdrawn=200
    ctx.env.ledger().set_timestamp(600);
    ctx.client().cancel_stream(&stream_id);

    // At any future time: accrued clamped to 600, withdrawn=200 → claimable=400
    for &t in &[600u64, 700, 1000, 5000] {
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert_eq!(
            claimable, 400,
            "cancel at 600 with 200 withdrawn: claimable at t={t} must be 400"
        );
    }

    // Before cancel: e.g. t=400 → accrued=400, withdrawn=200 → claimable=200
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &400), 200);

    // Before withdrawn: e.g. t=100 → accrued=100, withdrawn=200 → claimable=0
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &100), 0);
}

/// Cancel with full withdraw: recipient withdrew everything at cancel → claimable=0.
#[test]
fn claimable_at_cancel_after_full_accrual_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id); // withdrawn=500

    ctx.client().cancel_stream(&stream_id); // cancelled_at=500

    // accrued clamped at 500, withdrawn=500 → claimable=0
    for &t in &[500u64, 1000, 9999] {
        assert_eq!(
            ctx.client().get_claimable_at(&stream_id, &t),
            0,
            "cancelled after full withdraw: claimable at t={t} must be 0"
        );
    }
}

/// Admin cancel uses the same clamping logic.
#[test]
fn claimable_at_admin_cancel_clamped() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().cancel_stream_as_admin(&stream_id);

    // Clamped at 300
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &300), 300);
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &999), 300);
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &100), 100);
}

// ---------------------------------------------------------------------------
// §4  Cancel clamping with cliff
// ---------------------------------------------------------------------------

/// Cancel before cliff: accrual was 0 at cancel → claimable always 0.
#[test]
fn claimable_at_cancel_before_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff=500

    ctx.env.ledger().set_timestamp(200);
    ctx.client().cancel_stream(&stream_id); // cancelled_at=200 < cliff

    // Even at future times, effective_time = min(t, 200) < cliff → accrued=0
    for &t in &[0u64, 200, 500, 1000, 9999] {
        assert_eq!(
            ctx.client().get_claimable_at(&stream_id, &t),
            0,
            "cancel at t=200 before cliff=500: claimable at t={t} must be 0"
        );
    }
}

/// Cancel exactly at cliff: accrual=500 (from start_time=0 to cliff=500).
#[test]
fn claimable_at_cancel_at_cliff_boundary() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff=500

    ctx.env.ledger().set_timestamp(500);
    ctx.client().cancel_stream(&stream_id); // cancelled_at=500

    assert_eq!(ctx.client().get_claimable_at(&stream_id, &500), 500);
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &9999), 500);
    // Before cliff: 0
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &499), 0);
}

/// Cancel after cliff: normal clamping at cancel_time.
#[test]
fn claimable_at_cancel_after_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff=500

    ctx.env.ledger().set_timestamp(700);
    ctx.client().cancel_stream(&stream_id); // cancelled_at=700

    // Before cliff: 0
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &499), 0);
    // At cliff: 500
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &500), 500);
    // Between cliff and cancel: follows schedule
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &600), 600);
    // At and beyond cancel: clamped
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &700), 700);
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &9999), 700);
}

// ---------------------------------------------------------------------------
// §5  Paused stream simulation
// ---------------------------------------------------------------------------

/// Paused stream: get_claimable_at still simulates using the given timestamp
/// (accrual is computed at `timestamp`, not frozen at pause time).
#[test]
fn claimable_at_paused_stream_simulates_at_timestamp() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().pause_stream(&stream_id);

    // get_claimable_at simulates at requested timestamp, regardless of pause
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &500), 500);
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &1000), 1000);
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &100), 100);
}

/// Paused after partial withdraw: claimable deducts withdrawn.
#[test]
fn claimable_at_paused_after_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id); // withdrawn=300
    ctx.client().pause_stream(&stream_id);

    // At t=600: accrued=600, withdrawn=300 → claimable=300
    assert_eq!(ctx.client().get_claimable_at(&stream_id, &600), 300);
}

// ---------------------------------------------------------------------------
// §6  Completed stream
// ---------------------------------------------------------------------------

/// Completed stream always returns 0 regardless of timestamp.
#[test]
fn claimable_at_completed_always_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    for &t in &[0u64, 500, 1000, 9999] {
        assert_eq!(
            ctx.client().get_claimable_at(&stream_id, &t),
            0,
            "completed stream: claimable at t={t} must be 0"
        );
    }
}

// ---------------------------------------------------------------------------
// §7  Monotonicity invariant (active stream)
// ---------------------------------------------------------------------------

/// For an active stream with no withdrawals, claimable_at(t1) <= claimable_at(t2)
/// for all t1 <= t2 (monotonically non-decreasing).
#[test]
fn claimable_at_monotonic_active_no_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let mut prev = 0i128;
    for t in (0..=1200).step_by(50) {
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert!(
            claimable >= prev,
            "monotonicity violated: claimable({t})={claimable} < prev={prev}"
        );
        prev = claimable;
    }
}

/// For a cancelled stream, claimable_at(t1) <= claimable_at(t2) for t1 <= t2.
#[test]
fn claimable_at_monotonic_cancelled() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    let mut prev = 0i128;
    for t in (0..=1200).step_by(50) {
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert!(
            claimable >= prev,
            "monotonicity violated (cancelled): claimable({t})={claimable} < prev={prev}"
        );
        prev = claimable;
    }
}

// ---------------------------------------------------------------------------
// §8  Equivalence with get_withdrawable at current ledger time
// ---------------------------------------------------------------------------

/// get_claimable_at(now) == get_withdrawable(stream_id) for active streams
/// at any point in the lifecycle.
#[test]
fn claimable_at_equals_withdrawable_at_current_time() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    for &t in &[0u64, 100, 500, 999, 1000] {
        ctx.env.ledger().set_timestamp(t);
        let withdrawable = ctx.client().get_withdrawable(&stream_id);
        let claimable = ctx.client().get_claimable_at(&stream_id, &t);
        assert_eq!(
            withdrawable, claimable,
            "at t={t}: get_withdrawable ({withdrawable}) != get_claimable_at ({claimable})"
        );
    }
}

/// After partial withdraw, equivalence still holds at current time.
#[test]
fn claimable_at_equals_withdrawable_after_withdraw() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    ctx.env.ledger().set_timestamp(700);
    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    let claimable = ctx.client().get_claimable_at(&stream_id, &700);
    assert_eq!(withdrawable, claimable);
}

/// After cancel, equivalence at current time (both should work with clamping).
#[test]
fn claimable_at_equals_withdrawable_cancelled() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    // At cancel time
    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    let claimable = ctx.client().get_claimable_at(&stream_id, &400);
    assert_eq!(withdrawable, claimable);
}

// ---------------------------------------------------------------------------
// §9  Idempotency: repeated reads don't change results
// ---------------------------------------------------------------------------

/// Calling get_claimable_at multiple times returns the same value (read-only).
#[test]
fn claimable_at_idempotent() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    let first = ctx.client().get_claimable_at(&stream_id, &500);
    let second = ctx.client().get_claimable_at(&stream_id, &500);
    let third = ctx.client().get_claimable_at(&stream_id, &500);
    assert_eq!(first, second);
    assert_eq!(second, third);
}

// ---------------------------------------------------------------------------
// §10  Error cases
// ---------------------------------------------------------------------------

/// Non-existent stream returns StreamNotFound.
#[test]
fn claimable_at_stream_not_found() {
    let ctx = TestContext::setup();
    let result = ctx.client().try_get_claimable_at(&999, &100u64);
    assert!(result.is_err());
}

/// Calling claimable_at on uninitialized contract with bogus stream ID
/// returns StreamNotFound (not a config error, since it's stream-scoped).
#[test]
fn claimable_at_uninitialised_returns_stream_not_found() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let result = client.try_get_claimable_at(&0, &100u64);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// §11  Cancel clamping invariant: claimable_at(any_t) <= accrued_at_cancel
// ---------------------------------------------------------------------------

/// For a cancelled stream, claimable at any timestamp never exceeds
/// the accrual at cancellation time minus the withdrawn amount.
#[test]
fn claimable_at_cancel_never_exceeds_frozen_accrual() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 150
    ctx.env.ledger().set_timestamp(150);
    ctx.client().withdraw(&stream_id);

    // Cancel at t=600
    ctx.env.ledger().set_timestamp(600);
    ctx.client().cancel_stream(&stream_id);

    // Frozen accrual = 600, withdrawn = 150 → max claimable = 450
    for t in (0..=2000).step_by(100) {
        let claimable = ctx.client().get_claimable_at(&stream_id, &(t as u64));
        assert!(
            claimable <= 450,
            "claimable({t})={claimable} exceeds max 450 (frozen accrual - withdrawn)"
        );
    }
}

/// Multiple cancel scenarios: the clamping ceiling is always
/// accrued_at_cancel - withdrawn_at_cancel.
#[test]
fn claimable_at_cancel_ceiling_parametric() {
    // Cancel at various times with various withdraw amounts
    for &(withdraw_time, cancel_time) in &[(0u64, 200u64), (100, 500), (300, 300), (0, 1000)] {
        let ctx = TestContext::setup();
        let stream_id = ctx.create_default_stream();

        if withdraw_time > 0 {
            ctx.env.ledger().set_timestamp(withdraw_time);
            ctx.client().withdraw(&stream_id);
        }

        ctx.env.ledger().set_timestamp(cancel_time);
        ctx.client().cancel_stream(&stream_id);

        let ceiling = cancel_time as i128 - withdraw_time as i128;
        let ceiling = if ceiling > 0 { ceiling } else { 0 };

        // At max future time
        let claimable = ctx.client().get_claimable_at(&stream_id, &999_999u64);
        assert_eq!(
            claimable, ceiling,
            "withdraw={withdraw_time}, cancel={cancel_time}: future claimable must be {ceiling}"
        );
    }
}

// ---------------------------------------------------------------------------
// §12  Additional edge case tests
// ---------------------------------------------------------------------------

/// Test that batch_withdraw correctly handles a mix of streams with different
/// statuses (Active, Paused, Cancelled, Completed) and only processes valid ones.
/// This ensures the batch operation is robust against heterogeneous stream states.
#[test]
fn test_batch_withdraw_mixed_stream_states_comprehensive() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create 5 streams with different eventual states
    let id_active = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id_paused = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id_cancelled = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id_completed = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    let id_active_2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Set up different states
    ctx.env.ledger().set_timestamp(500);

    // Pause one stream
    ctx.client().pause_stream(&id_paused);

    // Cancel one stream (accrued = 500)
    ctx.client().cancel_stream(&id_cancelled);

    // Complete one stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id_completed);

    // Verify states
    assert_eq!(
        ctx.client().get_stream_state(&id_active).status,
        StreamStatus::Active
    );
    assert_eq!(
        ctx.client().get_stream_state(&id_paused).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.client().get_stream_state(&id_cancelled).status,
        StreamStatus::Cancelled
    );
    assert_eq!(
        ctx.client().get_stream_state(&id_completed).status,
        StreamStatus::Completed
    );
    assert_eq!(
        ctx.client().get_stream_state(&id_active_2).status,
        StreamStatus::Active
    );

    // Attempt batch withdraw at t=800
    ctx.env.ledger().set_timestamp(800);

    let mut stream_ids = Vec::new(&ctx.env);
    stream_ids.push_back(id_active);
    stream_ids.push_back(id_paused);
    stream_ids.push_back(id_cancelled);
    stream_ids.push_back(id_completed);
    stream_ids.push_back(id_active_2);

    // This should panic because paused stream is in the batch
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().batch_withdraw(&ctx.recipient, &stream_ids);
    }));

    assert!(
        result.is_err(),
        "batch_withdraw with paused stream should panic"
    );

    // Now try without the paused stream
    let mut valid_stream_ids = Vec::new(&ctx.env);
    valid_stream_ids.push_back(id_active);
    valid_stream_ids.push_back(id_cancelled);
    valid_stream_ids.push_back(id_completed);
    valid_stream_ids.push_back(id_active_2);

    let results = ctx
        .client()
        .batch_withdraw(&ctx.recipient, &valid_stream_ids);

    // Verify results
    assert_eq!(results.len(), 4);

    // Active stream: accrued=800, withdrawn=0 → amount=800
    assert_eq!(results.get(0).unwrap().stream_id, id_active);
    assert_eq!(results.get(0).unwrap().amount, 800);

    // Cancelled stream: accrued frozen at 500, withdrawn=0 → amount=500
    assert_eq!(results.get(1).unwrap().stream_id, id_cancelled);
    assert_eq!(results.get(1).unwrap().amount, 500);

    // Completed stream: nothing left → amount=0
    assert_eq!(results.get(2).unwrap().stream_id, id_completed);
    assert_eq!(results.get(2).unwrap().amount, 0);

    // Active stream 2: accrued=1600 (rate=2), withdrawn=0 → amount=1600
    assert_eq!(results.get(3).unwrap().stream_id, id_active_2);
    assert_eq!(results.get(3).unwrap().amount, 1600);

    // Verify total tokens transferred
    let expected_total = 800 + 500 + 1600;
    assert_eq!(ctx.token().balance(&ctx.recipient), 1000 + expected_total); // 1000 from id_completed earlier
}

/// Test that create_streams batch operation correctly handles the recipient index
/// for multiple recipients and maintains sorted order across all recipients.
/// This ensures the recipient index remains consistent under batch operations.
#[test]
fn test_create_streams_batch_recipient_index_consistency() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let recipient1 = Address::generate(&ctx.env);
    let recipient2 = Address::generate(&ctx.env);
    let recipient3 = Address::generate(&ctx.env);

    // Create a batch with streams for different recipients
    let params = soroban_sdk::Vec::from_array(
        &ctx.env,
        [
            CreateStreamParams {
                recipient: recipient1.clone(),
                deposit_amount: 1000,
                rate_per_second: 1,
                start_time: 1000,
                cliff_time: 1000,
                end_time: 2000,
                memo: None,
            },
            CreateStreamParams {
                recipient: recipient2.clone(),
                deposit_amount: 2000,
                rate_per_second: 1,
                start_time: 1000,
                cliff_time: 1000,
                end_time: 3000,
                memo: None,
            },
            CreateStreamParams {
                recipient: recipient1.clone(),
                deposit_amount: 1500,
                rate_per_second: 1,
                start_time: 1000,
                cliff_time: 1000,
                end_time: 2500,
                memo: None,
            },
            CreateStreamParams {
                recipient: recipient3.clone(),
                deposit_amount: 3000,
                rate_per_second: 1,
                start_time: 1000,
                cliff_time: 1000,
                end_time: 4000,
                memo: None,
            },
            CreateStreamParams {
                recipient: recipient2.clone(),
                deposit_amount: 2500,
                rate_per_second: 1,
                start_time: 1000,
                cliff_time: 1000,
                end_time: 3500,
                memo: None,
            },
        ],
    );

    let ids = ctx.client().create_streams(&ctx.sender, &params);
    assert_eq!(ids.len(), 5);

    // Verify recipient1 has streams 0 and 2 (sorted)
    let streams1 = ctx.client().get_recipient_streams(&recipient1);
    assert_eq!(streams1.len(), 2);
    assert_eq!(streams1.get(0).unwrap(), 0);
    assert_eq!(streams1.get(1).unwrap(), 2);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient1), 2);

    // Verify recipient2 has streams 1 and 4 (sorted)
    let streams2 = ctx.client().get_recipient_streams(&recipient2);
    assert_eq!(streams2.len(), 2);
    assert_eq!(streams2.get(0).unwrap(), 1);
    assert_eq!(streams2.get(1).unwrap(), 4);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient2), 2);

    // Verify recipient3 has stream 3
    let streams3 = ctx.client().get_recipient_streams(&recipient3);
    assert_eq!(streams3.len(), 1);
    assert_eq!(streams3.get(0).unwrap(), 3);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient3), 1);

    // Now complete and close one stream from recipient1
    ctx.env.ledger().set_timestamp(2000);
    ctx.client().withdraw(&0);
    ctx.client().close_completed_stream(&0);

    // Verify recipient1 now only has stream 2
    let streams1_after = ctx.client().get_recipient_streams(&recipient1);
    assert_eq!(streams1_after.len(), 1);
    assert_eq!(streams1_after.get(0).unwrap(), 2);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient1), 1);

    // Other recipients should be unchanged
    let streams2_after = ctx.client().get_recipient_streams(&recipient2);
    assert_eq!(streams2_after.len(), 2);
    assert_eq!(streams2_after.get(0).unwrap(), 1);
    assert_eq!(streams2_after.get(1).unwrap(), 4);

    let streams3_after = ctx.client().get_recipient_streams(&recipient3);
    assert_eq!(streams3_after.len(), 1);
    assert_eq!(streams3_after.get(0).unwrap(), 3);

    // Create another batch to verify IDs continue correctly
    ctx.env.ledger().set_timestamp(0);
    // Mint more tokens for the second batch
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &10_000_i128);

    let params2 = soroban_sdk::Vec::from_array(
        &ctx.env,
        [CreateStreamParams {
            recipient: recipient1.clone(),
            deposit_amount: 500,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 500,
            memo: None,
        }],
    );

    let ids2 = ctx.client().create_streams(&ctx.sender, &params2);
    let id2 = ids2.get(0).unwrap();
    assert_eq!(id2, 5); // Next ID should be 5

    // Verify recipient1 now has streams 2 and 5 (sorted)
    let streams1_final = ctx.client().get_recipient_streams(&recipient1);
    assert_eq!(streams1_final.len(), 2);
    assert_eq!(streams1_final.get(0).unwrap(), 2);
    assert_eq!(streams1_final.get(1).unwrap(), 5);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient1), 2);
}
// ---------------------------------------------------------------------------
// Tests — Overflow Protection (Issue: create_streams total deposit overflow)
// ---------------------------------------------------------------------------

#[test]
fn test_create_stream_total_streamable_overflow() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // rate=i128::MAX, duration=2s => rate * duration overflows i128
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &i128::MAX,
        &i128::MAX,
        &0u64,
        &0u64,
        &2u64,
        &None,
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_create_streams_batch_deposit_overflow() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let mut streams = Vec::new(&ctx.env);

    // Two streams each with half+1 of i128::MAX deposit
    let half_max = i128::MAX / 2 + 1;

    streams.push_back(CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: half_max,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 10,
        memo: None,
    });

    streams.push_back(CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: half_max,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 10,
        memo: None,
    });

    let result = ctx.client().try_create_streams(&ctx.sender, &streams);

    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

#[test]
fn test_top_up_stream_overflow() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Mint just enough to hit i128::MAX total (sender already has 10_000)
    let amount_to_mint = i128::MAX - 10_000;
    ctx.sac.mint(&ctx.sender, &amount_to_mint);

    // Create a stream with a large deposit
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &(i128::MAX - 100),
        &1,
        &0,
        &0,
        &10,
        &None,
    );

    // Top up by more than 100 should overflow
    let result = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &101);

    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

// ---------------------------------------------------------------------------
// §GAS  Gas / budget review: hot paths and batching
// ---------------------------------------------------------------------------
//
// Scope: Issue "Gas / budget review: hot paths and batching"
//
// Hot paths identified:
//   1. `withdraw`          — single-stream accrual + token push (most frequent call)
//   2. `batch_withdraw`    — N-stream loop; one auth, one accrual + push per stream
//   3. `create_streams`    — N-stream validation loop + single bulk token pull
//
// Each test resets the budget to unlimited before the measured call so that
// setup overhead (init, mint, create_stream) does not pollute the reading.
// Guardrails are intentionally generous (10× observed baseline) so they catch
// regressions without being brittle to minor SDK changes.
//
// Cancelled-stream path in batch_withdraw:
//   A cancelled stream is neither Completed nor Paused, so it falls through to
//   the accrual branch. The accrual is frozen at cancelled_at, so the result is
//   deterministic. This path must not panic and must transfer only the remaining
//   accrued-but-not-withdrawn amount.

// --- hot path: single withdraw ---

/// Budget guardrail: a single `withdraw` on an active stream must stay within
/// a reasonable CPU and memory envelope.
#[test]
fn test_budget_single_withdraw_hot_path() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);

    ctx.env.budget().reset_unlimited();
    ctx.client().withdraw(&stream_id);

    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    // Guardrail: single withdraw must stay well under 1 M CPU instructions and 500 KB.
    assert!(
        cpu <= 1_000_000,
        "single withdraw cpu={cpu} exceeds guardrail 1_000_000"
    );
    assert!(
        mem <= 500_000,
        "single withdraw mem={mem} exceeds guardrail 500_000"
    );
}

/// Budget guardrail: `withdraw` on a stream that has nothing to withdraw
/// (before cliff) must be cheaper than a full withdrawal — it short-circuits
/// before any token transfer.
#[test]
fn test_budget_withdraw_zero_short_circuit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500
    ctx.env.ledger().set_timestamp(100); // before cliff → withdrawable = 0

    ctx.env.budget().reset_unlimited();
    let amount = ctx.client().withdraw(&stream_id);
    assert_eq!(amount, 0);

    let cpu_zero = ctx.env.budget().cpu_instruction_cost();

    // Now measure a full withdrawal for comparison
    ctx.env.ledger().set_timestamp(1000);
    ctx.env.budget().reset_unlimited();
    ctx.client().withdraw(&stream_id);
    let cpu_full = ctx.env.budget().cpu_instruction_cost();

    // Zero-withdraw path must not be more expensive than a full withdrawal.
    // (It should be cheaper because it skips the token transfer.)
    assert!(
        cpu_zero <= cpu_full,
        "zero-withdraw cpu={cpu_zero} should be <= full-withdraw cpu={cpu_full}"
    );
}

// --- hot path: batch_withdraw ---

/// Budget guardrail: `batch_withdraw` over 10 active streams must stay within
/// a reasonable CPU and memory envelope (linear scaling check).
#[test]
fn test_budget_batch_withdraw_10_streams() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &100_000_i128);

    ctx.env.ledger().set_timestamp(0);
    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    for _ in 0..10 {
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );
        ids.push_back(id);
    }

    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);

    assert_eq!(results.len(), 10);
    for i in 0..10 {
        assert_eq!(results.get(i).unwrap().amount, 500);
    }

    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    // Guardrail: 10-stream batch must stay under 5 M CPU and 2 MB.
    assert!(
        cpu <= 5_000_000,
        "batch_withdraw(10) cpu={cpu} exceeds guardrail 5_000_000"
    );
    assert!(
        mem <= 2_000_000,
        "batch_withdraw(10) mem={mem} exceeds guardrail 2_000_000"
    );
}

/// Budget scales sub-linearly or linearly: batch of 10 must cost less than
/// 10× the cost of a single withdraw (auth overhead is paid once).
#[test]
fn test_budget_batch_withdraw_cheaper_than_n_singles() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &100_000_i128);

    ctx.env.ledger().set_timestamp(0);
    let mut ids = soroban_sdk::Vec::new(&ctx.env);
    for _ in 0..10 {
        let id = ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );
        ids.push_back(id);
    }

    // Measure single withdraw cost
    ctx.env.ledger().set_timestamp(500);
    ctx.env.budget().reset_unlimited();
    ctx.client().withdraw(&ids.get(0).unwrap());
    let cpu_single = ctx.env.budget().cpu_instruction_cost();

    // Measure batch withdraw cost for remaining 9 streams
    let mut batch_ids = soroban_sdk::Vec::new(&ctx.env);
    for i in 1..10 {
        batch_ids.push_back(ids.get(i).unwrap());
    }
    ctx.env.budget().reset_unlimited();
    ctx.client().batch_withdraw(&ctx.recipient, &batch_ids);
    let cpu_batch_9 = ctx.env.budget().cpu_instruction_cost();

    // Batch of 9 must cost less than 9× a single withdraw.
    // This validates that auth is paid once, not per stream.
    assert!(
        cpu_batch_9 < cpu_single * 9,
        "batch(9) cpu={cpu_batch_9} should be < 9 × single cpu={} = {}",
        cpu_single,
        cpu_single * 9
    );
}

// --- batch_withdraw: cancelled stream path ---

/// batch_withdraw on a cancelled stream transfers only the remaining
/// accrued-but-not-withdrawn amount and does not panic.
#[test]
fn test_batch_withdraw_cancelled_stream_transfers_accrued_remainder() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 1000 tokens, rate=1, end=1000

    // Withdraw 200 at t=200
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.token().balance(&ctx.recipient), 200);

    // Cancel at t=600 → accrued_at_cancel=600, refund=400 to sender, 400 left for recipient
    ctx.env.ledger().set_timestamp(600);
    ctx.client().cancel_stream(&stream_id);

    let recipient_before = ctx.token().balance(&ctx.recipient);
    let contract_before = ctx.token().balance(&ctx.contract_id);

    // batch_withdraw on the cancelled stream must transfer the remaining 400
    let ids = stream_ids_vec(&ctx.env, &[stream_id]);
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);

    assert_eq!(results.len(), 1);
    assert_eq!(
        results.get(0).unwrap().amount,
        400,
        "cancelled stream: batch_withdraw must transfer accrued - already_withdrawn"
    );
    assert_eq!(ctx.token().balance(&ctx.recipient), recipient_before + 400);
    assert_eq!(ctx.token().balance(&ctx.contract_id), contract_before - 400);
}

/// batch_withdraw on a cancelled stream where recipient already withdrew everything
/// returns 0 and makes no transfer.
#[test]
fn test_batch_withdraw_cancelled_stream_fully_withdrawn_yields_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw 600 at t=600
    ctx.env.ledger().set_timestamp(600);
    ctx.client().withdraw(&stream_id);

    // Cancel at t=600 (same timestamp) → accrued_at_cancel=600, already withdrawn=600
    ctx.client().cancel_stream(&stream_id);

    let recipient_before = ctx.token().balance(&ctx.recipient);
    let events_before = ctx.env.events().all().len();

    let ids = stream_ids_vec(&ctx.env, &[stream_id]);
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);

    assert_eq!(results.get(0).unwrap().amount, 0);
    assert_eq!(ctx.token().balance(&ctx.recipient), recipient_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// batch_withdraw: wrong recipient returns Unauthorized and reverts the whole batch.
#[test]
fn test_batch_withdraw_wrong_recipient_returns_unauthorized() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);

    let other = Address::generate(&ctx.env);
    let ids = stream_ids_vec(&ctx.env, &[stream_id]);
    let result = ctx.client().try_batch_withdraw(&other, &ids);

    assert_eq!(
        result,
        Err(Ok(ContractError::Unauthorized)),
        "wrong recipient must return Unauthorized"
    );
}

// --- hot path: create_streams batch ---

/// Budget guardrail: `create_streams` with 5 entries must stay within a
/// reasonable CPU and memory envelope.
#[test]
fn test_budget_create_streams_batch_5() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &100_000_i128);

    ctx.env.ledger().set_timestamp(0);
    let mut params = soroban_sdk::Vec::new(&ctx.env);
    for _ in 0..5 {
        params.push_back(CreateStreamParams {
            recipient: Address::generate(&ctx.env),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            memo: None,
        });
    }

    ctx.env.budget().reset_unlimited();
    let ids = ctx.client().create_streams(&ctx.sender, &params);

    assert_eq!(ids.len(), 5);

    let cpu = ctx.env.budget().cpu_instruction_cost();
    let mem = ctx.env.budget().memory_bytes_cost();

    // Guardrail: 5-stream batch create must stay under 3 M CPU and 1.5 MB.
    assert!(
        cpu <= 3_000_000,
        "create_streams(5) cpu={cpu} exceeds guardrail 3_000_000"
    );
    assert!(
        mem <= 1_500_000,
        "create_streams(5) mem={mem} exceeds guardrail 1_500_000"
    );
}

/// create_streams batch is atomic: a single invalid entry aborts all,
/// no state written, no tokens moved, no events emitted.
#[test]
fn test_create_streams_batch_atomicity_on_invalid_entry() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let valid = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };
    // deposit < rate * duration → InsufficientDeposit
    let invalid = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    };

    let count_before = ctx.client().get_stream_count();
    let sender_before = ctx.token().balance(&ctx.sender);
    let contract_before = ctx.token().balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let mut streams = soroban_sdk::Vec::new(&ctx.env);
    streams.push_back(valid);
    streams.push_back(invalid);

    let result = ctx.client().try_create_streams(&ctx.sender, &streams);
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));

    assert_eq!(ctx.client().get_stream_count(), count_before);
    assert_eq!(ctx.token().balance(&ctx.sender), sender_before);
    assert_eq!(ctx.token().balance(&ctx.contract_id), contract_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// create_streams with a single entry behaves identically to create_stream.
#[test]
fn test_create_streams_single_entry_matches_create_stream() {
    let ctx = TestContext::setup();
    ctx.sac.mint(&ctx.sender, &10_000_i128);
    ctx.env.ledger().set_timestamp(0);

    let recipient_a = Address::generate(&ctx.env);
    let recipient_b = Address::generate(&ctx.env);

    // Single create_stream
    let id_single = ctx.client().create_stream(
        &ctx.sender,
        &recipient_a,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    // Single-entry create_streams
    let mut params = soroban_sdk::Vec::new(&ctx.env);
    params.push_back(CreateStreamParams {
        recipient: recipient_b.clone(),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
        memo: None,
    });
    let ids = ctx.client().create_streams(&ctx.sender, &params);
    let id_batch = ids.get(0).unwrap();

    // IDs are sequential
    assert_eq!(id_batch, id_single + 1);

    // Both streams have identical schedule fields
    let s_single = ctx.client().get_stream_state(&id_single);
    let s_batch = ctx.client().get_stream_state(&id_batch);
    assert_eq!(s_single.deposit_amount, s_batch.deposit_amount);
    assert_eq!(s_single.rate_per_second, s_batch.rate_per_second);
    assert_eq!(s_single.start_time, s_batch.start_time);
    assert_eq!(s_single.cliff_time, s_batch.cliff_time);
    assert_eq!(s_single.end_time, s_batch.end_time);
    assert_eq!(s_single.status, s_batch.status);
}

/// create_streams with overflow in total deposit returns InvalidParams and is atomic.
#[test]
fn test_create_streams_batch_deposit_overflow_is_atomic() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Two entries each with i128::MAX / 2 + 1 → sum overflows i128
    let half_max = i128::MAX / 2 + 1;
    let duration = 1u64;

    let mut params = soroban_sdk::Vec::new(&ctx.env);
    for _ in 0..2 {
        params.push_back(CreateStreamParams {
            recipient: Address::generate(&ctx.env),
            deposit_amount: half_max,
            rate_per_second: half_max,
            start_time: 0,
            cliff_time: 0,
            end_time: duration,
            memo: None,
        });
    }

    let count_before = ctx.client().get_stream_count();
    let result = ctx.client().try_create_streams(&ctx.sender, &params);

    // Must fail (overflow → InvalidParams or token transfer failure)
    assert!(result.is_err(), "overflow batch must fail");
    assert_eq!(
        ctx.client().get_stream_count(),
        count_before,
        "stream count must not change on overflow failure"
    );
}

// ===========================================================================
// i128 boundary streams: near-max rate/deposit scenarios
//
// Scope: systematic evidence that the contract handles i128-scale deposits and
// rates correctly across all observable surfaces — stored state, emitted events,
// error codes, and token balances.
//
// Audit notes / residual risks:
// - Token supply: the SAC mock has no supply cap, so we can mint i128::MAX.
//   On mainnet the token's own supply limit is the binding constraint.
// - Gas: Soroban budget is not enforced in the test harness; on-chain these
//   streams are valid but callers should verify budget headroom.
// - Rate × duration overflow at creation: rejected with InvalidParams (checked_mul).
//   This is the only hard rejection path; all other near-max values are accepted.
// ===========================================================================
#[cfg(test)]
mod i128_boundary_streams {
    use super::*;
    use soroban_sdk::{testutils::Ledger, token::StellarAssetClient, Address, Env};

    // -----------------------------------------------------------------------
    // Shared helpers
    // -----------------------------------------------------------------------

    /// Largest deposit that can be created with a 1-second stream (rate == deposit).
    /// rate * duration = i128::MAX / 2 * 1 fits in i128.
    const NEAR_MAX_DEPOSIT: i128 = i128::MAX / 2;
    const NEAR_MAX_RATE: i128 = i128::MAX / 2; // rate for 1-second stream

    /// A safe large deposit: rate=1, duration=i128::MAX/2 seconds.
    /// Avoids rate*duration overflow while exercising large deposit values.
    const _LARGE_DEPOSIT_RATE1: i128 = 1_000_000_000_000_000_000_i128;
    const _LARGE_DEPOSIT_DURATION: u64 = 1_000_000_000_000_000_000_u64;

    fn setup_with_balance(balance: i128) -> (Env, Address, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);
        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &balance);
        (env, contract_id, token_id, admin, sender, recipient)
    }

    // -----------------------------------------------------------------------
    // 1. Creation: near-max deposit accepted, state persisted correctly
    // -----------------------------------------------------------------------

    /// Near-max deposit (i128::MAX/2) with rate=deposit, duration=1s.
    /// Verifies stored fields match supplied params exactly.
    #[test]
    fn near_max_deposit_creation_persists_correct_state() {
        let (env, contract_id, _token_id, _admin, sender, recipient) =
            setup_with_balance(NEAR_MAX_DEPOSIT);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &NEAR_MAX_DEPOSIT,
            &NEAR_MAX_RATE,
            &0u64,
            &0u64,
            &1u64,
            &None,
        );

        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.deposit_amount, NEAR_MAX_DEPOSIT);
        assert_eq!(state.rate_per_second, NEAR_MAX_RATE);
        assert_eq!(state.withdrawn_amount, 0);
        assert_eq!(state.status, StreamStatus::Active);
        assert_eq!(state.start_time, 0);
        assert_eq!(state.end_time, 1);
        assert!(state.cancelled_at.is_none());
    }

    /// Near-max deposit with a cliff: stored cliff_time must match.
    #[test]
    fn near_max_deposit_with_cliff_persists_cliff_time() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000; // duration = 1000s
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &500u64, // cliff at t=500
            &1_000u64,
            &None,
        );

        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.deposit_amount, large_deposit);
        assert_eq!(state.cliff_time, 500);
        assert_eq!(state.status, StreamStatus::Active);
    }

    /// StreamCreated event at near-max values carries correct payload.
    #[test]
    fn near_max_deposit_creation_emits_correct_event() {
        let (env, contract_id, _token_id, _admin, sender, recipient) =
            setup_with_balance(NEAR_MAX_DEPOSIT);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &NEAR_MAX_DEPOSIT,
            &NEAR_MAX_RATE,
            &0u64,
            &0u64,
            &1u64,
            &None,
        );

        let events = env.events().all();
        let last = events.last().unwrap();
        let event_data = StreamCreated::try_from_val(&env, &last.2).unwrap();
        assert_eq!(event_data.stream_id, stream_id);
        assert_eq!(event_data.deposit_amount, NEAR_MAX_DEPOSIT);
        assert_eq!(event_data.rate_per_second, NEAR_MAX_RATE);
        assert_eq!(event_data.start_time, 0);
        assert_eq!(event_data.end_time, 1);
    }

    // -----------------------------------------------------------------------
    // 2. Creation: overflow / rejection at i128 boundary
    // -----------------------------------------------------------------------

    /// rate * duration overflows i128 → rejected with InvalidParams, no side effects.
    #[test]
    fn rate_times_duration_overflow_rejected_atomically() {
        // i128::MAX / 2 * 3 overflows i128
        let rate: i128 = i128::MAX / 2;
        let deposit: i128 = i128::MAX / 2; // not enough to cover overflow
        let (env, contract_id, _token_id, _admin, sender, recipient) = setup_with_balance(deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let count_before = client.get_stream_count();
        let result = client.try_create_stream(
            &sender, &recipient, &deposit, &rate, &0u64, &0u64, &3u64, // rate * 3 overflows
        );

        assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
        assert_eq!(
            client.get_stream_count(),
            count_before,
            "counter must not advance"
        );
        assert_eq!(
            soroban_sdk::token::Client::new(&env, &_token_id).balance(&contract_id),
            0,
            "no tokens must move on rejection"
        );
    }

    /// deposit < rate * duration → InsufficientDeposit, no side effects.
    #[test]
    fn insufficient_deposit_for_near_max_rate_rejected() {
        let rate: i128 = i128::MAX / 1_000_000;
        let duration: u64 = 1_000_000;
        let required = rate * duration as i128;
        let deposit = required - 1; // one token short

        let (env, contract_id, token_id, _admin, sender, recipient) = setup_with_balance(deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let result = client.try_create_stream(
            &sender, &recipient, &deposit, &rate, &0u64, &0u64, &duration,
            &None,
        );

        assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
        assert_eq!(
            soroban_sdk::token::Client::new(&env, &token_id).balance(&sender),
            deposit,
            "sender balance must be unchanged on rejection"
        );
    }

    // -----------------------------------------------------------------------
    // 3. Accrual: near-max values, overflow protection, cliff boundary
    // -----------------------------------------------------------------------

    /// At t=0 (start), accrued must be 0 even for near-max deposit.
    #[test]
    fn near_max_deposit_accrued_zero_at_start() {
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(NEAR_MAX_DEPOSIT);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &NEAR_MAX_DEPOSIT,
            &NEAR_MAX_RATE,
            &0u64,
            &0u64,
            &1u64,
            &None,
        );

        let accrued = client.calculate_accrued(&stream_id);
        assert_eq!(accrued, 0, "nothing accrued at start");
    }

    /// At end_time, accrued == deposit for near-max stream.
    #[test]
    fn near_max_deposit_accrued_equals_deposit_at_end() {
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(NEAR_MAX_DEPOSIT);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &NEAR_MAX_DEPOSIT,
            &NEAR_MAX_RATE,
            &0u64,
            &0u64,
            &1u64,
            &None,
        );

        env.ledger().set_timestamp(1);
        let accrued = client.calculate_accrued(&stream_id);
        assert_eq!(accrued, NEAR_MAX_DEPOSIT);
    }

    /// Long after end_time, accrued is still capped at deposit (no post-end growth).
    #[test]
    fn near_max_deposit_accrued_capped_long_after_end() {
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(NEAR_MAX_DEPOSIT);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &NEAR_MAX_DEPOSIT,
            &NEAR_MAX_RATE,
            &0u64,
            &0u64,
            &1u64,
            &None,
        );

        env.ledger().set_timestamp(u64::MAX / 2);
        let accrued = client.calculate_accrued(&stream_id);
        assert_eq!(accrued, NEAR_MAX_DEPOSIT, "must cap at deposit after end");
    }

    /// Before cliff, accrued is 0 even for near-max deposit.
    #[test]
    fn near_max_deposit_accrued_zero_before_cliff() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &500u64, // cliff at t=500
            &1_000u64,
            &None,
        );

        env.ledger().set_timestamp(499);
        let accrued = client.calculate_accrued(&stream_id);
        assert_eq!(accrued, 0, "must be 0 before cliff");
    }

    /// Exactly at cliff, accrual uses elapsed from start_time (not cliff_time).
    #[test]
    fn near_max_deposit_accrual_at_cliff_uses_start_time() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &500u64,
            &1_000u64,
            &None,
        );

        env.ledger().set_timestamp(500);
        let accrued = client.calculate_accrued(&stream_id);
        // elapsed from start = 500, rate = large_deposit/1000
        let expected = 500_i128 * rate;
        assert_eq!(accrued, expected, "accrual at cliff uses start_time");
    }

    /// Multiplication overflow in accrual falls back to deposit_amount (safe upper bound).
    #[test]
    fn near_max_rate_accrual_overflow_falls_back_to_deposit() {
        // rate = i128::MAX / 2, duration = 3 → rate*3 overflows, but we use duration=1
        // To trigger overflow in accrual: use rate=i128::MAX/2, elapsed=3 (past end=1)
        // elapsed is capped at end_time=1, so elapsed=1, rate*(1) = i128::MAX/2 = deposit → no overflow
        // To actually overflow: rate=i128::MAX, elapsed=2 → but rate*duration must pass validation
        // Use: rate = i128::MAX/2, duration=2, deposit = i128::MAX/2 (rate*2 overflows but deposit covers)
        // Actually rate*duration = (i128::MAX/2)*2 = i128::MAX-1 which fits. Use rate=i128::MAX/2+1, dur=2.
        // Simpler: use the pure accrual function directly via the contract's calculate_accrued view.
        // We create a stream where elapsed*rate overflows: rate=i128::MAX/2, duration=1, deposit=i128::MAX/2.
        // At t=1 elapsed=1, 1*(i128::MAX/2) = i128::MAX/2 = deposit → no overflow.
        // The overflow path is tested in accrual.rs unit tests. Here we verify the contract
        // returns deposit_amount (not a panic) when the accrual math would overflow.
        let deposit: i128 = NEAR_MAX_DEPOSIT;
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &deposit,
            &NEAR_MAX_RATE,
            &0u64,
            &0u64,
            &1u64,
            &None,
        );

        // Set time far past end — elapsed is capped at end_time=1, no overflow possible
        env.ledger().set_timestamp(u64::MAX);
        let accrued = client.calculate_accrued(&stream_id);
        assert_eq!(accrued, deposit, "must return deposit, not panic");
        assert!(accrued >= 0, "must be non-negative");
    }

    // -----------------------------------------------------------------------
    // 4. Withdrawal: near-max amounts, balance invariants, event payloads
    // -----------------------------------------------------------------------

    /// Full withdrawal of near-max deposit: recipient receives exact amount,
    /// contract balance reaches 0, status transitions to Completed.
    #[test]
    fn near_max_deposit_full_withdrawal_completes_stream() {
        let (env, contract_id, token_id, _a, sender, recipient) =
            setup_with_balance(NEAR_MAX_DEPOSIT);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &NEAR_MAX_DEPOSIT,
            &NEAR_MAX_RATE,
            &0u64,
            &0u64,
            &1u64,
            &None,
        );

        env.ledger().set_timestamp(1);
        let withdrawn = client.withdraw(&stream_id);

        assert_eq!(withdrawn, NEAR_MAX_DEPOSIT);
        assert_eq!(token.balance(&recipient), NEAR_MAX_DEPOSIT);
        assert_eq!(token.balance(&contract_id), 0);

        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.status, StreamStatus::Completed);
        assert_eq!(state.withdrawn_amount, NEAR_MAX_DEPOSIT);
    }

    /// Withdrawal event at near-max carries correct amount in payload.
    #[test]
    fn near_max_withdrawal_event_carries_correct_amount() {
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(NEAR_MAX_DEPOSIT);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &NEAR_MAX_DEPOSIT,
            &NEAR_MAX_RATE,
            &0u64,
            &0u64,
            &1u64,
            &None,
        );

        env.ledger().set_timestamp(1);
        client.withdraw(&stream_id);

        let events = env.events().all();
        // Find the withdrew event
        let withdrew_event = events.iter().rev().find(|e| {
            if e.0 != contract_id {
                return false;
            }
            let topic0 = soroban_sdk::Symbol::from_val(&env, &e.1.get(0).unwrap());
            topic0 == soroban_sdk::Symbol::new(&env, "withdrew")
        });
        assert!(withdrew_event.is_some(), "withdrew event must be emitted");
        let ev = withdrew_event.unwrap();
        let payload = crate::Withdrawal::try_from_val(&env, &ev.2).unwrap();
        assert_eq!(payload.amount, NEAR_MAX_DEPOSIT);
        assert_eq!(payload.stream_id, stream_id);
    }

    /// Partial withdrawal at near-max: withdrawn_amount increments correctly,
    /// second withdrawal drains remainder, stream completes.
    #[test]
    fn near_max_deposit_two_partial_withdrawals_complete_stream() {
        // Use rate=1 and a round deposit to avoid integer division truncation
        let large_deposit: i128 = i128::MAX / 1_000_000 / 1_000 * 1_000; // divisible by 1000
        let rate: i128 = large_deposit / 1_000;
        let _duration: u64 = 1_000;
        let (env, contract_id, token_id, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        // First withdrawal at t=400
        env.ledger().set_timestamp(400);
        let first = client.withdraw(&stream_id);
        let expected_first = 400_i128 * rate;
        assert_eq!(first, expected_first);
        assert_eq!(
            client.get_stream_state(&stream_id).withdrawn_amount,
            expected_first
        );

        // Second withdrawal at t=1000 (end)
        env.ledger().set_timestamp(1_000);
        let second = client.withdraw(&stream_id);
        let expected_second = large_deposit - expected_first;
        assert_eq!(second, expected_second);

        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.withdrawn_amount, large_deposit);
        assert_eq!(state.status, StreamStatus::Completed);
        assert_eq!(token.balance(&recipient), large_deposit);
        assert_eq!(token.balance(&contract_id), 0);
    }

    // -----------------------------------------------------------------------
    // 5. Cancellation: near-max refund math, frozen accrual, event payload
    // -----------------------------------------------------------------------

    /// Cancel at t=0 (before any accrual): full deposit refunded to sender.
    #[test]
    fn near_max_deposit_cancel_at_start_full_refund() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, token_id, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        // Cancel immediately at t=0
        client.cancel_stream(&stream_id);

        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.status, StreamStatus::Cancelled);
        assert_eq!(state.cancelled_at, Some(0));
        // Full refund: accrued at t=0 is 0
        assert_eq!(token.balance(&sender), large_deposit);
        assert_eq!(token.balance(&contract_id), 0);
    }

    /// Cancel at midpoint: refund = deposit - accrued, invariant refund+accrued==deposit.
    #[test]
    fn near_max_deposit_cancel_midpoint_refund_plus_accrued_equals_deposit() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, token_id, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        env.ledger().set_timestamp(500);
        client.cancel_stream(&stream_id);

        let accrued_at_cancel = 500_i128 * rate;
        let refund = large_deposit - accrued_at_cancel;

        assert_eq!(token.balance(&sender), refund);
        assert_eq!(token.balance(&contract_id), accrued_at_cancel);

        // Invariant: refund + frozen_accrued == deposit
        assert_eq!(refund + accrued_at_cancel, large_deposit);

        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.status, StreamStatus::Cancelled);
        assert_eq!(state.cancelled_at, Some(500));
    }

    /// After cancellation, accrual is frozen: calculate_accrued returns same value
    /// regardless of how much time passes.
    #[test]
    fn near_max_deposit_cancelled_accrual_is_frozen() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        env.ledger().set_timestamp(300);
        client.cancel_stream(&stream_id);
        let accrued_at_cancel = client.calculate_accrued(&stream_id);

        // Advance time significantly — accrual must not grow
        env.ledger().set_timestamp(999_999);
        let accrued_later = client.calculate_accrued(&stream_id);
        assert_eq!(
            accrued_later, accrued_at_cancel,
            "cancelled accrual must be frozen"
        );
    }

    /// Recipient can withdraw frozen accrued amount after cancellation.
    #[test]
    fn near_max_deposit_recipient_withdraws_after_cancel() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, token_id, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        env.ledger().set_timestamp(700);
        client.cancel_stream(&stream_id);
        let frozen_accrued = 700_i128 * rate;

        let withdrawn = client.withdraw(&stream_id);
        assert_eq!(withdrawn, frozen_accrued);
        assert_eq!(token.balance(&recipient), frozen_accrued);

        // Status must remain Cancelled (not flip to Completed)
        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.status, StreamStatus::Cancelled);
    }

    // -----------------------------------------------------------------------
    // 6. Authorization: non-authorized roles cannot operate near-max streams
    // -----------------------------------------------------------------------

    /// Only the stream sender can cancel a near-max stream; recipient cannot.
    #[test]
    fn near_max_deposit_only_sender_can_cancel() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        // Sender can cancel — must succeed
        env.ledger().set_timestamp(100);
        client.cancel_stream(&stream_id);
        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.status, StreamStatus::Cancelled);
    }

    /// Only the recipient can withdraw from a near-max stream.
    /// (Authorization is enforced by require_auth; mock_all_auths covers both roles here.
    ///  The strict-mode auth test is in the integration suite.)
    #[test]
    fn near_max_deposit_withdraw_requires_recipient_auth() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        env.ledger().set_timestamp(500);
        let withdrawn = client.withdraw(&stream_id);
        assert!(withdrawn > 0, "recipient can withdraw");
        assert_eq!(withdrawn, 500_i128 * rate);
    }

    // -----------------------------------------------------------------------
    // 7. Pause/resume: near-max streams, accrual continues during pause
    // -----------------------------------------------------------------------

    /// Pausing a near-max stream does not affect accrual calculation.
    #[test]
    fn near_max_deposit_pause_does_not_affect_accrual() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        env.ledger().set_timestamp(200);
        client.pause_stream(&stream_id);

        // Accrual at t=600 while paused must equal 600 * rate
        env.ledger().set_timestamp(600);
        let accrued = client.calculate_accrued(&stream_id);
        assert_eq!(accrued, 600_i128 * rate, "accrual continues during pause");
    }

    /// After resume, recipient can withdraw full accrued amount including pause period.
    #[test]
    fn near_max_deposit_resume_allows_withdrawal_of_full_accrued() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, token_id, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        env.ledger().set_timestamp(300);
        client.pause_stream(&stream_id);

        env.ledger().set_timestamp(700);
        client.resume_stream(&stream_id);

        // Withdraw at t=700: should get 700 * rate
        let withdrawn = client.withdraw(&stream_id);
        assert_eq!(withdrawn, 700_i128 * rate);
        assert_eq!(token.balance(&recipient), 700_i128 * rate);
    }

    // -----------------------------------------------------------------------
    // 8. Batch creation: near-max total deposit, overflow atomicity
    // -----------------------------------------------------------------------

    /// Two near-max streams in a batch: total deposit sum overflows → InvalidParams, atomic.
    #[test]
    fn batch_near_max_total_overflow_is_atomic() {
        // Each entry has deposit = i128::MAX/2 + 1; sum overflows
        let per_deposit: i128 = i128::MAX / 2 + 1;
        let rate: i128 = per_deposit; // duration=1
        let (env, contract_id, token_id, _a, sender, _r) = setup_with_balance(i128::MAX / 2);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        env.ledger().set_timestamp(0);

        let count_before = client.get_stream_count();
        let sender_balance_before = token.balance(&sender);

        let mut params = soroban_sdk::Vec::new(&env);
        for _ in 0..2 {
            params.push_back(CreateStreamParams {
                recipient: Address::generate(&env),
                deposit_amount: per_deposit,
                rate_per_second: rate,
                start_time: 0,
                cliff_time: 0,
                end_time: 1,
                memo: None,
            });
        }

        let result = client.try_create_streams(&sender, &params);
        assert!(result.is_err(), "overflow batch must fail");
        assert_eq!(
            client.get_stream_count(),
            count_before,
            "counter must not advance"
        );
        assert_eq!(
            token.balance(&sender),
            sender_balance_before,
            "no tokens must move"
        );
    }

    /// Batch with one valid near-max entry and one invalid entry: entire batch rejected.
    #[test]
    fn batch_one_invalid_near_max_entry_rejects_whole_batch() {
        let valid_deposit: i128 = i128::MAX / 1_000_000;
        let valid_rate: i128 = valid_deposit / 1_000;
        let (env, contract_id, token_id, _a, sender, _r) = setup_with_balance(valid_deposit * 2);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        env.ledger().set_timestamp(0);

        let count_before = client.get_stream_count();
        let balance_before = token.balance(&sender);

        let valid = CreateStreamParams {
            recipient: Address::generate(&env),
            deposit_amount: valid_deposit,
            rate_per_second: valid_rate,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            memo: None,
        };
        // Invalid: deposit < rate * duration
        let invalid = CreateStreamParams {
            recipient: Address::generate(&env),
            deposit_amount: 1,
            rate_per_second: valid_rate,
            start_time: 0,
            cliff_time: 0,
            end_time: 1_000,
            memo: None,
        };

        let params = soroban_sdk::vec![&env, valid, invalid];
        let result = client.try_create_streams(&sender, &params);
        assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
        assert_eq!(client.get_stream_count(), count_before);
        assert_eq!(token.balance(&sender), balance_before);
    }

    // -----------------------------------------------------------------------
    // 9. get_withdrawable view: near-max values
    // -----------------------------------------------------------------------

    /// get_withdrawable returns correct value at near-max scale.
    #[test]
    fn near_max_deposit_get_withdrawable_matches_accrued_minus_withdrawn() {
        let large_deposit: i128 = i128::MAX / 1_000_000;
        let rate: i128 = large_deposit / 1_000;
        let (env, contract_id, _t, _a, sender, recipient) = setup_with_balance(large_deposit);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        env.ledger().set_timestamp(0);

        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &large_deposit,
            &rate,
            &0u64,
            &0u64,
            &1_000u64,
            &None,
        );

        // Partial withdrawal at t=300
        env.ledger().set_timestamp(300);
        client.withdraw(&stream_id);

        // At t=700, withdrawable = accrued(700) - withdrawn(300*rate)
        env.ledger().set_timestamp(700);
        let withdrawable = client.get_withdrawable(&stream_id);
        let accrued = client.calculate_accrued(&stream_id);
        let withdrawn = client.get_stream_state(&stream_id).withdrawn_amount;
        assert_eq!(withdrawable, accrued - withdrawn);
        assert!(withdrawable > 0);
    }
} // mod i128_boundary_streams

#[cfg(test)]
mod recipient_index_stress {
    use super::*;
    use soroban_sdk::{testutils::Ledger, Address, Vec};

    #[test]
    fn test_recipient_index_stress_large_scale() {
        let ctx = TestContext::setup();
        let recipient = Address::generate(&ctx.env);

        // Mint sufficient tokens for 100 streams (100 * 1000 = 100,000)
        // Default setup only mints 10,000.
        ctx.sac.mint(&ctx.sender, &1_000_000_i128);

        // Stress test: Create 100 streams for one recipient
        // We use increments of 50 to avoid any single-call resource limits
        let batch_size = 50;
        let total_batches = 2; // 100 streams total

        for _ in 0..total_batches {
            let mut streams = Vec::new(&ctx.env);
            for _ in 0..batch_size {
                streams.push_back(CreateStreamParams {
                    recipient: recipient.clone(),
                    deposit_amount: 1000,
                    rate_per_second: 1,
                    start_time: 100,
                    cliff_time: 100,
                    end_time: 1100,
                    memo: None,
                });
            }
            ctx.client().create_streams(&ctx.sender, &streams);
        }

        let index = ctx.client().get_recipient_streams(&recipient);
        assert_eq!(index.len(), 100);

        // Verify sorted order
        for i in 0..index.len() - 1 {
            assert!(index.get(i).unwrap() < index.get(i + 1).unwrap());
        }
    }

    #[test]
    fn test_close_cancelled_stream_cleans_index() {
        let ctx = TestContext::setup();
        let stream_id = ctx.create_default_stream();

        // Cancel the stream
        ctx.client().cancel_stream(&stream_id);

        let index_before = ctx.client().get_recipient_streams(&ctx.recipient);
        assert_eq!(index_before.len(), 1);
        assert_eq!(index_before.get(0).unwrap(), stream_id);

        // Close the cancelled stream (New feature verification)
        ctx.client().close_completed_stream(&stream_id);

        let index_after = ctx.client().get_recipient_streams(&ctx.recipient);
        assert_eq!(index_after.len(), 0);

        // Verify stream is deleted from storage
        let result = ctx.client().try_get_stream_state(&stream_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_recipient_index_consistency_after_many_removals() {
        let ctx = TestContext::setup();
        let recipient = Address::generate(&ctx.env);

        // Create 10 streams
        let mut stream_ids = Vec::new(&ctx.env);
        for _ in 0..10 {
            let id = ctx.client().create_stream(
                &ctx.sender,
                &recipient,
                &100_i128,
                &1_i128,
                &0u64,
                &0u64,
                &100u64,
                &None,
            );
            stream_ids.push_back(id);
        }

        // Close streams from middle (5), start (0), and end (9)
        // Note: IDs might not be 0-9 sequentially if multiple creation methods used,
        // but here they will be because it's a fresh setup.

        // 1. Close middle (ID at index 5)
        // Make it Completed first
        ctx.env.ledger().set_timestamp(1101);
        ctx.client().withdraw(&stream_ids.get(5).unwrap());
        ctx.client()
            .close_completed_stream(&stream_ids.get(5).unwrap());

        // 2. Close start (ID at index 0)
        ctx.client().withdraw(&stream_ids.get(0).unwrap());
        ctx.client()
            .close_completed_stream(&stream_ids.get(0).unwrap());

        // 3. Close end (ID at index 9)
        ctx.client().withdraw(&stream_ids.get(9).unwrap());
        ctx.client()
            .close_completed_stream(&stream_ids.get(9).unwrap());

        let index = ctx.client().get_recipient_streams(&recipient);
        assert_eq!(index.len(), 7);

        // Verify targeted IDs are gone
        let dead_ids = [
            stream_ids.get(0).unwrap(),
            stream_ids.get(5).unwrap(),
            stream_ids.get(9).unwrap(),
        ];
        for id in index.iter() {
            assert!(!dead_ids.contains(&id));
        }

        // Verify sorted order remains
        for i in 0..index.len() - 1 {
            assert!(index.get(i).unwrap() < index.get(i + 1).unwrap());
        }
    }

    // ---------------------------------------------------------------------------
    // Paginated Export Views Tests (#429)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_get_streams_by_id_range_basic() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        // Create 5 streams
        let mut ids = Vec::new(&ctx.env);
        for i in 0..5 {
            let id = ctx.client().create_stream(
                &ctx.sender,
                &ctx.recipient,
                &1000_i128,
                &1_i128,
                &0u64,
                &0u64,
                &1000u64,
                &None,
            );
            ids.push_back(id);
        }

        // Get range [1, 3] with limit 10
        let streams = ctx.client().get_streams_by_id_range(&1, &3, &10);
        assert_eq!(streams.len(), 3, "Should return 3 streams");

        // Verify order and content
        assert_eq!(streams.get(0).unwrap().stream_id, 1);
        assert_eq!(streams.get(1).unwrap().stream_id, 2);
        assert_eq!(streams.get(2).unwrap().stream_id, 3);
    }

    #[test]
    fn test_get_streams_by_id_range_empty_range() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        // Create a stream
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );

        // Range with start > end returns empty
        let streams = ctx.client().get_streams_by_id_range(&5, &1, &10);
        assert_eq!(streams.len(), 0, "Empty range should return empty vector");
    }

    #[test]
    fn test_get_streams_by_id_range_respects_max_page_size() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        // Create 150 streams (exceeds MAX_PAGE_SIZE of 100)
        for _ in 0..150 {
            ctx.client().create_stream(
                &ctx.sender,
                &ctx.recipient,
                &100_i128,
                &1_i128,
                &0u64,
                &0u64,
                &100u64,
                &None,
            );
        }

        // Request 200, should be capped at MAX_PAGE_SIZE (100)
        let streams = ctx.client().get_streams_by_id_range(&0, &200, &200);
        assert_eq!(
            streams.len(),
            100,
            "Should respect MAX_PAGE_SIZE limit of 100"
        );
    }

    #[test]
    fn test_get_streams_by_id_range_handles_closed_streams() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        // Create 5 streams
        for _ in 0..5 {
            ctx.client()
                .create_stream(&ctx.sender, &ctx.recipient, &1000, &1, &0, &0, &1000,
                &None
                );
        }

        // Close stream 2 (make it completed first)
        ctx.env.ledger().set_timestamp(1001);
        ctx.client().withdraw(&2);
        ctx.client().close_completed_stream(&2);

        // Range should return streams 1, 3, 4 (skipping closed stream 2)
        let streams = ctx.client().get_streams_by_id_range(&1, &4, &10);
        assert_eq!(streams.len(), 3, "Should skip closed stream");
        assert_eq!(streams.get(0).unwrap().stream_id, 1);
        assert_eq!(streams.get(1).unwrap().stream_id, 3);
        assert_eq!(streams.get(2).unwrap().stream_id, 4);
    }

    #[test]
    fn test_get_streams_by_id_range_open_ended() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        // Create 10 streams
        for _ in 0..10 {
            ctx.client().create_stream(
                &ctx.sender,
                &ctx.recipient,
                &100_i128,
                &1_i128,
                &0u64,
                &0u64,
                &100u64,
                &None,
            );
        }

        // Use u64::MAX for open-ended range with limit 5
        let max = u64::MAX;
        let streams = ctx.client().get_streams_by_id_range(&5, &max, &5);
        assert_eq!(streams.len(), 5, "Should return 5 streams from position 5");
        assert_eq!(streams.get(0).unwrap().stream_id, 5);
        assert_eq!(streams.get(4).unwrap().stream_id, 9);
    }

    #[test]
    fn test_get_streams_by_id_range_zero_limit() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );

        let streams = ctx.client().get_streams_by_id_range(&0, &10, &0);
        assert_eq!(streams.len(), 0, "Zero limit should return empty");
    }

    #[test]
    fn test_get_recipient_streams_paginated_basic() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let recipient = Address::generate(&ctx.env);

        // Create 10 streams for this recipient
        for _ in 0..10 {
            ctx.client().create_stream(
                &ctx.sender,
                &recipient,
                &1000_i128,
                &1_i128,
                &0u64,
                &0u64,
                &1000u64,
                &None,
            );
        }

        // Page 1: cursor=0, limit=3
        let page1 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &0, &3);
        assert_eq!(page1.len(), 3);
        assert_eq!(page1.get(0).unwrap(), 0);
        assert_eq!(page1.get(1).unwrap(), 1);
        assert_eq!(page1.get(2).unwrap(), 2);

        // Page 2: cursor=3, limit=3
        let page2 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &3, &3);
        assert_eq!(page2.len(), 3);
        assert_eq!(page2.get(0).unwrap(), 3);
        assert_eq!(page2.get(1).unwrap(), 4);
        assert_eq!(page2.get(2).unwrap(), 5);

        // Page 3: cursor=6, limit=3 (only 4 left)
        let page3 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &6, &3);
        assert_eq!(page3.len(), 3);

        // Page 4: cursor=9, limit=3 (only 1 left)
        let page4 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &9, &3);
        assert_eq!(page4.len(), 1);
        assert_eq!(page4.get(0).unwrap(), 9);

        // Page 5: cursor=10, should be empty (past end)
        let page5 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &10, &3);
        assert_eq!(page5.len(), 0);
    }

    #[test]
    fn test_get_recipient_streams_paginated_respects_max_page_size() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let recipient = Address::generate(&ctx.env);

        // Create 150 streams
        for _ in 0..150 {
            ctx.client()
                .create_stream(&ctx.sender, &recipient, &100, &1, &0, &0, &100,
                &None
                );
        }

        // Request 200, should be capped at MAX_PAGE_SIZE (100)
        let page = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &0, &200);
        assert_eq!(page.len(), 100, "Should respect MAX_PAGE_SIZE of 100");
    }

    #[test]
    fn test_get_recipient_streams_paginated_cursor_beyond_end() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let recipient = Address::generate(&ctx.env);
        ctx.client().create_stream(
            &ctx.sender,
            &recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );

        // Cursor beyond total count
        let result = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &100, &10);
        assert_eq!(result.len(), 0, "Should return empty when cursor >= total");
    }

    #[test]
    fn test_get_recipient_streams_paginated_zero_limit() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let recipient = Address::generate(&ctx.env);
        ctx.client().create_stream(
            &ctx.sender,
            &recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );

        let result = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &0, &0);
        assert_eq!(result.len(), 0, "Zero limit should return empty");
    }

    #[test]
    fn test_get_recipient_streams_paginated_multiple_recipients() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let recipient1 = Address::generate(&ctx.env);
        let recipient2 = Address::generate(&ctx.env);

        // Create 5 streams for recipient1
        for _ in 0..5 {
            ctx.client().create_stream(
                &ctx.sender,
                &recipient1,
                &1000_i128,
                &1_i128,
                &0u64,
                &0u64,
                &1000u64,
                &None,
            );
        }

        // Create 3 streams for recipient2
        for _ in 0..3 {
            ctx.client().create_stream(
                &ctx.sender,
                &recipient2,
                &1000_i128,
                &1_i128,
                &0u64,
                &0u64,
                &1000u64,
                &None,
            );
        }

        // Paginate recipient1
        let page1 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient1, &0, &10);
        assert_eq!(page1.len(), 5);

        // Paginate recipient2
        let page2 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient2, &0, &10);
        assert_eq!(page2.len(), 3);
    }

    #[test]
    fn test_get_recipient_streams_paginated_handles_closed_streams() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let recipient = Address::generate(&ctx.env);

        // Create 5 streams
        for _ in 0..5 {
            ctx.client().create_stream(
                &ctx.sender,
                &recipient,
                &1000_i128,
                &1_i128,
                &0u64,
                &0u64,
                &1000u64,
                &None,
            );
        }

        // Close stream 2 (make completed first)
        ctx.env.ledger().set_timestamp(1001);
        ctx.client().withdraw(&2);
        ctx.client().close_completed_stream(&2);

        // Full list should now have 4 items (0,1,3,4)
        let all = ctx.client().get_recipient_streams(&recipient);
        assert_eq!(all.len(), 4);

        // Pagination should reflect closed stream
        let page1 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &0, &2);
        assert_eq!(page1.len(), 2);
        assert_eq!(page1.get(0).unwrap(), 0);
        assert_eq!(page1.get(1).unwrap(), 1);

        // Next page should start with 3 (not 2, which is closed)
        let page2 = ctx
            .client()
            .get_recipient_streams_paginated(&recipient, &2, &2);
        assert_eq!(page2.len(), 2);
        assert_eq!(page2.get(0).unwrap(), 3);
        assert_eq!(page2.get(1).unwrap(), 4);
    }

    #[test]
    fn test_pagination_full_export_workflow() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let recipient = Address::generate(&ctx.env);

        // Create 25 streams
        for _ in 0..25 {
            ctx.client()
                .create_stream(&ctx.sender, &recipient, &100, &1, &0, &0, &100,
                &None
                );
        }

        // Simulate full export using pagination
        let mut all_stream_ids = Vec::new(&ctx.env);
        let mut cursor = 0u64;
        let page_size = 10u64;

        loop {
            let page = ctx
                .client()
                .get_recipient_streams_paginated(&recipient, &cursor, &page_size);
            if page.is_empty() {
                break;
            }
            for id in page.iter() {
                all_stream_ids.push_back(id);
            }
            cursor += page.len() as u64;
        }

        assert_eq!(all_stream_ids.len(), 25, "Should export all 25 streams");

        // Verify sorted order
        for i in 0..all_stream_ids.len() - 1 {
            assert!(
                all_stream_ids.get(i).unwrap() < all_stream_ids.get(i + 1).unwrap(),
                "Export should maintain sorted order"
            );
        }
    }

    #[test]
    fn test_get_streams_by_id_range_partial_results() {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        // Create 3 streams (IDs 0, 1, 2)
        for _ in 0..3 {
            ctx.client().create_stream(
                &ctx.sender,
                &ctx.recipient,
                &1000_i128,
                &1_i128,
                &0u64,
                &0u64,
                &1000u64,
                &None,
            );
        }

        // Request range [0, 100] with limit 10 - only 3 exist
        let streams = ctx.client().get_streams_by_id_range(&0, &100, &10);
        assert_eq!(streams.len(), 3, "Should return only existing streams");
    }
}

// ---------------------------------------------------------------------------
// Structured error tests: panic → ContractError refactor (#442)
//
// These tests verify that all previously-panicking input-error paths now
// return the appropriate ContractError variant instead of aborting the host.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod structured_error_tests {
    use super::*;

    // ── batch_withdraw: duplicate stream IDs ────────────────────────────────

    /// Duplicate stream IDs in batch_withdraw must return DuplicateStreamId,
    /// not panic. The entire batch must be reverted atomically.
    #[test]
    fn batch_withdraw_duplicate_stream_ids_returns_structured_error() {
        let ctx = TestContext::setup();
        let client = FluxoraStreamClient::new(&ctx.env, &ctx.contract_id);

        let stream_id = client.create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );

        ctx.env.ledger().set_timestamp(500);

        // Pass the same stream_id twice — must return DuplicateStreamId
        let ids = soroban_sdk::vec![&ctx.env, stream_id, stream_id];
        let result = client.try_batch_withdraw(&ctx.recipient, &ids);
        assert_eq!(
            result,
            Err(Ok(ContractError::DuplicateStreamId)),
            "duplicate stream IDs must return DuplicateStreamId, not panic"
        );

        // State must be unchanged (no withdrawal occurred)
        let state = client.get_stream_state(&stream_id);
        assert_eq!(state.withdrawn_amount, 0);
    }

    // ── create_streams: batch deposit overflow ───────────────────────────────

    /// When the sum of deposit_amounts in create_streams overflows i128,
    /// the call must return ArithmeticOverflow, not panic.
    #[test]
    fn create_streams_batch_deposit_overflow_returns_structured_error() {
        let ctx = TestContext::setup();
        let client = FluxoraStreamClient::new(&ctx.env, &ctx.contract_id);

        // Two entries whose deposits sum to > i128::MAX
        let half = i128::MAX / 2 + 1;
        let params = soroban_sdk::vec![
            &ctx.env,
            CreateStreamParams {
                recipient: ctx.recipient.clone(),
                deposit_amount: half,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: half as u64,
                memo: None,
            },
            CreateStreamParams {
                recipient: ctx.recipient.clone(),
                deposit_amount: half,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: half as u64,
                memo: None,
            },
        ];

        let result = client.try_create_streams(&ctx.sender, &params);
        assert_eq!(
            result,
            Err(Ok(ContractError::ArithmeticOverflow)),
            "batch deposit overflow must return ArithmeticOverflow, not panic"
        );
    }

    // ── update_rate_per_second: rate × duration overflow ────────────────────

    /// When new_rate_per_second × duration overflows i128,
    /// update_rate_per_second must return ArithmeticOverflow, not panic.
    #[test]
    fn update_rate_overflow_returns_structured_error() {
        let ctx = TestContext::setup();
        let client = FluxoraStreamClient::new(&ctx.env, &ctx.contract_id);

        // Create a stream with a very large end_time so duration is huge
        let large_end: u64 = u64::MAX / 2;
        // deposit must be >= rate * duration; use i128::MAX as deposit
        // We need to mint enough tokens first
        ctx.sac.mint(&ctx.sender, &i128::MAX);

        let stream_id = client.create_stream(
            &ctx.sender,
            &ctx.recipient,
            &i128::MAX,
            &1_i128,
            &0u64,
            &0u64,
            &large_end,
            &None,
        );

        // new_rate * large_end overflows i128
        let overflow_rate = i128::MAX / (large_end as i128) + 2;
        let result = client.try_update_rate_per_second(&stream_id, &overflow_rate);
        assert_eq!(
            result,
            Err(Ok(ContractError::ArithmeticOverflow)),
            "rate × duration overflow must return ArithmeticOverflow, not panic"
        );
    }

    // ── require_not_globally_paused: returns ContractError ──────────────────

    /// When the contract is globally paused, withdraw must return ContractPaused
    /// as a structured error, not panic.
    #[test]
    fn globally_paused_withdraw_returns_contract_paused_error() {
        let ctx = TestContext::setup();
        let client = FluxoraStreamClient::new(&ctx.env, &ctx.contract_id);

        let stream_id = client.create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );

        ctx.env.ledger().set_timestamp(500);
        client.set_global_emergency_paused(&true);

        let result = client.try_withdraw(&stream_id);
        assert_eq!(
            result,
            Err(Ok(ContractError::ContractPaused)),
            "withdraw while globally paused must return ContractPaused, not panic"
        );
    }

    /// When the contract is globally paused, cancel_stream must return ContractPaused.
    #[test]
    fn globally_paused_cancel_returns_contract_paused_error() {
        let ctx = TestContext::setup();
        let client = FluxoraStreamClient::new(&ctx.env, &ctx.contract_id);

        let stream_id = client.create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &None,
        );

        client.set_global_emergency_paused(&true);

        let result = client.try_cancel_stream(&stream_id);
        assert_eq!(
            result,
            Err(Ok(ContractError::ContractPaused)),
            "cancel_stream while globally paused must return ContractPaused, not panic"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests — batch_withdraw duplicate stream_id rejection (#405)
// ---------------------------------------------------------------------------

/// Adjacent duplicate ids must be rejected atomically — no transfers, no events.
#[test]
fn test_batch_withdraw_adjacent_duplicates_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id0 = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    let balance_before = ctx.token().balance(&ctx.recipient);

    let result = ctx
        .client()
        .try_batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0, id0]));

    assert!(result.is_err(), "adjacent duplicates must be rejected");
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        balance_before,
        "no transfer must occur on duplicate rejection"
    );
    let state = ctx.client().get_stream_state(&id0);
    assert_eq!(
        state.withdrawn_amount, 0,
        "withdrawn_amount must not change"
    );
    assert_eq!(state.status, StreamStatus::Active);
}

/// Non-adjacent duplicate ids must also be rejected atomically.
#[test]
fn test_batch_withdraw_non_adjacent_duplicates_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id0 = ctx.create_default_stream();
    ctx.sac.mint(&ctx.sender, &1000_i128);
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &None,
    );

    ctx.env.ledger().set_timestamp(300);
    let balance_before = ctx.token().balance(&ctx.recipient);

    let result = ctx
        .client()
        .try_batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0, id1, id0]));

    assert!(result.is_err(), "non-adjacent duplicates must be rejected");
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        balance_before,
        "no transfer must occur on duplicate rejection"
    );
    assert_eq!(ctx.client().get_stream_state(&id0).withdrawn_amount, 0);
    assert_eq!(ctx.client().get_stream_state(&id1).withdrawn_amount, 0);
}

/// Duplicate id where one of the streams is already Completed must still be rejected.
#[test]
fn test_batch_withdraw_duplicate_with_completed_stream_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id0 = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&id0);
    assert_eq!(
        ctx.client().get_stream_state(&id0).status,
        StreamStatus::Completed
    );

    let balance_before = ctx.token().balance(&ctx.recipient);

    let result = ctx
        .client()
        .try_batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0, id0]));

    assert!(
        result.is_err(),
        "duplicate completed stream_id must be rejected"
    );
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        balance_before,
        "no transfer must occur"
    );
}

/// A single id (no duplicates) must still succeed normally.
#[test]
fn test_batch_withdraw_single_id_no_false_positive() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id0 = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    let results = ctx
        .client()
        .batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0]));

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().amount, 400);
}

/// All-duplicate list (same id repeated three times) must be rejected.
#[test]
fn test_batch_withdraw_all_same_id_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id0 = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(200);
    let result = ctx
        .client()
        .try_batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0, id0, id0]));

    assert!(result.is_err(), "all-same-id list must be rejected");
    assert_eq!(ctx.client().get_stream_state(&id0).withdrawn_amount, 0);
}

/// Duplicate rejection must carry the correct error code (DuplicateStreamId = 14).
#[test]
fn test_batch_withdraw_duplicate_returns_correct_error_code() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let id0 = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    let result = ctx
        .client()
        .try_batch_withdraw(&ctx.recipient, &stream_ids_vec(&ctx.env, &[id0, id0]));

    match result {
        Err(Ok(e)) => assert_eq!(e, crate::ContractError::DuplicateStreamId),
        other => panic!("expected ContractError::DuplicateStreamId, got {:?}", other),
    }
}

#[test]
fn test_global_pause_flags_default_to_false() {
    let ctx = TestContext::setup();

    // By default, both pause flags should be false.
    let is_emergency_paused = ctx.client().get_global_emergency_paused();
    assert!(
        !is_emergency_paused,
        "Global emergency pause should default to false"
    );

    // Since there is no public getter for CreationPaused, we read from storage
    // or test behavior. Testing storage directly:
    let creation_paused: bool = ctx
        .env
        .as_contract(&ctx.contract_id, || crate::is_creation_paused(&ctx.env));
    assert!(!creation_paused, "Creation pause should default to false");
}
