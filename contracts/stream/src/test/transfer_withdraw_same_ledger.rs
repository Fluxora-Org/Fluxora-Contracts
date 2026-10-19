//! Issue #1835 — a recipient transfer in the same ledger as a withdrawal.
//!
//! `withdraw` settles against the recipient it reads out of storage at the
//! instant it runs, and `transfer_recipient` rewrites only the recipient slot,
//! leaving the accrual schedule, `deposited`, `withdrawn` and `status` alone.
//! That makes ordering inside one ledger observable: whichever call runs first
//! decides whether the payout lands with the old or the new recipient, and a
//! withdrawal never splits *across* two recipients.
//!
//! This module pins that outcome for both orders, end to end through the public
//! ABI, and asserts the properties that make it safe to reason about:
//!
//! * each payout settles to exactly one recipient — no double payment, no
//!   tokens stranded between the old and the new payee;
//! * the emitted events match storage and the token ledger;
//! * the balance sheet conserves the deposit (harness `assert_pool_exact`);
//! * the result is deterministic across two independent runs;
//! * a same-ledger transfer leaves later accrual untouched.
//!
//! `docs/ABI.md` already states the rule — "any balance the old recipient had
//! already accrued but not withdrawn moves with the stream to the new
//! recipient" — so the tests below assert the code's actual behaviour rather
//! than correcting the document.

use soroban_sdk::testutils::Events as _;
use soroban_sdk::{Address, Event, Map, Symbol, TryFromVal, Val};

use super::common::*;
use crate::events::{RecipientTransferred, Withdrawn};
use crate::StreamStatus;

use std::string::{String, ToString};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The events the *stream* contract published during the last invocation.
///
/// `Events::all()` only reports the most recent contract invocation, so this
/// must be called immediately after the operation under test — before any other
/// client call (including a read-only view) replaces the snapshot.
fn published_by_stream(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// Require that an already-captured event buffer holds exactly `expected`,
/// byte-for-byte.
///
/// The buffer must be captured immediately after the invocation under test:
/// any later contract call, including a token `balance` read, replaces the test
/// host's "most recent invocation" snapshot.
fn assert_published_only<E: Event>(
    h: &Harness,
    published: &[soroban_sdk::xdr::ContractEvent],
    expected: E,
    what: &str,
) {
    assert_eq!(
        published.to_vec(),
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "{what}: the stream contract must publish exactly this event",
    );
}

/// Reduce an event `Val` to a description that is stable across harnesses:
/// every address is replaced by the role it plays in the scenario, so two runs
/// with independently generated addresses can still be compared exactly.
fn norm_val(h: &Harness, v: &Val) -> String {
    if let Ok(addr) = Address::try_from_val(&h.env, v) {
        if addr == h.sender {
            return "role:sender".to_string();
        }
        if addr == h.recipient {
            return "role:old_recipient".to_string();
        }
        if addr == h.other {
            return "role:new_recipient".to_string();
        }
        return "role:unexpected".to_string();
    }
    if let Ok(n) = u64::try_from_val(&h.env, v) {
        return std::format!("u64:{n}");
    }
    if let Ok(n) = u32::try_from_val(&h.env, v) {
        return std::format!("u32:{n}");
    }
    if let Ok(n) = i128::try_from_val(&h.env, v) {
        return std::format!("i128:{n}");
    }
    if let Ok(s) = Symbol::try_from_val(&h.env, v) {
        return std::format!("sym:{}", s.to_string());
    }
    if let Ok(m) = Map::<Symbol, Val>::try_from_val(&h.env, v) {
        let mut fields: std::vec::Vec<String> = m
            .iter()
            .map(|(k, val)| std::format!("{}={}", k.to_string(), norm_val(h, &val)))
            .collect();
        fields.sort();
        return std::format!("map{{{}}}", fields.join(","));
    }
    "val:unhandled".to_string()
}

/// A stable transcript of the stream events published during the last
/// invocation, with addresses replaced by scenario roles.
fn norm_events(h: &Harness) -> std::vec::Vec<String> {
    published_by_stream(h)
        .into_iter()
        .map(|event| {
            let soroban_sdk::xdr::ContractEventBody::V0(body) = event.body;
            let mut parts: std::vec::Vec<String> = std::vec::Vec::new();
            for t in body.topics.iter() {
                let v = Val::try_from_val(&h.env, t).unwrap();
                parts.push(norm_val(h, &v));
            }
            let data = Val::try_from_val(&h.env, &body.data).unwrap();
            parts.push(norm_val(h, &data));
            parts.join("|")
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Withdraw, then transfer — the split settles to the old recipient only.
// ---------------------------------------------------------------------------

#[test]
fn withdraw_then_transfer_in_the_same_ledger_splits_exactly_once() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let id = h.create_simple(deposit, 100 * DAY);

    let old_before = h.balance(&h.recipient);
    let new_before = h.balance(&h.other);

    h.advance(40 * DAY); // 400 ONE vested, 400 ONE withdrawable

    // Same ledger, order 1: the withdrawal settles first...
    let payout = h.client.withdraw(&id, &Some(150 * ONE));
    let withdraw_events = published_by_stream(&h);
    let after_withdraw = h.get(id);

    // ...and the transfer follows with no clock movement between the calls.
    h.client.transfer_recipient(&id, &h.other);
    let transfer_events = published_by_stream(&h);

    // The withdrawal paid the old recipient and nobody else.
    assert_eq!(payout, 150 * ONE, "the withdrawal settles 150 ONE");
    assert_eq!(
        h.balance(&h.recipient) - old_before,
        150 * ONE,
        "the old recipient receives exactly the withdrawn amount",
    );
    assert_eq!(
        h.balance(&h.other),
        new_before,
        "the transfer itself moves no tokens",
    );
    assert_eq!(h.pool(), 850 * ONE, "the unwithdrawn claim stays pooled");

    // Emitted events: one `withdrawn` from the first call, one
    // `recipient_transferred` from the second, each pinned to ground truth.
    let expected_withdrawn = Withdrawn {
        stream_id: id,
        recipient: h.recipient.clone(),
        amount: payout,
        withdrawn: after_withdraw.withdrawn,
        deposited: after_withdraw.deposited,
        status: after_withdraw.status,
    };
    assert_eq!(
        withdraw_events,
        std::vec![expected_withdrawn.to_xdr(&h.env, &h.contract_id)],
        "the Withdrawn event must name the old recipient and match storage",
    );
    assert_published_only(
        &h,
        &transfer_events,
        RecipientTransferred {
            stream_id: id,
            old_recipient: h.recipient.clone(),
            new_recipient: h.other.clone(),
        },
        "withdraw then transfer",
    );

    // Storage: the transfer rewrote only the recipient slot.
    let settled = h.get(id);
    assert_eq!(settled.recipient, h.other);
    assert_eq!(settled.deposited, deposit);
    assert_eq!(settled.withdrawn, 150 * ONE);
    assert_eq!(settled.status, StreamStatus::Active);
    assert_eq!(settled.end_time, T0 + 100 * DAY, "schedule is untouched");

    // The already-accrued-but-unwithdrawn 250 ONE moved with the stream: the
    // new recipient, not the old one, owns it from here on.
    h.advance(60 * DAY); // now past maturity
    assert_eq!(
        h.client.withdraw(&id, &None),
        850 * ONE,
        "the new recipient drains the remaining claim",
    );

    assert_eq!(
        h.balance(&h.recipient) - old_before,
        150 * ONE,
        "the old recipient is paid once and never again",
    );
    assert_eq!(
        h.balance(&h.other) - new_before,
        850 * ONE,
        "the new recipient receives the remainder",
    );
    assert_eq!(
        (h.balance(&h.recipient) - old_before) + (h.balance(&h.other) - new_before),
        deposit,
        "the two payouts partition the deposit exactly",
    );
    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 2. Transfer, then withdraw — every token goes to the new recipient.
// ---------------------------------------------------------------------------

#[test]
fn transfer_then_withdraw_in_the_same_ledger_pays_only_the_new_recipient() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let id = h.create_simple(deposit, 100 * DAY);

    let old_before = h.balance(&h.recipient);
    let new_before = h.balance(&h.other);

    h.advance(40 * DAY); // 400 ONE vested

    // Same ledger, order 2: transfer first, then withdraw.
    h.client.transfer_recipient(&id, &h.other);
    let transfer_events = published_by_stream(&h);

    let payout = h.client.withdraw(&id, &Some(150 * ONE));
    let withdraw_events = published_by_stream(&h);
    let settled = h.get(id);

    assert_published_only(
        &h,
        &transfer_events,
        RecipientTransferred {
            stream_id: id,
            old_recipient: h.recipient.clone(),
            new_recipient: h.other.clone(),
        },
        "transfer then withdraw: transfer event",
    );
    assert_eq!(
        transfer_events.len(),
        1,
        "the transfer must publish a single event",
    );

    // The withdrawal reads the *new* recipient out of storage and pays them.
    assert_eq!(payout, 150 * ONE);
    assert_eq!(
        h.balance(&h.recipient),
        old_before,
        "the old recipient receives nothing once the transfer runs first",
    );
    assert_eq!(h.balance(&h.other) - new_before, 150 * ONE);

    let expected_withdrawn = Withdrawn {
        stream_id: id,
        recipient: h.other.clone(),
        amount: payout,
        withdrawn: settled.withdrawn,
        deposited: settled.deposited,
        status: settled.status,
    };
    assert_eq!(
        withdraw_events,
        std::vec![expected_withdrawn.to_xdr(&h.env, &h.contract_id)],
        "the Withdrawn event must name the new recipient",
    );

    assert_eq!(settled.recipient, h.other);
    assert_eq!(settled.withdrawn, 150 * ONE);
    assert_eq!(settled.deposited, deposit);
    assert_eq!(settled.status, StreamStatus::Active);
    h.assert_pool_exact();

    // Settle the rest at maturity — still only the new recipient.
    h.advance(60 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 850 * ONE);
    assert_eq!(h.balance(&h.recipient), old_before);
    assert_eq!(
        h.balance(&h.other) - new_before,
        deposit,
        "the new recipient takes the whole deposit",
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 3. Determinism — the same scenario twice, on two fresh harnesses.
// ---------------------------------------------------------------------------

/// The observables of one full run. Addresses never appear: the snapshot and
/// the normalised events are role-resolved, so two independent harnesses can be
/// compared exactly.
#[derive(Debug, PartialEq)]
struct Transcript {
    first_payout: i128,
    second_payout: i128,
    old_recipient_delta: i128,
    new_recipient_delta: i128,
    sender_delta: i128,
    recipient_slot_is_new: bool,
    final_snapshot: TestSnapshot,
    events: std::vec::Vec<String>,
}

/// The identical same-ledger scenario, run end to end on a fresh harness.
fn run_same_ledger_split() -> Transcript {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let old_before = h.balance(&h.recipient);
    let new_before = h.balance(&h.other);
    let sender_before = h.balance(&h.sender);

    h.advance(40 * DAY);

    let mut events: std::vec::Vec<String> = std::vec::Vec::new();

    // Withdraw, then transfer, in the same ledger.
    let first = h.client.withdraw(&id, &Some(150 * ONE));
    events.extend(norm_events(&h));

    h.client.transfer_recipient(&id, &h.other);
    events.extend(norm_events(&h));

    // Drain the remainder at maturity.
    h.advance(60 * DAY);
    let second = h.client.withdraw(&id, &None);
    events.extend(norm_events(&h));

    let final_snapshot = h.snapshot();
    let recipient_slot_is_new = h.get(id).recipient == h.other;
    h.assert_pool_exact();

    Transcript {
        first_payout: first,
        second_payout: second,
        old_recipient_delta: h.balance(&h.recipient) - old_before,
        new_recipient_delta: h.balance(&h.other) - new_before,
        sender_delta: h.balance(&h.sender) - sender_before,
        recipient_slot_is_new,
        final_snapshot,
        events,
    }
}

#[test]
fn the_same_ledger_split_is_deterministic_across_runs() {
    let first = run_same_ledger_split();
    let second = run_same_ledger_split();

    // Two fresh harnesses (fresh addresses, fresh env) must agree exactly on
    // payouts, final records and the emitted event stream.
    assert_eq!(
        first, second,
        "the same-ledger split must be deterministic across runs",
    );

    // Guard against a vacuous comparison: the transcript must describe a
    // non-trivial split whose events carry role-resolved addresses.
    assert_eq!(first.first_payout, 150 * ONE);
    assert_eq!(first.second_payout, 850 * ONE);
    assert_eq!(first.old_recipient_delta, 150 * ONE);
    assert_eq!(first.new_recipient_delta, 850 * ONE);
    assert_eq!(first.sender_delta, 0);
    assert!(first.recipient_slot_is_new);
    assert_eq!(first.events.len(), 3, "withdraw, transfer, withdraw");
    assert!(first.events[0].starts_with("sym:withdrawn"));
    assert!(first.events[0].contains("role:old_recipient"));
    assert!(first.events[1].starts_with("sym:recipient_transferred"));
    assert!(first.events[1].contains("role:old_recipient"));
    assert!(first.events[1].contains("role:new_recipient"));
    assert!(first.events[2].starts_with("sym:withdrawn"));
    assert!(first.events[2].contains("role:new_recipient"));
}

// ---------------------------------------------------------------------------
// 4. The transfer does not disturb later accrual.
// ---------------------------------------------------------------------------

#[test]
fn a_same_ledger_transfer_does_not_change_later_accrual() {
    let h = Harness::new();
    let control = h.create_simple(1_000 * ONE, 100 * DAY);
    let moved = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(40 * DAY);

    // Same ledger: withdraw then transfer, on `moved` only.
    assert_eq!(h.client.withdraw(&moved, &Some(150 * ONE)), 150 * ONE);
    h.client.transfer_recipient(&moved, &h.other);

    // Only the recipient slot changed — the schedule is identical to control.
    let control_record = h.get(control);
    let moved_record = h.get(moved);
    assert_eq!(moved_record.recipient, h.other);
    assert_eq!(control_record.start_time, moved_record.start_time);
    assert_eq!(control_record.end_time, moved_record.end_time);
    assert_eq!(control_record.cliff_time, moved_record.cliff_time);
    assert_eq!(control_record.deposited, moved_record.deposited);
    assert_eq!(moved_record.withdrawn, 150 * ONE);

    // Later accrual tracks the control stream exactly at every checkpoint.
    for step in [DAY, 19 * DAY] {
        h.advance(step);
        assert_eq!(
            h.client.vested_of(&control),
            h.client.vested_of(&moved),
            "a same-ledger transfer must not change later accrual",
        );
    }

    // At day 60 both have vested 600 ONE; the transferred stream has already
    // paid out 150, so 450 remains for the new recipient.
    assert_eq!(h.client.vested_of(&moved), 600 * ONE);
    assert_eq!(h.client.withdrawable_of(&moved), 450 * ONE);

    let old_before = h.balance(&h.recipient);
    let new_before = h.balance(&h.other);
    assert_eq!(h.client.withdraw(&moved, &None), 450 * ONE);
    assert_eq!(h.balance(&h.other) - new_before, 450 * ONE);
    assert_eq!(
        h.balance(&h.recipient),
        old_before,
        "the old recipient earns nothing after the transfer",
    );

    // The control stream is unaffected and still owes its full unwithdrawn 600.
    assert_eq!(h.client.withdrawable_of(&control), 600 * ONE);
    assert_eq!(h.get(control).recipient, h.recipient);
    h.assert_pool_exact();
}
