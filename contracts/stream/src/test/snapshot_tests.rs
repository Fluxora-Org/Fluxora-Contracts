//! Regression tests for issue #1593 — snapshot mechanism.
//!
//! Verifies that [`Harness::snapshot`] produces a correct, deterministic,
//! credential-free state capture that is safe to emit in CI output.
//!
//! # What is covered
//!
//! | Category | Tests |
//! |---|---|
//! | Correctness | Ledger fields, stream counts, balance values |
//! | Boundary | Empty contract, single stream, multiple streams |
//! | Progression | Snapshot before/after operation reflects the change |
//! | Authorization | Operations that fail auth leave snapshot unchanged |
//! | Failure context | Error paths expose pre-condition snapshot without a crash |
//! | Determinism | Two snapshots at the same instant are identical |
//! | Display | `Display` output contains every key datum as a string |
//!
//! None of the existing tests are modified; this file adds new coverage only.

use super::common::*;
use crate::{Error, StreamStatus};

// ---------------------------------------------------------------------------
// Correctness — field values match the ledger and token state
// ---------------------------------------------------------------------------

#[test]
fn snapshot_reflects_ledger_timestamp_and_sequence() {
    let h = Harness::new();
    let snap = h.snapshot();

    assert_eq!(snap.ledger_timestamp, T0, "initial timestamp must be T0");
    // Sequence number is set by Harness::new via the first `set_timestamp`
    // call; we just require it round-trips correctly.
    assert_eq!(
        snap.ledger_sequence,
        h.env.ledger().sequence(),
        "sequence must match the current ledger",
    );
}

#[test]
fn snapshot_reflects_ledger_state_after_advance() {
    let h = Harness::new();
    h.advance(7 * DAY);

    let snap = h.snapshot();
    assert_eq!(snap.ledger_timestamp, T0 + 7 * DAY);
    assert_eq!(
        snap.ledger_sequence,
        h.env.ledger().sequence(),
        "sequence must track the advance",
    );
}

#[test]
fn snapshot_stream_count_is_zero_for_fresh_harness() {
    let h = Harness::new();
    let snap = h.snapshot();

    assert_eq!(snap.stream_count, 0);
    assert!(snap.streams.is_empty());
}

#[test]
fn snapshot_balances_for_fresh_harness() {
    let h = Harness::new();
    let snap = h.snapshot();

    // Sender is minted 1_000_000 ONE in Harness::new.
    assert_eq!(snap.balance_sender, 1_000_000 * ONE);
    // Recipient has not been minted anything.
    assert_eq!(snap.balance_recipient, 0);
    // Other is also minted 1_000_000 ONE.
    assert_eq!(snap.balance_other, 1_000_000 * ONE);
    // Pool is empty until a stream is created.
    assert_eq!(snap.balance_pool, 0);
}

// ---------------------------------------------------------------------------
// Stream field correctness — single stream
// ---------------------------------------------------------------------------

#[test]
fn snapshot_captures_single_stream_fields_at_creation() {
    let h = Harness::new();
    let deposit = 500 * ONE;
    let duration = 100 * DAY;
    let id = h.create_simple(deposit, duration);

    let snap = h.snapshot();
    assert_eq!(snap.stream_count, 1);

    let ss = &snap.streams[id as usize];
    assert_eq!(ss.id, id);
    assert_eq!(ss.deposited, deposit);
    assert_eq!(ss.withdrawn, 0);
    // Snapshot taken at start_time == now, so no time has elapsed yet.
    assert_eq!(ss.vested, 0, "vested must be 0 immediately at start");
    assert_eq!(ss.withdrawable, 0);
    assert_eq!(ss.status, StreamStatus::Active);
    assert_eq!(ss.paused_at, None);
    assert_eq!(ss.paused_total, 0);
}

#[test]
fn snapshot_vested_advances_with_time() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let id = h.create_simple(deposit, 100 * DAY);

    h.advance(25 * DAY);

    let snap = h.snapshot();
    let ss = &snap.streams[id as usize];

    // 25 % of 1000 ONE = 250 ONE
    assert_eq!(ss.vested, 250 * ONE);
    assert_eq!(ss.withdrawable, 250 * ONE);
}

#[test]
fn snapshot_withdrawable_is_zero_after_full_withdrawal() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(50 * DAY);
    h.client.withdraw(&id, &None); // drain all available

    let snap = h.snapshot();
    let ss = &snap.streams[id as usize];

    assert_eq!(ss.withdrawable, 0, "nothing withdrawable after full drain");
    assert_eq!(ss.withdrawn, 500 * ONE, "withdrawn must record the drain");
}

// ---------------------------------------------------------------------------
// Boundary — multiple streams, paused stream, cancelled stream
// ---------------------------------------------------------------------------

#[test]
fn snapshot_covers_all_streams() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(200 * ONE, 50 * DAY);
    let c = h.create_simple(300 * ONE, 200 * DAY);

    let snap = h.snapshot();
    assert_eq!(snap.stream_count, 3);
    assert_eq!(snap.streams.len(), 3);

    assert_eq!(snap.streams[a as usize].deposited, 100 * ONE);
    assert_eq!(snap.streams[b as usize].deposited, 200 * ONE);
    assert_eq!(snap.streams[c as usize].deposited, 300 * ONE);
}

#[test]
fn snapshot_captures_paused_stream_state() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(10 * DAY);
    h.client.pause(&id);

    let snap = h.snapshot();
    let ss = &snap.streams[id as usize];

    assert_eq!(ss.status, StreamStatus::Paused);
    assert!(
        ss.paused_at.is_some(),
        "paused_at must be populated while paused"
    );
    assert_eq!(ss.paused_at.unwrap(), T0 + 10 * DAY);
}

#[test]
fn snapshot_captures_cancelled_stream_state() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);

    let snap = h.snapshot();
    let ss = &snap.streams[id as usize];

    assert_eq!(ss.status, StreamStatus::Cancelled);
    // After cancel, deposited is rewritten to the amount vested at cancel.
    assert_eq!(
        ss.deposited,
        300 * ONE,
        "cancelled deposited = vested at cancel"
    );
    assert_eq!(
        ss.withdrawable,
        300 * ONE,
        "full vested amount remains withdrawable"
    );
}

#[test]
fn snapshot_pool_balance_matches_sum_of_liabilities() {
    let h = Harness::new();
    h.create_simple(300 * ONE, 100 * DAY);
    h.create_simple(700 * ONE, 50 * DAY);

    h.advance(10 * DAY);

    let snap = h.snapshot();
    let liability_sum: i128 = snap
        .streams
        .iter()
        .map(|ss| ss.deposited - ss.withdrawn)
        .sum();
    assert_eq!(
        snap.balance_pool, liability_sum,
        "pool balance must equal sum of (deposited - withdrawn)",
    );
}

// ---------------------------------------------------------------------------
// Progression — snapshot before vs after an operation
// ---------------------------------------------------------------------------

#[test]
fn snapshot_before_and_after_create_reflects_transfer() {
    let h = Harness::new();
    let before = h.snapshot();

    let deposit = 200 * ONE;
    h.create_simple(deposit, 100 * DAY);
    let after = h.snapshot();

    assert_eq!(after.stream_count, before.stream_count + 1);
    assert_eq!(after.balance_sender, before.balance_sender - deposit);
    assert_eq!(after.balance_pool, before.balance_pool + deposit);
    // Recipient and other are unaffected.
    assert_eq!(after.balance_recipient, before.balance_recipient);
    assert_eq!(after.balance_other, before.balance_other);
}

#[test]
fn snapshot_before_and_after_withdraw_shows_movement() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(40 * DAY);

    let before = h.snapshot();
    h.client.withdraw(&id, &None);
    let after = h.snapshot();

    let withdrawn = 400 * ONE;
    assert_eq!(
        after.balance_recipient,
        before.balance_recipient + withdrawn
    );
    assert_eq!(after.balance_pool, before.balance_pool - withdrawn);
    assert_eq!(after.streams[id as usize].withdrawn, withdrawn);
    assert_eq!(after.streams[id as usize].withdrawable, 0);
}

#[test]
fn snapshot_before_and_after_cancel_shows_refund() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let id = h.create_simple(deposit, 100 * DAY);
    h.advance(20 * DAY);

    let before = h.snapshot();
    h.client.cancel(&id);
    let after = h.snapshot();

    let vested_at_cancel = 200 * ONE;
    let refund = deposit - vested_at_cancel;

    assert_eq!(after.balance_sender, before.balance_sender + refund);
    assert_eq!(after.balance_pool, before.balance_pool - refund);
    assert_eq!(after.streams[id as usize].status, StreamStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Authorization — failed auth must leave snapshot unchanged
// ---------------------------------------------------------------------------

/// When `withdraw` is rejected for lack of authorization, the ledger and token
/// state must be identical to the pre-call snapshot (no partial state change).
#[test]
fn snapshot_unchanged_after_unauthorized_withdraw() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let before = h.snapshot();

    // Revoke all mocked authorization so the call is rejected.
    h.env.mock_auths(&[]);
    // The SDK aborts (panics) rather than returning a typed error when auth
    // is missing entirely; `try_*` surfaces this as `Err(Abort)`.  Either
    // outcome confirms the call was rejected.
    let rejected = h.client.try_withdraw(&id, &None).is_err();
    assert!(
        rejected,
        "withdraw must be rejected without auth; pre-state:\n{before}"
    );

    // Re-enable auths so we can query state.
    h.env.mock_all_auths();
    let after = h.snapshot();

    assert_eq!(
        before, after,
        "snapshot must not change after an unauthorized call\nbefore={before}\nafter={after}",
    );
}

/// When `cancel` is rejected for lack of authorization, nothing changes.
#[test]
fn snapshot_unchanged_after_unauthorized_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);

    let before = h.snapshot();

    h.env.mock_auths(&[]);
    let rejected = h.client.try_cancel(&id).is_err();
    assert!(
        rejected,
        "cancel must be rejected without auth; pre-state:\n{before}"
    );

    h.env.mock_all_auths();
    let after = h.snapshot();

    assert_eq!(
        before, after,
        "snapshot must not change after an unauthorized cancel\nbefore={before}\nafter={after}",
    );
}

// ---------------------------------------------------------------------------
// Failure context — snapshot in assertion messages on error paths
// ---------------------------------------------------------------------------

/// A stream that does not exist must return StreamNotFound; the snapshot
/// captures the empty state that explains the failure.
#[test]
fn snapshot_provides_context_on_stream_not_found() {
    let h = Harness::new();
    let snap = h.snapshot();

    let err = h.client.try_get_stream(&999).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::StreamNotFound,
        "expected StreamNotFound; state at failure:\n{snap}",
    );
}

/// Withdraw on a stream before the cliff is rejected; the snapshot explains
/// the timing state at the point of failure.
#[test]
fn snapshot_provides_context_on_premature_withdraw_before_cliff() {
    let h = Harness::new();
    let start = h.now();
    let cliff = start + 30 * DAY;
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        cliff,
        true,
        true,
        true,
    );

    h.advance(15 * DAY); // still before cliff

    let snap = h.snapshot();
    let err = h.client.try_withdraw(&id, &Some(1)).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::NothingToWithdraw,
        "expected NothingToWithdraw before cliff; state at failure:\n{snap}",
    );
}

// ---------------------------------------------------------------------------
// Determinism — two snapshots at the same instant must be equal
// ---------------------------------------------------------------------------

#[test]
fn snapshot_is_deterministic_at_fixed_instant() {
    let h = Harness::new();
    h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(20 * DAY);

    let snap_a = h.snapshot();
    let snap_b = h.snapshot();

    assert_eq!(
        snap_a, snap_b,
        "two snapshots at the same instant must be identical",
    );
}

/// Snapshots taken after advancing time must reflect the new timestamp and
/// the updated vested amounts, confirming the snapshot is not cached.
#[test]
fn snapshot_is_not_cached_across_time_advances() {
    let h = Harness::new();
    h.create_simple(1_000 * ONE, 100 * DAY);

    let snap_before = h.snapshot();
    h.advance(10 * DAY);
    let snap_after = h.snapshot();

    assert_ne!(snap_before.ledger_timestamp, snap_after.ledger_timestamp);
    assert_ne!(
        snap_before.streams[0].vested, snap_after.streams[0].vested,
        "vested must increase after time advance",
    );
}

// ---------------------------------------------------------------------------
// Display — output contains every key datum
// ---------------------------------------------------------------------------

#[test]
fn display_output_contains_ledger_timestamp() {
    let h = Harness::new();
    let snap = h.snapshot();
    let output = std::format!("{snap}");

    assert!(
        output.contains(&std::format!("{}", snap.ledger_timestamp)),
        "Display output must include ledger_timestamp\n{output}",
    );
}

#[test]
fn display_output_contains_all_balance_fields() {
    let h = Harness::new();
    h.create_simple(100 * ONE, 100 * DAY);
    let snap = h.snapshot();
    let output = std::format!("{snap}");

    // All four balance labels must appear.
    for label in &["balances:", "sender=", "recipient=", "other=", "pool="] {
        assert!(
            output.contains(label),
            "Display must contain '{label}'\n{output}",
        );
    }
}

#[test]
fn display_output_contains_stream_status() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.pause(&id);

    let snap = h.snapshot();
    let output = std::format!("{snap}");

    assert!(
        output.contains("Paused"),
        "Display must show Paused status\n{output}",
    );
}

#[test]
fn dump_snapshot_emits_to_stderr_without_panicking() {
    // This test just checks that dump_snapshot does not panic or error.
    // The actual stderr output is visible with `-- --nocapture`.
    let h = Harness::new();
    h.create_simple(500 * ONE, 50 * DAY);
    h.advance(25 * DAY);
    h.dump_snapshot(); // must not panic
}
