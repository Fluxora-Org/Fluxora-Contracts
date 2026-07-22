extern crate std;

use fluxora_stream::{
    ContractError, DataKey, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

#[allow(dead_code)]
struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
    sac: StellarAssetClient<'a>,
    contract_id: Address,
}

impl<'a> TestContext<'a> {
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
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        sac.mint(&sender, &100_000_i128);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            client,
            sender,
            recipient,
            token,
            sac,
            contract_id,
        }
    }

    /// Create a default linear stream: deposit=1000, rate=1/s, 0..1000s, no cliff.
    fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
        )
    }
}

// ---------------------------------------------------------------------------
// 1. Active stream top-up
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_active_stream_deposit_reflected() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    ctx.client.top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.end_time, 1_000);
}

// ---------------------------------------------------------------------------
// 2. Paused stream top-up — matches documented behaviour
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_paused_stream_matches_spec() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance ledger sequence past cooldown, then pause the stream
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
    ctx.env.ledger().set_timestamp(400);
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Paused);

    // Top up while paused
    ctx.client.top_up_stream(&stream_id, &ctx.sender, &300_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_300);
    assert_eq!(state.status, StreamStatus::Paused); // status unchanged
    assert_eq!(state.end_time, 1_000); // schedule unchanged
}

// ---------------------------------------------------------------------------
// 3. Completed stream top-up — rejected with InvalidState
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_completed_stream_rejected() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Advance ledger sequence past withdrawal frequency check, then complete
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
    ctx.env.ledger().set_timestamp(1000);
    ctx.client.withdraw(&stream_id);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    let result = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &100_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    // Verify no side effects
    let state_after = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state_after.deposit_amount, state.deposit_amount);
}

// ---------------------------------------------------------------------------
// 4. Cancelled stream top-up — rejected with InvalidState
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_cancelled_stream_rejected() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Cancel the stream
    ctx.env.ledger().set_timestamp(100);
    ctx.client.cancel_stream(&stream_id);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    let result = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &100_i128);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    // Verify no side effects
    let state_after = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state_after.deposit_amount, state.deposit_amount);
}

// ---------------------------------------------------------------------------
// 5. Near end_time (T-1) top-up — accrual/withdrawable update correctly
// ---------------------------------------------------------------------------

#[test]
fn test_top_up_near_end_updates_accrual() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // deposit=1000, rate=1/s, 0..1000

    // At T-1: accrued = min(1 * 999, 1000) = 999
    ctx.env.ledger().set_timestamp(999);
    let accrued_before = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued_before, 999);

    // Top up by 500 → deposit becomes 1500
    ctx.client.top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);

    // Accrual at same timestamp: min(1 * 999, 1500) = 999 (unchanged at same time)
    let accrued_after = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued_after, 999);

    // Move to end_time: accrued = min(1 * 1000, 1500) = 1000
    ctx.env.ledger().set_timestamp(1000);
    let accrued_at_end = ctx.client.calculate_accrued(&stream_id);
    assert_eq!(accrued_at_end, 1_000);

    // withdrawable = accrued - withdrawn = 1000 - 0 = 1000
    let withdrawable = ctx.client.get_withdrawable(&stream_id);
    assert_eq!(withdrawable, 1_000);
}

// ---------------------------------------------------------------------------
// 6. Near-ceiling TotalLiabilities overflow — must return typed error, not panic
// ---------------------------------------------------------------------------
//
// Regression guard for the fund-accounting bug where the TotalLiabilities
// increment used `.unwrap_or(i128::MAX)` (silent wrap / silent clamp) instead
// of checked arithmetic that propagates a typed ContractError.
//
// The scenario:
//   - TotalLiabilities is seeded to (i128::MAX - 1) via env.as_contract, matching
//     the pattern used in storage_key_compat.rs (discriminant 14, Instance storage).
//   - stream.deposit_amount is small enough that deposit_amount + top_up_amount
//     does NOT overflow (the Checks-phase guard passes).
//   - TotalLiabilities + top_up_amount DOES overflow i128.
//
// Expected result: ContractError::ArithmeticOverflow is returned.
// No partial state mutation must occur (deposit_amount unchanged, no event).

#[test]
fn test_top_up_near_ceiling_total_liabilities_returns_overflow_error() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // Seed TotalLiabilities to one less than the maximum i128 value so that
    // any positive top-up amount will overflow it.
    let near_max: i128 = i128::MAX - 1;
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::TotalLiabilities, &near_max);
    });

    // Capture state before the attempted top-up.
    let state_before = ctx.client.get_stream_state(&stream_id);
    let events_before = ctx.env.events().all().len();

    // A top-up of 2 would push TotalLiabilities from (i128::MAX - 1) to
    // (i128::MAX + 1), which overflows i128.  The deposit_amount + 2 is
    // still well within i128 range, so the Checks-phase deposit guard passes
    // and execution reaches the TotalLiabilities increment.
    let result = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &2_i128);

    // Must return a typed ArithmeticOverflow error, not a panic.
    assert_eq!(
        result,
        Err(Ok(ContractError::ArithmeticOverflow)),
        "near-ceiling TotalLiabilities overflow must return ArithmeticOverflow"
    );

    // --- No partial state mutation ---

    // deposit_amount must be unchanged.
    let state_after = ctx.client.get_stream_state(&stream_id);
    assert_eq!(
        state_after.deposit_amount, state_before.deposit_amount,
        "deposit_amount must not change on a rejected top-up"
    );

    // No new events must have been emitted (the top_up event fires only on success).
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no event must be emitted on a rejected top-up"
    );

    // TotalLiabilities must not have been mutated — it stays at near_max.
    let cid2 = ctx.contract_id.clone();
    ctx.env.as_contract(&cid2, || {
        let stored_liabilities: i128 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLiabilities)
            .expect("TotalLiabilities must still be present");
        assert_eq!(
            stored_liabilities, near_max,
            "TotalLiabilities must not be mutated on a rejected top-up"
        );
    });
}

// ---------------------------------------------------------------------------
// 7. Just-under-ceiling TotalLiabilities — top-up still succeeds normally
// ---------------------------------------------------------------------------
//
// Companion to test 6: confirms that a top-up whose amount exactly fits within
// the remaining headroom of TotalLiabilities succeeds and increments the counter
// by the precise top-up amount.
//
// The scenario:
//   - TotalLiabilities is seeded to (i128::MAX - 500).
//   - Top-up amount is 500, so TotalLiabilities will land exactly at i128::MAX
//     (no overflow).
//
// Expected result: Ok(()), deposit_amount increases, event is emitted,
// TotalLiabilities == i128::MAX.

#[test]
fn test_top_up_just_under_ceiling_total_liabilities_succeeds() {
    let ctx = TestContext::setup();

    // Mint enough tokens for the 500-unit top-up (sender already has 100_000
    // from TestContext::setup, so no additional mint needed).
    let stream_id = ctx.create_default_stream();

    let headroom: i128 = 500;
    let seed_liabilities: i128 = i128::MAX - headroom;
    let cid = ctx.contract_id.clone();
    ctx.env.as_contract(&cid, || {
        ctx.env
            .storage()
            .instance()
            .set(&DataKey::TotalLiabilities, &seed_liabilities);
    });

    let state_before = ctx.client.get_stream_state(&stream_id);
    let events_before = ctx.env.events().all().len();

    // Top-up with exactly the remaining headroom — must not overflow.
    let result = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &headroom);
    assert!(
        result.is_ok(),
        "top-up just under ceiling must succeed, got: {:?}",
        result
    );

    // deposit_amount must increase by headroom.
    let state_after = ctx.client.get_stream_state(&stream_id);
    assert_eq!(
        state_after.deposit_amount,
        state_before.deposit_amount + headroom,
        "deposit_amount must reflect the top-up amount"
    );
    assert_eq!(
        state_after.status,
        StreamStatus::Active,
        "stream must remain Active after a successful top-up"
    );

    // Exactly one new event must have been emitted (the top_up event).
    assert_eq!(
        ctx.env.events().all().len(),
        events_before + 1,
        "exactly one top_up event must be emitted on success"
    );

    // TotalLiabilities must now equal exactly i128::MAX.
    let cid2 = ctx.contract_id.clone();
    ctx.env.as_contract(&cid2, || {
        let stored_liabilities: i128 = ctx
            .env
            .storage()
            .instance()
            .get(&DataKey::TotalLiabilities)
            .expect("TotalLiabilities must be present");
        assert_eq!(
            stored_liabilities,
            i128::MAX,
            "TotalLiabilities must equal i128::MAX after a just-under-ceiling top-up"
        );
    });
}
