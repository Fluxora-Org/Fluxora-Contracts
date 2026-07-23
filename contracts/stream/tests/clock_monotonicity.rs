extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    _token: TokenClient<'a>,
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
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        TestContext {
            env,
            contract_id,
            sender,
            recipient,
            _token: token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn create_stream(&self) -> u64 {
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0_i128,
            &None,
        )
    }

    /// Set the ledger to an explicit (sequence, timestamp) pair, decoupling the two axes
    /// independently.  This mirrors the real Stellar network behaviour where rapid ledger
    /// closes can advance the sequence number much faster (or slower) than wall-clock time.
    fn set_ledger(&self, sequence: u32, timestamp: u64) {
        self.env.ledger().set(LedgerInfo {
            sequence_number: sequence,
            timestamp,
            protocol_version: 20,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 16,
            max_entry_ttl: 6_312_000,
        });
    }
}

#[test]
fn equal_ledger_timestamp_is_accepted() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 500);

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 500);
}

#[test]
fn retrograde_get_withdrawable_returns_clock_regression() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 500);

    ctx.env.ledger().set_timestamp(499);
    let result = ctx.client().try_get_withdrawable(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::ClockRegression)));
}

#[test]
fn retrograde_withdraw_returns_clock_regression_before_state_changes() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(600);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 600);

    ctx.env.ledger().set_timestamp(500);
    let result = ctx.client().try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::ClockRegression)));

    ctx.env.ledger().set_timestamp(600);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 600);
}

#[test]
fn forward_progress_after_regression_attempt_is_still_accepted() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(300);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 300);

    ctx.env.ledger().set_timestamp(299);
    assert_eq!(
        ctx.client().try_get_withdrawable(&stream_id),
        Err(Ok(ContractError::ClockRegression))
    );

    ctx.env.ledger().set_timestamp(301);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 301);
}

// ---------------------------------------------------------------------------
// Ledger-sequence vs. timestamp decoupling tests
//
// Stellar ledgers can advance their sequence number (block height) and their
// timestamp independently: a burst of rapid ledger closes pushes the sequence
// far ahead while the timestamp barely moves, and a slow-close period does the
// opposite.  The accrual formula is defined solely in terms of wall-clock
// seconds (`env.ledger().timestamp()`).  Ledger sequence is used only for
// DoS-prevention rate limits (pause/resume cooldown and withdrawal-frequency
// guard) — it is intentionally NOT part of accrual math.
//
// These two tests pin that invariant in a regression-proof form:
//   1. Sequence advances rapidly, timestamp stays static → accrued amount
//      matches the timestamp-only expectation, not sequence count.
//   2. Timestamp advances normally, sequence is held constant → normal accrual
//      still works; the sequence gate passes because MIN_WITHDRAW_INTERVAL_LEDGERS
//      = 1 and we start at sequence 0 with last_withdraw_ledger = 0.
// ---------------------------------------------------------------------------

/// # Sequence-advances-fast / timestamp-static
///
/// Simulate a burst of rapid ledger closes: advance the ledger sequence number
/// by 10 000 while holding the timestamp at a constant 400 seconds.  Accrual
/// must equal exactly 400 tokens (rate 1 token/s × 400 s) — the sequence count
/// has no influence on the amount owed to the recipient.
///
/// The withdrawal-frequency guard uses sequence numbers, so we also advance
/// sequence by at least MIN_WITHDRAW_INTERVAL_LEDGERS (= 1) to let the
/// withdrawal succeed.  The test confirms that crossing the sequence threshold
/// does NOT inflate the accrued amount: it remains 400, not 10 000.
///
/// Any accidental sequence-to-accrual dependency would cause the asserted
/// amount to diverge from 400 and this test to fail, surfacing the bug.
#[test]
fn sequence_advances_fast_timestamp_static_accrual_is_timestamp_only() {
    let ctx = TestContext::setup();

    // Start at sequence 0, timestamp 0 — stream starts at t=0.
    ctx.set_ledger(0, 0);
    let stream_id = ctx.create_stream();

    // Burst: 10 000 ledger closes, but only 400 seconds of wall-clock time.
    // The sequence jumps far ahead; the timestamp barely moves.
    ctx.set_ledger(10_000, 400);

    // Accrual must be 400 (rate=1 × 400 seconds elapsed) — NOT 10 000.
    // If there were any accidental dependency on sequence the result would differ.
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 400,
        "accrual must equal elapsed_seconds × rate (400), not the ledger sequence count (10 000)"
    );

    // Withdrawal also succeeds: current_sequence (10 000) − last_withdraw_ledger (0)
    // = 10 000 >= MIN_WITHDRAW_INTERVAL_LEDGERS (1), so the DoS gate passes.
    // The withdrawn amount must match the timestamp-based accrual.
    let withdrawn = ctx.client().withdraw(&stream_id).unwrap();
    assert_eq!(
        withdrawn, 400,
        "withdraw must transfer exactly the timestamp-based accrued amount, not the sequence count"
    );
}

/// # Timestamp-advances / sequence-static (or minimal)
///
/// Simulate the inverse: timestamp advances by 700 seconds while the sequence
/// number is held at the minimum value needed to pass the withdrawal-frequency
/// DoS gate (MIN_WITHDRAW_INTERVAL_LEDGERS = 1, so sequence 1 is enough).
///
/// Accrual must equal exactly 700 tokens — the frozen sequence does not block
/// or reduce accrual in any way.
///
/// This confirms the positive path: timestamp-driven accrual continues to work
/// correctly even when the ledger sequence is not advancing at a proportional
/// rate, ruling out any latent sequence ≥ threshold guard inside the accrual path.
#[test]
fn timestamp_advances_sequence_static_normal_accrual_works() {
    let ctx = TestContext::setup();

    // Start at sequence 0, timestamp 0.
    ctx.set_ledger(0, 0);
    let stream_id = ctx.create_stream();

    // Only 1 ledger close occurs (sequence = 1), but 700 seconds of wall-clock
    // time elapse.  The sequence barely moved; time advanced normally.
    ctx.set_ledger(1, 700);

    // Accrual is purely timestamp-driven: 700 tokens (rate=1 × 700 s).
    let accrued = ctx.client().calculate_accrued(&stream_id);
    assert_eq!(
        accrued, 700,
        "accrual must equal elapsed_seconds × rate (700) regardless of the low sequence count (1)"
    );

    // Withdrawal succeeds: current_sequence (1) − last_withdraw_ledger (0)
    // = 1 >= MIN_WITHDRAW_INTERVAL_LEDGERS (1), so the DoS gate passes.
    let withdrawn = ctx.client().withdraw(&stream_id).unwrap();
    assert_eq!(
        withdrawn, 700,
        "withdraw must transfer the full timestamp-based accrued amount (700)"
    );

    // After withdrawing, get_withdrawable must be 0.
    let remaining = ctx.client().get_withdrawable(&stream_id);
    assert_eq!(
        remaining, 0,
        "no tokens should remain after withdrawing the full accrued amount"
    );
}
