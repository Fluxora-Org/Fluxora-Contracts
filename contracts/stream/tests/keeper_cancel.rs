extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, KeeperCancelled, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Symbol, TryFromVal,
};

// Grace period in seconds (mirrors KEEPER_GRACE_PERIOD_SECONDS in lib.rs).
const GRACE: u64 = 604_800;
// Keeper fee basis points (mirrors KEEPER_FEE_BPS).
const FEE_BPS: i128 = 50;

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    keeper: Address,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
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
        let keeper = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &1_000_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &200_000);

        Ctx {
            env,
            contract_id,
            sender,
            recipient,
            keeper,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }
}

// ---------------------------------------------------------------------------
// Helper: create a simple stream and return its ID
// ---------------------------------------------------------------------------

fn create_stream(ctx: &Ctx<'_>, deposit: i128, rate: i128, start: u64, end: u64) -> u64 {
    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &start,
        &start, // cliff == start
        &end,
        &0_i128,
        &None,
    )
}

// ---------------------------------------------------------------------------
// Happy path: keeper cancels a fully-accrued expired stream with no prior withdrawals
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_fully_accrued_no_prior_withdrawals() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    // Stream: deposit=1000, rate=1/s, start=0, end=1000
    let stream_id = create_stream(&ctx, 1000, 1, 0, 1000);

    // Advance past end_time + grace period
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);

    let client = ctx.client();
    client.keeper_cancel(&stream_id, &ctx.keeper);

    // Fully accrued: recipient gets all 1000, sender gets 0, keeper gets 0
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
    assert_eq!(ctx.token.balance(&ctx.sender), 1_000_000 - 1000);
    assert_eq!(ctx.token.balance(&ctx.keeper), 0);

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Happy path: keeper cancels a partially-accrued stream and receives fee
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_partial_accrual_fee_paid() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    // Stream: deposit=10000, rate=5/s, start=0, end=1000 → fully accrued at end=5000 (< 10000)
    // Deposit is 10000, rate*duration = 5*1000 = 5000.
    // This means 5000 tokens are unstreamed → sender refund_gross = 5000.
    let stream_id = create_stream(&ctx, 10_000, 5, 0, 1000);

    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);

    let client = ctx.client();
    client.keeper_cancel(&stream_id, &ctx.keeper);

    // accrued = min(5 * 1000, 10000) = 5000 (cap is deposit=10000, so 5000)
    // recipient_amount = 5000 - 0 = 5000
    // sender_refund_gross = 10000 - 5000 = 5000
    // keeper_fee = 5000 * 50 / 10000 = 25
    // sender_refund = 5000 - 25 = 4975
    let expected_accrued = 5000_i128;
    let sender_refund_gross = 10_000 - expected_accrued;
    let keeper_fee = sender_refund_gross * FEE_BPS / 10_000;
    let sender_refund = sender_refund_gross - keeper_fee;

    assert_eq!(ctx.token.balance(&ctx.recipient), expected_accrued);
    assert_eq!(
        ctx.token.balance(&ctx.sender),
        1_000_000 - 10_000 + sender_refund
    );
    assert_eq!(ctx.token.balance(&ctx.keeper), keeper_fee);

    let stream = client.get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Happy path: recipient previously withdrew some; keeper distributes remainder
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_with_prior_withdrawal() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit=2000, rate=1/s, start=0, end=2000 → fully accrued
    let stream_id = create_stream(&ctx, 2000, 1, 0, 2000);

    // Recipient withdraws 500 at t=500
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id);
    assert_eq!(ctx.token.balance(&ctx.recipient), 500);

    // Advance past grace period
    ctx.env.ledger().set_timestamp(2000 + GRACE + 1);

    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    // accrued = 2000 (fully streamed), recipient_amount = 2000 - 500 = 1500, sender_refund_gross = 0
    assert_eq!(ctx.token.balance(&ctx.recipient), 500 + 1500);
    assert_eq!(ctx.token.balance(&ctx.sender), 1_000_000 - 2000);
    assert_eq!(ctx.token.balance(&ctx.keeper), 0);
}

// ---------------------------------------------------------------------------
// Error: grace period has not elapsed yet
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_too_early_errors() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = create_stream(&ctx, 1000, 1, 0, 1000);

    // Just past end_time but still within grace period
    ctx.env.ledger().set_timestamp(1000 + GRACE - 1);

    let result = ctx.client().try_keeper_cancel(&stream_id, &ctx.keeper);
    assert_eq!(result, Err(Ok(ContractError::KeeperGracePeriodNotElapsed)));
}

// ---------------------------------------------------------------------------
// Error: stream is already in terminal state
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_already_cancelled_errors() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = create_stream(&ctx, 1000, 1, 0, 1000);

    // Sender cancels normally
    ctx.client().cancel_stream(&stream_id);

    // Advance past grace period
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);

    let result = ctx.client().try_keeper_cancel(&stream_id, &ctx.keeper);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

#[test]
fn test_keeper_cancel_completed_stream_errors() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = create_stream(&ctx, 1000, 1, 0, 1000);

    // Fully withdraw at end
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    // Advance past grace period
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);

    let result = ctx.client().try_keeper_cancel(&stream_id, &ctx.keeper);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

// ---------------------------------------------------------------------------
// Error: stream not found
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_nonexistent_stream_errors() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);

    let result = ctx.client().try_keeper_cancel(&9999u64, &ctx.keeper);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

// ---------------------------------------------------------------------------
// Edge case: zero refund → keeper fee is zero
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_zero_unstreamed_no_fee() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit == rate * duration → zero unstreamed at end
    let stream_id = create_stream(&ctx, 1000, 1, 0, 1000);

    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);
    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    assert_eq!(ctx.token.balance(&ctx.keeper), 0);
    assert_eq!(ctx.token.balance(&ctx.recipient), 1000);
}

// ---------------------------------------------------------------------------
// State: stream status and cancelled_at are set correctly after keeper_cancel
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_sets_terminal_state() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = create_stream(&ctx, 2000, 1, 0, 2000);

    let cancel_ts = 2000 + GRACE + 100;
    ctx.env.ledger().set_timestamp(cancel_ts);

    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.cancelled_at, Some(cancel_ts));
}

// ---------------------------------------------------------------------------
// Paused stream: keeper_cancel works on Active or Paused streams
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_paused_stream_succeeds() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = create_stream(&ctx, 2000, 1, 0, 2000);

    // Pause the stream at t=500
    ctx.env.ledger().set_timestamp(500);
    ctx.client()
        .pause_stream(&stream_id, &fluxora_stream::PauseReason::Operational);

    // Advance past grace period (paused streams are still eligible)
    ctx.env.ledger().set_timestamp(2000 + GRACE + 1);

    // Should succeed even though stream is Paused
    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Invariant: recipient_amount + sender_refund + keeper_fee == deposit - withdrawn
// ---------------------------------------------------------------------------

#[test]
fn test_keeper_cancel_token_conservation() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 5_000_i128;
    let stream_id = create_stream(&ctx, deposit, 3, 0, 1000);

    // Partial withdrawal at t=200
    ctx.env.ledger().set_timestamp(200);
    let withdrawn = ctx.client().withdraw(&stream_id);

    // Advance past grace period
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);

    let sender_before = ctx.token.balance(&ctx.sender);
    let recipient_before = ctx.token.balance(&ctx.recipient);
    let keeper_before = ctx.token.balance(&ctx.keeper);

    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    let sender_delta = ctx.token.balance(&ctx.sender) - sender_before;
    let recipient_delta = ctx.token.balance(&ctx.recipient) - recipient_before;
    let keeper_delta = ctx.token.balance(&ctx.keeper) - keeper_before;

    assert_eq!(
        sender_delta + recipient_delta + keeper_delta,
        deposit - withdrawn,
        "all tokens must be conserved: sum of payouts == deposit - prior withdrawals"
    );
}

// ---------------------------------------------------------------------------
// #645: KeeperCancelled event payload verification
// ---------------------------------------------------------------------------

/// Helper: find the KeeperCancelled event in the env event log.
fn find_keeper_cancelled_event(ctx: &Ctx<'_>) -> KeeperCancelled {
    let events = ctx.env.events().all();
    for i in 0..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        // Check first topic is symbol "kp_cncl"
        if let Some(topic_val) = event.1.iter().next() {
            if let Ok(sym) = Symbol::try_from_val(&ctx.env, &topic_val) {
                if sym.to_string() == "kp_cncl" {
                    return KeeperCancelled::try_from_val(&ctx.env, &event.2)
                        .expect("event data must deserialize as KeeperCancelled");
                }
            }
        }
    }
    panic!("KeeperCancelled event not found");
}

/// Event payload has correct stream_id, keeper, and reconciling fee split (partial accrual).
#[test]
fn test_keeper_cancel_event_payload_partial_accrual() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit=10_000, rate=5/s, duration=1000 → accrued=5000 at end
    let stream_id = create_stream(&ctx, 10_000, 5, 0, 1000);
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);
    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    let ev = find_keeper_cancelled_event(&ctx);

    let accrued = 5_000_i128;
    let refund_gross = 10_000 - accrued; // 5_000
    let expected_keeper_fee = refund_gross * FEE_BPS / 10_000; // 25
    let expected_sender_refund = refund_gross - expected_keeper_fee; // 4_975
    let expected_recipient = accrued; // 5_000

    assert_eq!(ev.stream_id, stream_id);
    assert_eq!(ev.keeper, ctx.keeper);
    assert_eq!(ev.keeper_fee, expected_keeper_fee);
    assert_eq!(ev.recipient_amount, expected_recipient);
    assert_eq!(ev.sender_refund, expected_sender_refund);

    // Reconciliation: keeper_fee + recipient_amount + sender_refund == deposit
    assert_eq!(ev.keeper_fee + ev.recipient_amount + ev.sender_refund, 10_000);
}

/// Event payload for a fully-accrued stream: keeper_fee == 0, no sender refund.
#[test]
fn test_keeper_cancel_event_payload_fully_accrued_zero_fee() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    // deposit == rate * duration → fully accrued, no sender refund
    let stream_id = create_stream(&ctx, 1000, 1, 0, 1000);
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);
    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    let ev = find_keeper_cancelled_event(&ctx);

    assert_eq!(ev.stream_id, stream_id);
    assert_eq!(ev.keeper, ctx.keeper);
    assert_eq!(ev.keeper_fee, 0);
    assert_eq!(ev.sender_refund, 0);
    assert_eq!(ev.recipient_amount, 1000);

    // Reconciliation
    assert_eq!(ev.keeper_fee + ev.recipient_amount + ev.sender_refund, 1000);
}

/// Event reflects actual transferred amounts: event matches token balance deltas.
#[test]
fn test_keeper_cancel_event_matches_actual_transfers() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = create_stream(&ctx, 10_000, 5, 0, 1000);
    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);

    let sender_before = ctx.token.balance(&ctx.sender);
    let recipient_before = ctx.token.balance(&ctx.recipient);
    let keeper_before = ctx.token.balance(&ctx.keeper);

    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    let ev = find_keeper_cancelled_event(&ctx);

    assert_eq!(ctx.token.balance(&ctx.recipient) - recipient_before, ev.recipient_amount);
    assert_eq!(ctx.token.balance(&ctx.sender) - sender_before, ev.sender_refund);
    assert_eq!(ctx.token.balance(&ctx.keeper) - keeper_before, ev.keeper_fee);
}

/// Event is emitted after transfers (CEI): status is Cancelled when event fires.
/// We verify indirectly: state is terminal and event was emitted in the same tx.
#[test]
fn test_keeper_cancel_event_emitted_after_terminal_state_written() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = create_stream(&ctx, 2000, 1, 0, 2000);
    ctx.env.ledger().set_timestamp(2000 + GRACE + 1);

    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    // Event was emitted (find_keeper_cancelled_event panics otherwise)
    let ev = find_keeper_cancelled_event(&ctx);
    assert_eq!(ev.stream_id, stream_id);

    // State is terminal — confirms CEI: write happened before event
    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(stream.status, StreamStatus::Cancelled);
}

/// Event reconciles to deposit when recipient had prior withdrawals.
#[test]
fn test_keeper_cancel_event_reconciles_with_prior_withdrawal() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = create_stream(&ctx, 10_000, 5, 0, 1000);

    // Recipient withdraws at t=200: accrued=1000, withdrawn=1000
    ctx.env.ledger().set_timestamp(200);
    ctx.client().withdraw(&stream_id);
    let withdrawn = ctx.token.balance(&ctx.recipient); // 1000

    ctx.env.ledger().set_timestamp(1000 + GRACE + 1);
    ctx.client().keeper_cancel(&stream_id, &ctx.keeper);

    let ev = find_keeper_cancelled_event(&ctx);

    // keeper_fee + recipient_amount + sender_refund == deposit - withdrawn
    assert_eq!(
        ev.keeper_fee + ev.recipient_amount + ev.sender_refund,
        10_000 - withdrawn,
    );
}
