//! Adversarial and deterministic tests for `CliffOnly` streams covering
//! `keeper_cancel`, `bulk_cancel_streams`, and `cancel_stream`.
//!
//! # Motivation (issue #1193)
//!
//! `keeper_cancel` and `bulk_cancel_streams` both call
//! `accrual::calculate_accrued_amount_checkpointed` generically across
//! `StreamKind`, without any `StreamKind`-specific branch in the caller.
//! For `StreamKind::CliffOnly`, the accrued amount is either `0` or the full
//! `deposit_amount` (never a partial value), which means the "unstreamed
//! portion" (`sender_refund_gross`) and keeper-fee math collapse to edge cases
//! that the existing Linear-stream test suites do not exercise.
//!
//! # What is tested
//!
//! 1. **CliffOnly + keeper_cancel after cliff** → `sender_refund_gross == 0`,
//!    `keeper_fee == 0`, full deposit flows to recipient.
//! 2. **CliffOnly + bulk_cancel_streams before cliff** → `recipient_amount == 0`,
//!    full deposit refunded to sender.
//! 3. **CliffOnly + cancel_stream before cliff** → `recipient_amount == 0`,
//!    full deposit refunded to sender.
//! 4. **CliffOnly + bulk_cancel_streams after cliff** → recipient gets full
//!    deposit, sender gets 0.
//! 5. **Event payloads** (`KeeperCancelled`, `StreamCancelled`) are consistent
//!    with actual token transfers.
//! 6. **TotalLiabilities** decreases correctly in all cancellation paths.
//!
//! # Constraints
//!
//! - `keeper_cancel` requires `now >= end_time + KEEPER_GRACE_PERIOD_SECONDS`.
//! - Stream validation requires `start_time <= cliff_time <= end_time`.
//! - Therefore `keeper_cancel` on a CliffOnly stream always fires *after* the
//!   cliff, meaning `accrued == deposit_amount` and `sender_refund_gross == 0`.
//!   The "before cliff" adversarial path is exercised through `cancel_stream`
//!   and `bulk_cancel_streams` (sender-side operations with no grace period).
//!
//! # Running
//!
//! ```bash
//! cargo test -p fluxora_stream --features testutils --test cliff_only_variant
//! ```

#![cfg(test)]

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, KeeperCancelled,
    StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, Symbol, TryFromVal,
};

// ---------------------------------------------------------------------------
// Constants (mirror production values from lib.rs)
// ---------------------------------------------------------------------------

/// Seconds past `end_time` before a keeper may cancel.
const GRACE: u64 = 604_800;
/// Keeper incentive fee in basis points (0.5 %).
const FEE_BPS: i128 = 50;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct Ctx {
    env: Env,
    contract_id: Address,
    token_id: Address,
    sender: Address,
    recipient: Address,
    keeper: Address,
    admin: Address,
}

impl Ctx {
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

        FluxoraStreamClient::new(&env, &contract_id).init(&token_id, &admin);

        StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_000_i128);

        TokenClient::new(&env, &token_id).approve(
            &sender,
            &contract_id,
            &i128::MAX,
            &200_000u32,
        );

        env.ledger().set_timestamp(0);

        Ctx {
            env,
            contract_id,
            token_id,
            sender,
            recipient,
            keeper,
            admin,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    fn balance(&self, addr: &Address) -> i128 {
        self.token().balance(addr)
    }

    fn contract_balance(&self) -> i128 {
        self.token().balance(&self.contract_id)
    }
}

// ---------------------------------------------------------------------------
// Helper: create a CliffOnly stream
// ---------------------------------------------------------------------------

fn create_cliff_stream(
    ctx: &Ctx,
    deposit: i128,
    start: u64,
    cliff: u64,
    end: u64,
) -> u64 {
    ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: 0, // CliffOnly must have rate=0
            start_time: start,
            cliff_time: cliff,
            end_time: end,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::CliffOnly,
            irrevocable: None,
            witness: None,
        },
    )
}

// ---------------------------------------------------------------------------
// Helper: extract KeeperCancelled event from env log
// ---------------------------------------------------------------------------

fn find_keeper_cancelled(ctx: &Ctx) -> KeeperCancelled {
    let events = ctx.env.events().all();
    for i in 0..events.len() {
        let ev = events.get(i).unwrap();
        if ev.0 != ctx.contract_id {
            continue;
        }
        if let Some(tv) = ev.1.iter().next() {
            if let Ok(sym) = Symbol::try_from_val(&ctx.env, &tv) {
                if sym.to_string() == "kp_cncl" {
                    return KeeperCancelled::try_from_val(&ctx.env, &ev.2)
                        .expect("event data must deserialize as KeeperCancelled");
                }
            }
        }
    }
    panic!("KeeperCancelled event not found in event log");
}

// ===========================================================================
// 1. CliffOnly + keeper_cancel after cliff → zero fee, full to recipient
// ===========================================================================

/// CliffOnly stream past `end_time + GRACE`, past `cliff_time`:
/// accrued == deposit_amount → sender_refund_gross == 0 → keeper_fee == 0.
/// Recipient receives the full deposit.
#[test]
fn test_cliffonly_keeper_cancel_after_cliff_zero_fee() {
    let ctx = Ctx::setup();
    let deposit: i128 = 5_000;

    // start=0, cliff=500, end=1000
    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);

    // Advance past end_time + grace period (well past cliff)
    let cancel_ts = 1_000 + GRACE + 1;
    ctx.env.ledger().set_timestamp(cancel_ts);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let keeper_before = ctx.balance(&ctx.keeper);
    let liabilities_before = ctx.client().get_total_liabilities();

    ctx.client().keeper_cancel(&sid, &ctx.keeper);

    // ── Balance assertions ────────────────────────────────────────────────
    let recipient_delta = ctx.balance(&ctx.recipient) - recipient_before;
    let sender_delta = ctx.balance(&ctx.sender) - sender_before;
    let keeper_delta = ctx.balance(&ctx.keeper) - keeper_before;

    assert_eq!(recipient_delta, deposit, "recipient must receive full deposit");
    assert_eq!(sender_delta, 0, "sender must receive 0 (accrued == deposit)");
    assert_eq!(keeper_delta, 0, "keeper fee must be 0 (refund_gross == 0)");
    assert_eq!(ctx.contract_balance(), 0, "contract must hold 0 after cancel");

    // ── TotalLiabilities ──────────────────────────────────────────────────
    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        liabilities_before - deposit,
        "TotalLiabilities must decrease by full deposit"
    );

    // ── Stream state ──────────────────────────────────────────────────────
    let stream = ctx.client().get_stream_state(&sid);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.cancelled_at, Some(cancel_ts));

    // ── KeeperCancelled event ─────────────────────────────────────────────
    let ev = find_keeper_cancelled(&ctx);
    assert_eq!(ev.stream_id, sid);
    assert_eq!(ev.keeper, ctx.keeper);
    assert_eq!(ev.keeper_fee, 0);
    assert_eq!(ev.recipient_amount, deposit);
    assert_eq!(ev.sender_refund, 0);
    assert_eq!(
        ev.keeper_fee + ev.recipient_amount + ev.sender_refund,
        deposit,
        "event amounts must sum to deposit"
    );
}

/// CliffOnly stream fully withdrawn after cliff becomes Completed;
/// keeper_cancel must reject Completed streams with InvalidState.
///
/// For CliffOnly, the full deposit is withdrawable at any time >= cliff,
/// so after a post-cliff withdrawal the stream is Completed and
/// keeper_cancel is a no-op path.
#[test]
fn test_cliffonly_keeper_cancel_rejects_completed_stream() {
    let ctx = Ctx::setup();
    let deposit: i128 = 5_000;

    // start=0, cliff=500, end=1000
    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);

    // Full withdrawal after cliff → stream becomes Completed
    ctx.env.ledger().set_timestamp(600);
    ctx.env.ledger().set_sequence_number(1);
    let withdrawn = ctx.client().withdraw(&sid);
    assert_eq!(withdrawn, deposit, "CliffOnly: full deposit withdrawable at t=600");

    let stream = ctx.client().get_stream_state(&sid);
    assert_eq!(stream.status, StreamStatus::Completed);

    // keeper_cancel must reject Completed streams
    ctx.env.ledger().set_timestamp(1_000 + GRACE + 1);
    let result = ctx.client().try_keeper_cancel(&sid, &ctx.keeper);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

// ===========================================================================
// 2. CliffOnly + bulk_cancel_streams before cliff → full refund to sender
// ===========================================================================

/// CliffOnly stream cancelled via `bulk_cancel_streams` before `cliff_time`:
/// accrued == 0 → recipient_amount == 0, full deposit refunded to sender.
#[test]
fn test_cliffonly_bulk_cancel_before_cliff_full_refund() {
    let ctx = Ctx::setup();
    let deposit: i128 = 2_000;

    // start=0, cliff=500, end=1000
    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);

    // Advance to t=200 (before cliff=500)
    ctx.env.ledger().set_timestamp(200);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let liabilities_before = ctx.client().get_total_liabilities();

    ctx.client().bulk_cancel_streams(&ctx.sender, &vec![&ctx.env, sid]);

    // ── Balance assertions ────────────────────────────────────────────────
    let recipient_delta = ctx.balance(&ctx.recipient) - recipient_before;
    let sender_delta = ctx.balance(&ctx.sender) - sender_before;

    assert_eq!(recipient_delta, 0, "recipient must receive 0 before cliff");
    assert_eq!(sender_delta, deposit, "sender must receive full deposit refund");
    assert_eq!(ctx.contract_balance(), 0, "contract must hold 0 after cancel");

    // ── TotalLiabilities ──────────────────────────────────────────────────
    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        liabilities_before - deposit,
        "TotalLiabilities must decrease by full deposit"
    );

    // ── Stream state ──────────────────────────────────────────────────────
    let stream = ctx.client().get_stream_state(&sid);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.withdrawn_amount, 0);
}

/// CliffOnly stream cancelled via `bulk_cancel_streams` after `cliff_time`:
/// accrued == deposit_amount → recipient gets full deposit, sender gets 0.
#[test]
fn test_cliffonly_bulk_cancel_after_cliff_recipient_gets_all() {
    let ctx = Ctx::setup();
    let deposit: i128 = 2_000;

    // start=0, cliff=500, end=1000
    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);

    // Advance to t=600 (after cliff=500)
    ctx.env.ledger().set_timestamp(600);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let liabilities_before = ctx.client().get_total_liabilities();

    ctx.client().bulk_cancel_streams(&ctx.sender, &vec![&ctx.env, sid]);

    // ── Balance assertions ────────────────────────────────────────────────
    let recipient_delta = ctx.balance(&ctx.recipient) - recipient_before;
    let sender_delta = ctx.balance(&ctx.sender) - sender_before;

    assert_eq!(recipient_delta, deposit, "recipient must receive full deposit after cliff");
    assert_eq!(sender_delta, 0, "sender must receive 0 (fully accrued)");
    assert_eq!(ctx.contract_balance(), 0, "contract must hold 0 after cancel");

    // ── TotalLiabilities ──────────────────────────────────────────────────
    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        liabilities_before - deposit,
        "TotalLiabilities must decrease by full deposit"
    );

    // ── Stream state ──────────────────────────────────────────────────────
    let stream = ctx.client().get_stream_state(&sid);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.withdrawn_amount, deposit);
}

// ===========================================================================
// 3. CliffOnly + cancel_stream before cliff → full refund to sender
// ===========================================================================

/// CliffOnly stream cancelled via `cancel_stream` (sender-side) before
/// `cliff_time`: accrued == 0 → recipient gets 0, sender gets full refund.
#[test]
fn test_cliffonly_cancel_stream_before_cliff_full_refund() {
    let ctx = Ctx::setup();
    let deposit: i128 = 1_500;

    // start=0, cliff=500, end=1000
    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);

    // Advance to t=300 (before cliff=500)
    ctx.env.ledger().set_timestamp(300);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let liabilities_before = ctx.client().get_total_liabilities();

    ctx.client().cancel_stream(&sid);

    // ── Balance assertions ────────────────────────────────────────────────
    let recipient_delta = ctx.balance(&ctx.recipient) - recipient_before;
    let sender_delta = ctx.balance(&ctx.sender) - sender_before;

    assert_eq!(recipient_delta, 0, "recipient must receive 0 before cliff");
    assert_eq!(sender_delta, deposit, "sender must receive full deposit refund");
    assert_eq!(ctx.contract_balance(), 0, "contract must hold 0 after cancel");

    // ── TotalLiabilities ──────────────────────────────────────────────────
    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        liabilities_before - deposit,
        "TotalLiabilities must decrease by full deposit"
    );

    // ── Stream state ──────────────────────────────────────────────────────
    let stream = ctx.client().get_stream_state(&sid);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.withdrawn_amount, 0);
}

/// CliffOnly stream cancelled via `cancel_stream` (sender-side) after
/// `cliff_time`: accrued == deposit → recipient gets full deposit, sender gets 0.
#[test]
fn test_cliffonly_cancel_stream_after_cliff_recipient_gets_all() {
    let ctx = Ctx::setup();
    let deposit: i128 = 1_500;

    // start=0, cliff=500, end=1000
    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);

    // Advance to t=700 (after cliff=500)
    ctx.env.ledger().set_timestamp(700);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let liabilities_before = ctx.client().get_total_liabilities();

    ctx.client().cancel_stream(&sid);

    // ── Balance assertions ────────────────────────────────────────────────
    let recipient_delta = ctx.balance(&ctx.recipient) - recipient_before;
    let sender_delta = ctx.balance(&ctx.sender) - sender_before;

    assert_eq!(recipient_delta, deposit, "recipient must receive full deposit after cliff");
    assert_eq!(sender_delta, 0, "sender must receive 0 (fully accrued)");
    assert_eq!(ctx.contract_balance(), 0, "contract must hold 0 after cancel");

    // ── TotalLiabilities ──────────────────────────────────────────────────
    let liabilities_after = ctx.client().get_total_liabilities();
    assert_eq!(
        liabilities_after,
        liabilities_before - deposit,
        "TotalLiabilities must decrease by full deposit"
    );

    // ── Stream state ──────────────────────────────────────────────────────
    let stream = ctx.client().get_stream_state(&sid);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.withdrawn_amount, deposit);
}

// ===========================================================================
// 4. Event payload consistency for CliffOnly cancellation
// ===========================================================================

/// `KeeperCancelled` event payload matches actual token transfers.
#[test]
fn test_cliffonly_keeper_cancel_event_matches_transfers() {
    let ctx = Ctx::setup();
    let deposit: i128 = 3_000;

    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);

    ctx.env.ledger().set_timestamp(1_000 + GRACE + 1);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let keeper_before = ctx.balance(&ctx.keeper);

    ctx.client().keeper_cancel(&sid, &ctx.keeper);

    let ev = find_keeper_cancelled(&ctx);

    assert_eq!(
        ctx.balance(&ctx.recipient) - recipient_before,
        ev.recipient_amount,
        "event recipient_amount must match actual recipient delta"
    );
    assert_eq!(
        ctx.balance(&ctx.sender) - sender_before,
        ev.sender_refund,
        "event sender_refund must match actual sender delta"
    );
    assert_eq!(
        ctx.balance(&ctx.keeper) - keeper_before,
        ev.keeper_fee,
        "event keeper_fee must match actual keeper delta"
    );
}

/// `StreamCancelled` event is emitted for bulk_cancel of CliffOnly streams.
#[test]
fn test_cliffonly_bulk_cancel_emits_cancelled_event() {
    let ctx = Ctx::setup();
    let deposit: i128 = 1_000;

    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);
    ctx.env.ledger().set_timestamp(200);

    ctx.client().bulk_cancel_streams(&ctx.sender, &vec![&ctx.env, sid]);

    // Verify a "cancelled" event was emitted for the stream
    let events = ctx.env.events().all();
    let cancelled_count = events
        .iter()
        .filter(|e| {
            e.1.iter()
                .next()
                .and_then(|t| Symbol::try_from_val(&ctx.env, &t).ok())
                .map(|s| s.to_string() == "cancelled")
                .unwrap_or(false)
        })
        .count();

    assert!(cancelled_count >= 1, "must emit at least one 'cancelled' event");
}

// ===========================================================================
// 5. Token conservation across CliffOnly cancellation paths
// ===========================================================================

/// Conservation: payouts must equal deposit (no tokens created/destroyed).
#[test]
fn test_cliffonly_keeper_cancel_conservation() {
    let ctx = Ctx::setup();
    let deposit: i128 = 4_000;

    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);
    ctx.env.ledger().set_timestamp(1_000 + GRACE + 1);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let keeper_before = ctx.balance(&ctx.keeper);
    let contract_before = ctx.contract_balance();

    ctx.client().keeper_cancel(&sid, &ctx.keeper);

    let total_delta = (ctx.balance(&ctx.recipient) - recipient_before)
        + (ctx.balance(&ctx.sender) - sender_before)
        + (ctx.balance(&ctx.keeper) - keeper_before);

    assert_eq!(
        total_delta, deposit,
        "conservation: payouts must equal deposit"
    );
    assert_eq!(
        contract_before - ctx.contract_balance(),
        deposit,
        "contract balance must decrease by full deposit"
    );
}

/// Conservation: bulk_cancel before cliff refunds full deposit to sender.
#[test]
fn test_cliffonly_bulk_cancel_before_cliff_conservation() {
    let ctx = Ctx::setup();
    let deposit: i128 = 4_000;

    let sid = create_cliff_stream(&ctx, deposit, 0, 500, 1_000);
    ctx.env.ledger().set_timestamp(200);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let contract_before = ctx.contract_balance();

    ctx.client().bulk_cancel_streams(&ctx.sender, &vec![&ctx.env, sid]);

    let total_delta = (ctx.balance(&ctx.recipient) - recipient_before)
        + (ctx.balance(&ctx.sender) - sender_before);

    assert_eq!(
        total_delta, deposit,
        "conservation: payouts must equal deposit (all to sender before cliff)"
    );
    assert_eq!(
        contract_before - ctx.contract_balance(),
        deposit,
        "contract balance must decrease by full deposit"
    );
}

// ===========================================================================
// 6. Edge-case: CliffOnly keeper_cancel at exactly the grace period boundary
// ===========================================================================

/// CliffOnly stream cancelled at `end_time + GRACE` (exactly at boundary)
/// must succeed (inclusive check: `now < end_time + GRACE` is the rejection).
#[test]
fn test_cliffonly_keeper_cancel_exactly_at_grace_boundary() {
    let ctx = Ctx::setup();
    let deposit: i128 = 1_000;

    // start=0, cliff=100, end=500
    let sid = create_cliff_stream(&ctx, deposit, 0, 100, 500);

    ctx.env.ledger().set_timestamp(500 + GRACE);
    ctx.client().keeper_cancel(&sid, &ctx.keeper);

    assert_eq!(ctx.balance(&ctx.recipient), deposit);
    assert_eq!(
        ctx.client().get_stream_state(&sid).status,
        StreamStatus::Cancelled
    );
}

// ===========================================================================
// 7. Edge-case: CliffOnly keeper_cancel too early (grace period not elapsed)
// ===========================================================================

#[test]
fn test_cliffonly_keeper_cancel_too_early_errors() {
    let ctx = Ctx::setup();
    let deposit: i128 = 1_000;

    let sid = create_cliff_stream(&ctx, deposit, 0, 100, 500);

    // t = end + GRACE - 1 → too early
    ctx.env.ledger().set_timestamp(500 + GRACE - 1);
    let result = ctx.client().try_keeper_cancel(&sid, &ctx.keeper);
    assert_eq!(result, Err(Ok(ContractError::KeeperGracePeriodNotElapsed)));
}

// ===========================================================================
// 8. Edge-case: CliffOnly bulk_cancel with zero accrued (before cliff)
//   verifies withdrawn_amount is correctly 0
// ===========================================================================

#[test]
fn test_cliffonly_bulk_cancel_before_cliff_withdrawn_amount_zero() {
    let ctx = Ctx::setup();
    let deposit: i128 = 3_000;

    let sid = create_cliff_stream(&ctx, deposit, 0, 1_000, 2_000);

    // t=500, before cliff=1000
    ctx.env.ledger().set_timestamp(500);

    ctx.client().bulk_cancel_streams(&ctx.sender, &vec![&ctx.env, sid]);

    let stream = ctx.client().get_stream_state(&sid);
    assert_eq!(stream.status, StreamStatus::Cancelled);
    assert_eq!(stream.withdrawn_amount, 0, "no accrual before cliff → withdrawn must be 0");
}

// ===========================================================================
// 9. Multiple CliffOnly streams in a single bulk_cancel
// ===========================================================================

#[test]
fn test_cliffonly_bulk_cancel_multiple_streams_mixed_cliff() {
    let ctx = Ctx::setup();

    let s1 = create_cliff_stream(&ctx, 1_000, 0, 500, 1_000);
    let s2 = create_cliff_stream(&ctx, 2_000, 0, 500, 1_000);

    // t=700, after cliff for both
    ctx.env.ledger().set_timestamp(700);

    let sender_before = ctx.balance(&ctx.sender);
    let recipient_before = ctx.balance(&ctx.recipient);
    let liabilities_before = ctx.client().get_total_liabilities();

    ctx.client().bulk_cancel_streams(&ctx.sender, &vec![&ctx.env, s1, s2]);

    let recipient_delta = ctx.balance(&ctx.recipient) - recipient_before;
    let sender_delta = ctx.balance(&ctx.sender) - sender_before;

    assert_eq!(recipient_delta, 3_000, "recipient gets both full deposits");
    assert_eq!(sender_delta, 0, "sender gets 0 (both fully accrued)");
    assert_eq!(
        ctx.client().get_total_liabilities(),
        liabilities_before - 3_000,
    );

    for sid in [s1, s2] {
        assert_eq!(
            ctx.client().get_stream_state(&sid).status,
            StreamStatus::Cancelled
        );
    }
}
