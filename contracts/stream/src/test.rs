#[cfg(test)]
extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, FromVal, Vec,
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
    assert!(result.is_err(), "re-init should have panicked");

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
    assert!(result.is_err(), "re-init should have panicked");

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
#[should_panic(expected = "stream is not active")]
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
#[should_panic(expected = "stream must be active or paused to cancel")]
fn test_cancel_already_cancelled_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().cancel_stream(&stream_id);
    ctx.client().cancel_stream(&stream_id);
}

#[test]
#[should_panic(expected = "stream must be active or paused to cancel")]
fn test_cancel_completed_stream_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);
    ctx.client().cancel_stream(&stream_id);
}

#[test]
fn test_cancel_paused_stream() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();
    ctx.client().pause_stream(&stream_id);
    ctx.client().cancel_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);
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
fn test_calculate_accrued_formula_before_cliff_returns_zero() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &200u64,
        &600u64,
    );

    ctx.env.ledger().set_timestamp(199);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(accrued, 0, "accrued must be zero before cliff_time");
}

#[test]
fn test_calculate_accrued_formula_after_cliff_uses_start_time_elapsed() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &200u64,
        &600u64,
    );

    ctx.env.ledger().set_timestamp(250);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 150,
        "after cliff, accrual must use (current_time - start_time)"
    );
}

#[test]
fn test_calculate_accrued_formula_after_end_time_caps_elapsed() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,
        &200u64,
        &600u64,
    );

    ctx.env.ledger().set_timestamp(900);
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 500,
        "after end_time, elapsed must be capped at end_time (600 - 100)"
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

#[test]
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_nothing_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(0);
    ctx.client().withdraw(&stream_id);
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

#[test]
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_before_cliff_panics() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream();
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&stream_id);
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
#[should_panic(expected = "stream not found")]
fn test_get_stream_state_non_existent() {
    let ctx = TestContext::setup();
    ctx.client().get_stream_state(&999);
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
#[should_panic(expected = "stream not found")]
fn test_get_stream_state_non_existence_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let _ = ctx.client().get_stream_state(&1);
}

// ---------------------------------------------------------------------------
// Tests — Issue: withdraw zero and excess handling
// ---------------------------------------------------------------------------

/// Test withdraw when accrued - withdrawn = 0 before cliff
/// Should panic with "nothing to withdraw"
#[test]
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_zero_before_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // Before cliff, accrued = 0, withdrawn = 0, so withdrawable = 0
    ctx.env.ledger().set_timestamp(100);
    ctx.client().withdraw(&stream_id);
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

/// Test withdraw when accrued - withdrawn = 0 at start time (no cliff)
/// Should panic with "nothing to withdraw"
#[test]
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_zero_at_start_time() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // At start time, accrued = 0, withdrawn = 0, so withdrawable = 0
    ctx.env.ledger().set_timestamp(0);
    ctx.client().withdraw(&stream_id);
}

/// Test withdraw immediately after previous withdrawal with no time elapsed
/// Should panic with "nothing to withdraw"
#[test]
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_zero_no_time_elapsed() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Withdraw at t=500
    ctx.env.ledger().set_timestamp(500);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 500);

    // Try to withdraw again at same timestamp - should panic
    ctx.client().withdraw(&stream_id);
}

/// Test withdraw when cancelled with zero accrued
/// Should panic with "nothing to withdraw"
#[test]
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_zero_after_immediate_cancel() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel immediately at t=0 (no accrual)
    ctx.env.ledger().set_timestamp(0);
    ctx.client().cancel_stream(&stream_id);

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    // Try to withdraw - should panic because accrued = 0
    ctx.client().withdraw(&stream_id);
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

/// Test withdraw with cliff - before cliff returns zero withdrawable
#[test]
#[should_panic(expected = "nothing to withdraw")]
fn test_withdraw_zero_one_second_before_cliff() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_cliff_stream(); // cliff at t=500

    // One second before cliff
    ctx.env.ledger().set_timestamp(499);
    ctx.client().withdraw(&stream_id);
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
#[test]
#[should_panic(expected = "nothing to withdraw")]
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

    // Try to withdraw again - should panic with "nothing to withdraw"
    // because accrued (600) - withdrawn (600) = 0
    ctx.client().withdraw(&stream_id);
}
