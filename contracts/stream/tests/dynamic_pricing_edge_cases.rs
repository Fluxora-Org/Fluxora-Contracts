//! Regression tests locking in the **dynamic-pricing edge-case behavior** of
//! `contracts/stream/src/lib.rs`. Companion to issue **#1315** ("Harden
//! dynamic pricing edge cases").
//!
//! These tests pin the current branches and boundary conditions that the
//! dynamic-pricing entrypoints share across multiple gates:
//!
//! | Entrypoint                          | kind != Linear | decommissioned | paused | terminal | irrevocable |
//! |-------------------------------------|----------------|----------------|--------|----------|-------------|
//! | `update_rate_per_second`            | `UnsupportedStreamKind` (28) | `InvalidState` (2) | allowed | rejected | allowed |
//! | `decrease_rate_per_second`          | `UnsupportedStreamKind` (28) | `InvalidState` (2) | allowed | rejected | allowed |
//! | `top_up_stream`                     | `UnsupportedStreamKind` (28) | `InvalidState` (2) | allowed | rejected | allowed |
//! | `shorten_stream_end_time`           | `UnsupportedStreamKind` (28) | **allowed**        | allowed | rejected | **rejected** |
//! | `extend_stream_end_time`            | `UnsupportedStreamKind` (28) | `InvalidState` (2) | allowed | rejected | allowed |
//!
//! Where "terminal" means `stream.status == Completed || Cancelled`.
//!
//! These tests do **not** introduce any new behavior — they only lock in the
//! branches and boundary conditions that are already implemented so that
//! future refactors cannot silently regress them.
//!
//! Existing test files cover other dimensions of these entrypoints
//! (rate-cap, cooldowns, rate-decrease checkpoint math, dust-threshold):
//! `tests/max_rate_per_second.rs`, `tests/rate_bounds.rs`,
//! `tests/rate_decrease_after_withdraw.rs`, `tests/dust_threshold.rs`,
//! `tests/pause_semantics.rs`.

#![cfg(test)]

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

const INITIAL_BALANCE: i128 = 1_000_000_000;
const LINEAR_DEPOSIT: i128 = 1_000;
const LINEAR_RATE: i128 = 10;
const STREAM_DURATION_SECS: u64 = 1_000;

/// Allocation big enough for every fixture (deposit, refunds, top-ups).
const SENDER_ALLOWANCE: i128 = 1_000_000_000_000;

struct Ctx<'a> {
    env: Env,
    #[allow(dead_code)]
    contract_id: Address,
    client: FluxoraStreamClient<'a>,
    #[allow(dead_code)]
    admin: Address,
    sender: Address,
    recipient: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
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
        let stellar_asset = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        stellar_asset.mint(&sender, &INITIAL_BALANCE);
        token.approve(&sender, &contract_id, &i128::MAX, &1_000_000u32);

        client.init(&token_id, &admin);

        Self {
            env,
            contract_id,
            client,
            admin,
            sender,
            recipient,
            token,
        }
    }

    /// Build a `CreateStreamParams` for a Linear stream starting at
    /// `ledger.timestamp()` with `duration` seconds and `rate` tokens/sec.
    ///
    /// Deposit is set to `rate * duration * 2` so it remains sufficient
    /// even after the rate is increased (`update_rate_per_second`) or the
    /// schedule is doubled (`extend_stream_end_time`). All tests that use
    /// these helpers rely on this headroom; do not shrink it without
    /// auditing the consumers.
    fn linear_params(&self, rate: i128, duration: u64, irrevocable: Option<bool>) -> CreateStreamParams {
        let now = self.env.ledger().timestamp();
        CreateStreamParams {
            recipient: self.recipient.clone(),
            deposit_amount: rate * duration as i128 * 2,
            rate_per_second: rate,
            start_time: now,
            cliff_time: now,
            end_time: now + duration,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable,
            witness: None,
        }
    }

    /// Create a Linear stream and return its id.
    fn create_linear(&self, rate: i128, duration: u64, irrevocable: Option<bool>) -> u64 {
        self.client
            .create_stream(&self.sender, &self.linear_params(rate, duration, irrevocable))
    }

    /// Create a `CliffOnly` stream — `rate_per_second` must be `0` per
    /// `validate_stream_params_with_self_policy`.
    fn create_cliff_only(&self, deposit: i128) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: 0,
                start_time: now,
                cliff_time: now,
                end_time: now + STREAM_DURATION_SECS,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::CliffOnly,
                irrevocable: None,
                witness: None,
            },
        )
    }

    /// Create a `CliffSlope` stream — `rate_per_second > 0` and deposit must
    /// cover `rate * (end - cliff)`.
    fn create_cliff_slope(&self, rate: i128, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        // Cliff = start so the whole schedule is "post-cliff".
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: rate * duration as i128,
                rate_per_second: rate,
                start_time: now,
                cliff_time: now,
                end_time: now + duration,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::CliffSlope,
                irrevocable: None,
                witness: None,
            },
        )
    }

    /// Advance the ledger sequence past the pause/resume cooldown window
    /// (`MIN_PAUSE_INTERVAL_LEDGERS == 17`).
    fn clear_pause_cooldown(&self) {
        self.env.ledger().with_mut(|l| {
            l.sequence_number += 32;
        });
    }
}

// ---------------------------------------------------------------------------
// Stream-kind branching
// ---------------------------------------------------------------------------
//
// `update_rate_per_second`, `decrease_rate_per_second`, `top_up_stream`,
// `shorten_stream_end_time`, and `extend_stream_end_time` all share the
// branch `if stream.kind != StreamKind::Linear { return Err(
// ContractError::UnsupportedStreamKind) }`. CliffOnly and CliffSlope
// streams are one-shot / post-cliff designs whose economic state cannot be
// safely re-priced by changing the linear rate.

/// `update_rate_per_second` is only supported on `Linear` streams; both
/// `CliffOnly` and `CliffSlope` must be rejected with `UnsupportedStreamKind`.
#[test]
fn test_update_rate_per_second_kind_branching() {
    let ctx = Ctx::setup();

    let linear_id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);
    let cliff_only_id = ctx.create_cliff_only(LINEAR_DEPOSIT);
    let cliff_slope_id = ctx.create_cliff_slope(LINEAR_RATE, STREAM_DURATION_SECS);

    // Linear: succeeds.
    assert_eq!(
        ctx.client.try_update_rate_per_second(&linear_id, &(LINEAR_RATE + 1)),
        Ok(Ok(()))
    );

    // CliffOnly: rejected.
    assert_eq!(
        ctx.client.try_update_rate_per_second(&cliff_only_id, &(LINEAR_RATE + 1)),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );

    // CliffSlope: rejected.
    assert_eq!(
        ctx.client.try_update_rate_per_second(&cliff_slope_id, &(LINEAR_RATE + 1)),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
}

/// `decrease_rate_per_second` is only supported on `Linear` streams.
#[test]
fn test_decrease_rate_per_second_kind_branching() {
    let ctx = Ctx::setup();

    let linear_id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);
    let cliff_only_id = ctx.create_cliff_only(LINEAR_DEPOSIT);
    let cliff_slope_id = ctx.create_cliff_slope(LINEAR_RATE, STREAM_DURATION_SECS);

    // Linear: succeeds (any strictly-smaller positive rate).
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&linear_id, &(LINEAR_RATE - 1)),
        Ok(Ok(()))
    );

    // CliffOnly: rejected.
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&cliff_only_id, &1),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );

    // CliffSlope: rejected.
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&cliff_slope_id, &1),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
}

/// `top_up_stream` is only supported on `Linear` streams.
#[test]
fn test_top_up_stream_kind_branching() {
    let ctx = Ctx::setup();

    let linear_id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);
    let cliff_only_id = ctx.create_cliff_only(LINEAR_DEPOSIT);
    let cliff_slope_id = ctx.create_cliff_slope(LINEAR_RATE, STREAM_DURATION_SECS);

    // Linear: succeeds.
    assert_eq!(ctx.client.try_top_up_stream(&linear_id, &ctx.sender, &1), Ok(Ok(())));

    // CliffOnly: rejected.
    assert_eq!(
        ctx.client.try_top_up_stream(&cliff_only_id, &ctx.sender, &1),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );

    // CliffSlope: rejected.
    assert_eq!(
        ctx.client.try_top_up_stream(&cliff_slope_id, &ctx.sender, &1),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
}

/// `shorten_stream_end_time` is only supported on `Linear` streams.
#[test]
fn test_shorten_stream_end_time_kind_branching() {
    let ctx = Ctx::setup();

    let linear_id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);
    let cliff_only_id = ctx.create_cliff_only(LINEAR_DEPOSIT);
    let cliff_slope_id = ctx.create_cliff_slope(LINEAR_RATE, STREAM_DURATION_SECS);

    // Linear: succeeds with a strictly-earlier end_time.
    let now = ctx.env.ledger().timestamp();
    assert_eq!(
        ctx.client.try_shorten_stream_end_time(&linear_id, &(now + STREAM_DURATION_SECS / 2)),
        Ok(Ok(()))
    );

    // CliffOnly: rejected.
    assert_eq!(
        ctx.client.try_shorten_stream_end_time(&cliff_only_id, &(now + STREAM_DURATION_SECS / 2)),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );

    // CliffSlope: rejected.
    assert_eq!(
        ctx.client.try_shorten_stream_end_time(&cliff_slope_id, &(now + STREAM_DURATION_SECS / 2)),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
}

/// `extend_stream_end_time` is only supported on `Linear` streams.
#[test]
fn test_extend_stream_end_time_kind_branching() {
    let ctx = Ctx::setup();

    let linear_id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);
    let cliff_only_id = ctx.create_cliff_only(LINEAR_DEPOSIT);
    let cliff_slope_id = ctx.create_cliff_slope(LINEAR_RATE, STREAM_DURATION_SECS);

    // Linear: succeeds with a strictly-later end_time (deposit covers extended schedule).
    let now = ctx.env.ledger().timestamp();
    assert_eq!(
        ctx.client
            .try_extend_stream_end_time(&linear_id, &(now + STREAM_DURATION_SECS * 2)),
        Ok(Ok(()))
    );

    // CliffOnly: rejected.
    assert_eq!(
        ctx.client
            .try_extend_stream_end_time(&cliff_only_id, &(now + STREAM_DURATION_SECS * 2)),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );

    // CliffSlope: rejected.
    assert_eq!(
        ctx.client
            .try_extend_stream_end_time(&cliff_slope_id, &(now + STREAM_DURATION_SECS * 2)),
        Err(Ok(ContractError::UnsupportedStreamKind))
    );
}

// ---------------------------------------------------------------------------
// Status interaction
// ---------------------------------------------------------------------------
//
// The pause gate only blocks **withdrawal** — sender-side rate changes,
// schedule mutations, top-ups, and pause/resume itself must remain
// functional while a stream is `Paused`. (Time-terminal override may flip
// status; tests use a stream that is well clear of `end_time`.)
//
// Accrual continues while paused, so a rate change made during pause must
// checkpoint the *full* accrued amount under the old rate.

/// `update_rate_per_second`, `decrease_rate_per_second`, and `top_up_stream`
/// must all succeed on a `Paused` Linear stream and checkpoint accrual
/// across the pause window.
#[test]
fn test_dynamic_pricing_paused_allows_mutations() {
    let ctx = Ctx::setup();
    let id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);

    // Advance 100s, then pause.
    ctx.env.ledger().with_mut(|l| l.timestamp += 100);
    ctx.clear_pause_cooldown();
    ctx.client
        .pause_stream(&id, &PauseReason::Operational);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Paused
    );

    // Advance another 100s while paused — accrual continues.
    ctx.env.ledger().with_mut(|l| l.timestamp += 100);

    // Pre-mutation: total accrued = 200 * 10 = 2000 (continuous through pause).
    assert_eq!(ctx.client.calculate_accrued(&id), 2000);

    // update_rate_per_second while Paused must succeed.
    assert_eq!(
        ctx.client.try_update_rate_per_second(&id, &(LINEAR_RATE + 5)),
        Ok(Ok(()))
    );

    // Decrease must succeed on a Paused stream. `check_and_bump_rate_cooldown`
    // is shared by both rate-mutation entrypoints (`MIN_RATE_INTERVAL_LEDGERS`
    // = 17 ledgers), so we must advance the ledger past the cooldown between
    // the two consecutive rate mutations.
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&id, &LINEAR_RATE),
        Ok(Ok(()))
    );

    // Top-up must succeed on a Paused stream.
    assert_eq!(ctx.client.try_top_up_stream(&id, &ctx.sender, &500), Ok(Ok(())));

    // After all three mutations the stream is still Paused (sender-side ops
    // do not flip status) and the rate is back at `LINEAR_RATE`.
    // Note: the intermediate `decrease_rate_per_second` resets
    // `deposit_amount` to `checkpointed_amount + new_rate * remaining_seconds`,
    // then the top-up adds 500 on top of that — we do not pin the exact
    // deposit here because that arithmetic is exhaustively covered by
    // `tests/rate_decrease_after_withdraw.rs`. This test only asserts that
    // the Paused-status gate does *not* block dynamic-pricing mutations.
    let after = ctx.client.get_stream_state(&id);
    assert_eq!(after.status, StreamStatus::Paused);
    assert_eq!(after.rate_per_second, LINEAR_RATE);
    // Deposit must strictly exceed the post-decrease ceiling (the top-up
    // added 500 raw units on top of it).
    assert!(
        after.deposit_amount
            >= ctx.client.calculate_accrued(&id) + LINEAR_RATE
                * (STREAM_DURATION_SECS - 200) as i128,
        "post-top-up deposit must cover the remaining schedule at the new rate"
    );
}

// ---------------------------------------------------------------------------
// Decommissioned gate
// ---------------------------------------------------------------------------
//
// When `set_stream_decommissioned(stream_id, sender, true)` is active, the
// per-entrypoint `decommissioned.unwrap_or(false)` gate must reject every
// dynamic-pricing mutation except pause/resume/cancel/withdraw (which the
// docs already cover).
//
// `shorten_stream_end_time` is intentionally NOT in the
// decommissioned-block list per the current implementation — it is a
// wind-down path similar to cancel.

/// `update_rate_per_second`, `decrease_rate_per_second`, `top_up_stream`,
/// and `extend_stream_end_time` must all return `InvalidState` while the
/// stream is `decommissioned`.
#[test]
fn test_dynamic_pricing_decommissioned_blocks_mutations() {
    let ctx = Ctx::setup();
    let id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);

    // Activate decommission mode as the sender.
    ctx.client
        .set_stream_decommissioned(&id, &ctx.sender, &true);
    assert_eq!(
        ctx.client.get_stream_state(&id).decommissioned,
        Some(true)
    );

    // All four mutation entrypoints must reject with InvalidState.
    assert_eq!(
        ctx.client.try_update_rate_per_second(&id, &(LINEAR_RATE + 1)),
        Err(Ok(ContractError::InvalidState))
    );
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&id, &(LINEAR_RATE - 1)),
        Err(Ok(ContractError::InvalidState))
    );
    assert_eq!(
        ctx.client.try_top_up_stream(&id, &ctx.sender, &1),
        Err(Ok(ContractError::InvalidState))
    );
    assert_eq!(
        ctx.client.try_extend_stream_end_time(
            &id,
            &(ctx.env.ledger().timestamp() + STREAM_DURATION_SECS * 2)
        ),
        Err(Ok(ContractError::InvalidState))
    );
}

/// Decommissioned streams remain mutable via `set_stream_decommissioned`
/// itself — the sender can toggle the flag back off. (Sender reverts are
/// independently covered by `tests/decommission.rs`; this test only checks
/// the basic reversibility gate.)
#[test]
fn test_dynamic_pricing_decommissioned_is_reversible() {
    let ctx = Ctx::setup();
    let id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);

    ctx.client
        .set_stream_decommissioned(&id, &ctx.sender, &true);
    ctx.client
        .set_stream_decommissioned(&id, &ctx.sender, &false);
    assert_eq!(
        ctx.client.get_stream_state(&id).decommissioned,
        Some(false)
    );

    // After reversal, mutations work again.
    assert_eq!(
        ctx.client.try_update_rate_per_second(&id, &(LINEAR_RATE + 1)),
        Ok(Ok(()))
    );
}

// ---------------------------------------------------------------------------
// Irrevocable flag scope
// ---------------------------------------------------------------------------
//
// `irrevocable: Some(true)` only blocks paths that **transfer value back
// to the sender** (cancel, shorten, and clearing decommission). It does
// not block rate increases, rate decreases (which refund), top-ups, or
// schedule extensions. This test pins that exact split.

/// Irrevocable streams must reject `shorten_stream_end_time` (value back
/// to sender), but allow `update_rate_per_second`, `decrease_rate_per_second`,
/// `top_up_stream`, and `extend_stream_end_time`.
#[test]
fn test_dynamic_pricing_irrevocable_blocks_shorten_only() {
    let ctx = Ctx::setup();
    let id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, Some(true));
    assert_eq!(
        ctx.client.get_stream_state(&id).irrevocable,
        Some(true)
    );

    // shorten_stream_end_time must revert with Unauthorized.
    let now = ctx.env.ledger().timestamp();
    assert_eq!(
        ctx.client.try_shorten_stream_end_time(&id, &(now + STREAM_DURATION_SECS / 2)),
        Err(Ok(ContractError::Unauthorized))
    );

    // update_rate_per_second succeeds on irrevocable streams.
    assert_eq!(
        ctx.client.try_update_rate_per_second(&id, &(LINEAR_RATE + 5)),
        Ok(Ok(()))
    );

    // decrease_rate_per_second (with refund) succeeds on irrevocable streams.
    assert_eq!(
        ctx.client.try_decrease_rate_per_second(&id, &LINEAR_RATE),
        Ok(Ok(()))
    );

    // top_up_stream succeeds on irrevocable streams.
    assert_eq!(
        ctx.client.try_top_up_stream(&id, &ctx.sender, &500),
        Ok(Ok(()))
    );

    // extend_stream_end_time succeeds on irrevocable streams (the deposit
    // already covers the extended schedule at the current rate).
    assert_eq!(
        ctx.client
            .try_extend_stream_end_time(&id, &(now + STREAM_DURATION_SECS * 2)),
        Ok(Ok(()))
    );
}

// ---------------------------------------------------------------------------
// Terminal-state guards
// ---------------------------------------------------------------------------
//
// Completed and Cancelled streams must reject every dynamic-pricing
// mutation. Most entrypoints return `StreamTerminalState` (13); the
// remainder return `InvalidState` (2). Both are acceptable as long as
// the mutation is rejected.

/// `update_rate_per_second`, `decrease_rate_per_second`, `top_up_stream`,
/// `shorten_stream_end_time`, and `extend_stream_end_time` must all be
/// rejected on a `Completed` stream.
#[test]
fn test_dynamic_pricing_completed_stream_blocks_mutations() {
    let ctx = Ctx::setup();
    // Short, fully-drainable stream.
    let id = ctx.create_linear(LINEAR_RATE, 10, None);

    // Advance past end_time and drain to reach Completed.
    ctx.env.ledger().with_mut(|l| l.timestamp += 11);
    ctx.client.withdraw(&id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Completed
    );

    let now = ctx.env.ledger().timestamp();

    // Every dynamic-pricing mutation must reject. The contract returns
    // either `StreamTerminalState` or `InvalidState` for terminal streams;
    // we accept either as a valid "blocked" signal so this test stays
    // stable across minor contract revisions.
    let rejected_errs = [
        ctx.client
            .try_update_rate_per_second(&id, &(LINEAR_RATE + 1))
            .unwrap_err()
            .unwrap(),
        ctx.client
            .try_decrease_rate_per_second(&id, &(LINEAR_RATE - 1))
            .unwrap_err()
            .unwrap(),
        ctx.client
            .try_top_up_stream(&id, &ctx.sender, &1)
            .unwrap_err()
            .unwrap(),
        ctx.client
            .try_shorten_stream_end_time(&id, &(now + 5))
            .unwrap_err()
            .unwrap(),
        ctx.client
            .try_extend_stream_end_time(&id, &(now + 100))
            .unwrap_err()
            .unwrap(),
    ];

    for err in rejected_errs {
        assert!(
            err == ContractError::StreamTerminalState || err == ContractError::InvalidState,
            "terminal stream must reject mutation; got {err:?}"
        );
    }
}

/// Same as above for a `Cancelled` stream — every dynamic-pricing mutation
/// must be rejected.
#[test]
fn test_dynamic_pricing_cancelled_stream_blocks_mutations() {
    let ctx = Ctx::setup();
    let id = ctx.create_linear(LINEAR_RATE, STREAM_DURATION_SECS, None);

    ctx.client.cancel_stream(&id);
    assert_eq!(
        ctx.client.get_stream_state(&id).status,
        StreamStatus::Cancelled
    );

    let now = ctx.env.ledger().timestamp();

    let rejected_errs = [
        ctx.client
            .try_update_rate_per_second(&id, &(LINEAR_RATE + 1))
            .unwrap_err()
            .unwrap(),
        ctx.client
            .try_decrease_rate_per_second(&id, &(LINEAR_RATE - 1))
            .unwrap_err()
            .unwrap(),
        ctx.client
            .try_top_up_stream(&id, &ctx.sender, &1)
            .unwrap_err()
            .unwrap(),
        ctx.client
            .try_shorten_stream_end_time(&id, &(now + STREAM_DURATION_SECS / 2))
            .unwrap_err()
            .unwrap(),
        ctx.client
            .try_extend_stream_end_time(&id, &(now + STREAM_DURATION_SECS * 2))
            .unwrap_err()
            .unwrap(),
    ];

    for err in rejected_errs {
        assert!(
            err == ContractError::StreamTerminalState || err == ContractError::InvalidState,
            "cancelled stream must reject mutation; got {err:?}"
        );
    }
}

// (No smoke test — every dynamic-pricing entrypoint is already exercised by
// the targeted tests above. Adding a redundant `try_*::<T>()` call here
// cannot work because the Soroban client methods are not generic.)