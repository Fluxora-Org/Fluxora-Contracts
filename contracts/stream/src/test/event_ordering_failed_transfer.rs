//! Invariant: no success event is emitted or persisted when a token transfer
//! fails and the operation reverts.
//!
//! # The invariant
//!
//! **Every state-changing entry point either completes atomically — emitting
//! exactly one success event — or reverts completely, emitting zero events.**
//! There is no partial state: the Soroban host discards every storage write and
//! every event published during an invocation if that invocation does not return
//! `Ok(…)`. A failed token transfer inside `withdraw`, `cancel`, or `top_up`
//! is one such revert trigger.
//!
//! Formally:
//!
//! ```text
//! ∀ op ∈ {withdraw, cancel, top_up, batch_withdraw, delegate_withdraw,
//!          delegate_cancel, delegate_top_up}:
//!
//!   token_transfer_fails(op) ⟹ events_emitted_by_stream_contract(op) = ∅
//! ```
//!
//! # Why this matters
//!
//! Events are the primary indexer signal: a `Withdrawn`, `Cancelled` or
//! `ToppedUp` event is how an off-chain system learns that a stream's
//! accounting changed. If a success event could survive a reverting operation
//! the indexer would credit the wrong amount to a recipient or mark a stream
//! cancelled when the cancellation never settled, causing silent discrepancies
//! that do not surface until a reconciliation job runs.
//!
//! # Mechanism
//!
//! Soroban implements the guarantee at the host level: every storage write and
//! every event published during a contract invocation are held in a buffer. If
//! the invocation returns `Err` (or panics), the host discards the buffer
//! without committing it. From the caller's perspective the transaction either
//! settled completely or did not happen at all — there is no intermediate state.
//!
//! This module proves the guarantee is honoured in the three mutating paths
//! that end with a token transfer:
//!
//! | entry point | success event | transfer direction |
//! |---|---|---|
//! | `withdraw` / `batch_withdraw` | `Withdrawn` | pool → recipient |
//! | `cancel` | `Cancelled` | pool → sender (refund) |
//! | `top_up` | `ToppedUp` | sender → pool (deposit pull) |
//!
//! For each path the test:
//! 1. Creates a stream with a real SAC token whose pool can be surgically
//!    drained or whose sender balance can be clawed back.
//! 2. Engineers the transfer failure (clawback on the pool for outbound
//!    transfers, clawback on the sender for deposit pulls).
//! 3. Calls the entry point via `try_*` and asserts the error is
//!    `TokenTransferFailed`.
//! 4. Reads `env.events().all()` and asserts **zero** stream-contract events
//!    were published — neither a success event nor any other kind.
//! 5. Reads stream state and token balances and asserts they are unchanged.
//!
//! The `PanicToken` path in [`super::token_errors`] covers the case where the
//! token raises a host trap (`InvokeError::Abort`) rather than a typed contract
//! error. That surfaces as `TokenTransferFailed` in the test host for the same
//! reason (`TokenMissing` is a real-WASM-only variant), and the atomicity
//! guarantee is identical — but the mechanism of failure is different, so both
//! are exercised to give confidence that the invariant holds regardless of how
//! the token fails.
//!
//! # Relationship to `test::withdrawal_atomicity`
//!
//! `test::withdrawal_atomicity` uses `std::panic::catch_unwind` to handle the
//! raw Rust panic that a `SAC.set_authorized` deauthorization produces in the
//! test host, and asserts that stream-storage fields are byte-for-byte
//! unchanged after the panic. That module proves *storage* atomicity.
//!
//! This module is complementary: it uses the `try_*` client methods (which
//! return `Result` instead of panicking) against a clawback-enabled SAC, and
//! asserts **event** atomicity — specifically that the event buffer is also
//! discarded. The two modules together cover every observable side effect.

#![cfg(test)]

extern crate std;

use soroban_sdk::testutils::{Address as _, Events as _, IssuerFlags};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, Env, Symbol, TryFromVal, TryIntoVal};

use super::common::{Harness, DAY, ONE};
use crate::{Error, StreamStatus};

// ---------------------------------------------------------------------------
// Shared setup helpers
// ---------------------------------------------------------------------------

/// Create a fresh clawback-enabled SAC so tests can drain either the pool or
/// a specific address out-of-band.
fn make_clawback_token<'a>(env: &'a Env) -> (Address, TokenClient<'a>, StellarAssetClient<'a>) {
    let admin = Address::generate(env);
    let asset = env.register_stellar_asset_contract_v2(admin);
    asset.issuer().set_flag(IssuerFlags::ClawbackEnabledFlag);
    let token = asset.address();
    (
        token.clone(),
        TokenClient::new(env, &token),
        StellarAssetClient::new(env, &token),
    )
}

/// Assert that the stream-contract published **zero** events during the most
/// recent invocation.
///
/// `env.events().all()` retains only the most recent invocation, so this must
/// be called immediately after the failing entry-point call.
///
/// Filtering by `contract_id` drops token-contract events (e.g. a `transfer`
/// event the SAC might have emitted before the clawback reverted everything).
fn assert_no_stream_events(h: &Harness, label: &str) {
    let stream_events: std::vec::Vec<_> = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();

    assert!(
        stream_events.is_empty(),
        "{label}: expected zero stream-contract events after a reverting operation, \
         but {n} event(s) survived:\n{stream_events:#?}",
        n = stream_events.len(),
    );
}

// ---------------------------------------------------------------------------
// Invariant: `withdraw` — no `Withdrawn` event on pool drain
// ---------------------------------------------------------------------------

/// When the pool is drained before `withdraw` is called, the token transfer
/// fails and the operation reverts. The `Withdrawn` event must not appear.
///
/// This is the outbound-transfer failure path: `apply_withdrawal` saves the
/// incremented `withdrawn` counter to storage first (checks-effects-interactions
/// ordering), then calls the token; the host rolls back that storage write
/// along with the event buffer when the token returns an error.
#[test]
fn withdraw_token_failure_emits_no_withdrawn_event() {
    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.env.ledger().timestamp();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    h.advance(30 * DAY);

    assert!(
        h.client.withdrawable_of(&id) > 0,
        "pre-condition: something to withdraw"
    );

    // Drain the pool so the outbound transfer will fail.
    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);
    assert_eq!(tc.balance(&contract_id), 0, "pool is empty");

    // Snapshot stream state before the failing call.
    let stream_before = h.client.get_stream(&id);
    let recipient_balance_before = tc.balance(&h.recipient);

    // The call must fail with TokenTransferFailed.
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::TokenTransferFailed,
        "withdraw with empty pool must return TokenTransferFailed",
    );

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(&h, "withdraw_token_failure_emits_no_withdrawn_event");

    // ── State is unchanged ─────────────────────────────────────────────────
    let stream_after = h.client.get_stream(&id);
    assert_eq!(
        stream_after.withdrawn, stream_before.withdrawn,
        "withdrawn counter must not be incremented after a reverted withdrawal",
    );
    assert_eq!(
        stream_after.status, stream_before.status,
        "stream status must not change after a reverted withdrawal",
    );
    assert_eq!(
        tc.balance(&h.recipient),
        recipient_balance_before,
        "recipient balance must not change after a reverted withdrawal",
    );
}

/// Same invariant for `batch_withdraw`. The batch is all-or-nothing — a token
/// failure on any element reverts the whole call, including payouts already
/// computed and partially applied to earlier elements.
#[test]
fn batch_withdraw_token_failure_emits_no_withdrawn_event() {
    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(2_000 * ONE));
    let start = h.env.ledger().timestamp();
    let a = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    let b = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    h.advance(50 * DAY);

    assert!(
        h.client.withdrawable_of(&a) > 0,
        "pre-condition: stream A has balance"
    );
    assert!(
        h.client.withdrawable_of(&b) > 0,
        "pre-condition: stream B has balance"
    );

    // Drain the pool entirely.
    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);

    let stream_a_before = h.client.get_stream(&a);
    let stream_b_before = h.client.get_stream(&b);
    let recipient_balance_before = tc.balance(&h.recipient);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[a, b]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(&h, "batch_withdraw_token_failure_emits_no_withdrawn_event");

    // ── State for both streams unchanged ───────────────────────────────────
    let stream_a_after = h.client.get_stream(&a);
    let stream_b_after = h.client.get_stream(&b);
    assert_eq!(
        stream_a_after.withdrawn, stream_a_before.withdrawn,
        "stream A withdrawn unchanged"
    );
    assert_eq!(
        stream_a_after.status, stream_a_before.status,
        "stream A status unchanged"
    );
    assert_eq!(
        stream_b_after.withdrawn, stream_b_before.withdrawn,
        "stream B withdrawn unchanged"
    );
    assert_eq!(
        stream_b_after.status, stream_b_before.status,
        "stream B status unchanged"
    );
    assert_eq!(
        tc.balance(&h.recipient),
        recipient_balance_before,
        "recipient balance unchanged after reverted batch_withdraw",
    );
}

// ---------------------------------------------------------------------------
// Invariant: `cancel` — no `Cancelled` event when refund transfer fails
// ---------------------------------------------------------------------------

/// When the pool is drained before `cancel`, the refund transfer fails and the
/// operation reverts. The `Cancelled` event must not appear, and the stream
/// must still be `Active` — the cancel did not take effect.
///
/// This is critical: an indexer that saw a spurious `Cancelled` event would
/// mark the stream settled, remove it from active monitoring, and the recipient
/// would have no way to learn that their future withdrawals are still owed.
#[test]
fn cancel_token_failure_emits_no_cancelled_event() {
    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.env.ledger().timestamp();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    h.advance(30 * DAY);

    // Confirm there is a non-zero refund outstanding so the token transfer
    // code path is actually reached.
    let refundable = h.client.refundable_of(&id);
    assert!(
        refundable > 0,
        "pre-condition: non-zero refund to trigger transfer"
    );

    // Drain the pool so the refund transfer will fail.
    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);

    let stream_before = h.client.get_stream(&id);
    let sender_balance_before = tc.balance(&h.sender);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::TokenTransferFailed,
        "cancel with empty pool must return TokenTransferFailed",
    );

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(&h, "cancel_token_failure_emits_no_cancelled_event");

    // ── State unchanged: still Active, not Cancelled ───────────────────────
    let stream_after = h.client.get_stream(&id);
    assert_eq!(
        stream_after.status,
        StreamStatus::Active,
        "stream must remain Active after a reverted cancel",
    );
    assert_eq!(
        stream_after.deposited, stream_before.deposited,
        "deposited must not be rewritten after a reverted cancel",
    );
    assert_eq!(
        stream_after.end_time, stream_before.end_time,
        "end_time must not be collapsed after a reverted cancel",
    );
    assert_eq!(
        tc.balance(&h.sender),
        sender_balance_before,
        "sender must not receive any refund after a reverted cancel",
    );
}

/// When `cancel` is called on a fully-vested stream the refund is zero, so no
/// token transfer is made. This case must succeed (not fail), and it must emit
/// exactly one `Cancelled` event. This is the complementary "positive path"
/// check that proves the zero-transfer guard works correctly — a token that
/// panics on zero-value transfers (the `ZeroGuardToken` from `token_errors.rs`)
/// is not needed here because the standard SAC simply succeeds the cancel
/// without making any transfer call.
///
/// The key assertion is that the `Cancelled` event *does* appear here — i.e.
/// we are not accidentally suppressing success events.
#[test]
fn cancel_with_zero_refund_emits_exactly_one_cancelled_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(100 * DAY); // fully vested; refundable == 0

    assert_eq!(
        h.client.refundable_of(&id),
        0,
        "pre-condition: nothing to refund"
    );

    h.client.cancel(&id);

    // Exactly one stream event, and it is `cancelled`.
    let stream_events: std::vec::Vec<_> = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();

    assert_eq!(
        stream_events.len(),
        1,
        "cancel with zero refund must emit exactly one event (the Cancelled event)",
    );

    // Verify the event name is `cancelled`.
    let soroban_sdk::xdr::ContractEventBody::V0(ref body) = stream_events[0].body;
    let mut topics = soroban_sdk::vec![&h.env];
    for t in body.topics.iter() {
        topics.push_back(soroban_sdk::Val::try_from_val(&h.env, t).unwrap());
    }
    let event_name: Symbol = topics.get(0).unwrap().try_into_val(&h.env).unwrap();
    assert_eq!(
        event_name,
        Symbol::new(&h.env, "cancelled"),
        "the single event must be the `cancelled` success event",
    );
}

// ---------------------------------------------------------------------------
// Invariant: `cancel` — no event when pool is drained mid-stream (paused)
// ---------------------------------------------------------------------------

/// Same invariant for a paused stream: the cancel settles against the frozen
/// clock, but the refund transfer still fails when the pool is empty. No event.
#[test]
fn cancel_while_paused_token_failure_emits_no_event() {
    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.env.ledger().timestamp();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    h.advance(30 * DAY);
    h.client.pause(&id);
    h.advance(20 * DAY);

    // Refundable based on the frozen clock (30 days of accrual = 30%, so
    // 70% is still refundable).
    let refundable = h.client.refundable_of(&id);
    assert!(refundable > 0, "pre-condition: non-zero refund when paused");

    // Drain the pool.
    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);

    let stream_before = h.client.get_stream(&id);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(&h, "cancel_while_paused_token_failure_emits_no_event");

    // ── Stream unchanged: still Paused, not Cancelled ──────────────────────
    let stream_after = h.client.get_stream(&id);
    assert_eq!(
        stream_after.status,
        StreamStatus::Paused,
        "must remain Paused"
    );
    assert_eq!(
        stream_after.deposited, stream_before.deposited,
        "deposited unchanged"
    );
    assert_eq!(
        stream_after.end_time, stream_before.end_time,
        "end_time unchanged"
    );
}

// ---------------------------------------------------------------------------
// Invariant: `top_up` — no `ToppedUp` event when deposit pull fails
// ---------------------------------------------------------------------------

/// When the sender has insufficient balance the deposit pull fails and the
/// operation reverts. The `ToppedUp` event must not appear, and the stream's
/// `deposited` and `end_time` must be unchanged.
///
/// Unlike `withdraw` and `cancel`, `top_up` pulls tokens *into* the pool
/// (inbound transfer). The state is also written before the token call
/// (schedule arithmetic happens first), so the rollback covers both the storage
/// write and the event.
#[test]
fn top_up_token_failure_emits_no_topped_up_event() {
    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);

    admin.mint(&h.sender, &(2_000 * ONE));
    let start = h.env.ledger().timestamp();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    h.advance(10 * DAY);

    let stream_before = h.client.get_stream(&id);

    // Drain the sender's remaining balance so the deposit pull will fail.
    let remaining = tc.balance(&h.sender);
    if remaining > 0 {
        admin.clawback(&h.sender, &remaining);
    }
    assert_eq!(tc.balance(&h.sender), 0, "sender has no balance");

    let err = h.client.try_top_up(&id, &(200 * ONE)).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::TokenTransferFailed,
        "top_up with empty sender balance must return TokenTransferFailed",
    );

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(&h, "top_up_token_failure_emits_no_topped_up_event");

    // ── Stream state unchanged ─────────────────────────────────────────────
    let stream_after = h.client.get_stream(&id);
    assert_eq!(
        stream_after.deposited, stream_before.deposited,
        "deposited must not change after a reverted top_up",
    );
    assert_eq!(
        stream_after.end_time, stream_before.end_time,
        "end_time must not extend after a reverted top_up",
    );
}

/// Same invariant when `top_up` is called on a paused stream and the sender
/// has no balance. Pausing does not affect the revert guarantee.
#[test]
fn top_up_while_paused_token_failure_emits_no_event() {
    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);

    admin.mint(&h.sender, &(2_000 * ONE));
    let start = h.env.ledger().timestamp();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    h.advance(25 * DAY);
    h.client.pause(&id);

    let stream_before = h.client.get_stream(&id);

    // Drain sender balance.
    let remaining = tc.balance(&h.sender);
    if remaining > 0 {
        admin.clawback(&h.sender, &remaining);
    }

    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(&h, "top_up_while_paused_token_failure_emits_no_event");

    // ── State: still Paused, deposited and end_time unchanged ──────────────
    let stream_after = h.client.get_stream(&id);
    assert_eq!(stream_after.status, StreamStatus::Paused, "still Paused");
    assert_eq!(
        stream_after.deposited, stream_before.deposited,
        "deposited unchanged"
    );
    assert_eq!(
        stream_after.end_time, stream_before.end_time,
        "end_time unchanged"
    );
}

// ---------------------------------------------------------------------------
// Invariant: delegate paths — no event on token failure
// ---------------------------------------------------------------------------

/// `delegate_withdraw` goes through `apply_withdrawal` and therefore the same
/// rollback guarantee applies: a failed token transfer emits no `Withdrawn`
/// event.
#[test]
fn delegate_withdraw_token_failure_emits_no_withdrawn_event() {
    use crate::op;

    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.env.ledger().timestamp();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );

    // Grant WITHDRAW permission to `other`.
    h.client
        .grant_delegate(&id, &h.recipient, &h.other, &op::WITHDRAW, &None);

    h.advance(40 * DAY);
    assert!(h.client.withdrawable_of(&id) > 0, "pre-condition");

    // Drain the pool.
    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);

    let stream_before = h.client.get_stream(&id);

    let err = h
        .client
        .try_delegate_withdraw(&id, &h.other, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(
        &h,
        "delegate_withdraw_token_failure_emits_no_withdrawn_event",
    );

    let stream_after = h.client.get_stream(&id);
    assert_eq!(
        stream_after.withdrawn, stream_before.withdrawn,
        "withdrawn unchanged"
    );
    assert_eq!(
        stream_after.status, stream_before.status,
        "status unchanged"
    );
    assert_eq!(tc.balance(&h.recipient), 0, "recipient balance unchanged");
}

/// `delegate_cancel` goes through the same cancel logic. A failed refund
/// transfer must emit no `Cancelled` event.
#[test]
fn delegate_cancel_token_failure_emits_no_cancelled_event() {
    use crate::op;

    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.env.ledger().timestamp();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );

    // Grant CANCEL to `other`.
    h.client
        .grant_delegate(&id, &h.sender, &h.other, &op::CANCEL, &None);

    h.advance(25 * DAY);
    assert!(h.client.refundable_of(&id) > 0, "pre-condition");

    // Drain the pool.
    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);

    let stream_before = h.client.get_stream(&id);

    let err = h
        .client
        .try_delegate_cancel(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(&h, "delegate_cancel_token_failure_emits_no_cancelled_event");

    let stream_after = h.client.get_stream(&id);
    assert_eq!(stream_after.status, StreamStatus::Active, "still Active");
    assert_eq!(
        stream_after.deposited, stream_before.deposited,
        "deposited unchanged"
    );
    assert_eq!(
        stream_after.end_time, stream_before.end_time,
        "end_time unchanged"
    );
}

/// `delegate_top_up` pulls from the sender's balance. A failed pull must emit
/// no `ToppedUp` event.
#[test]
fn delegate_top_up_token_failure_emits_no_topped_up_event() {
    use crate::op;

    let h = Harness::new();
    let env = &h.env;
    let (token, tc, admin) = make_clawback_token(env);

    admin.mint(&h.sender, &(2_000 * ONE));
    let start = h.env.ledger().timestamp();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );

    // Grant TOP_UP to `other`.
    h.client
        .grant_delegate(&id, &h.sender, &h.other, &op::TOP_UP, &None);

    h.advance(10 * DAY);
    let stream_before = h.client.get_stream(&id);

    // Drain the sender's balance.
    let remaining = tc.balance(&h.sender);
    if remaining > 0 {
        admin.clawback(&h.sender, &remaining);
    }

    let err = h
        .client
        .try_delegate_top_up(&id, &h.other, &(200 * ONE))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // ── Invariant: zero stream events ──────────────────────────────────────
    assert_no_stream_events(&h, "delegate_top_up_token_failure_emits_no_topped_up_event");

    let stream_after = h.client.get_stream(&id);
    assert_eq!(
        stream_after.deposited, stream_before.deposited,
        "deposited unchanged"
    );
    assert_eq!(
        stream_after.end_time, stream_before.end_time,
        "end_time unchanged"
    );
}

// ---------------------------------------------------------------------------
// Positive path: verify success events DO fire on non-reverting operations
// ---------------------------------------------------------------------------
//
// These smoke-check that the `assert_no_stream_events` helper itself does not
// produce false-passes by confirming that a *successful* withdraw, cancel, and
// top_up each emit exactly their expected success event. If this section fails
// it means the token plumbing in the tests above is broken, not the invariant.

/// A successful `withdraw` emits exactly one `Withdrawn` event.
#[test]
fn successful_withdraw_emits_exactly_one_withdrawn_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    h.client.withdraw(&id, &None);

    let stream_events: std::vec::Vec<_> = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();

    assert_eq!(
        stream_events.len(),
        1,
        "exactly one stream event on success"
    );
    let soroban_sdk::xdr::ContractEventBody::V0(ref body) = stream_events[0].body;
    let mut topics = soroban_sdk::vec![&h.env];
    for t in body.topics.iter() {
        topics.push_back(soroban_sdk::Val::try_from_val(&h.env, t).unwrap());
    }
    let name: Symbol = topics.get(0).unwrap().try_into_val(&h.env).unwrap();
    assert_eq!(
        name,
        Symbol::new(&h.env, "withdrawn"),
        "event must be `withdrawn`"
    );
}

/// A successful `cancel` emits exactly one `Cancelled` event.
#[test]
fn successful_cancel_emits_exactly_one_cancelled_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    h.client.cancel(&id);

    let stream_events: std::vec::Vec<_> = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();

    assert_eq!(
        stream_events.len(),
        1,
        "exactly one stream event on success"
    );
    let soroban_sdk::xdr::ContractEventBody::V0(ref body) = stream_events[0].body;
    let mut topics = soroban_sdk::vec![&h.env];
    for t in body.topics.iter() {
        topics.push_back(soroban_sdk::Val::try_from_val(&h.env, t).unwrap());
    }
    let name: Symbol = topics.get(0).unwrap().try_into_val(&h.env).unwrap();
    assert_eq!(
        name,
        Symbol::new(&h.env, "cancelled"),
        "event must be `cancelled`"
    );
}

/// A successful `top_up` emits exactly one `ToppedUp` event.
#[test]
fn successful_top_up_emits_exactly_one_topped_up_event() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.top_up(&id, &(100 * ONE));

    let stream_events: std::vec::Vec<_> = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();

    assert_eq!(
        stream_events.len(),
        1,
        "exactly one stream event on success"
    );
    let soroban_sdk::xdr::ContractEventBody::V0(ref body) = stream_events[0].body;
    let mut topics = soroban_sdk::vec![&h.env];
    for t in body.topics.iter() {
        topics.push_back(soroban_sdk::Val::try_from_val(&h.env, t).unwrap());
    }
    let name: Symbol = topics.get(0).unwrap().try_into_val(&h.env).unwrap();
    assert_eq!(
        name,
        Symbol::new(&h.env, "topped_up"),
        "event must be `topped_up`"
    );
}
