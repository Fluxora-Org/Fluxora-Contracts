//! Boundary coverage for `halt` (issue #2047).
//!
//! # What this module pins
//!
//! [`FluxoraStream::halt`](crate::FluxoraStream::halt) is the contract-level
//! emergency stop. Issue #2047 asks for its boundary behaviour to be pinned at
//! zero, one, the maximum accepted value, and one past it, with the guarantee
//! that nothing panics and every refusal is a typed error.
//!
//! `halt` takes no value arguments — its only parameter is the implicit `Env` —
//! so the values whose extremes matter are the ones the halt path actually
//! reads and writes:
//!
//! * **The halt state.** The contract is halted at most once at a time, so the
//!   count of accepted halts is `0` or `1`, and one past that `1` is a second
//!   `halt`, refused with [`Error::ContractAlreadyHalted`].
//! * **The ledger timestamp.** `halt` records `env.ledger().timestamp()` as
//!   `halted_at`, and [`resume_contract`](crate::FluxoraStream::resume_contract)
//!   derives `halted_for` from it with a saturating subtraction. That
//!   subtraction is the only arithmetic in the halt path, and a clock at either
//!   extreme is what exercises it.
//!
//! Both axes are driven through the public ABI. Behaviour is unchanged and no
//! existing test is modified: this module only adds the missing coverage.

#![cfg(test)]

use soroban_sdk::testutils::{Events as _, Ledger as _};
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::{ContractHalted, ContractResumed};
use crate::Error;

/// Every event the stream contract emitted in the most recent invocation.
///
/// `Events::all()` describes only the invocation that just ran, so a failed
/// (rolled back) call reports nothing — which is what makes the "a refused halt
/// emits nothing" assertions meaningful.
fn published_events(h: &Harness) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

// ---------------------------------------------------------------------------
// Zero and one
// ---------------------------------------------------------------------------

/// Zero is covered explicitly: with no operator ever installed there have been
/// zero halts, and both halt entry points refuse with their documented typed
/// error rather than panicking.
#[test]
fn halt_zero_is_covered_explicitly() {
    let h = Harness::new();

    assert_eq!(h.client.halt_operator(), None, "zero operators installed");
    assert!(!h.client.halted(), "zero halts engaged");

    assert_eq!(
        h.client.try_halt().unwrap_err().unwrap(),
        Error::HaltOperatorNotSet,
    );
    // Events are read before any further contract call: `Events::all()`
    // describes only the most recent invocation.
    assert!(
        published_events(&h).is_empty(),
        "a refused halt emits no event"
    );

    assert_eq!(
        h.client.try_resume_contract().unwrap_err().unwrap(),
        Error::HaltOperatorNotSet,
    );
    assert!(
        published_events(&h).is_empty(),
        "a refused resume emits no event"
    );

    // A refused halt leaves the zero state exactly as it was.
    assert!(!h.client.halted());
    assert_eq!(h.client.halt_operator(), None);
}

/// One is covered explicitly: exactly one operator and exactly one engaged
/// halt, whose single event records the halt instant.
#[test]
fn halt_one_is_covered_explicitly() {
    let h = Harness::new();
    let halted_at = h.now();
    h.client.set_halt_operator(&h.sender);

    h.client.halt();

    // Capture the events before any further contract call.
    let events = published_events(&h);
    assert!(h.client.halted());
    assert_eq!(h.client.halt_operator(), Some(h.sender.clone()));

    let expected = ContractHalted {
        operator: h.sender.clone(),
        halted_at,
    };
    assert_eq!(
        events,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "exactly one halt event, recording the halt instant",
    );
}

// ---------------------------------------------------------------------------
// The maximum accepted value, and one past it
// ---------------------------------------------------------------------------

/// The maximum accepted value of the halt instant: `u64::MAX` is the latest
/// ledger time there is, `halt` accepts it, and the recorded instant is exact.
#[test]
fn halt_maximum_accepted_value_succeeds() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);

    h.env.ledger().set_timestamp(u64::MAX);
    h.client.halt();

    let events = published_events(&h);
    assert!(h.client.halted());
    let expected = ContractHalted {
        operator: h.sender.clone(),
        halted_at: u64::MAX,
    };
    assert_eq!(
        events,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the maximum halt instant is accepted unchanged",
    );
}

/// One past the maximum is refused with the documented error: the most halts
/// the contract can be in is one, so a second `halt` is not a silent no-op.
///
/// The refusal is the typed [`Error::ContractAlreadyHalted`], it emits no
/// event, and it leaves the first halt — operator and recorded instant — exactly
/// as it was.
#[test]
fn halt_one_past_the_maximum_is_refused_with_the_documented_error() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);
    h.client.halt();
    let halted_at = h.now();

    let err = h.client.try_halt().unwrap_err().unwrap();

    assert_eq!(err, Error::ContractAlreadyHalted);
    assert!(
        published_events(&h).is_empty(),
        "a refused halt emits no event"
    );
    assert!(h.client.halted(), "the first halt is left engaged");
    assert_eq!(h.client.halt_operator(), Some(h.sender.clone()));

    // The refused second halt must not have moved the first halt's instant:
    // resuming immediately reports zero seconds halted, from `halted_at`.
    h.client.resume_contract();
    let events = published_events(&h);
    let expected = ContractResumed {
        operator: h.sender.clone(),
        resumed_at: halted_at,
        halted_for: 0,
    };
    assert_eq!(
        events,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "the refused second halt did not overwrite the first halt's instant",
    );
}

// ---------------------------------------------------------------------------
// The ledger timestamp at its extremes
// ---------------------------------------------------------------------------

/// The zero end of the time value: halting at ledger timestamp zero records
/// `halted_at == 0`, and resuming at the same instant reports `halted_for == 0`
/// — not a negative or wrapped duration.
#[test]
fn halt_at_ledger_timestamp_zero_records_exactly_zero() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);

    h.env.ledger().set_timestamp(0);
    h.client.halt();

    let halt_events = published_events(&h);
    assert!(h.client.halted());
    let halted = ContractHalted {
        operator: h.sender.clone(),
        halted_at: 0,
    };
    assert_eq!(
        halt_events,
        std::vec![halted.to_xdr(&h.env, &h.contract_id)],
    );

    h.client.resume_contract();
    let resume_events = published_events(&h);
    let resumed = ContractResumed {
        operator: h.sender.clone(),
        resumed_at: 0,
        halted_for: 0,
    };
    assert_eq!(
        resume_events,
        std::vec![resumed.to_xdr(&h.env, &h.contract_id)],
    );
}

/// The one end of the time value: halting at ledger timestamp one, then
/// resuming one second later, reports `halted_for == 1` exactly — the
/// off-by-one guard on the halt duration.
#[test]
fn halt_at_ledger_timestamp_one_records_exactly_one() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);

    h.env.ledger().set_timestamp(1);
    h.client.halt();

    let halted = ContractHalted {
        operator: h.sender.clone(),
        halted_at: 1,
    };
    assert_eq!(
        published_events(&h),
        std::vec![halted.to_xdr(&h.env, &h.contract_id)],
    );

    h.env.ledger().set_timestamp(2);
    h.client.resume_contract();

    let resumed = ContractResumed {
        operator: h.sender.clone(),
        resumed_at: 2,
        halted_for: 1,
    };
    assert_eq!(
        published_events(&h),
        std::vec![resumed.to_xdr(&h.env, &h.contract_id)],
        "one second halted is exactly one, not zero and not two",
    );
}

/// No case panics: the halt duration is computed by subtraction, and a clock
/// that sits behind the halt instant must saturate to zero rather than
/// underflow the `u64`. Halt at the top of the range, resume with the clock at
/// zero, and require the typed result — no trap.
#[test]
fn resume_contract_halted_for_saturates_and_never_panics() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);

    h.env.ledger().set_timestamp(u64::MAX);
    h.client.halt();
    h.env.ledger().set_timestamp(0);

    h.client.resume_contract();

    let events = published_events(&h);
    assert!(!h.client.halted());
    let resumed = ContractResumed {
        operator: h.sender.clone(),
        resumed_at: 0,
        halted_for: 0,
    };
    assert_eq!(
        events,
        std::vec![resumed.to_xdr(&h.env, &h.contract_id)],
        "a clock behind the halt instant saturates to zero rather than panicking",
    );
}
