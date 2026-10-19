//! `halt` called twice in a row (issue #2049).
//!
//! # Why this module exists
//!
//! Repeat submission happens routinely on chain: a client times out, never sees
//! the first transaction land, and retries. If a second `halt` were a silent
//! no-op — or worse, re-applied its effect — nothing in the suite would catch
//! it, because no test called the entry point twice and then inspected the
//! result.
//!
//! # The behaviour being pinned
//!
//! The second call is **refused**, not absorbed: it returns the typed
//! [`Error::ContractAlreadyHalted`], leaves the halt exactly as the first call
//! left it (same operator, same recorded instant), moves no tokens, and emits
//! no second event. That is the safe answer for a retry after a timeout — the
//! retry can neither double-apply the halt nor move its start time.
//!
//! The same sentence is stated in
//! [`FluxoraStream::halt`](crate::FluxoraStream::halt)'s doc comment under
//! "Repeat submission", so the behaviour and its documentation cannot drift
//! apart.
//!
//! Behaviour is unchanged: this is coverage only.

#![cfg(test)]

use soroban_sdk::testutils::Events as _;
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::ContractHalted;
use crate::{DataKey, Error};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Every event the stream contract emitted in the most recent invocation.
///
/// `Events::all()` describes only the invocation that just ran, so this must be
/// read before any further contract call — and a rolled-back (refused) call
/// reports nothing, which is what makes the "no second event" assertion real
/// rather than a vacuous one.
fn published_events(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// The halt instant recorded in storage, or `None` when the contract is live.
///
/// `DataKey::HaltedAt` doubles as the halt flag, so reading it captures the
/// whole of the halt state a first call could have written.
fn recorded_halt_instant(h: &Harness) -> Option<u64> {
    h.env.as_contract(&h.contract_id, || {
        h.env.storage().instance().get(&DataKey::HaltedAt)
    })
}

// ---------------------------------------------------------------------------
// Acceptance criterion: the second call is refused with a typed error, and
// calling twice leaves the same state as calling once
// ---------------------------------------------------------------------------

/// Halting twice is refused with the documented typed error, and the halt state
/// after the second call is exactly what the first call left behind.
#[test]
fn halt_twice_refuses_the_second_call_with_a_typed_error() {
    let h = Harness::new();
    let instant = h.now();
    h.client.set_halt_operator(&h.sender);

    // First call: engages the halt and records the ledger instant.
    h.client.halt();
    let after_one = recorded_halt_instant(&h);
    assert_eq!(
        after_one,
        Some(instant),
        "the first call records the halt instant",
    );

    // Second call: refused, typed — not a host error and not a panic.
    let err = h.client.try_halt().unwrap_err().unwrap();
    assert_eq!(err, Error::ContractAlreadyHalted);

    // Calling twice leaves the same state as calling once.
    assert_eq!(
        recorded_halt_instant(&h),
        after_one,
        "the second call must not move the recorded halt instant",
    );
    assert!(h.client.halted());
    assert_eq!(h.client.halt_operator(), Some(h.sender.clone()));
}

// ---------------------------------------------------------------------------
// Acceptance criterion: token balances are asserted after both calls
// ---------------------------------------------------------------------------

/// `halt` moves no funds, so the pool and both parties' balances are identical
/// before the first call, after the first call, and after the refused second.
///
/// The pool is funded by minting straight to the contract rather than through
/// `create_stream`: this test is about the halt and should not stand or fall
/// with the creation path. `halt` never calls the token contract, so the
/// assertion holds however the balance got there — and because the pool is
/// non-zero, a stray transfer in either call would show up.
#[test]
fn halt_twice_moves_no_tokens() {
    let h = Harness::new();
    h.token_admin.mint(&h.contract_id, &(500 * ONE));
    h.client.set_halt_operator(&h.sender);

    let pool_before = h.pool();
    let sender_before = h.balance(&h.sender);
    let recipient_before = h.balance(&h.recipient);
    assert!(pool_before > 0, "fixture precondition: the pool is funded");

    h.client.halt();
    assert_eq!(h.pool(), pool_before, "the first halt moved pool funds");
    assert_eq!(h.balance(&h.sender), sender_before);
    assert_eq!(h.balance(&h.recipient), recipient_before);

    assert_eq!(
        h.client.try_halt().unwrap_err().unwrap(),
        Error::ContractAlreadyHalted,
    );

    assert_eq!(
        h.pool(),
        pool_before,
        "the refused second halt moved pool funds"
    );
    assert_eq!(h.balance(&h.sender), sender_before);
    assert_eq!(h.balance(&h.recipient), recipient_before);
}

// ---------------------------------------------------------------------------
// Acceptance criterion: the emitted events are asserted for both calls
// ---------------------------------------------------------------------------

/// The first call emits exactly one `ContractHalted` carrying the instant; the
/// refused second call emits nothing at all.
#[test]
fn halt_twice_emits_one_event_for_the_first_call_and_none_for_the_second() {
    let h = Harness::new();
    let halted_at = h.now();
    h.client.set_halt_operator(&h.sender);

    h.client.halt();
    // Read before any further contract call: `Events::all()` describes only the
    // invocation that just ran.
    let first = published_events(&h);
    let expected = ContractHalted {
        operator: h.sender.clone(),
        halted_at,
    };
    assert_eq!(
        first,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the first halt emits exactly one contract_halted event",
    );

    let err = h.client.try_halt().unwrap_err().unwrap();
    assert_eq!(err, Error::ContractAlreadyHalted);
    let second = published_events(&h);
    assert!(
        second.is_empty(),
        "the refused second halt must emit no event",
    );
}
