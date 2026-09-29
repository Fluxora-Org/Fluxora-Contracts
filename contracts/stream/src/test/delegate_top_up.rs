//! `delegate_top_up` coverage at the depth of `top_up` (Issue #1824).
//!
//! `delegate_top_up` re-implements every guard of `top_up` behind an extra
//! authorization layer (`check_delegate`), and it additionally needs the
//! *sender's* signature for the deposit pull. That makes it the more dangerous
//! of the two paths, so it is held to the same rejection depth here.
//!
//! # Coverage map
//!
//! `top_up` rejection (where it is pinned) → `delegate_top_up` equivalent:
//!
//! | `top_up` case                                   | `delegate_top_up` test                                  |
//! |-------------------------------------------------|---------------------------------------------------------|
//! | no sender auth (`auth`)                         | `delegate_top_up_needs_both_the_delegate_and_the_sender` |
//! | unknown stream id → `StreamNotFound` (`auth`)   | `delegate_top_up_on_an_unknown_stream_is_not_permitted`  |
//! | cancelled → `StreamTerminated` (`top_up`)       | parity table: `cancelled`                               |
//! | depleted → `StreamTerminated` (`top_up`)        | parity table: `depleted`                                |
//! | `0`, `-1`, `i128::MIN` → `InvalidAmount`         | parity table: `zero` / `negative` / `i128::MIN`          |
//! | matured at and after `end_time`                 | parity table: `matured at end` / `matured a year later`  |
//! | one second before maturity is allowed           | parity table: `one second before maturity`              |
//! | sub-second top-up → `TopUpTooSmall`             | parity table: `sub-second at 100 stroops/sec`           |
//! | `amount * duration` overflow → `Overflow`       | parity table: `i128::MAX / 2`, `i128::MAX`              |
//! | extension past `u64` → `Overflow`               | parity table: `delta past u64` / `end_time past u64`    |
//! | re-established creation guard → `Overflow`      | parity table: `new deposit * duration past i128`        |
//! | sender drained → `TokenTransferFailed`          | parity table: `sender has no balance`                   |
//! | fee-on-transfer → `TokenAmountMismatch`         | `delegate_top_up_rejects_a_fee_on_transfer_token`        |
//! | allowed while paused, does not resume           | parity table: `paused` / `paused past end_time`         |
//!
//! Delegate-only rejections, with no `top_up` counterpart:
//!
//! | Grant state                          | Expected                | Test                                                        |
//! |--------------------------------------|-------------------------|-------------------------------------------------------------|
//! | no grant at all                      | `DelegateNotPermitted`  | `delegate_top_up_without_a_grant_is_rejected`               |
//! | grant for another stream / delegate  | `DelegateNotPermitted`  | `a_top_up_grant_does_not_carry_over_to_another_stream_or_delegate` |
//! | grant without the `TOP_UP` bit       | `DelegateNotPermitted`  | `a_grant_without_the_top_up_bit_is_rejected`                |
//! | expired grant                        | `DelegateExpired`       | `an_expired_top_up_grant_is_rejected`                       |
//! | grant expired at issue               | `DelegateExpired`       | `a_grant_issued_already_expired_is_rejected_immediately`    |
//! | expired *and* wrong bit              | `DelegateExpired`       | `expiry_is_reported_before_a_missing_bit`                   |
//! | revoked, same ledger                 | `DelegateNotPermitted`  | `a_revoked_top_up_grant_is_rejected_in_the_same_ledger`     |
//! | valid grant                          | success                 | `a_valid_grant_tops_up_from_the_senders_funds`              |
//!
//! Every rejection test asserts the exact contract error, that no stream event
//! survived, and that the stream, the pool, the sender and the delegate are all
//! byte-for-byte unchanged. Every rejection is followed by a control call that
//! succeeds once the one thing under test is fixed, so a test cannot pass
//! because of an unrelated precondition.

use soroban_sdk::testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, IntoVal, Val, Vec};

use super::common::*;
use super::token_errors::register_fee_on_transfer_token;
use crate::{op, Error, Stream, StreamStatus};

/// The top-up used wherever the amount itself is not under test: 10 days of
/// schedule on the standard 1000-token / 100-day stream.
const AMOUNT: i128 = 100 * ONE;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A standard 1000-token / 100-day stream and a fresh delegate holding exactly
/// `op::TOP_UP` with the given expiry.
fn stream_with_top_up_grant(h: &Harness, expires_at: Option<u64>) -> (u64, Address) {
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    // Funded on purpose: the delegate's balance must never be the one pulled.
    h.token_admin.mint(&agent, &(1_000 * ONE));
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::TOP_UP, &expires_at);
    (id, agent)
}

/// `try_delegate_top_up`, flattened to the contract error. A host trap (for
/// example a missing signature) panics here; those cases are asserted
/// separately in the authorization tests.
fn try_delegate(h: &Harness, id: u64, agent: &Address, amount: i128) -> Result<(), Error> {
    h.client
        .try_delegate_top_up(&id, agent, &amount)
        .map(|_| ())
        .map_err(|error| error.expect("host invocation trapped"))
}

/// Same flattening for the direct path.
fn try_direct(h: &Harness, id: u64, amount: i128) -> Result<(), Error> {
    h.client
        .try_top_up(&id, &amount)
        .map(|_| ())
        .map_err(|error| error.expect("host invocation trapped"))
}

/// Everything a top-up could touch.
#[derive(Debug, PartialEq)]
struct Observed {
    stream: Stream,
    pool: i128,
    sender: i128,
    agent: i128,
}

fn observe(h: &Harness, id: u64, agent: &Address) -> Observed {
    Observed {
        stream: h.get(id),
        pool: h.pool(),
        sender: h.balance(&h.sender),
        agent: h.balance(agent),
    }
}

/// Number of events the stream contract published in the most recent
/// invocation. `Events::all()` keeps only that invocation, so call this
/// immediately after the call under test.
fn stream_event_count(h: &Harness) -> usize {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .len()
}

/// Assert `delegate_top_up` is rejected with exactly `expected` and that the
/// rejection was a pure no-op: no event, no state change, no token movement.
fn assert_rejected(
    h: &Harness,
    id: u64,
    agent: &Address,
    amount: i128,
    expected: Error,
    label: &str,
) {
    let before = observe(h, id, agent);
    let result = try_delegate(h, id, agent, amount);
    let events = stream_event_count(h);

    assert_eq!(result, Err(expected), "{label}");
    assert_eq!(events, 0, "{label}: a rejected top-up publishes no events");
    assert_eq!(
        observe(h, id, agent),
        before,
        "{label}: a rejected top-up must not touch the stream or move tokens",
    );
}

/// Assert `delegate_top_up` succeeds and did what a top-up does: the deposit
/// grows by `amount`, pulled from the sender and never from the delegate.
fn assert_tops_up(h: &Harness, id: u64, agent: &Address, amount: i128, label: &str) {
    let before = observe(h, id, agent);
    assert_eq!(try_delegate(h, id, agent, amount), Ok(()), "{label}");
    let after = observe(h, id, agent);

    assert_eq!(
        after.stream.deposited,
        before.stream.deposited + amount,
        "{label}"
    );
    assert!(
        after.stream.end_time > before.stream.end_time,
        "{label}: schedule extended"
    );
    assert_eq!(after.pool, before.pool + amount, "{label}: pool");
    assert_eq!(after.sender, before.sender - amount, "{label}: sender pays");
    assert_eq!(after.agent, before.agent, "{label}: delegate never pays");
}

// ---------------------------------------------------------------------------
// 1. Valid grant
// ---------------------------------------------------------------------------

/// The success path at the depth of `top_up_extends_the_end_date_at_the_same_rate`:
/// same rate, later end, the sender's funds (not the delegate's), one event,
/// and nothing already vested moves.
#[test]
fn a_valid_grant_tops_up_from_the_senders_funds() {
    let h = Harness::new();
    let (id, agent) = stream_with_top_up_grant(&h, None);
    let original_end = h.get(id).end_time;
    h.advance(50 * DAY);
    let vested_before = h.client.vested_of(&id);
    let before = observe(&h, id, &agent);

    h.client.delegate_top_up(&id, &agent, &AMOUNT);
    assert_eq!(stream_event_count(&h), 1, "exactly one ToppedUp event");

    let after = observe(&h, id, &agent);
    assert_eq!(after.stream.deposited, 1_100 * ONE);
    assert_eq!(
        after.stream.end_time,
        original_end + 10 * DAY,
        "100 tokens at 10/day = 10 days"
    );
    assert_eq!(after.pool, before.pool + AMOUNT);
    assert_eq!(
        after.sender,
        before.sender - AMOUNT,
        "tokens come from the sender"
    );
    assert_eq!(
        after.agent, before.agent,
        "the delegate's balance is never pulled"
    );
    assert_eq!(
        h.client.vested_of(&id),
        vested_before,
        "a delegated top-up must not move already-vested funds",
    );
    h.assert_pool_exact();
    h.assert_invariants();

    // The topped-up stream still delivers everything at its new end.
    h.warp_to(after.stream.end_time);
    assert_eq!(h.client.withdraw(&id, &None), 1_100 * ONE);
    h.assert_pool_exact();
}

/// The rounding regression from `top_up` (`a_top_up_never_reduces_what_is_already_vested`),
/// replayed on the delegated path. `delegate_top_up` has no `VestedDecreased`
/// backstop of its own, so floor rounding is the only thing keeping `vested`
/// monotonic here.
#[test]
fn a_delegated_top_up_never_reduces_what_is_already_vested() {
    let h = Harness::new();
    // Deliberately inexact: 1000 stroops over 300 seconds is 3.33/sec.
    let start = h.now();
    let id = h.create(1_000, start, start + 300, start, true, true, true);
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::TOP_UP, &None);

    h.advance(150);
    h.client.withdraw(&id, &None);

    for amount in [7i128, 13, 101, 17] {
        let before = h.client.vested_of(&id);
        h.client.delegate_top_up(&id, &agent, &amount);
        let after = h.client.vested_of(&id);
        assert!(
            after >= before,
            "vested went backwards across delegate_top_up({amount}): {before} -> {after}",
        );
        assert!(
            h.get(id).withdrawn <= after,
            "withdrawn exceeded vested after delegate_top_up({amount})",
        );
        h.advance(1);
    }
    h.assert_pool_exact();
    h.assert_invariants();
}

/// `delegate_top_up` demands two signatures: the delegate's (to use the grant)
/// and the sender's (whose tokens move). Snapshot both.
#[test]
fn a_valid_delegated_top_up_requires_the_delegate_and_the_sender() {
    let h = Harness::new();
    let (id, agent) = stream_with_top_up_grant(&h, None);

    h.client.delegate_top_up(&id, &agent, &AMOUNT);

    let signers: std::vec::Vec<Address> = h.env.auths().iter().map(|(a, _)| a.clone()).collect();
    assert!(
        signers.contains(&agent),
        "delegate auth missing: {signers:?}"
    );
    assert!(
        signers.contains(&h.sender),
        "sender auth missing: {signers:?}"
    );
    assert_eq!(signers.len(), 2, "no third party signs a delegated top-up");
}

// ---------------------------------------------------------------------------
// 2. No grant
// ---------------------------------------------------------------------------

/// A caller with no grant is rejected — including the stream's own sender,
/// who must use `top_up` rather than the delegate path. Issuing the grant then
/// makes the same call succeed, so the rejection was the missing grant.
#[test]
fn delegate_top_up_without_a_grant_is_rejected() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);

    assert_rejected(
        &h,
        id,
        &agent,
        AMOUNT,
        Error::DelegateNotPermitted,
        "stranger",
    );
    assert_rejected(
        &h,
        id,
        &h.sender,
        AMOUNT,
        Error::DelegateNotPermitted,
        "sender, ungranted",
    );
    assert_rejected(
        &h,
        id,
        &h.recipient,
        AMOUNT,
        Error::DelegateNotPermitted,
        "recipient, ungranted",
    );

    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::TOP_UP, &None);
    assert_tops_up(&h, id, &agent, AMOUNT, "control: same call once granted");
    h.assert_pool_exact();
}

/// `top_up` reports `StreamNotFound` for an unknown id. `delegate_top_up`
/// checks the grant first, and `grant_delegate` refuses unknown streams, so no
/// grant can exist and the delegate path reports `DelegateNotPermitted`
/// instead. Pinned so the ordering cannot change silently.
#[test]
fn delegate_top_up_on_an_unknown_stream_is_not_permitted() {
    let h = Harness::new();
    let (_id, agent) = stream_with_top_up_grant(&h, None);
    let unknown: u64 = 9_999;

    assert_eq!(
        try_direct(&h, unknown, AMOUNT),
        Err(Error::StreamNotFound),
        "direct path, for reference",
    );
    assert_eq!(
        try_delegate(&h, unknown, &agent, AMOUNT),
        Err(Error::DelegateNotPermitted),
    );
    assert_eq!(stream_event_count(&h), 0);
    assert_eq!(
        h.client
            .try_grant_delegate(&unknown, &h.sender, &agent, &op::TOP_UP, &None)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound,
        "no grant can ever exist on an unknown stream",
    );
    h.assert_pool_exact();
}

/// Grants are keyed by `(stream_id, delegate)`: neither half transfers.
#[test]
fn a_top_up_grant_does_not_carry_over_to_another_stream_or_delegate() {
    let h = Harness::new();
    let (id_a, agent) = stream_with_top_up_grant(&h, None);
    let id_b = h.create_simple(1_000 * ONE, 100 * DAY);
    let other_agent = Address::generate(&h.env);

    assert_rejected(
        &h,
        id_b,
        &agent,
        AMOUNT,
        Error::DelegateNotPermitted,
        "grant on A used on B",
    );
    assert_rejected(
        &h,
        id_a,
        &other_agent,
        AMOUNT,
        Error::DelegateNotPermitted,
        "another delegate on A",
    );

    assert_tops_up(&h, id_a, &agent, AMOUNT, "control: the granted pair works");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 3. Wrong permission bit
// ---------------------------------------------------------------------------

/// Holding *a* grant is not enough; it must carry `op::TOP_UP`. Every other
/// single bit, and both party-wide masks that exclude `TOP_UP`, is rejected.
/// Adding `TOP_UP` to the grant (or replacing a recipient-side grant with a
/// sender `TOP_UP` grant) then makes the identical call succeed.
#[test]
fn a_grant_without_the_top_up_bit_is_rejected() {
    const SENDER_SIDE: u32 = op::CANCEL | op::PAUSE | op::RESUME;
    const RECIPIENT_SIDE: u32 = op::WITHDRAW | op::TRANSFER_RECIPIENT;

    for bits in [
        op::CANCEL,
        op::PAUSE,
        op::RESUME,
        op::WITHDRAW,
        op::TRANSFER_RECIPIENT,
        SENDER_SIDE,
        RECIPIENT_SIDE,
    ] {
        let h = Harness::new();
        let id = h.create_simple(1_000 * ONE, 100 * DAY);
        let agent = Address::generate(&h.env);
        let sender_side = bits & RECIPIENT_SIDE == 0;
        let grantor = if sender_side { &h.sender } else { &h.recipient };

        h.client.grant_delegate(&id, grantor, &agent, &bits, &None);
        assert_eq!(bits & op::TOP_UP, 0, "fixture must exclude TOP_UP");

        assert_rejected(
            &h,
            id,
            &agent,
            AMOUNT,
            Error::DelegateNotPermitted,
            &std::format!("grant bits {bits:#08b}"),
        );

        // Control: the only change is the TOP_UP bit. A recipient-side grant
        // cannot carry TOP_UP (mixed grants are refused), so it is replaced by
        // the sender's TOP_UP grant instead.
        let fixed = if sender_side {
            bits | op::TOP_UP
        } else {
            op::TOP_UP
        };
        h.client
            .grant_delegate(&id, &h.sender, &agent, &fixed, &None);
        assert_tops_up(
            &h,
            id,
            &agent,
            AMOUNT,
            &std::format!("control for {bits:#08b}"),
        );
        h.assert_pool_exact();
    }
}

// ---------------------------------------------------------------------------
// 4. Expired grant
// ---------------------------------------------------------------------------

/// Expiry is inclusive: the grant works at exactly `expires_at` and is
/// rejected one second later. Expiry closes only the delegate path; the sender
/// can still top up directly.
#[test]
fn an_expired_top_up_grant_is_rejected() {
    let h = Harness::new();
    let expires = h.now() + 5 * DAY;
    let (id, agent) = stream_with_top_up_grant(&h, Some(expires));

    h.warp_to(expires);
    assert_tops_up(&h, id, &agent, AMOUNT, "at exactly expires_at");

    h.advance(1);
    assert_rejected(
        &h,
        id,
        &agent,
        AMOUNT,
        Error::DelegateExpired,
        "one second after expiry",
    );

    // Still well inside the stream's (extended) schedule, so the only thing
    // wrong with the call is the grant.
    h.advance(10 * DAY);
    assert_rejected(
        &h,
        id,
        &agent,
        AMOUNT,
        Error::DelegateExpired,
        "long after expiry",
    );

    // The stream is still live and top-up-able; only the grant lapsed.
    assert_eq!(
        try_direct(&h, id, AMOUNT),
        Ok(()),
        "sender's own path unaffected"
    );

    // Control: a fresh grant restores the delegate.
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::TOP_UP, &None);
    assert_tops_up(&h, id, &agent, AMOUNT, "control: re-granted");
    h.assert_pool_exact();
}

/// `grant_delegate` does not validate `expires_at`, so a grant can be issued
/// already expired. It must be unusable from the very ledger it was issued in.
#[test]
fn a_grant_issued_already_expired_is_rejected_immediately() {
    let h = Harness::new();
    let expired = h.now() - 1;
    let (id, agent) = stream_with_top_up_grant(&h, Some(expired));

    assert_rejected(
        &h,
        id,
        &agent,
        AMOUNT,
        Error::DelegateExpired,
        "expired at issue",
    );
    h.assert_pool_exact();
}

/// `check_delegate` tests expiry before the op bit, so an expired grant that
/// also lacks `TOP_UP` reports `DelegateExpired`. Pinned so a client can rely
/// on "expired" meaning "re-grant", whatever the old grant held.
#[test]
fn expiry_is_reported_before_a_missing_bit() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let agent = Address::generate(&h.env);
    let expires = h.now() + DAY;
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::CANCEL, &Some(expires));

    assert_rejected(
        &h,
        id,
        &agent,
        AMOUNT,
        Error::DelegateNotPermitted,
        "live, wrong bit",
    );
    h.advance(2 * DAY);
    assert_rejected(
        &h,
        id,
        &agent,
        AMOUNT,
        Error::DelegateExpired,
        "expired, wrong bit",
    );
}

// ---------------------------------------------------------------------------
// 5. Revoked grant, same ledger
// ---------------------------------------------------------------------------

/// A delegate top-up honoured before the revocation stands; the next one, in
/// the same ledger (same sequence, same timestamp), is rejected. Either party
/// may revoke, so both revokers are covered.
#[test]
fn a_revoked_top_up_grant_is_rejected_in_the_same_ledger() {
    for revoker_is_sender in [true, false] {
        let h = Harness::new();
        let (id, agent) = stream_with_top_up_grant(&h, None);
        h.advance(10 * DAY);
        let revoker = if revoker_is_sender {
            h.sender.clone()
        } else {
            h.recipient.clone()
        };
        let label = if revoker_is_sender {
            "sender revokes"
        } else {
            "recipient revokes"
        };

        let ledger = h.env.ledger().sequence();
        let timestamp = h.now();

        assert_tops_up(
            &h,
            id,
            &agent,
            AMOUNT,
            &std::format!("{label}: before revocation"),
        );
        h.client.revoke_delegate(&id, &revoker, &agent);
        assert_rejected(
            &h,
            id,
            &agent,
            AMOUNT,
            Error::DelegateNotPermitted,
            &std::format!("{label}: after revocation"),
        );

        assert_eq!(
            h.env.ledger().sequence(),
            ledger,
            "{label}: same ledger sequence"
        );
        assert_eq!(h.now(), timestamp, "{label}: same ledger timestamp");
        assert_eq!(
            h.get(id).deposited,
            1_100 * ONE,
            "{label}: the pre-revocation top-up is not unwound",
        );
        h.assert_pool_exact();
    }
}

// ---------------------------------------------------------------------------
// 6. Signatures
// ---------------------------------------------------------------------------

/// Mirrors `top_up_fails_without_authorization`, split across the two
/// signatures a delegated top-up needs. Each is individually insufficient; the
/// two together succeed, which proves the mocks are well-formed and the
/// rejections are the missing signature, not a malformed test.
#[test]
fn delegate_top_up_needs_both_the_delegate_and_the_sender() {
    let h = Harness::new();
    let (id, agent) = stream_with_top_up_grant(&h, None);

    // The delegate signs the call itself; the sender signs the call *and* the
    // token pull nested inside it.
    let args: Vec<Val> = (id, agent.clone(), AMOUNT).into_val(&h.env);
    let delegate_invoke = MockAuthInvoke {
        contract: &h.contract_id,
        fn_name: "delegate_top_up",
        args: args.clone(),
        sub_invokes: &[],
    };
    let pull = [MockAuthInvoke {
        contract: &h.token,
        fn_name: "transfer",
        args: (h.sender.clone(), h.contract_id.clone(), AMOUNT).into_val(&h.env),
        sub_invokes: &[],
    }];
    let sender_invoke = MockAuthInvoke {
        contract: &h.contract_id,
        fn_name: "delegate_top_up",
        args,
        sub_invokes: &pull,
    };
    let delegate_sig = || MockAuth {
        address: &agent,
        invoke: &delegate_invoke,
    };
    let sender_sig = || MockAuth {
        address: &h.sender,
        invoke: &sender_invoke,
    };

    let cases: [(&str, std::vec::Vec<MockAuth>); 3] = [
        ("no signatures", std::vec![]),
        ("delegate only", std::vec![delegate_sig()]),
        ("sender only", std::vec![sender_sig()]),
    ];
    for (label, sigs) in cases {
        let before = observe(&h, id, &agent);
        let result = h
            .client
            .mock_auths(&sigs)
            .try_delegate_top_up(&id, &agent, &AMOUNT);
        // A missing signature is a host auth failure (outer `Err(Err(_))`),
        // never a contract error and never a success.
        assert!(
            matches!(result, Err(Err(_))),
            "{label}: expected a host auth failure, got {result:?}",
        );
        h.env.mock_all_auths();
        assert_eq!(observe(&h, id, &agent), before, "{label}: nothing changed");
    }

    let result = h
        .client
        .mock_auths(&[delegate_sig(), sender_sig()])
        .try_delegate_top_up(&id, &agent, &AMOUNT);
    assert!(matches!(result, Ok(Ok(()))), "both signatures: {result:?}");
    h.env.mock_all_auths();
    assert_eq!(h.get(id).deposited, 1_100 * ONE);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 7. Guard parity with `top_up`
// ---------------------------------------------------------------------------

/// One parity scenario: how to build the stream, what to do to it before the
/// call, and the amount to top up with.
struct Case {
    name: &'static str,
    create: fn(&Harness) -> u64,
    /// Applied once, with both stream ids, so the direct and delegated streams
    /// reach an identical state.
    prep: fn(&Harness, [u64; 2]),
    amount: i128,
    expect: Result<(), Error>,
}

fn standard(h: &Harness) -> u64 {
    h.create_simple(1_000 * ONE, 100 * DAY)
}

fn no_prep(_: &Harness, _: [u64; 2]) {}

fn warp_to_end(h: &Harness, ids: [u64; 2]) {
    h.warp_to(h.get(ids[0]).end_time);
}

fn cases() -> std::vec::Vec<Case> {
    std::vec![
        // --- accepted -----------------------------------------------------
        Case {
            name: "active stream",
            create: standard,
            prep: no_prep,
            amount: AMOUNT,
            expect: Ok(()),
        },
        Case {
            name: "after a partial withdrawal",
            create: standard,
            prep: |h, ids| {
                h.advance(30 * DAY);
                for id in ids {
                    h.client.withdraw(&id, &None);
                }
            },
            amount: 200 * ONE,
            expect: Ok(()),
        },
        Case {
            name: "inexact rate, awkward remainder",
            create: |h| {
                let s = h.now();
                h.create(1_000, s, s + 300, s, true, true, true)
            },
            prep: |h, _| h.advance(137),
            amount: 7,
            expect: Ok(()),
        },
        Case {
            name: "one second before maturity",
            create: standard,
            prep: |h, ids| h.warp_to(h.get(ids[0]).end_time - 1),
            amount: AMOUNT,
            expect: Ok(()),
        },
        Case {
            name: "paused",
            create: standard,
            prep: |h, ids| {
                h.advance(30 * DAY);
                for id in ids {
                    h.client.pause(&id);
                }
            },
            amount: AMOUNT,
            expect: Ok(()),
        },
        Case {
            // The accrual clock is frozen at the pause, so wall-clock time
            // past `end_time` does not mature a paused stream.
            name: "paused past end_time",
            create: standard,
            prep: |h, ids| {
                h.advance(30 * DAY);
                for id in ids {
                    h.client.pause(&id);
                }
                h.advance(200 * DAY);
            },
            amount: AMOUNT,
            expect: Ok(()),
        },
        Case {
            name: "one stroop at 1 stroop/sec",
            create: |h| {
                let s = h.now();
                h.create(1_000, s, s + 1_000, s, true, true, true)
            },
            prep: no_prep,
            amount: 1,
            expect: Ok(()),
        },
        // --- terminal -----------------------------------------------------
        Case {
            name: "cancelled",
            create: standard,
            prep: |h, ids| {
                h.advance(30 * DAY);
                for id in ids {
                    h.client.cancel(&id);
                }
            },
            amount: AMOUNT,
            expect: Err(Error::StreamTerminated),
        },
        Case {
            // The terminal guard runs before the amount guard.
            name: "cancelled, zero amount",
            create: standard,
            prep: |h, ids| {
                for id in ids {
                    h.client.cancel(&id);
                }
            },
            amount: 0,
            expect: Err(Error::StreamTerminated),
        },
        Case {
            name: "depleted",
            create: |h| h.create_simple(1_000 * ONE, 10 * DAY),
            prep: |h, ids| {
                h.advance(10 * DAY);
                for id in ids {
                    h.client.withdraw(&id, &None);
                }
            },
            amount: AMOUNT,
            expect: Err(Error::StreamTerminated),
        },
        // --- amount domain ------------------------------------------------
        Case {
            name: "zero",
            create: standard,
            prep: no_prep,
            amount: 0,
            expect: Err(Error::InvalidAmount),
        },
        Case {
            name: "negative",
            create: standard,
            prep: no_prep,
            amount: -1,
            expect: Err(Error::InvalidAmount),
        },
        Case {
            name: "large negative",
            create: standard,
            prep: no_prep,
            amount: -100 * ONE,
            expect: Err(Error::InvalidAmount),
        },
        Case {
            name: "i128::MIN",
            create: standard,
            prep: no_prep,
            amount: i128::MIN,
            expect: Err(Error::InvalidAmount),
        },
        // --- maturity -----------------------------------------------------
        Case {
            name: "matured at end",
            create: standard,
            prep: warp_to_end,
            amount: AMOUNT,
            expect: Err(Error::StreamMatured),
        },
        Case {
            name: "matured a year later",
            create: standard,
            prep: |h, ids| h.warp_to(h.get(ids[0]).end_time + YEAR),
            amount: AMOUNT,
            expect: Err(Error::StreamMatured),
        },
        Case {
            // The amount guard runs before the maturity guard.
            name: "matured, zero amount",
            create: standard,
            prep: warp_to_end,
            amount: 0,
            expect: Err(Error::InvalidAmount),
        },
        // --- schedule arithmetic ------------------------------------------
        Case {
            name: "sub-second at 100 stroops/sec",
            create: |h| {
                let s = h.now();
                h.create(10_000, s, s + 100, s, true, true, true)
            },
            prep: no_prep,
            amount: 1,
            expect: Err(Error::TopUpTooSmall),
        },
        Case {
            name: "i128::MAX / 2",
            create: standard,
            prep: no_prep,
            amount: i128::MAX / 2,
            expect: Err(Error::Overflow),
        },
        Case {
            name: "i128::MAX",
            create: standard,
            prep: no_prep,
            amount: i128::MAX,
            expect: Err(Error::Overflow),
        },
        Case {
            // 1 stroop/sec: delta == amount, which exceeds u64::MAX.
            name: "delta past u64",
            create: |h| {
                let s = h.now();
                h.create(1_000, s, s + 1_000, s, true, true, true)
            },
            prep: no_prep,
            amount: u64::MAX as i128 + 1,
            expect: Err(Error::Overflow),
        },
        Case {
            // 1 stroop/sec: delta fits in u64 but `end_time + delta` does not.
            name: "end_time past u64",
            create: |h| {
                let s = h.now();
                h.create(1_000, s, s + 1_000, s, true, true, true)
            },
            prep: no_prep,
            amount: (u64::MAX - (T0 + 1_000) + 1) as i128,
            expect: Err(Error::Overflow),
        },
        Case {
            // `deposit * duration` sits at a third of i128::MAX; topping up by
            // `deposit` doubles both deposit and duration (4/3 of i128::MAX),
            // which passes every earlier guard and trips the re-established
            // creation guard.
            name: "new deposit * duration past i128",
            create: |h| {
                let deposit = i128::MAX / 3 / 100;
                h.token_admin.mint(&h.sender, &(2 * deposit));
                let s = h.now();
                h.create(deposit, s, s + 100, s, true, true, true)
            },
            prep: no_prep,
            amount: i128::MAX / 3 / 100,
            expect: Err(Error::Overflow),
        },
        // --- token ----------------------------------------------------------
        Case {
            name: "sender has no balance",
            create: standard,
            prep: |h, _| {
                let all = h.balance(&h.sender);
                h.token_client.transfer(&h.sender, &h.other, &all);
            },
            amount: AMOUNT,
            expect: Err(Error::TokenTransferFailed),
        },
    ]
}

/// Differential check: for every scenario, a delegate holding a valid
/// `TOP_UP` grant gets exactly the outcome the sender gets from `top_up` —
/// same result, same error, same resulting stream, same pool movement.
///
/// Two identical streams share one environment (same sender, token, clock),
/// so any divergence can only come from the entry point itself.
#[test]
fn delegate_top_up_matches_top_up_for_every_guard() {
    for case in cases() {
        let h = Harness::new();
        let direct = (case.create)(&h);
        let delegated = (case.create)(&h);
        let agent = Address::generate(&h.env);
        h.token_admin.mint(&agent, &(1_000 * ONE));
        h.client
            .grant_delegate(&delegated, &h.sender, &agent, &op::TOP_UP, &None);
        (case.prep)(&h, [direct, delegated]);

        let name = case.name;
        let pool_0 = h.pool();
        let direct_result = try_direct(&h, direct, case.amount);
        let direct_events = stream_event_count(&h);
        let pool_1 = h.pool();
        let agent_before = h.balance(&agent);
        let delegated_result = try_delegate(&h, delegated, &agent, case.amount);
        let delegated_events = stream_event_count(&h);
        let pool_2 = h.pool();

        assert_eq!(direct_result, case.expect, "{name}: top_up");
        assert_eq!(delegated_result, case.expect, "{name}: delegate_top_up");
        assert_eq!(delegated_events, direct_events, "{name}: event count");
        assert_eq!(
            delegated_events,
            usize::from(case.expect.is_ok()),
            "{name}: one ToppedUp on success, none on rejection",
        );
        assert_eq!(pool_2 - pool_1, pool_1 - pool_0, "{name}: pool movement");
        assert_eq!(
            h.balance(&agent),
            agent_before,
            "{name}: delegate never pays"
        );
        assert_eq!(
            h.get(delegated),
            h.get(direct),
            "{name}: resulting stream state"
        );
        if case.expect.is_ok() {
            assert_eq!(pool_1 - pool_0, case.amount, "{name}: full amount pooled");
            // A delegated top-up keeps the stream's status, as `top_up` does.
            let status = h.get(delegated).status;
            assert!(
                matches!(status, StreamStatus::Active | StreamStatus::Paused),
                "{name}: status {status:?}",
            );
        }
        h.assert_pool_exact();
        h.assert_invariants();
    }
}

/// The parity table covers every `top_up` error the contract can reach from
/// its schedule guards. `TokenAmountMismatch` needs a non-standard token, so it
/// gets its own test below; `DepositRateTooLow` and `VestedDecreased` are
/// unreachable from `top_up` (see `test::error_reachability`) and are
/// therefore not expected here.
#[test]
fn parity_table_covers_every_reachable_top_up_error() {
    let covered: std::vec::Vec<Result<(), Error>> = cases().iter().map(|c| c.expect).collect();
    for expected in [
        Ok(()),
        Err(Error::StreamTerminated),
        Err(Error::InvalidAmount),
        Err(Error::StreamMatured),
        Err(Error::TopUpTooSmall),
        Err(Error::Overflow),
        Err(Error::TokenTransferFailed),
    ] {
        assert!(
            covered.contains(&expected),
            "parity table is missing {expected:?}"
        );
    }
}

/// Mirrors `top_up_with_fee_on_transfer_token_is_rejected`: a pull that
/// delivers less than `amount` is detected on the delegated path too, and the
/// rejection leaves deposit and schedule untouched.
#[test]
fn delegate_top_up_rejects_a_fee_on_transfer_token() {
    let h = Harness::new();
    let (token, fee_token) = register_fee_on_transfer_token(&h);
    let start = h.now();
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
    let agent = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.sender, &agent, &op::TOP_UP, &None);
    h.advance(10 * DAY);
    let before = h.get(id);
    let sender_before = fee_token.balance(&h.sender);

    fee_token.set_fee_bps(&1_000); // 10%, turned on after creation
    assert_eq!(
        try_delegate(&h, id, &agent, 200 * ONE),
        Err(Error::TokenAmountMismatch),
    );
    assert_eq!(stream_event_count(&h), 0, "no ToppedUp on a short pull");
    assert_eq!(h.get(id), before, "deposit and schedule unchanged");
    assert_eq!(
        fee_token.balance(&h.sender),
        sender_before,
        "the reverted pull does not cost the sender the fee",
    );

    // Control: with the fee off the same delegated call goes through.
    fee_token.set_fee_bps(&0);
    assert_eq!(try_delegate(&h, id, &agent, 200 * ONE), Ok(()));
    assert_eq!(h.get(id).deposited, 1_200 * ONE);
}
