extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamRelativeParams, FluxoraStream, FluxoraStreamClient, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

#[allow(dead_code)]
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
        // Provide sufficient allowance for tests that don't explicitly test allowances.
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 4000,
            rate_per_second: 2,
            start_delay: 100,
            cliff_delay: 500,
            duration: 2000,
            memo: None,
            metadata: None,
        },
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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 100,
            cliff_delay: 100,
            duration: 0,
            memo: None,
            metadata: None,
        },
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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 500,
            cliff_delay: 100,
            duration: 1000,
            memo: None,
            metadata: None,
        },
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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 100,
            cliff_delay: 2000,
            duration: 1000,
            memo: None,
            metadata: None,
        },
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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: u64::MAX,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: u64::MAX - 500,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
    );

    let state = ctx.client().get_stream_state(&stream_id);
    // start_time = 5000, which is == current_time (not < current_time)
    assert_eq!(state.start_time, 5000);
    assert!(
        state.start_time >= 5000,
        "start_time must be >= current_time"
    );
}

/// Test that create_stream_relative preserves deposit validation.
/// Deposit must cover the total streamable amount.
#[test]
fn create_stream_relative_insufficient_deposit_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let result = ctx.client().try_create_stream_relative(
        &ctx.sender,
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 500,
            rate_per_second: 2,
            start_delay: 0,
            cliff_delay: 0,
            duration: 300,
            memo: None,
            metadata: None,
        },
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
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.sender.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
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

    let params = vec![
        &ctx.env,
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 100,
            cliff_delay: 200,
            duration: 1000,
            memo: None,
            metadata: None,
        },
    ];

    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);
    assert_eq!(ids.len(), 1);
    assert_eq!(ids.get_unchecked(0), 0);

    let state = ctx.client().get_stream_state(&0);
    assert_eq!(state.start_time, 2100); // 2000 + 100
    assert_eq!(state.cliff_time, 2200); // 2000 + 200
    assert_eq!(state.end_time, 3100); // 2100 + 1000
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
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: recipient2.clone(),
            deposit_amount: 4000, // 2 * 2000
            rate_per_second: 2,
            start_delay: 100,
            cliff_delay: 100,
            duration: 2000,
            memo: None,
            metadata: None,
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
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: recipient2.clone(),
            deposit_amount: 500,
            rate_per_second: 2,
            start_delay: 0,
            cliff_delay: 0,
            duration: 0, // INVALID: duration = 0,
            memo: None,
            metadata: None,
        },
    ];

    let result = ctx
        .client()
        .try_create_streams_relative(&ctx.sender, &params);
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
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r1,
            deposit_amount: 100,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 100,
            memo: None,
            metadata: None,
        },
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r2,
            deposit_amount: 400, // 2 * 200
            rate_per_second: 2,
            start_delay: 500,
            cliff_delay: 600,
            duration: 200,
            memo: None,
            metadata: None,
        },
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r3,
            deposit_amount: 900, // 3 * 300
            rate_per_second: 3,
            start_delay: 1000,
            cliff_delay: 1200,
            duration: 300,
            memo: None,
            metadata: None,
        },
    ];

    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);
    assert_eq!(ids.len(), 3);

    // Verify all three streams created with correct schedules
    let s0 = ctx.client().get_stream_state(&ids.get_unchecked(0));
    assert_eq!(s0.start_time, 10000);
    assert_eq!(s0.end_time, 10100);

    let s1 = ctx.client().get_stream_state(&ids.get_unchecked(1));
    assert_eq!(s1.start_time, 10500);
    assert_eq!(s1.cliff_time, 10600);
    assert_eq!(s1.end_time, 10700);

    let s2 = ctx.client().get_stream_state(&ids.get_unchecked(2));
    assert_eq!(s2.start_time, 11000);
    assert_eq!(s2.cliff_time, 11200);
    assert_eq!(s2.end_time, 11300);

    // Verify total deposit transferred (100 + 400 + 900 = 1400)
    assert_eq!(ctx.token.balance(&ctx.sender), 8_600);
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1_400);
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
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r1,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0, // cliff at current time
            duration: 1000,
            memo: None,
            metadata: None,
        },
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r2,
            deposit_amount: 2000,
            rate_per_second: 1,
            start_delay: 500,
            cliff_delay: 1500, // cliff 500 seconds after start
            duration: 1000,
            memo: None,
            metadata: None,
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
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r1,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: u64::MAX, // overflow
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
    ];

    let result = ctx
        .client()
        .try_create_streams_relative(&ctx.sender, &params);
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
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r1,
            deposit_amount: 1000,
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r2,
            deposit_amount: -100, // Invalid: negative amount
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1000,
            memo: None,
            metadata: None,
        },
    ];

    let result = ctx
        .client()
        .try_create_streams_relative(&ctx.sender, &params);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

// ============================================================================
// Tests: create_streams_relative — anchor invariant
// ============================================================================

/// An empty relative batch returns an empty vector and emits no events or
/// token transfers.
///
/// Authorization is still provided via `mock_all_auths` (the same auth that
/// covers any non-empty call), confirming that a single sender authorization
/// is all that is needed for the batch. The contract must not modify any state
/// when the vector is empty.
#[test]
fn create_streams_relative_empty_batch_emits_no_events() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(1000);

    let events_before = ctx.env.events().all().len();
    let count_before = ctx.client().get_stream_count();
    let sender_balance_before = ctx.token.balance(&ctx.sender);
    let contract_balance_before = ctx.token.balance(&ctx.contract_id);

    let params: soroban_sdk::Vec<CreateStreamRelativeParams> = soroban_sdk::Vec::new(&ctx.env);
    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);

    assert_eq!(ids.len(), 0, "empty batch must return empty id list");
    assert_eq!(
        ctx.client().get_stream_count(),
        count_before,
        "empty batch must not create any streams"
    );
    assert_eq!(
        ctx.token.balance(&ctx.sender),
        sender_balance_before,
        "empty batch must not transfer tokens from sender"
    );
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_balance_before,
        "empty batch must not change contract token balance"
    );
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "empty batch must emit no events"
    );
}

/// All elements in a relative batch compute their absolute times from the same
/// ledger timestamp captured once at the start of the call.
///
/// # Anchor invariant
///
/// Let `T = ledger.timestamp()` at the moment `create_streams_relative` is
/// invoked. For every element `i` in the batch:
///
/// ```text
/// start_time[i] = T + start_delay[i]
/// cliff_time[i] = T + cliff_delay[i]
/// end_time[i]   = start_time[i] + duration[i]
/// ```
///
/// This single-capture design prevents anchor-drift: a bug where different
/// elements in the same batch resolve to different base timestamps, making
/// some streams start in the past.
#[test]
fn create_streams_relative_all_elements_share_same_anchor_timestamp() {
    let ctx = TestContext::setup();

    // Pin the ledger to a fixed anchor. Every element must derive its absolute
    // times from exactly this value.
    const ANCHOR: u64 = 5_000;
    ctx.env.ledger().set_timestamp(ANCHOR);

    let r1 = Address::generate(&ctx.env);
    let r2 = Address::generate(&ctx.env);
    let r3 = Address::generate(&ctx.env);

    // Three elements with distinct offsets. All deposit amounts exactly cover
    // rate_per_second * duration to satisfy validation.
    let params = vec![
        &ctx.env,
        // Element 0: starts immediately at the anchor (zero delay)
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r1,
            deposit_amount: 1_000, // 1 * 1000
            rate_per_second: 1,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1_000,
            memo: None,
            metadata: None,
        },
        // Element 1: starts 200 s after anchor with a 200 s cliff
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r2,
            deposit_amount: 1_600, // 2 * 800
            rate_per_second: 2,
            start_delay: 200,
            cliff_delay: 200,
            duration: 800,
            memo: None,
            metadata: None,
        },
        // Element 2: starts 500 s after anchor with a 700 s cliff
        CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r3,
            deposit_amount: 4_500, // 3 * 1500
            rate_per_second: 3,
            start_delay: 500,
            cliff_delay: 700,
            duration: 1_500,
            memo: None,
            metadata: None,
        },
    ];

    let ids = ctx.client().create_streams_relative(&ctx.sender, &params);
    assert_eq!(ids.len(), 3);

    // Anchor-invariant assertions: every element's times are computed from
    // the same ANCHOR, not from any independently sampled timestamp.
    let cases: &[(u32, u64, u64, u64)] = &[
        // (element index, start_delay, cliff_delay, duration)
        (0, 0, 0, 1_000),
        (1, 200, 200, 800),
        (2, 500, 700, 1_500),
    ];

    for &(idx, start_delay, cliff_delay, duration) in cases {
        let stream = ctx.client().get_stream_state(&ids.get_unchecked(idx));

        let expected_start = ANCHOR + start_delay;
        let expected_cliff = ANCHOR + cliff_delay;
        let expected_end = expected_start + duration;

        assert_eq!(
            stream.start_time, expected_start,
            "element {idx}: start_time must be anchor({ANCHOR}) + start_delay({start_delay})"
        );
        assert_eq!(
            stream.cliff_time, expected_cliff,
            "element {idx}: cliff_time must be anchor({ANCHOR}) + cliff_delay({cliff_delay})"
        );
        assert_eq!(
            stream.end_time, expected_end,
            "element {idx}: end_time must be start_time({expected_start}) + duration({duration})"
        );
    }

    // Also assert that every stream was actually created (stream count grew by 3).
    assert_eq!(ctx.client().get_stream_count(), 3);
}

/// A single-element batch with zero delays produces the same absolute times as
/// `create_stream_relative` called directly with identical zero delays.
///
/// This confirms that the batch code path and the single-entry code path share
/// the same anchor-computation logic: both capture `ledger.timestamp()` once
/// and apply offsets identically. Zero-offset is a valid degenerate case that
/// must be accepted by both APIs.
#[test]
fn create_streams_relative_zero_offset_parity_with_single() {
    let ctx = TestContext::setup();

    // Fix the ledger so both calls (single then batch) see the same timestamp.
    const ANCHOR: u64 = 3_000;
    ctx.env.ledger().set_timestamp(ANCHOR);

    let r_single = Address::generate(&ctx.env);
    let r_batch = Address::generate(&ctx.env);

    // Call 1: single create_stream_relative with zero delays
    let single_id = ctx.client().create_stream_relative(
        &ctx.sender,
        &CreateStreamRelativeParams {
            kind: fluxora_stream::StreamKind::Linear,
            withdraw_dust_threshold: None,
            recipient: r_single,
            deposit_amount: 2_000,
            rate_per_second: 2,
            start_delay: 0,
            cliff_delay: 0,
            duration: 1_000,
            memo: None,
            metadata: None,
        },
    );

    // Call 2: single-element batch create_streams_relative with the same zero delays
    let batch_ids = ctx.client().create_streams_relative(
        &ctx.sender,
        &vec![
            &ctx.env,
            CreateStreamRelativeParams {
                kind: fluxora_stream::StreamKind::Linear,
                withdraw_dust_threshold: None,
                recipient: r_batch,
                deposit_amount: 2_000,
                rate_per_second: 2,
                start_delay: 0,
                cliff_delay: 0,
                duration: 1_000,
                memo: None,
                metadata: None,
            },
        ],
    );

    let single_state = ctx.client().get_stream_state(&single_id);
    let batch_state = ctx.client().get_stream_state(&batch_ids.get_unchecked(0));

    // Both paths must produce identical absolute times from the same anchor.
    assert_eq!(
        single_state.start_time, batch_state.start_time,
        "zero-offset single and batch must produce the same start_time"
    );
    assert_eq!(
        single_state.cliff_time, batch_state.cliff_time,
        "zero-offset single and batch must produce the same cliff_time"
    );
    assert_eq!(
        single_state.end_time, batch_state.end_time,
        "zero-offset single and batch must produce the same end_time"
    );

    // Verify both anchor exactly at ledger.timestamp() (start_delay = 0)
    assert_eq!(
        single_state.start_time, ANCHOR,
        "zero start_delay must anchor start_time to ledger.timestamp()"
    );
    assert_eq!(
        single_state.cliff_time, ANCHOR,
        "zero cliff_delay must anchor cliff_time to ledger.timestamp()"
    );
    assert_eq!(single_state.end_time, ANCHOR + 1_000);
}
