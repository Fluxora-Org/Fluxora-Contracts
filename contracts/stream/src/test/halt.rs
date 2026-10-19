//! Contract-level emergency halt (issue #1818).
//!
//! # What this suite is for
//!
//! `pause` acts on one stream, and only if that stream was created `pausable`.
//! An operator who discovers an exploit therefore has no way to stop settlement
//! across every stream at once. This module covers the contract-level halt that
//! closes that gap, against the acceptance criteria in the issue:
//!
//! 1. an authorised party can halt state-changing entry points;
//! 2. read methods remain available while halted;
//! 3. the halt requires explicit resumption (there is no timeout);
//! 4. entering and leaving the halt emits an event;
//! 5. halting the contract refuses **every** mutating entry point while reads
//!    still answer.
//!
//! # Design under test
//!
//! The contract has no admin. The halt is deliberately opt-in: a one-shot
//! [`set_halt_operator`](crate::FluxoraStream::set_halt_operator) installs the
//! only address that may halt and resume, and a deployment that never calls it
//! has no operator, cannot be halted, and behaves exactly as it did before the
//! feature existed. That is what keeps every pre-existing test in this suite —
//! none of which installs an operator — valid unchanged.
//!
//! [`halt`](crate::FluxoraStream::halt) is checked **first** in every mutating
//! entry point, before authorization and before any other precondition (see
//! `FluxoraStream::require_not_halted`). Two consequences the tests below pin
//! explicitly:
//!
//! * the refusal is [`Error::ContractHalted`] (34) regardless of what the call
//!   would otherwise have returned, and
//! * the three halt entry points themselves stay reachable while halted, so the
//!   operator can always lift the halt — a circuit breaker with no off switch
//!   would be a worse bug than the one it mitigates.
//!
//! Because the halt is checked before argument validation, one fixture with an
//! *active* stream and one with a *paused* stream cover every mutating entry
//! point with genuinely valid preconditions; the ordering is asserted
//! independently by `halt_is_checked_before_validation_and_authorization`.

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, Symbol, TryFromVal, TryIntoVal, Val};

use super::common::*;
use crate::{op, Error};

extern crate std;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Assert that a `try_*` call is refused by the contract-level halt.
///
/// Fails if the call succeeds, and fails if it fails with anything other than
/// [`Error::ContractHalted`] — a host auth error would mean the guard ran after
/// `require_auth`, and any other typed error would mean it ran after validation.
macro_rules! assert_refused_by_halt {
    ($label:expr, $call:expr) => {{
        let err = $call
            .err()
            .unwrap_or_else(|| panic!("{} was accepted while the contract is halted", $label))
            .unwrap_or_else(|host| panic!("{} failed with a host error: {host:?}", $label));
        assert_eq!(
            err,
            Error::ContractHalted,
            "{} must be refused with ContractHalted (34)",
            $label
        );
    }};
}

/// Extract every event emitted by the stream contract, in emission order, as
/// `(topics, data)`. Mirrors `test::events`; kept local so this module stays
/// readable on its own.
fn drain_events(h: &Harness) -> std::vec::Vec<(soroban_sdk::Vec<Val>, Val)> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
        .into_iter()
        .map(|event| {
            let soroban_sdk::xdr::ContractEventBody::V0(body) = event.body;
            let mut topics = soroban_sdk::vec![&h.env];
            for t in body.topics.iter() {
                topics.push_back(Val::try_from_val(&h.env, t).unwrap());
            }
            let data = Val::try_from_val(&h.env, &body.data).unwrap();
            (topics, data)
        })
        .collect()
}

/// The `topic[0]` event name.
fn topic_name(h: &Harness, event: &(soroban_sdk::Vec<Val>, Val)) -> Symbol {
    event
        .0
        .get(0)
        .expect("every contractevent has at least one topic")
        .try_into_val(&h.env)
        .expect("topic[0] is a Symbol")
}

/// The `operator` topic every halt event carries.
fn topic_operator(h: &Harness, event: &(soroban_sdk::Vec<Val>, Val)) -> Address {
    event
        .0
        .get(1)
        .expect("halt events carry the operator as topic[1]")
        .try_into_val(&h.env)
        .expect("topic[1] is an Address")
}

/// Decode one named field out of an event payload.
///
/// A `#[contractevent]` publishes its non-topic fields as a `Map<Symbol, Val>`
/// keyed by field name, even when there is only one.
fn map_payload_u64(h: &Harness, data: &Val, name: &str) -> u64 {
    let map: soroban_sdk::Map<Symbol, Val> = data
        .try_into_val(&h.env)
        .expect("payload is a Map<Symbol, Val>");
    map.get(Symbol::new(&h.env, name))
        .unwrap_or_else(|| panic!("payload has no `{name}` field"))
        .try_into_val(&h.env)
        .expect("field value decodes as u64")
}

/// An operator-installed, *active*-stream, halted contract.
///
/// The stream is created before the halt so every mutating call below has a
/// genuinely valid precondition and the refusal is attributable to the halt
/// alone. Time is advanced before the halt so `withdraw` has something to pay.
///
/// Two delegates are needed because a grant is stored per `(stream_id,
/// delegate)` with the grantor implied by the bits: sender-side and
/// recipient-side operations cannot share one grant.
///
/// Returns `(harness, stream_id, sender_agent, recipient_agent)`.
fn halted_active_fixture() -> (Harness<'static>, u64, Address, Address) {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    // Delegates holding every grantable operation, so the six `delegate_*`
    // calls are valid calls being refused rather than invalid ones.
    let sender_agent = Address::generate(&h.env);
    let recipient_agent = Address::generate(&h.env);
    h.client.grant_delegate(
        &id,
        &h.sender,
        &sender_agent,
        &(op::CANCEL | op::PAUSE | op::RESUME | op::TOP_UP),
        &None,
    );
    h.client.grant_delegate(
        &id,
        &h.recipient,
        &recipient_agent,
        &(op::WITHDRAW | op::TRANSFER_RECIPIENT),
        &None,
    );

    h.client.set_halt_operator(&h.sender);
    h.client.halt();
    assert!(
        h.client.halted(),
        "fixture precondition: contract is halted"
    );
    (h, id, sender_agent, recipient_agent)
}

/// An operator-installed, *paused*-stream, halted contract, so `resume` and
/// `delegate_resume` have a genuinely valid precondition.
fn halted_paused_fixture() -> (Harness<'static>, u64, Address) {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.pause(&id);

    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::RESUME, &None);

    h.client.set_halt_operator(&h.sender);
    h.client.halt();
    assert!(
        h.client.halted(),
        "fixture precondition: contract is halted"
    );
    (h, id, agent)
}

// ---------------------------------------------------------------------------
// Defaults: no operator, no halt, nothing to rewrite
// ---------------------------------------------------------------------------

/// A deployment that never installs an operator is not haltable, and says so.
///
/// This is the compatibility property the whole design rests on: every
/// pre-existing test (and every existing deployment) has no operator, so
/// `halted()` reads `false` and the export surface behaves as before.
#[test]
fn a_fresh_contract_is_neither_halted_nor_haltable() {
    let h = Harness::new();
    assert!(!h.client.halted(), "a fresh contract is not halted");
    assert_eq!(h.client.halt_operator(), None, "no operator is installed");

    // With no operator there is no key to halt or resume.
    assert_eq!(
        h.client.try_halt().unwrap_err().unwrap(),
        Error::HaltOperatorNotSet
    );
    assert_eq!(
        h.client.try_resume_contract().unwrap_err().unwrap(),
        Error::HaltOperatorNotSet
    );

    // And the unhalted contract still does its job.
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    assert_eq!(h.client.stream_count(), 1);
    assert!(h.client.stream_exists(&id));
}

// ---------------------------------------------------------------------------
// Installing the operator
// ---------------------------------------------------------------------------

/// The one-shot setter installs the operator, demands its authorization, and
/// emits exactly one `halt_operator_set` event carrying it.
#[test]
fn set_halt_operator_installs_the_operator_and_emits_an_event() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);

    // Both `Events::all()` and `env.auths()` describe only the most recent
    // invocation, so both must be read before any further contract call.
    let events = drain_events(&h);
    let auths = h.env.auths();
    assert_eq!(auths.len(), 1, "exactly one require_auth is demanded");
    assert_eq!(auths[0].0, h.sender, "the operator's own auth is required");

    assert_eq!(h.client.halt_operator(), Some(h.sender.clone()));

    assert_eq!(events.len(), 1, "exactly one event on install");
    assert_eq!(
        topic_name(&h, &events[0]),
        Symbol::new(&h.env, "halt_operator_set")
    );
    assert_eq!(topic_operator(&h, &events[0]), h.sender);
    assert_eq!(events[0].0.len(), 2, "topic[0] + operator = 2");
}

/// Negative: with no authorization at all the setter is rejected and no
/// operator is installed.
#[test]
#[should_panic(expected = "Unauthorized")]
fn set_halt_operator_requires_the_operator_authorization() {
    let h = Harness::new();
    h.env.mock_auths(&[]);
    h.client.set_halt_operator(&h.sender);
}

/// A rejected install leaves the contract exactly as it was.
#[test]
fn rejected_set_halt_operator_installs_nothing() {
    let h = Harness::new();
    h.env.mock_auths(&[]);
    let _ = h.client.try_set_halt_operator(&h.sender);
    h.env.mock_all_auths();

    assert_eq!(h.client.halt_operator(), None, "no operator was installed");
    assert!(!h.client.halted(), "the contract is not halted");
    assert_eq!(
        h.client.try_halt().unwrap_err().unwrap(),
        Error::HaltOperatorNotSet
    );
}

/// The setter has no rotation path: a second call is refused and the first
/// operator keeps the authority.
#[test]
fn set_halt_operator_is_one_shot() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);

    let err = h
        .client
        .try_set_halt_operator(&h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::HaltOperatorAlreadySet);
    assert_eq!(
        h.client.halt_operator(),
        Some(h.sender.clone()),
        "the installed operator is unchanged by a rejected rotation"
    );
}

// ---------------------------------------------------------------------------
// Engaging and lifting the halt
// ---------------------------------------------------------------------------

/// `halt` engages the flag, demands the operator's auth, and emits exactly one
/// `contract_halted` event carrying the operator and the halt timestamp.
#[test]
fn halt_engages_the_halt_and_emits_an_event() {
    let h = Harness::new();
    let halted_at = h.now();
    h.client.set_halt_operator(&h.sender);

    h.client.halt();

    let events = drain_events(&h);
    let auths = h.env.auths();
    assert!(!auths.is_empty(), "halt demands authorization");
    assert_eq!(auths[0].0, h.sender, "the operator's auth is required");
    assert!(h.client.halted());

    assert_eq!(events.len(), 1, "exactly one event on halt");
    assert_eq!(
        topic_name(&h, &events[0]),
        Symbol::new(&h.env, "contract_halted")
    );
    assert_eq!(topic_operator(&h, &events[0]), h.sender);
    assert_eq!(events[0].0.len(), 2, "topic[0] + operator = 2");
    assert_eq!(
        map_payload_u64(&h, &events[0].1, "halted_at"),
        halted_at,
        "the event records the ledger time the halt was engaged"
    );
}

/// Negative: without the operator's authorization `halt` aborts and the
/// contract stays live.
#[test]
fn unauthorized_halt_does_not_engage_the_halt() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);

    h.env.mock_auths(&[]);
    assert!(h.client.try_halt().is_err(), "halt must demand auth");
    h.env.mock_all_auths();

    assert!(!h.client.halted(), "a rejected halt changes nothing");
}

/// `resume_contract` restores settlement, demands the operator's auth, and
/// emits exactly one `contract_resumed` event with the duration halted.
#[test]
fn resume_contract_lifts_the_halt_and_emits_an_event() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);
    h.client.halt();
    h.advance(3 * DAY);
    let resumed_at = h.now();

    h.client.resume_contract();

    let events = drain_events(&h);
    let auths = h.env.auths();
    assert!(!auths.is_empty(), "resume demands authorization");
    assert_eq!(auths[0].0, h.sender, "the operator's auth is required");
    assert!(!h.client.halted());

    assert_eq!(events.len(), 1, "exactly one event on resume");
    assert_eq!(
        topic_name(&h, &events[0]),
        Symbol::new(&h.env, "contract_resumed")
    );
    assert_eq!(topic_operator(&h, &events[0]), h.sender);
    assert_eq!(events[0].0.len(), 2, "topic[0] + operator = 2");
    assert_eq!(map_payload_u64(&h, &events[0].1, "resumed_at"), resumed_at);
    assert_eq!(map_payload_u64(&h, &events[0].1, "halted_for"), 3 * DAY);
}

/// Negative: only the operator can lift the halt.
#[test]
fn unauthorized_resume_contract_leaves_the_contract_halted() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);
    h.client.halt();

    h.env.mock_auths(&[]);
    assert!(
        h.client.try_resume_contract().is_err(),
        "resume must demand auth"
    );
    h.env.mock_all_auths();

    assert!(h.client.halted(), "a rejected resume changes nothing");
}

/// One-shot semantics on both edges: halting twice and resuming when not
/// halted are both typed errors rather than silent no-ops.
#[test]
fn halt_and_resume_are_rejected_when_they_would_be_no_ops() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);

    assert_eq!(
        h.client.try_resume_contract().unwrap_err().unwrap(),
        Error::ContractNotHalted
    );

    h.client.halt();
    assert_eq!(
        h.client.try_halt().unwrap_err().unwrap(),
        Error::ContractAlreadyHalted
    );

    h.client.resume_contract();
    assert_eq!(
        h.client.try_resume_contract().unwrap_err().unwrap(),
        Error::ContractNotHalted
    );
}

/// The halt is explicit-resumption-only: it does not expire with ledger time.
#[test]
fn the_halt_does_not_time_out() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);
    h.client.halt();

    h.advance(365 * DAY);
    assert!(h.client.halted(), "the halt outlives any timeout");
    assert_refused_by_halt!("top_up", h.client.try_top_up(&1, &(ONE)));
}

// ---------------------------------------------------------------------------
// Acceptance criterion: every mutating entry point is refused (#1818)
// ---------------------------------------------------------------------------

/// **The acceptance test.** With the contract halted, every state-changing
/// entry point is refused with [`Error::ContractHalted`], on a fixture where
/// each call's precondition genuinely holds.
///
/// The eighteen guarded entry points are enumerated here as the docs do:
/// the fourteen stream mutations plus the four delegated variants and the two
/// TTL-maintenance calls that `docs/ABI.md` lists as "Maintenance".
#[test]
fn halted_contract_refuses_every_mutating_entry_point() {
    let (h, id, sender_agent, recipient_agent) = halted_active_fixture();
    let now = h.now();

    // Creation
    assert_refused_by_halt!(
        "create_stream",
        h.client.try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &(100 * ONE),
            &now,
            &(now + 10 * DAY),
            &now,
            &true,
            &true,
            &true,
            &None,
        )
    );

    // Funding and settlement
    assert_refused_by_halt!("top_up", h.client.try_top_up(&id, &(10 * ONE)));
    assert_refused_by_halt!("withdraw", h.client.try_withdraw(&id, &Some(ONE)));
    assert_refused_by_halt!(
        "batch_withdraw",
        h.client.try_batch_withdraw(&h.recipient, &h.ids(&[id]))
    );

    // Lifecycle
    assert_refused_by_halt!("cancel", h.client.try_cancel(&id));
    assert_refused_by_halt!("pause", h.client.try_pause(&id));
    assert_refused_by_halt!(
        "transfer_recipient",
        h.client.try_transfer_recipient(&id, &h.other)
    );

    // Delegation administration
    assert_refused_by_halt!(
        "grant_delegate",
        h.client
            .try_grant_delegate(&id, &h.sender, &sender_agent, &op::CANCEL, &None)
    );
    assert_refused_by_halt!(
        "revoke_delegate",
        h.client.try_revoke_delegate(&id, &h.sender, &sender_agent)
    );

    // Delegated mutations
    assert_refused_by_halt!(
        "delegate_withdraw",
        h.client
            .try_delegate_withdraw(&id, &recipient_agent, &Some(ONE))
    );
    assert_refused_by_halt!(
        "delegate_cancel",
        h.client.try_delegate_cancel(&id, &sender_agent)
    );
    assert_refused_by_halt!(
        "delegate_pause",
        h.client.try_delegate_pause(&id, &sender_agent)
    );
    assert_refused_by_halt!(
        "delegate_top_up",
        h.client
            .try_delegate_top_up(&id, &sender_agent, &(10 * ONE))
    );
    assert_refused_by_halt!(
        "delegate_transfer_recipient",
        h.client
            .try_delegate_transfer_recipient(&id, &recipient_agent, &h.other)
    );

    // TTL maintenance
    assert_refused_by_halt!("extend_stream_ttl", h.client.try_extend_stream_ttl(&id));
    assert_refused_by_halt!(
        "batch_extend_ttl",
        h.client.try_batch_extend_ttl(&h.ids(&[id]))
    );

    // `resume` and `delegate_resume` need a paused stream, which cannot also be
    // active. They are covered by the paused fixture — still with valid
    // preconditions, because the guard precedes validation.
    let (h, id, agent) = halted_paused_fixture();
    assert_refused_by_halt!("resume", h.client.try_resume(&id));
    assert_refused_by_halt!("delegate_resume", h.client.try_delegate_resume(&id, &agent));
}

/// The halt refusal is ordering, not validation: it fires before argument
/// validation, before capability checks, and even before authorization.
///
/// This is what makes the enumeration above meaningful — the same 34 comes back
/// whether the call would have succeeded, been rejected as invalid, or been
/// rejected for missing auth.
#[test]
fn halt_is_checked_before_validation_and_authorization() {
    let (h, id, _, _) = halted_active_fixture();

    // Nonsensical arguments (zero amount) and a capability the stream does not
    // need to satisfy: still the halt, not `InvalidAmount` or anything else.
    assert_refused_by_halt!("withdraw(0)", h.client.try_withdraw(&id, &Some(0)));
    assert_refused_by_halt!(
        "withdraw(missing stream)",
        h.client.try_withdraw(&999, &Some(ONE))
    );

    // No authorization at all: still the halt, not a host auth failure.
    h.env.mock_auths(&[]);
    assert_refused_by_halt!("cancel(no auth)", h.client.try_cancel(&id));
    assert_refused_by_halt!("top_up(no auth)", h.client.try_top_up(&id, &(ONE)));
    h.env.mock_all_auths();
}

/// A halted contract changes no stream and moves no funds: the halt itself is
/// a flag, not a settlement.
#[test]
fn halt_changes_no_stream_state() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    let before = h.get(id);
    let pool_before = h.pool();
    let sender_before = h.balance(&h.sender);
    let recipient_before = h.balance(&h.recipient);

    h.client.set_halt_operator(&h.sender);
    h.client.halt();

    assert_eq!(h.get(id), before, "the halt rewrites no stream field");
    assert_eq!(h.pool(), pool_before, "the halt moves no funds");
    assert_eq!(h.balance(&h.sender), sender_before);
    assert_eq!(h.balance(&h.recipient), recipient_before);
    h.assert_invariants();
}

// ---------------------------------------------------------------------------
// Acceptance criterion: reads remain available while halted (#1818)
// ---------------------------------------------------------------------------

/// Every read method still answers while the contract is halted, including the
/// two halt views and the accrual views that keep climbing.
#[test]
fn every_read_method_still_answers_while_halted() {
    let (h, id, _, _) = halted_active_fixture();

    // Halt views.
    assert!(h.client.halted());
    assert_eq!(h.client.halt_operator(), Some(h.sender.clone()));

    // Stream views.
    let stream = h.client.get_stream(&id);
    assert_eq!(stream.deposited, 1_000 * ONE);
    assert_eq!(h.client.stream_count(), 1);
    assert!(h.client.stream_exists(&id));
    assert!(!h.client.stream_exists(&999));

    // Accrual keeps vesting while halted; only settlement stops.
    let vested_before = h.client.vested_of(&id);
    let withdrawable_before = h.client.withdrawable_of(&id);
    assert!(withdrawable_before > 0, "the fixture has accrued");

    h.advance(5 * DAY);

    assert!(h.client.vested_of(&id) > vested_before, "vesting continues");
    assert!(
        h.client.withdrawable_of(&id) > withdrawable_before,
        "withdrawable keeps climbing"
    );
    assert!(h.client.refundable_of(&id) >= 0);

    // Reads on a missing id still report the ordinary typed error.
    assert_eq!(
        h.client.try_get_stream(&999).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
}

// ---------------------------------------------------------------------------
// The escape hatch, and resumption
// ---------------------------------------------------------------------------

/// The operator can always lift the halt: the halt entry points are exempt from
/// the guard, so a halted contract is never locked shut.
#[test]
fn the_operator_can_always_leave_the_halt() {
    let (h, _, _, _) = halted_active_fixture();

    // `halt` is reachable while halted (and reports the no-op), and
    // `resume_contract` restores the contract.
    assert_eq!(
        h.client.try_halt().unwrap_err().unwrap(),
        Error::ContractAlreadyHalted
    );
    h.client.resume_contract();
    assert!(!h.client.halted());

    // And it can be engaged again.
    h.client.halt();
    assert!(h.client.halted());
    h.client.resume_contract();
    assert!(!h.client.halted());
}

/// Lifting the halt restores the whole mutating surface, from exactly the state
/// that was halted.
#[test]
fn resume_restores_the_mutating_surface() {
    let (h, id, sender_agent, _recipient_agent) = halted_active_fixture();
    let stream_before = h.get(id);

    h.client.resume_contract();
    assert!(!h.client.halted());

    h.client.top_up(&id, &(100 * ONE));
    h.client.pause(&id);
    h.client.resume(&id);
    h.client.delegate_top_up(&id, &sender_agent, &(10 * ONE));
    h.client.withdraw(&id, &None);
    h.client.extend_stream_ttl(&id);

    let stream_after = h.get(id);
    assert_eq!(
        stream_after.deposited,
        stream_before.deposited + 110 * ONE,
        "the halted state is where settlement resumed from"
    );
    assert_eq!(stream_after.status, stream_before.status);
    h.assert_invariants();
}
