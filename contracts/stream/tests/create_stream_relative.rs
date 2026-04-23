extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamRelativeParams, FluxoraStream, FluxoraStreamClient, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
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

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }
}

// ============================================================================
// Tests: create_stream_relative
// ============================================================================

/// Test that create_stream_relative with zero delays creates an immediate stream.
/// This is the simplest case: start_delay=0, cliff_delay=0, duration=X.
#[test]
fn create_stream_relative_zero_delays_immediate_start() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let stream_id = ctx.client().create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,  // start_delay: 0 -> start_time = 1000
        &0u64,  // cliff_delay: 0 -> cliff_time = 1000
        &1000u64, // duration: 1000 -> end_time = 1000 + 1000 = 2000
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.stream_id, 0);
    assert_eq!(state.start_time, 1000);
    assert_eq!(state.cliff_time, 1000);
    assert_eq!(state.end_time, 2000);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(ctx.token.balance(&ctx.sender), 9_000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_000);
}

/// Test that create_stream_relative with positive delays correctly offsets times.
#[test]
fn create_stream_relative_positive_delays_future_start() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let stream_id = ctx.client().create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &2_i128,
        &100u64,  // start_delay: 100 -> start_time = 1100
        &500u64,  // cliff_delay: 500 -> cliff_time = 1500
        &2000u64, // duration: 2000 -> end_time = 1100 + 2000 = 3100
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.start_time, 1100);
    assert_eq!(state.cliff_time, 1500);
    assert_eq!(state.end_time, 3100);
}

/// Test that create_stream_relative validates duration > 0.
/// When start_delay = cliff_delay, the duration must still be positive.
#[test]
fn create_stream_relative_zero_duration_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let result = ctx.client().try_create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,  // start_delay
        &100u64,  // cliff_delay
        &0u64,    // duration: 0 -> INVALID (start_time == end_time)
    );

    // Should fail because end_time would equal start_time
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Test cliff bounds: cliff_delay must correspond to cliff_time in range [start_time, end_time).
#[test]
fn create_stream_relative_cliff_less_than_start_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let result = ctx.client().try_create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &500u64,  // start_delay -> start_time = 1500
        &100u64,  // cliff_delay -> cliff_time = 1100 (< start_time)
        &1000u64, // duration
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Test cliff bounds: cliff_delay must correspond to cliff_time in range [start_time, end_time].
#[test]
fn create_stream_relative_cliff_greater_than_end_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    // start_time = 1100, end_time = 2100
    // cliff_time = 3000 (> end_time) -> INVALID
    let result = ctx.client().try_create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &100u64,   // start_delay -> start_time = 1100
        &2000u64,  // cliff_delay -> cliff_time = 3000 (> end_time = 2100)
        &1000u64,  // duration -> end_time = 2100
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Test underflow prevention: start_delay overflow check.
/// Adding delay to current_time should not cause u64 overflow.
#[test]
fn create_stream_relative_start_delay_overflow_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(u64::MAX - 100);

    let result = ctx.client().try_create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &u64::MAX, // start_delay overflow -> INVALID
        &0u64,
        &1000u64,
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Test underflow prevention: duration overflow check.
/// Adding duration to start_time should not cause u64 overflow.
#[test]
fn create_stream_relative_duration_overflow_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let result = ctx.client().try_create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &(u64::MAX - 500), // start_delay -> start_time near u64::MAX
        &0u64,
        &1000u64, // duration overflow -> INVALID
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Test that create_stream_relative never produces StartTimeInPast errors.
/// Even with current_time at ledger timestamp, all computed times are >= current_time.
#[test]
fn create_stream_relative_never_start_time_in_past() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(5000);

    // Even with zero delays, start_time = current_time (not past)
    let stream_id = ctx.client().create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    let state = ctx.client().get_stream_state(&stream_id);
    // start_time = 5000, which is == current_time (not < current_time)
    assert_eq!(state.start_time, 5000);
    assert!(state.start_time >= 5000, "start_time must be >= current_time");
}

/// Test that create_stream_relative preserves deposit validation.
/// Deposit must cover the total streamable amount.
#[test]
fn create_stream_relative_insufficient_deposit_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let result = ctx.client().try_create_stream_relative(
        &ctx.sender,
        &ctx.recipient,
        &500_i128,        // deposit_amount: 500
        &2_i128,          // rate_per_second: 2
        &0u64,
        &0u64,
        &300u64,          // duration: 300 -> streamable = 2 * 300 = 600 > 500
    );

    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
}

/// Test that create_stream_relative rejects self-streaming.
#[test]
fn create_stream_relative_rejects_self_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let result = ctx.client().try_create_stream_relative(
        &ctx.sender,
        &ctx.sender, // invalid: sender == recipient
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

// ============================================================================
// Tests: create_streams_relative (batch)
// ============================================================================

/// Test that create_streams_relative with a single entry creates a stream correctly.
#[test]
fn create_streams_relative_single_entry() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(2000);

    let params = vec![&ctx.env, CreateStreamRelativeParams {
        recipient: ctx.recipient,
        deposit_amount: 1000,
        rate_per_second: 1,
        start_delay: 100,
        cliff_delay: 200,
        duration: 1000,
    }];

    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);
    assert_eq!(ids.len(), 1);
    assert_eq!(ids.get_unchecked(0), 0);

    let state = ctx.client().get_stream_state(&0);
    assert_eq!(state.start_time, 2100);    // 2000 + 100
    assert_eq!(state.cliff_time, 2200);    // 2000 + 200
    assert_eq!(state.end_time, 3100);      // 2100 + 1000
}

/// Test that create_streams_relative with multiple entries creates all streams atomically.
#[test]
fn create_streams_relative_multiple_entries_sequential_ids() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let recipient2 = Address::generate(&ctx.env);
    let params = vec![
        &ctx.env,
        CreateStreamRelativeParams {
            recipient: ctx.recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
        },
        CreateStreamRelativeParams {
            recipient: recipient2,
            deposit_amount: 2000,
            rate_per_second: 2,
            start_delay: 100,
            cliff_delay: 100,
            duration: 2000,
        },
    ];

    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids.get_unchecked(0), 0);
    assert_eq!(ids.get_unchecked(1), 1);

    let state0 = ctx.client().get_stream_state(&0);
    assert_eq!(state0.recipient, ctx.recipient);
    assert_eq!(state0.start_time, 1000);

    let state1 = ctx.client().get_stream_state(&1);
    assert_eq!(state1.recipient, recipient2);
    assert_eq!(state1.start_time, 1100);
}

/// Test that create_streams_relative with an empty vector succeeds without side effects.
#[test]
fn create_streams_relative_empty_batch_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token.balance(&ctx.sender);
    let contract_balance_before = ctx.token.balance(&ctx.contract_id);

    let params: soroban_sdk::Vec<CreateStreamRelativeParams> = soroban_sdk::Vec::new(&ctx.env);
    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);

    assert_eq!(ids.len(), 0);
    assert_eq!(ctx.client().get_stream_count(), stream_count_before);
    assert_eq!(ctx.token.balance(&ctx.sender), sender_balance_before);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_balance_before);
}

/// Test that create_streams_relative is atomic: if one entry is invalid, all streams fail.
#[test]
fn create_streams_relative_invalid_entry_fails_atomically() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let stream_count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token.balance(&ctx.sender);

    let recipient2 = Address::generate(&ctx.env);
    let params = vec![
        &ctx.env,
        CreateStreamRelativeParams {
            recipient: ctx.recipient,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
        },
        CreateStreamRelativeParams {
            recipient: recipient2,
            deposit_amount: 500,
            rate_per_second: 2,
            start_delay: 0,
            cliff_delay: 0,
            duration: 0, // INVALID: duration = 0
        },
    ];

    let result = ctx.client().try_create_streams_relative(&ctx.sender, &params);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Verify atomicity: no streams created, no tokens transferred
    assert_eq!(ctx.client().get_stream_count(), stream_count_before);
    assert_eq!(ctx.token.balance(&ctx.sender), sender_balance_before);
}

/// Test that create_streams_relative with all entries having unique recipients and times.
#[test]
fn create_streams_relative_diverse_schedules() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(10000);

    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);
    let r3 = Address::generate(&ctx.env);

    let params = vec![
        &ctx.env,
        CreateStreamRelativeParams {
            recipient: r1,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
        },
        CreateStreamRelativeParams {
            recipient: r2,
            deposit_amount: 2000,
            rate_per_second: 2,
            start_delay: 500,
            cliff_delay: 1000,
            duration: 2000,
        },
        CreateStreamRelativeParams {
            recipient: r3,
            deposit_amount: 3000,
            rate_per_second: 3,
            start_delay: 1000,
            cliff_delay: 2000,
            duration: 3000,
        },
    ];

    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);
    assert_eq!(ids.len(), 3);

    // Verify all three streams created with correct schedules
    let s0 = ctx.client().get_stream_state(&ids.get_unchecked(0));
    assert_eq!(s0.start_time, 10000);
    assert_eq!(s0.end_time, 11000);

    let s1 = ctx.client().get_stream_state(&ids.get_unchecked(1));
    assert_eq!(s1.start_time, 10500);
    assert_eq!(s1.cliff_time, 11000);
    assert_eq!(s1.end_time, 12500);

    let s2 = ctx.client().get_stream_state(&ids.get_unchecked(2));
    assert_eq!(s2.start_time, 11000);
    assert_eq!(s2.cliff_time, 12000);
    assert_eq!(s2.end_time, 14000);

    // Verify total deposit transferred
    assert_eq!(ctx.token.balance(&ctx.sender), 4_000); // 10000 - 6000
    assert_eq!(ctx.token.balance(&ctx.contract_id), 6_000);
}

/// Test that create_streams_relative correctly computes cliff times independently per entry.
#[test]
fn create_streams_relative_independent_cliff_times() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);

    let params = vec![
        &ctx.env,
        CreateStreamRelativeParams {
            recipient: r1,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,  // cliff at current time
            duration: 1000,
        },
        CreateStreamRelativeParams {
            recipient: r2,
            deposit_amount: 2000,
            rate_per_second: 1,
            start_delay: 500,
            cliff_delay: 1500, // cliff 500 seconds after start
            duration: 1000,
        },
    ];

    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);

    let s0 = ctx.client().get_stream_state(&ids.get_unchecked(0));
    assert_eq!(s0.start_time, 1000);
    assert_eq!(s0.cliff_time, 1000);

    let s1 = ctx.client().get_stream_state(&ids.get_unchecked(1));
    assert_eq!(s1.start_time, 1500);
    assert_eq!(s1.cliff_time, 2500);
}

/// Test that overflow in batch parameters is caught for each entry.
#[test]
fn create_streams_relative_batch_overflow_detection() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(u64::MAX - 100);

    let r1 = Address::generate(&ctx.env);

    let params = vec![
        &ctx.env,
        CreateStreamRelativeParams {
            recipient: r1,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: u64::MAX, // overflow
            cliff_delay: 0,
            duration: 1000,
        },
    ];

    let result = ctx.client().try_create_streams_relative(&ctx.sender, &params);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Test deposit and rate validation still applied in batch relative creation.
#[test]
fn create_streams_relative_batch_validates_amounts() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);

    let params = vec![
        &ctx.env,
        CreateStreamRelativeParams {
            recipient: r1,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
        },
        CreateStreamRelativeParams {
            recipient: r2,
            deposit_amount: -100, // Invalid: negative amount
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
        },
    ];

    let result = ctx.client().try_create_streams_relative(&ctx.sender, &params);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}
