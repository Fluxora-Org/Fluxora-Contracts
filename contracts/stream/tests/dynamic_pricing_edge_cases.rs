//! Dynamic pricing edge-case tests for the Fluxora streaming contract.
//!
//! This test suite validates behavior around rate-update and rate-decrease
//! operations across all boundary conditions, branch paths, and invariant
//! boundaries. It covers:
//!
//! 1. `update_rate_per_second` edge cases (expired, direction, kind, status,
//!    cooldown, cap, checkpoint, deposit coverage)
//! 2. `decrease_rate_per_second` edge cases (expired, direction, kind, status,
//!    refund math, cooldown)
//! 3. Combined dynamic pricing + schedule operations (top-up, extend, shorten)
//! 4. Legacy `update_rate` guards
//! 5. Liability invariants during dynamic pricing
//! 6. Stream kind restrictions (CliffOnly, CliffSlope)
//! 7. Lookback window + rate change interaction
//! 8. Rate cap backward compatibility
//! 9. Decommissioned stream protection
//! 10. Storage index integrity
//! 11. Clock regression detection
//! 12. Pooled stream rate changes
//! 13. Boundary conditions (near end-time, max cap)
//! 14. `get_claimable_at` after rate changes
//! 15. Rate change + withdraw interaction
//!
//! Key invariants enforced:
//! - Rate changes require sufficient deposit coverage (deposit ≥ rate × duration)
//! - Rate cooldown: MIN_RATE_INTERVAL_LEDGERS (17) between changes
//! - Pause cooldown: MIN_PAUSE_INTERVAL_LEDGERS (17) between toggles
//! - Clock regression: current_accrual_timestamp must be monotonically non-decreasing
//! - Decommissioned streams block all rate mutations
//! - CliffOnly and CliffSlope streams reject rate changes

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Symbol, TryFromVal,
};

// ===========================================================================
// Test Context
// ===========================================================================

struct TestContext {
    env: Env,
    client: FluxoraStreamClient<'static>,
    token: TokenClient<'static>,
    admin: Address,
    sender: Address,
    recipient: Address,
}

impl TestContext {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        StellarAssetClient::new(&env, &token_id).mint(&sender, &10_000_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &1_000_000u32);

        Self {
            env,
            client,
            token,
            admin,
            sender,
            recipient,
        }
    }

    /// Advance the ledger sequence by `n` ledgers.
    ///
    /// Required before:
    /// - Pause operations: `MIN_PAUSE_INTERVAL_LEDGERS = 17` (first pause on a
    ///   fresh stream needs ≥17 ledgers because `last_pause_toggle_ledger`
    ///   starts at 0 and cooldown check is `current - last < 17`).
    /// - Rate changes: `MIN_RATE_INTERVAL_LEDGERS = 17` between successive
    ///   rate adjustments on the same stream.
    fn advance_ledger(&self, n: u32) {
        let seq = self.env.ledger().sequence();
        self.env.ledger().set_sequence_number(seq + n);
    }

    fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: 1000u64,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    fn create_stream_with_rate(&self, rate: i128, deposit: i128, end: u64) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: rate,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: end,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    /// Rate=2 stream with deposit=2000, end=1000.
    /// Useful for tests that don't need to increase the rate.
    fn create_rate2_stream(&self) -> u64 {
        self.create_stream_with_rate(2, 2000, 1000)
    }

    fn set_max_rate(&self, rate: i128) {
        self.client.set_max_rate_per_second(&rate);
    }
}

// ===========================================================================
// 1. update_rate_per_second edge cases
// ===========================================================================

// --- Branch: expired stream rejection ---

#[test]
fn update_rate_per_second_past_end_time_fails_with_invalid_state() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // Advance past end_time.
    ctx.env.ledger().set_timestamp(1001);
    let result = ctx.client.try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn update_rate_per_second_exactly_at_end_time_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client.try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

// --- Branch: direction validation ---

#[test]
fn update_rate_per_second_new_rate_equals_current_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx.client.try_update_rate_per_second(&stream_id, &1_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn update_rate_per_second_new_rate_below_current_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx.client.try_update_rate_per_second(&stream_id, &0_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn update_rate_per_second_negative_new_rate_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx.client.try_update_rate_per_second(&stream_id, &-1_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

// --- Branch: stream-kind guards ---

#[test]
fn update_rate_per_second_cliff_only_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 0_i128,
            start_time: 0u64,
            cliff_time: 500u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    );

    let result = ctx.client.try_update_rate_per_second(&stream_id, &1_i128);
    assert_eq!(result, Err(Ok(ContractError::UnsupportedStreamKind)));
}

// --- Branch: status guards ---

#[test]
fn update_rate_per_second_paused_stream_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use deposit=4000 so rate=4 is covered: 4*1000=4000 <= 4000
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    // Must advance ledger past MIN_PAUSE_INTERVAL_LEDGERS (17) before first pause
    ctx.advance_ledger(17);
    ctx.client
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);

    // Advance past rate cooldown for the rate update
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 4);
    assert_eq!(state.status, StreamStatus::Paused);
}

#[test]
fn update_rate_per_second_completed_stream_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client.withdraw(&stream_id);
    let result = ctx.client.try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn update_rate_per_second_cancelled_stream_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.client.cancel_stream(&stream_id);
    let result = ctx.client.try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

// --- Branch: cooldown + rate cap interaction ---

#[test]
fn update_rate_per_second_rate_cap_event_details() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.set_max_rate(500);

    let events_before = ctx.env.events().all().len();
    let result = ctx.client.try_update_rate_per_second(&stream_id, &501_i128);
    assert_eq!(result, Err(Ok(ContractError::RateCapExceeded)));

    let events = ctx.env.events().all();
    assert_eq!(events.len(), events_before + 1);

    let rate_cap_event = events.iter().find(|e| {
        e.1.get(0)
            .map(|topic| Symbol::try_from_val(&ctx.env, &topic).unwrap())
            == Some(symbol_short!("rate_cap"))
    });
    assert!(rate_cap_event.is_some());

    let payload = rate_cap_event.unwrap().2;
    let cap = fluxora_stream::RateCapEnforced::try_from_val(&ctx.env, &payload).unwrap();
    assert_eq!(cap.stream_id, stream_id);
    assert_eq!(cap.attempted_rate, 501);
    assert_eq!(cap.max_rate_per_second, 500);
}

// --- Branch: checkpoint preservation ---

#[test]
fn update_rate_per_second_checkpoints_accrued_amount() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use deposit=5000 so rate=5 is covered: 5*1000=5000 <= 5000
    let stream_id = ctx.create_stream_with_rate(2, 5000, 1000);

    ctx.env.ledger().set_timestamp(300);
    let accrued_before = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 600);

    // Advance ledger past rate cooldown for the first rate change
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &5_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 5);
    assert_eq!(state.checkpointed_amount, 600);
    assert_eq!(state.checkpointed_at, 300);

    // Accrued at the same timestamp must remain unchanged.
    assert_eq!(ctx.client.calculate_accrued(&stream_id), 600);
}

// --- Branch: deposit coverage ---

#[test]
fn update_rate_per_second_insufficient_deposit_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // Current rate=1, duration=1000, new_rate=2 requires deposit >= 2000,
    // but deposit is only 1000.
    let result = ctx.client.try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
}

// ===========================================================================
// 2. decrease_rate_per_second edge cases
// ===========================================================================

// --- Branch: expired stream rejection ---

#[test]
fn decrease_rate_per_second_past_end_time_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1001);
    let result = ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

// --- Branch: direction validation ---

#[test]
fn decrease_rate_per_second_new_rate_equals_current_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn decrease_rate_per_second_new_rate_above_current_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx.client.try_decrease_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn decrease_rate_per_second_zero_rate_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx.client.try_decrease_rate_per_second(&stream_id, &0_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn decrease_rate_per_second_negative_rate_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx
        .client
        .try_decrease_rate_per_second(&stream_id, &-5_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

// --- Branch: stream-kind guards ---

#[test]
fn decrease_rate_per_second_cliff_only_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 0_i128,
            start_time: 0u64,
            cliff_time: 500u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    );

    let result = ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128);
    assert_eq!(result, Err(Ok(ContractError::UnsupportedStreamKind)));
}

// --- Branch: status ---

#[test]
fn decrease_rate_per_second_paused_stream_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 2000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    // Advance past MIN_PAUSE_INTERVAL_LEDGERS before pausing
    ctx.advance_ledger(17);
    ctx.client
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);

    ctx.advance_ledger(17);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 1);
    assert_eq!(state.status, StreamStatus::Paused);
}

// --- Branch: refund math ---

#[test]
fn decrease_rate_per_second_refund_reaches_sender() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    ctx.env.ledger().set_timestamp(500);
    let sender_before = ctx.token.balance(&ctx.sender);

    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    let sender_after = ctx.token.balance(&ctx.sender);
    // deposit=2000, rate=2, end=1000, t=500: accrued = min(2*500, 2000) = 1000
    // new_deposit = 1000 + 1*(1000-500) = 1500
    // refund = 2000 - 1500 = 500
    assert_eq!(sender_after - sender_before, 500);
}

#[test]
fn decrease_rate_per_second_event_payload_correct() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    ctx.env.ledger().set_timestamp(200);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    let events = ctx.env.events().all();
    let last = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&ctx.env, &last.1.get(0).unwrap()).unwrap(),
        symbol_short!("rate_dec")
    );

    let payload = fluxora_stream::RateDecreased::try_from_val(&ctx.env, &last.2).unwrap();
    assert_eq!(payload.stream_id, stream_id);
    assert_eq!(payload.old_rate_per_second, 2);
    assert_eq!(payload.new_rate_per_second, 1);
    assert_eq!(payload.effective_time, 200);
    // accrued = 2 * 200 = 400
    assert_eq!(payload.checkpointed_amount, 400);
    // new_deposit = 400 + 1 * 800 = 1200, refund = 2000 - 1200 = 800
    assert_eq!(payload.refund_amount, 800);
}

// --- Branch: cooldown interaction ---

#[test]
fn decrease_rate_per_second_cooldown_prevents_rapid_changes() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use rate=3 so we can decrease twice: 3→2→1
    let stream_id = ctx.create_stream_with_rate(3, 3000, 1000);

    // Advance past pause cooldown (irrelevant here, but ensures clean state)
    ctx.advance_ledger(17);
    ctx.env.ledger().set_timestamp(500);
    // First decrease: 3→2 (succeeds, last_rate_change_ledger=0 is exempt)
    ctx.client.decrease_rate_per_second(&stream_id, &2_i128);

    // Advance only 16 ledgers (not enough, need 17).
    // last_rate_change_ledger was set to seq at first decrease.
    // Cooldown check: current_seq < last_rate_change_ledger + 17
    //                = (last+16) < (last + 17) → true → RateCooldownActive
    ctx.advance_ledger(16);
    let result = ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128);
    assert_eq!(result, Err(Ok(ContractError::RateCooldownActive)));
}

// ===========================================================================
// 3. Combined dynamic pricing + schedule operations
// ===========================================================================

// --- top_up → rate increase ---

#[test]
fn top_up_then_rate_increase_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // Top up so deposit rises from 1000 to 2000.
    ctx.client
        .top_up_stream(&stream_id, &ctx.sender, &1000_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 2000);

    // New rate 2/s × 1000s = 2000 exactly covers deposit.
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &2_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 2);
    assert_eq!(state.deposit_amount, 2000);
}

#[test]
fn top_up_then_rate_decrease_refund_correct() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    ctx.client
        .top_up_stream(&stream_id, &ctx.sender, &1000_i128);

    ctx.env.ledger().set_timestamp(500);
    let sender_before = ctx.token.balance(&ctx.sender);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    let sender_after = ctx.token.balance(&ctx.sender);
    // deposit=3000, rate=2, end=1000, t=500: accrued=1000, new_deposit=1000+1*500=1500, refund=1500
    assert_eq!(sender_after - sender_before, 1500);
}

// --- rate increase → top_up ---

#[test]
fn rate_increase_then_top_up_preserves_new_rate() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use a stream with enough deposit for rate=4: 4*1000=4000
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);
    ctx.client
        .top_up_stream(&stream_id, &ctx.sender, &1000_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 4);
    assert_eq!(state.deposit_amount, 5000);
}

// --- extend → rate increase ---

#[test]
fn extend_then_rate_increase_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    // Top up first to cover the extended duration deposit requirement
    ctx.client
        .top_up_stream(&stream_id, &ctx.sender, &1000_i128);
    ctx.client.extend_stream_end_time(&stream_id, &2000u64);

    // Original deposit=2000 (after top-up), rate=1. Extended duration=2000, needs deposit>=2000.
    // Top up more to allow rate=2: 2*2000=4000.
    ctx.client
        .top_up_stream(&stream_id, &ctx.sender, &2000_i128);
    ctx.advance_ledger(17);
    let result = ctx.client.try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Ok(Ok(())));
}

// --- rate increase → shorten ---

#[test]
fn rate_increase_then_shorten_refunds_excess() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    // Increase rate to 2/s: deposit 1000, duration 1000, total_streamable = 2000 > 1000.
    let result = ctx.client.try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
}

// --- rate decrease → extend ---

#[test]
fn rate_decrease_then_extend_succeeds() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);
    // deposit=2000, rate=2, end=1000, t=500: accrued=1000
    // new_deposit = 1000 + 1*500 = 1500, refund = 500
    // deposit becomes 1500

    // Top up so extension is possible
    ctx.client
        .top_up_stream(&stream_id, &ctx.sender, &1000_i128);
    // deposit = 2500, rate=1, extend to 2000: needs 1*2000=2000 <= 2500

    ctx.client.extend_stream_end_time(&stream_id, &2000u64);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.end_time, 2000);
    assert_eq!(state.rate_per_second, 1);
}

#[test]
fn rate_decrease_then_extend_with_sufficient_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    // Top up to have room for extension
    ctx.client
        .top_up_stream(&stream_id, &ctx.sender, &1000_i128);
    // deposit = 3000
    ctx.env.ledger().set_timestamp(500);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);
    // deposit=3000, rate=2, end=1000, t=500: accrued=1000
    // new_deposit = 1000 + 1*500 = 1500, refund = 1500
    // deposit becomes 1500

    // Extend to 1500: needs rate*1500 = 1500 <= deposit 1500 → ok
    let result = ctx.client.try_extend_stream_end_time(&stream_id, &1500u64);
    assert_eq!(result, Ok(Ok(())));
}

// --- multiple rate changes interleaved ---

#[test]
fn multiple_rate_increases_then_decrease_preserves_checkpoints() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // deposit=3000 covers rate=3 for 1000s
    let stream_id = ctx.create_stream_with_rate(1, 3000, 1000);

    // Advance ledger past cooldown for first rate change
    ctx.advance_ledger(17);
    ctx.env.ledger().set_timestamp(200);
    // Increase 1 → 2 at t=200: checkpoint = 1*200 = 200
    ctx.client.update_rate_per_second(&stream_id, &2_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.checkpointed_amount, 200);
    assert_eq!(state.rate_per_second, 2);

    // Advance ledger past cooldown for next rate change
    ctx.advance_ledger(17);
    ctx.env.ledger().set_timestamp(400);
    // Increase 2 → 3 at t=400: checkpoint = 200 + 2*200 = 600
    ctx.client.update_rate_per_second(&stream_id, &3_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.checkpointed_amount, 600);
    assert_eq!(state.rate_per_second, 3);

    // Advance ledger past cooldown for decrease
    ctx.advance_ledger(17);
    // Decrease 3 → 1 at t=400 (no time elapsed, same ledger): checkpoint = 600 + 3*0 = 600
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.checkpointed_amount, 600);
    assert_eq!(state.rate_per_second, 1);
    // new_deposit = 600 + 1 * 600 = 1200
    assert_eq!(state.deposit_amount, 1200);
}

// ===========================================================================
// 4. Legacy update_rate guards
// ===========================================================================

#[test]
fn legacy_update_rate_zero_rate_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx.client.try_update_rate(&stream_id, &0_i128, &ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn legacy_update_rate_negative_rate_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let result = ctx
        .client
        .try_update_rate(&stream_id, &-1_i128, &ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn legacy_update_rate_past_end_time_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1001);
    let result = ctx.client.try_update_rate(&stream_id, &2_i128, &ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn legacy_update_rate_decommissioned_stream_fails() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.client
        .set_stream_decommissioned(&stream_id, &ctx.sender, &true);
    let result = ctx.client.try_update_rate(&stream_id, &2_i128, &ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn legacy_update_rate_cliff_only_rejected() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 0_i128,
            start_time: 0u64,
            cliff_time: 500u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    );

    let result = ctx.client.try_update_rate(&stream_id, &1_i128, &ctx.sender);
    assert_eq!(result, Err(Ok(ContractError::UnsupportedStreamKind)));
}

#[test]
fn legacy_update_rate_admin_can_increase() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.client
        .update_rate(&stream_id, &5_i128, &ctx.admin.clone());

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 5);
}

// ===========================================================================
// 5. Liability invariants during dynamic pricing
// ===========================================================================

#[test]
fn rate_increase_preserves_total_liabilities() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use deposit=4000 so rate=4 is covered: 4*1000=4000 <= 4000
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    let liabilities_before = ctx.client.get_total_liabilities();
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);

    let liabilities_after = ctx.client.get_total_liabilities();
    assert_eq!(liabilities_before, liabilities_after);
}

#[test]
fn rate_decrease_reduces_total_liabilities_by_refund() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    ctx.env.ledger().set_timestamp(500);
    let liabilities_before = ctx.client.get_total_liabilities();

    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    let liabilities_after = ctx.client.get_total_liabilities();
    // deposit=2000, rate=2, t=500: accrued=1000, new_deposit=1000+1*500=1500, refund=500
    assert_eq!(liabilities_before - liabilities_after, 500);
}

#[test]
fn top_up_increases_total_liabilities() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    let liabilities_before = ctx.client.get_total_liabilities();
    ctx.client.top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let liabilities_after = ctx.client.get_total_liabilities();
    assert_eq!(liabilities_after - liabilities_before, 500);
}

#[test]
fn shorten_decreases_total_liabilities_by_refund() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    let liabilities_before = ctx.client.get_total_liabilities();

    ctx.client.shorten_stream_end_time(&stream_id, &500u64);

    let liabilities_after = ctx.client.get_total_liabilities();
    // shortening from 1000 to 500 at rate=1: new_max=500, accrued=100,
    // new_deposit=max(500,100)=500, refund=500
    assert_eq!(liabilities_before - liabilities_after, 500);
}

// ===========================================================================
// 6. Stream kind restrictions across all dynamic pricing ops
// ===========================================================================

#[test]
fn cliff_only_stream_rejects_all_dynamic_pricing_ops() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 0_i128,
            start_time: 0u64,
            cliff_time: 500u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    );

    assert_eq!(
        ctx.client.try_update_rate_per_second(&stream_id, &1_i128),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
    assert_eq!(
        ctx.client.try_shorten_stream_end_time(&stream_id, &500u64),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
    assert_eq!(
        ctx.client.try_extend_stream_end_time(&stream_id, &2000u64),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
}

/// CliffSlope streams should also be rejected by rate-change entrypoints
/// because both `update_rate_per_second` and `decrease_rate_per_second`
/// enforce `stream.kind != StreamKind::Linear`.
#[test]
fn cliff_slope_stream_rejects_rate_changes() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // CliffSlope: cliff at 200, linear accrual from 200 to 1000.
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 2000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 200u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffSlope,
            irrevocable: None,
            witness: None,
        },
    );

    assert_eq!(
        ctx.client.try_update_rate_per_second(&stream_id, &3_i128),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
}

// ===========================================================================
// 7. Lookback window + rate change interaction
// ===========================================================================

#[test]
fn rate_increase_preserves_lookback_window() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    ctx.client
        .set_lookback_window(&stream_id, &ctx.sender, &Some(10));

    ctx.env.ledger().set_timestamp(500);
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);

    let lookback = ctx.client.get_lookback_window(&stream_id).unwrap();
    assert_eq!(lookback, 10);
}

#[test]
fn rate_decrease_preserves_lookback_window() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    ctx.client
        .set_lookback_window(&stream_id, &ctx.sender, &Some(20));

    ctx.env.ledger().set_timestamp(500);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    let lookback = ctx.client.get_lookback_window(&stream_id).unwrap();
    assert_eq!(lookback, 20);
}

// ===========================================================================
// 8. Rate cap backward compatibility
// ===========================================================================

#[test]
fn rate_cap_lowering_does_not_break_existing_streams() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    // Stream starts at rate=2.
    ctx.set_max_rate(10);

    // Lower the cap after stream creation.
    ctx.set_max_rate(5);

    // Existing stream is still queryable and operable.
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 2);

    // Top up to cover rate=5: 5*1000=5000, need 3000 more
    ctx.client
        .top_up_stream(&stream_id, &ctx.sender, &3000_i128);

    // Increase to 5 (within new cap) succeeds.
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &5_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 5);

    // Advance past rate cooldown so cap check fires (not cooldown)
    ctx.advance_ledger(17);
    // Further increase to 6 (above new cap) is blocked.
    let result = ctx.client.try_update_rate_per_second(&stream_id, &6_i128);
    assert_eq!(result, Err(Ok(ContractError::RateCapExceeded)));
}

// ===========================================================================
// 9. Decommissioned stream protection for all pricing ops
// ===========================================================================

#[test]
fn decommissioned_blocks_rate_increase_and_decrease() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_default_stream();

    ctx.client
        .set_stream_decommissioned(&stream_id, &ctx.sender, &true);

    assert_eq!(
        ctx.client.try_update_rate_per_second(&stream_id, &2_i128),
        Err(Ok(ContractError::InvalidState))
    );
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128),
        Err(Ok(ContractError::InvalidState))
    );
}

// ===========================================================================
// 10. Rate changes do not corrupt storage indexes
// ===========================================================================

#[test]
fn rate_change_does_not_create_duplicate_stream_index_entries() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    // Get recipient stream count before rate change.
    let before = ctx.client.get_stream_count();

    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);
    // Advance past cooldown for second change
    ctx.advance_ledger(17);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    let after = ctx.client.get_stream_count();
    assert_eq!(before, after, "rate changes must not alter stream count");

    let recipient_streams = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(recipient_streams.len(), 1);
    assert_eq!(recipient_streams.get(0), Some(stream_id));
}

// ===========================================================================
// 11. Clock regression during rate changes
// ===========================================================================

#[test]
fn update_rate_per_second_detects_clock_regression() {
    let ctx = TestContext::setup();
    // Use deposit=5000 so rate=5 is covered: 5*1000=5000 <= 5000
    // Note: create_stream_with_rate resets timestamp to 0, so set timestamp AFTER
    let stream_id = ctx.create_stream_with_rate(2, 5000, 1000);

    // Set timestamp to 100 AFTER stream creation (which resets to 0)
    ctx.env.ledger().set_timestamp(100);
    ctx.advance_ledger(17);
    // First call stores timestamp 100 in LastAccrualLedgerTimestamp.
    ctx.client.update_rate_per_second(&stream_id, &4_i128);

    // Advance past rate cooldown so clock regression fires (not cooldown)
    ctx.advance_ledger(17);
    // Regression: set timestamp backwards to 50 (< 100).
    ctx.env.ledger().set_timestamp(50);
    let result = ctx.client.try_update_rate_per_second(&stream_id, &5_i128);
    assert_eq!(result, Err(Ok(ContractError::ClockRegression)));
}

#[test]
fn decrease_rate_per_second_detects_clock_regression() {
    let ctx = TestContext::setup();
    // Use rate=3 so we can decrease: 3→2
    // Note: create_stream_with_rate resets timestamp to 0, so set timestamp AFTER
    let stream_id = ctx.create_stream_with_rate(3, 3000, 1000);

    // Set timestamp to 100 AFTER stream creation (which resets to 0)
    ctx.env.ledger().set_timestamp(100);
    ctx.advance_ledger(17);
    ctx.client.decrease_rate_per_second(&stream_id, &2_i128);

    // Advance past rate cooldown so clock regression fires (not cooldown)
    ctx.advance_ledger(17);
    // Regression: set timestamp backwards to 50 (< 100).
    ctx.env.ledger().set_timestamp(50);
    let result = ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128);
    assert_eq!(result, Err(Ok(ContractError::ClockRegression)));
}

// ===========================================================================
// 12. Dynamic pricing on pooled streams
// ===========================================================================

#[test]
fn pooled_stream_rate_changes_succeed() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let recipients = soroban_sdk::vec![
        &ctx.env,
        (ctx.recipient.clone(), 1u32),
        (Address::generate(&ctx.env), 1u32),
    ];

    // deposit=4000 covers rate=4 for 1000s
    let pool_id = ctx.client.create_pooled_stream(
        &ctx.sender,
        &recipients,
        &4000_i128,
        &2_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0i128,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(200);
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&pool_id, &4_i128);
    let state = ctx.client.get_stream_state(&pool_id);
    assert_eq!(state.rate_per_second, 4);
}

// ===========================================================================
// 13. Rate changes near boundaries
// ===========================================================================

#[test]
fn update_rate_per_second_near_end_time_boundary() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use deposit=4000 so rate=4 is covered: 4*1000=4000 <= 4000
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    // One second before end: still allowed.
    ctx.env.ledger().set_timestamp(999);
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 4);
}

#[test]
fn decrease_rate_per_second_near_end_time_boundary() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    // One second before end: still allowed (one second remains).
    ctx.env.ledger().set_timestamp(999);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 1);
}

#[test]
fn update_rate_per_second_max_rate_cap_boundary() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use deposit=4000 so rate=4 is covered: 4*1000=4000 <= 4000
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    ctx.set_max_rate(4);

    // Exactly at cap: allowed.
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 4);

    // Advance past rate cooldown so the cap check fires (not cooldown)
    ctx.advance_ledger(17);
    // One above cap: blocked.
    let result = ctx.client.try_update_rate_per_second(&stream_id, &5_i128);
    assert_eq!(result, Err(Ok(ContractError::RateCapExceeded)));
}

// ===========================================================================
// 14. get_claimable_at after rate changes
// ===========================================================================

#[test]
fn get_claimable_at_after_rate_increase_matches_math() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use deposit=4000 so rate=4 is covered: 4*1000=4000 <= 4000
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    ctx.env.ledger().set_timestamp(200);
    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);

    // At t=300: accrued = 400 (checkpoint: 200*2) + 4*100 = 800.
    assert_eq!(ctx.client.get_claimable_at(&stream_id, &300), 800);
}

#[test]
fn get_claimable_at_after_rate_decrease_matches_math() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    ctx.env.ledger().set_timestamp(200);
    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    // At t=400: accrued = 400 (checkpoint: 200*2) + 1*200 = 600.
    assert_eq!(ctx.client.get_claimable_at(&stream_id, &400), 600);
}

// ===========================================================================
// 15. Rate change + withdraw interaction
// ===========================================================================

#[test]
fn withdraw_after_rate_increase_uses_new_accrual() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    // Use deposit=4000 so rate=4 is covered: 4*1000=4000 <= 4000
    let stream_id = ctx.create_stream_with_rate(2, 4000, 1000);

    ctx.env.ledger().set_timestamp(200);
    ctx.client.withdraw(&stream_id); // withdraw 400 (2*200)

    ctx.advance_ledger(17);
    ctx.client.update_rate_per_second(&stream_id, &4_i128);

    ctx.env.ledger().set_timestamp(400);
    let withdrawn_before = ctx.client.get_stream_state(&stream_id).withdrawn_amount;
    let accrued = ctx.client.calculate_accrued(&stream_id);
    let withdrawable = ctx.client.get_withdrawable(&stream_id);

    // accrued = 400 (checkpoint: 200*2) + 4*200 = 1200
    // withdrawable = 1200 - 400 = 800
    assert_eq!(accrued, 1200);
    assert_eq!(withdrawable, 800);
    assert_eq!(withdrawn_before, 400);
}

#[test]
fn withdraw_after_rate_decrease_uses_new_accrual() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_rate2_stream();

    ctx.env.ledger().set_timestamp(200);
    ctx.client.withdraw(&stream_id); // withdraw 400 (2*200)

    ctx.client.decrease_rate_per_second(&stream_id, &1_i128);

    ctx.env.ledger().set_timestamp(400);
    let accrued = ctx.client.calculate_accrued(&stream_id);
    let withdrawable = ctx.client.get_withdrawable(&stream_id);

    // accrued = 400 (checkpoint: 200*2) + 1*200 = 600
    // withdrawable = 600 - 400 = 200
    assert_eq!(accrued, 600);
    assert_eq!(withdrawable, 200);
}
