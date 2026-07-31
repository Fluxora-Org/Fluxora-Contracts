//! Tests for `get_cliff_status` and the documented ledger close-time skew
//! tolerance window (`accrual::MAX_LEDGER_CLOSE_SKEW_SECS`).
//!
//! # Motivation (issue #1486)
//!
//! `StreamKind::CliffOnly` and `CliffSlope` streams gate accrual on a strict
//! `env.ledger().timestamp() >= cliff_time` comparison. Stellar ledger close
//! times average 5-6 seconds but are not fixed to an exact cadence, so a
//! cliff set to land exactly on an expected ledger boundary can unlock a few
//! seconds later than an off-chain integrator's naive fixed-cadence
//! expectation, even though the contract logic is correct. `get_cliff_status`
//! exposes `Pending` / `WithinSkewWindow` / `Unlocked` so clients can
//! distinguish "not yet due" from "due imminently" instead of guessing.
//!
//! # What is tested
//!
//! 1. `Pending` well before the cliff, and just outside the skew window.
//! 2. `WithinSkewWindow` at the lower boundary and just before the cliff.
//! 3. `Unlocked` at and after the cliff.
//! 4. `get_cliff_status` never alters withdrawal correctness: `Unlocked`
//!    status coincides exactly with the moment `withdraw`/`calculate_accrued`
//!    start paying out, and `WithinSkewWindow` still blocks withdrawal.
//! 5. `Cancelled` streams freeze cliff status at `cancelled_at`, consistent
//!    with `calculate_accrued`'s frozen-accrual semantics.
//! 6. `StreamNotFound` for a nonexistent stream.
//! 7. Behavior across all three `StreamKind` variants (Linear, CliffOnly,
//!    CliffSlope).
//!
//! # Running
//!
//! ```bash
//! cargo test -p fluxora_stream --test cliff_close_time_skew
//! ```

#![cfg(test)]

use fluxora_stream::accrual::{CliffStatus, MAX_LEDGER_CLOSE_SKEW_SECS};
use fluxora_stream::{ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
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

        FluxoraStreamClient::new(&env, &contract_id).init(&token_id, &admin);
        StellarAssetClient::new(&env, &token_id).mint(&sender, &1_000_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &200_000u32);

        env.ledger().set_timestamp(0);

        Ctx {
            env,
            contract_id,
            sender,
            recipient,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn create_cliff_only(&self, deposit: i128, start: u64, cliff: u64, end: u64) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: 0, // CliffOnly requires rate=0
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

    fn create_cliff_slope(
        &self,
        deposit: i128,
        rate: i128,
        start: u64,
        cliff: u64,
        end: u64,
    ) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: rate,
                start_time: start,
                cliff_time: cliff,
                end_time: end,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::CliffSlope,
                irrevocable: None,
                witness: None,
            },
        )
    }

    fn create_linear(&self, deposit: i128, rate: i128, start: u64, cliff: u64, end: u64) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: rate,
                start_time: start,
                cliff_time: cliff,
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
}

// ---------------------------------------------------------------------------
// Named constant is exposed and matches the documented value (10 seconds)
// ---------------------------------------------------------------------------

#[test]
fn skew_constant_is_documented_ten_seconds() {
    assert_eq!(MAX_LEDGER_CLOSE_SKEW_SECS, 10);
}

// ---------------------------------------------------------------------------
// Pending / WithinSkewWindow / Unlocked classification via the live contract
// ---------------------------------------------------------------------------

#[test]
fn pending_well_before_cliff() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Pending);
}

#[test]
fn pending_one_second_outside_skew_window() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env
        .ledger()
        .set_timestamp(1_000 - MAX_LEDGER_CLOSE_SKEW_SECS - 1);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Pending);
}

#[test]
fn within_skew_window_at_lower_boundary() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env
        .ledger()
        .set_timestamp(1_000 - MAX_LEDGER_CLOSE_SKEW_SECS);
    assert_eq!(
        ctx.client().get_cliff_status(&id),
        CliffStatus::WithinSkewWindow
    );
}

#[test]
fn within_skew_window_one_second_before_cliff() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(999);
    assert_eq!(
        ctx.client().get_cliff_status(&id),
        CliffStatus::WithinSkewWindow
    );
}

#[test]
fn unlocked_exactly_at_cliff() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(1_000);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
}

#[test]
fn unlocked_after_cliff() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(1_500);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
}

// ---------------------------------------------------------------------------
// get_cliff_status never alters withdrawal correctness
// ---------------------------------------------------------------------------

/// While `WithinSkewWindow`, withdrawal must still be exactly as blocked as
/// `Pending` — the skew window is purely observational and must not advance
/// the real `>= cliff_time` unlock gate by even one second.
#[test]
fn within_skew_window_still_blocks_withdrawal() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env
        .ledger()
        .set_timestamp(1_000 - MAX_LEDGER_CLOSE_SKEW_SECS);
    assert_eq!(
        ctx.client().get_cliff_status(&id),
        CliffStatus::WithinSkewWindow
    );
    assert_eq!(ctx.client().calculate_accrued(&id), 0);
    assert_eq!(ctx.client().get_withdrawable(&id), 0);
}

/// The instant `get_cliff_status` reports `Unlocked` must coincide exactly
/// with the moment `calculate_accrued`/`get_withdrawable` start paying out —
/// the skew-window view and the real unlock gate must never disagree.
#[test]
fn unlocked_status_coincides_with_real_unlock_gate() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    // One second before the cliff: still not unlocked in either view.
    ctx.env.ledger().set_timestamp(999);
    assert_ne!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
    assert_eq!(ctx.client().calculate_accrued(&id), 0);

    // Exactly at the cliff: both views flip to unlocked simultaneously.
    ctx.env.ledger().set_timestamp(1_000);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
    assert_eq!(ctx.client().calculate_accrued(&id), 1_000);
    assert_eq!(ctx.client().get_withdrawable(&id), 1_000);
}

/// A full withdraw() succeeds once `Unlocked`, and the reported status is
/// unaffected by whether a withdrawal has already happened.
#[test]
fn withdraw_succeeds_once_unlocked_and_status_is_consistent_after() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(1_000);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);

    let withdrawn = ctx.client().withdraw(&id, &None);
    assert_eq!(withdrawn, 1_000);

    // Status remains Unlocked after withdrawal (it reflects time, not balance).
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
}

// ---------------------------------------------------------------------------
// Cancelled streams: cliff status freezes at cancelled_at
// ---------------------------------------------------------------------------

/// A stream cancelled while still `WithinSkewWindow` must keep reporting
/// `WithinSkewWindow` forever after — it must not silently flip to
/// `Unlocked` just because wall-clock time keeps advancing past the cliff,
/// mirroring `calculate_accrued`'s frozen-accrual-at-cancellation semantics.
#[test]
fn cancelled_stream_freezes_status_at_cancellation_time() {
    let ctx = Ctx::setup();
    // CliffSlope so cancellation before the cliff is meaningful (no lump sum).
    let id = ctx.create_cliff_slope(1_000, 1, 0, 1_000, 2_000);

    ctx.env
        .ledger()
        .set_timestamp(1_000 - MAX_LEDGER_CLOSE_SKEW_SECS);
    assert_eq!(
        ctx.client().get_cliff_status(&id),
        CliffStatus::WithinSkewWindow
    );

    ctx.client().cancel_stream(&id);

    // Advance far past the cliff after cancellation.
    ctx.env.ledger().set_timestamp(5_000);
    assert_eq!(
        ctx.client().get_cliff_status(&id),
        CliffStatus::WithinSkewWindow,
        "cancelled stream's cliff status must stay frozen at cancelled_at, not track wall-clock time"
    );
}

/// A stream cancelled after the cliff was already reached keeps reporting
/// `Unlocked` (frozen, not re-evaluated), matching `calculate_accrued`.
#[test]
fn cancelled_stream_after_cliff_stays_unlocked() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_slope(1_000, 1, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(1_500);
    ctx.client().cancel_stream(&id);

    ctx.env.ledger().set_timestamp(9_999);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[test]
fn nonexistent_stream_returns_stream_not_found() {
    let ctx = Ctx::setup();
    let result = ctx.client().try_get_cliff_status(&999);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

// ---------------------------------------------------------------------------
// All stream kinds
// ---------------------------------------------------------------------------

#[test]
fn cliff_slope_pending_within_and_unlocked() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_slope(1_000, 1, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Pending);

    ctx.env.ledger().set_timestamp(995);
    assert_eq!(
        ctx.client().get_cliff_status(&id),
        CliffStatus::WithinSkewWindow
    );

    ctx.env.ledger().set_timestamp(1_000);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
}

/// `Linear` streams (no meaningful cliff: `cliff_time == start_time`) report
/// `Unlocked` immediately at `start_time`, since there is no pre-cliff window.
#[test]
fn linear_stream_with_no_cliff_is_unlocked_from_start() {
    let ctx = Ctx::setup();
    let id = ctx.create_linear(1_000, 1, 0, 0, 1_000);

    ctx.env.ledger().set_timestamp(0);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
}

/// `Linear` stream with a real cliff still classifies Pending/WithinSkewWindow
/// before it, exactly like CliffOnly/CliffSlope.
#[test]
fn linear_stream_with_cliff_classifies_before_unlock() {
    let ctx = Ctx::setup();
    // deposit must cover rate * (end - start) = 1 * 2_000 = 2_000.
    let id = ctx.create_linear(2_000, 1, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(200);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Pending);

    ctx.env.ledger().set_timestamp(995);
    assert_eq!(
        ctx.client().get_cliff_status(&id),
        CliffStatus::WithinSkewWindow
    );

    ctx.env.ledger().set_timestamp(1_000);
    assert_eq!(ctx.client().get_cliff_status(&id), CliffStatus::Unlocked);
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

/// Repeated calls at the same timestamp return the same status (pure view).
#[test]
fn get_cliff_status_is_deterministic() {
    let ctx = Ctx::setup();
    let id = ctx.create_cliff_only(1_000, 0, 1_000, 2_000);

    ctx.env.ledger().set_timestamp(995);
    let a = ctx.client().get_cliff_status(&id);
    let b = ctx.client().get_cliff_status(&id);
    assert_eq!(a, b);
}
