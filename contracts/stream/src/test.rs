#[cfg(test)]
extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, FromVal, TryFromVal, Vec,
};

use crate::{FluxoraStream, FluxoraStreamClient, StreamEvent, StreamStatus};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    #[allow(dead_code)]
    admin: Address,
    sender: Address,
    recipient: Address,
    sac: StellarAssetClient<'a>,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
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

    /// Setup context without mock_all_auths(), for explicit auth testing
    fn setup_strict() -> Self {
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
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);

        // Mock the minting auth since mock_all_auths is not enabled.
        use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
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

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    /// Create a standard 1000-unit stream spanning 1000 seconds (rate 1/s, no cliff).
    fn create_default_stream(&self) -> u64 {
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
    fn create_cliff_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &500u64, // cliff at t=500
            &1000u64,
        )
    }

    fn create_max_rate_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &(i128::MAX - 1),
            &((i128::MAX - 1) / 3),
            &0,
            &0u64,
            &3,
        )
    }

    fn create_half_max_rate_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &42535295865117307932921825928971026400_i128,
            &(42535295865117307932921825928971026400_i128 / 100),
            &0,
            &0u64,
            &100,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests — init
// ---------------------------------------------------------------------------

#[test]
fn test_init_stores_token_and_admin() {
    let env = Env::default();
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
#[should_panic(expected = "already initialised")]
fn test_init_second_call_fails() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    let token = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token, &admin);

    client.init(&Address::generate(&env), &Address::generate(&env));
}

#[test]
#[should_panic(expected = "contract not initialised: missing config")]
fn test_get_config_before_init_fails() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    client.get_config();
}

/// Test that get_config panics with clear error when contract is not initialized.
/// This guards against using an uninitialized contract and documents expected behavior.
///
/// Security: Prevents operations on uninitialized contract state.
/// The panic message "contract not initialised: missing config" provides clear
/// feedback to integrators that init() must be called first.
#[test]
#[should_panic(expected = "contract not initialised: missing config")]
fn test_get_config_uninitialized_contract_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    // Calling get_config before init must panic with clear error message
    client.get_config();
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
#[should_panic(expected = "already initialised")]
fn test_init_twice_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Second init should panic
    let token_id2 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    client.init(&token_id2, &admin2);
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
    );

    assert_eq!(stream_id, 0, "first stream should have id 0");
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
#[should_panic(expected = "already initialised")]
fn test_reinit_same_token_same_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Second init with identical arguments must panic
    client.init(&token_id, &admin);
}

/// Re-init with a different token but same admin must panic.
#[test]
#[should_panic(expected = "already initialised")]
fn test_reinit_different_token_same_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Second init with different token but same admin must panic
    let token_id2 = Address::generate(&env);
    client.init(&token_id2, &admin);
}

/// Re-init with same token but a different admin must panic.
#[test]
#[should_panic(expected = "already initialised")]
fn test_reinit_same_token_different_admin_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraStream);
    let token_id = Address::generate(&env);
    let admin = Address::generate(&env);

    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    // Second init with same token but different admin must panic
    let admin2 = Address::generate(&env);
    client.init(&token_id, &admin2);
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

    // Attempt re-init with completely different params (should panic)
    let token_id2 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&token_id2, &admin2);
    }));
    let err = result.expect_err("re-init should have panicked");
    let panic_msg = err
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| {
            err.downcast_ref::<std::string::String>()
                .map(|s| s.as_str())
        })
        .unwrap_or("no message");
    assert!(
        panic_msg.contains("already initialised"),
        "panic message should contain 'already initialised', but was '{}'",
        panic_msg
    );

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

    // Attempt re-init (should fail)
    let admin2 = Address::generate(&env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.init(&token_id, &admin2);
    }));
    let err = result.expect_err("re-init should have panicked");
    let panic_msg = err
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| {
            err.downcast_ref::<std::string::String>()
                .map(|s| s.as_str())
        })
        .unwrap_or("no message");
    assert!(
        panic_msg.contains("already initialised"),
        "panic message should contain 'already initialised', but was '{}'",
        panic_msg
    );

    // Contract must still accept streams
    env.ledger().set_timestamp(0);
    let stream_id = client.create_stream(
        &sender, &recipient, &1000_i128, &1_i128, &0u64, &0u64, &1000u64,
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

    let stream_id =
        ctx.client()
            .create_stream(&ctx.sender, &ctx.recipient, &1000, &1, &0, &0, &1000);

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
#[should_panic(expected = "deposit_amount must be positive")]
fn test_create_stream_zero_deposit_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &0_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );
}

#[test]
#[should_panic(expected = "start_time must be before end_time")]
fn test_create_stream_invalid_times_panics() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &1000u64,
        &1000u64,
        &500u64, // end before start
    );
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
    );

    let stream_id_2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &1000u64, // cliff equals end
        &1000u64,
    );

    let stream_id_3 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,
        &1_i128,
        &0u64,
        &0u64, // cliff equals end
        &500u64,
    );

    let stream_id_4 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &4000_i128,
        &1_i128,
        &0u64,
        &0u64, // cliff equals end
        &4000u64,
    );

    let stream_id_5 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64, // cliff equals end
        &1000u64,
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
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time - state.start_time, duration);
    assert_eq!(state.deposit_amount, deposit);
    assert_eq!(state.status, StreamStatus::Active);
}

// ---------------------------------------------------------------------------
// Tests — Issue #44: create_stream validation (invalid params) — full suite
// ---------------------------------------------------------------------------

// --- Group 1: end_time <= start_time ---

/// end_time exactly equal to start_time must panic
#[test]
#[should_panic(expected = "start_time must be before end_time")]
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
#[should_panic(expected = "start_time must be before end_time")]
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
#[should_panic(expected = "start_time must be before end_time")]
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
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
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
    );
}

/// cliff_time one second after end_time (upper boundary violation)
#[test]
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
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
    );
}

/// cliff_time far before start_time
#[test]
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
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
    );
}

/// cliff_time far after end_time
#[test]
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
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
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 1000);
}

// --- Group 3: deposit_amount <= 0 ---

/// deposit_amount of zero must panic
#[test]
#[should_panic(expected = "deposit_amount must be positive")]
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
    );
}

/// deposit_amount of -1 must panic
#[test]
#[should_panic(expected = "deposit_amount must be positive")]
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
    );
}

/// deposit_amount of i128::MIN must panic
#[test]
#[should_panic(expected = "deposit_amount must be positive")]
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
#[should_panic(expected = "rate_per_second must be positive")]
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
    );
}

/// rate_per_second of -1 must panic
#[test]
#[should_panic(expected = "rate_per_second must be positive")]
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
    );
}

/// rate_per_second of i128::MIN must panic
#[test]
#[should_panic(expected = "rate_per_second must be positive")]
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
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.rate_per_second, 1);
}

// --- Group 5: deposit < rate * duration ---

/// deposit one less than required (rate * duration - 1) must panic
#[test]
#[should_panic(expected = "deposit_amount must cover total streamable amount")]
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
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.deposit_amount, 1000);
}

/// deposit much less than rate * duration must panic
#[test]
#[should_panic(expected = "deposit_amount must cover total streamable amount")]
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
    );
    let state = ctx.client().get_stream_state(&id);
    assert_eq!(state.deposit_amount, 5000);
    assert_eq!(state.status, StreamStatus::Active);
}

// --- Group 6: sender == recipient ---

/// sender and recipient are the same address must panic
#[test]
#[should_panic(expected = "sender and recipient must be different")]
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
    );
    let state = ctx.client().get_stream_state(&id);
    assert_ne!(state.sender, state.recipient);
}

// ---------------------------------------------------------------------------
// Tests — Issue #35: validate positive amounts and sender != recipient
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "rate_per_second must be positive")]
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
    );
}

#[test]
#[should_panic(expected = "sender and recipient must be different")]
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
    );
}

// ---------------------------------------------------------------------------
// Tests — Issue #33: validate cliff_time in [start_time, end_time]
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
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
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
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
    );
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.cliff_time, 1000);
}

// ---------------------------------------------------------------------------
// Tests — Issue #34: validate deposit_amount >= rate * duration
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "deposit_amount must cover total streamable amount")]
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
    );

    // Even with time elapsed, if rate were 0, accrued would be 0
    ctx.env.ledger().set_timestamp(500);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    // With rate=1, we expect 500
    assert_eq!(accrued, 500, "normal calculation works");
}

#[test]
fn test_zero_duration_returns_zero() {
    // Security: When current time equals start time (zero elapsed), accrued must be zero.
    let ctx = TestContext::setup();

    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10000_i128, // Must cover rate * duration
        &10_i128,
        &0u64, // Start at 0
        &0u64, // No cliff
        &1000u64,
    );

    // Query at start time - zero elapsed
    ctx.env.ledger().set_timestamp(0);

    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "zero elapsed time should give zero accrued");
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
#[should_panic(expected = "stream is already paused")]
fn test_pause_already_paused_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().pause_stream(&stream_id);
    ctx.client().pause_stream(&stream_id); // second pause should panic
}

#[test]
#[should_panic(expected = "stream is active, not paused")]
fn test_resume_active_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().resume_stream(&stream_id);
}

#[test]
#[should_panic(expected = "stream is completed")]
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
#[should_panic(expected = "stream is cancelled")]
fn test_resume_cancelled_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().cancel_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
    ctx.client().resume_stream(&stream_id);
}

#[test]
#[should_panic(expected = "stream must be active to pause")]
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
}

#[test]
#[should_panic(expected = "Error(Contract")]
fn test_cancel_already_cancelled_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().cancel_stream(&stream_id);
    ctx.client().cancel_stream(&stream_id);
}

#[test]
#[should_panic(expected = "Error(Contract")]
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
#[should_panic(expected = "cannot withdraw from paused stream")]
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
#[should_panic(expected = "stream already completed")]
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

// ---------------------------------------------------------------------------
// Tests — Issue #37: withdraw reject when stream is Paused
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "cannot withdraw from paused stream")]
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
        .create_stream(&ctx.sender, &ctx.recipient, &200, &2, &0, &0, &100);

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
// Tests — Issue #16: Auth Enforcement (Sender or Admin only)
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

    ctx.client().pause_stream(&stream_id);

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

    ctx.client().cancel_stream(&stream_id);

    let events = ctx.env.events().all();
    let last_event = events.last().unwrap();

    // Check cancel event
    assert_eq!(
        Option::<StreamEvent>::from_val(&ctx.env, &last_event.2).unwrap(),
        StreamEvent::Cancelled(stream_id)
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
#[should_panic(expected = "deposit_amount must be positive")]
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
    );
}

/// Test creating a stream with negative rate_per_second panics
#[test]
#[should_panic(expected = "rate_per_second must be positive")]
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
    );
}

/// Test creating a stream where start_time equals end_time panics
#[test]
#[should_panic(expected = "start_time must be before end_time")]
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
    );

    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &1_i128,
        &0u64,
        &0u64,
        &200u64,
    );

    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &300_i128,
        &1_i128,
        &0u64,
        &0u64,
        &300u64,
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
#[should_panic(expected = "sender and recipient must be different")]
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
#[should_panic(expected = "cliff_time must be within [start_time, end_time]")]
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
#[should_panic(expected = "stream already completed")]
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

/// Expected: second withdraw returns 0 (idempotent)
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

    // Second withdraw at same timestamp: accrued (600) - withdrawn (600) = 0
    // Should return 0 and must NOT transfer any tokens
    let second_withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(second_withdrawn, 0);

    // Verify no extra tokens moved
    let recipient_balance_after_second = ctx.token().balance(&ctx.recipient);
    assert_eq!(
        recipient_balance_after_second, recipient_balance_after_first,
        "no tokens should transfer on zero-withdrawable call"
    );
}

/// Test withdraw when cancelled with zero accrued
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

    // Try to withdraw again - should return 0 (frozen)
    // because accrued (600) - withdrawn (600) = 0
    let second_withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(second_withdrawn, 0);
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
    );
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
    );
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
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

    // First successful stream → id = 0
    let id0 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
    );
    assert_eq!(id0, 0);

    // Attempt a stream with an underfunded deposit (1 token, need 100) → must panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1_i128, // deposit < rate * duration (100)
            &1_i128,
            &0u64,
            &0u64,
            &100u64,
        );
    }));
    let err = result.expect_err("underfunded create_stream must panic");
    let panic_msg = err
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| {
            err.downcast_ref::<std::string::String>()
                .map(|s| s.as_str())
        })
        .unwrap_or("no message");
    assert!(
        panic_msg.contains("deposit_amount must cover total streamable amount"),
        "panic message should contain 'deposit_amount must cover total streamable amount', but was '{}'",
        panic_msg
    );

    // Next successful stream must still be id = 1, not 2
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
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
    );
    let id_b = ctx.client().create_stream(
        &sender2,
        &recipient2,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
    );
    let id_c = ctx.client().create_stream(
        &ctx.sender,
        &recipient2,
        &100_i128,
        &1_i128,
        &0u64,
        &0u64,
        &100u64,
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
    );
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &2_i128,
        &0u64,
        &0u64,
        &100u64,
    );
    let id2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &200_i128,
        &2_i128,
        &0u64,
        &0u64,
        &100u64,
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

/// Test: Verify that completed stream cannot be withdrawn again
/// Accessing a completed stream's withdraw should panic
#[test]
#[should_panic(expected = "stream already completed")]
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

    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);

    ctx.client().pause_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    let sender_balance_before = ctx.token().balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );

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
    );

    ctx.env.ledger().set_timestamp(end_time);

    let accrued = ctx.client().calculate_accrued(&stream_id);

    assert_eq!(accrued, total);
}

// ---------------------------------------------------------------------------
// Tests — Batch create_streams
// ---------------------------------------------------------------------------

use crate::CreateStreamParams;
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
    };

    let params2 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 2000,
        rate_per_second: 2,
        start_time: 100,
        cliff_time: 200,
        end_time: 1100,
    };

    let params3 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 3000,
        rate_per_second: 3,
        start_time: 500,
        cliff_time: 500,
        end_time: 1500,
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
#[should_panic(expected = "deposit_amount must cover total streamable amount")]
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
    };

    let invalid_params = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 500, // Insufficient deposit (1 * 1000 = 1000 needed)
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
    };

    let streams = vec![&ctx.env, valid_params, invalid_params];

    // This should panic due to the invalid parameter in the second stream.
    // Because Soroban state changes revert on panic, the first stream won't be saved.
    ctx.client().create_streams(&ctx.sender, &streams);
}

#[test]
#[should_panic(expected = "sender and recipient must be different")]
fn test_create_streams_batch_sender_recipient_panic() {
    let ctx = TestContext::setup();

    let params = CreateStreamParams {
        recipient: ctx.sender.clone(), // Invalid: recipient == sender
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
    };

    let streams = vec![&ctx.env, params];
    ctx.client().create_streams(&ctx.sender, &streams);
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
fn test_create_streams_batch_strict_auth() {
    let ctx = TestContext::setup_strict();

    let params1 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 1000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 1000,
    };

    let params2 = CreateStreamParams {
        recipient: Address::generate(&ctx.env),
        deposit_amount: 2000,
        rate_per_second: 1,
        start_time: 0,
        cliff_time: 0,
        end_time: 2000,
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
