//! Withdrawal from a stream whose token contract has been destroyed (#1883).
//!
//! A stream's `token` is a contract address, and a contract address can stop
//! resolving: the token was deployed on a different network, its state was
//! rolled back, or its code is simply gone. The stream record survives, the
//! pool it backs is untouched, and the recipient is still owed — but every
//! payout route through that token is dead. That is the state this module puts
//! the contract in, and the behaviour it pins.
//!
//! # Why it needs a test of its own
//!
//! `token_transfer` funnels every token sub-invocation failure into one of two
//! stable, typed stream-level errors, and `test::token_errors` covers the first
//! of them — [`Error::TokenTransferFailed`] — thoroughly: a token that panics,
//! a token that takes a fee, a pool drained by clawback. The second,
//! [`Error::TokenMissing`], is the one that file explicitly leaves unexercised:
//!
//! > In the test host all sub-invocation failures are contract-typed, so this
//! > variant is verified through its discriminant value only.
//!
//! Nor does any existing test stage a stream whose *token address has no code*.
//! So nothing proves the promise that matters most when the token is gone: that
//! the payout fails **closed** — a typed stream error, no tokens moved, no
//! accounting drift, no half-written stream, no event — rather than paying
//! against a dead route, trapping untyped, or stranding the stream.
//!
//! # Which of the two categories the failure lands in
//!
//! The same underlying host error, `Error(Context, InvalidAction)` — "no
//! contract at that address" — is observed differently by the two execution
//! modes:
//!
//! * **native test host** — the host error comes back *as a value* from the
//!   `try_transfer` sub-invocation, which `token_transfer` classifies as a
//!   failed transfer: [`Error::TokenTransferFailed`];
//! * **WASM (testnet/mainnet)** — the identical host error traps the VM before
//!   it can be returned, so the `Err(InvokeError::Abort)` arm fires and the
//!   contract returns [`Error::TokenMissing`].
//!
//! `test::token_errors` documents the same split for a panicking token. Both
//! outcomes are what the contract promises, so the tests below assert the
//! *set* — while
//! [`a_destroyed_token_is_classified_by_this_host_as_transfer_failed`] pins the
//! concrete value observed here, so a change in classification cannot pass
//! unnoticed. What must never happen, and is asserted explicitly, is the
//! failure escaping as an untyped host error.
//!
//! # How the destroyed token is staged
//!
//! The SDK test host can remove a *user* storage entry but has no counterpart
//! for a contract *instance*: once `env.register(...)` has run there is no API
//! to un-deploy it. Nor can the stream be born pointing at a code-less address,
//! because `create_stream` pulls the deposit *through* the token before writing
//! the entry — creation itself is the first thing that would fail.
//!
//! So the fixture stages the record the contract will actually load: create the
//! stream normally, then rewrite the stored record's `token` field to a
//! contract address that was never deployed. `Address::generate` produces
//! exactly such an address — an `ScAddress::Contract` with no instance entry,
//! so every invocation of it fails. This is the technique `test::missing` uses
//! to fabricate a deleted stream record, and it is honest about what it stages:
//! the condition under test is "the token this stream names has no code", and
//! that is precisely the record the contract loads. Every assertion below is
//! about how the contract behaves towards that record, which does not depend on
//! how the record came to be.
//!
//! The deposit stays in the *real* token's pool, so this is the dangerous
//! shape of the failure: money is committed and owed while its payout route is
//! gone.
//!
//! # Scope
//!
//! This is the outbound surface — `withdraw`, `batch_withdraw`, and `cancel`'s
//! refund leg — which all route through `token_transfer` and therefore inherit
//! its classification. The deposit legs (`create_stream`, `top_up`) read the
//! token's balance in `pull_deposit` *outside* `token_transfer`, so a destroyed
//! token there surfaces as an untyped host abort instead of a stream-level
//! error. That asymmetry is real and deliberately left to its own change rather
//! than frozen into an assertion here.

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::Address;

use super::common::*;
use crate::{storage, Error, StreamStatus};

/// Point `stream_id`'s stored record at a contract address with no deployed
/// code, and return that address.
///
/// See the module docs for why this is the only way to stage a destroyed token
/// in the test host.
fn destroy_stream_token(h: &Harness, stream_id: u64) -> Address {
    let dead = Address::generate(&h.env);
    h.env.as_contract(&h.contract_id, || {
        let mut stream = storage::load_stream(&h.env, stream_id)
            .expect("stream must exist before its token is destroyed");
        stream.token = dead.clone();
        storage::save_stream(&h.env, stream_id, &stream);
    });
    dead
}

/// Restore `stream_id`'s token to the harness's live token contract — the
/// inverse of [`destroy_stream_token`].
fn restore_stream_token(h: &Harness, stream_id: u64) {
    h.env.as_contract(&h.contract_id, || {
        let mut stream = storage::load_stream(&h.env, stream_id).unwrap();
        stream.token = h.token.clone();
        storage::save_stream(&h.env, stream_id, &stream);
    });
}

/// Unwrap a generated `try_*` result into the *typed* contract error it failed
/// with.
///
/// A generated `try_*` returns
/// `Result<Result<Ret, decode error>, Result<contract error, InvokeError>>`:
/// the outer `Err`'s payload is what carries the contract's own outcome, `Ok`
/// for a typed [`Error`] and `Err` for a host trap. The decode error is
/// `soroban_sdk::Error` for primitive returns and `ConversionError` for `()`,
/// hence the extra generic parameters.
///
/// Panics with a distinct message for the two outcomes that would break the
/// contract's promise:
///
/// * the call succeeded when the payout route was gone;
/// * the failure escaped the contract as an untyped host error
///   (`InvokeError`) instead of being classified.
fn typed_error<T: core::fmt::Debug, D: core::fmt::Debug, I: core::fmt::Debug>(
    result: Result<Result<T, D>, Result<Error, I>>,
    what: &str,
) -> Error {
    match result {
        Err(Ok(err)) => err,
        Ok(Ok(value)) => {
            panic!("{what}: expected a typed failure, but the call succeeded: {value:?}")
        }
        Ok(Err(decode)) => panic!("{what}: the return value failed to decode: {decode:?}"),
        Err(Err(invoke)) => panic!("{what}: failure escaped the contract untyped: {invoke:?}"),
    }
}

/// The two categories `token_transfer` maps a dead payout route onto. Which one
/// is seen depends on the execution mode, not on what went wrong — see the
/// module docs.
fn is_token_error(err: Error) -> bool {
    matches!(err, Error::TokenMissing | Error::TokenTransferFailed)
}

/// Every stream-contract event emitted by the most recent invocation.
fn stream_events(h: &Harness) -> usize {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .len()
}

/// `withdraw` against a stream whose token contract is gone fails closed: a
/// typed token error, no payout, and a stream that is byte-for-byte unchanged.
#[test]
fn withdraw_fails_closed_when_the_token_contract_is_gone() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);
    destroy_stream_token(&h, id);

    let before = h.get(id);
    let pool_before = h.pool();
    let recipient_before = h.balance(&h.recipient);
    let owed = h.client.withdrawable_of(&id);
    assert!(owed > 0, "the fixture must owe the recipient something");

    let err = typed_error(h.client.try_withdraw(&id, &None), "withdraw");
    assert!(
        is_token_error(err),
        "a token address with no code must surface as a stream-level token \
         error, got {err:?}",
    );

    // Rollback: the failed call left no partial write behind.
    assert_eq!(h.get(id), before, "a failed withdraw must not mutate state");
    assert_eq!(h.get(id).withdrawn, 0, "nothing may be marked withdrawn");
    assert_eq!(h.get(id).status, StreamStatus::Active);
    assert_eq!(h.pool(), pool_before, "no tokens may leave the pool");
    assert_eq!(
        h.balance(&h.recipient),
        recipient_before,
        "the recipient must not be paid"
    );
    assert_eq!(
        h.client.withdrawable_of(&id),
        owed,
        "the amount owed must be unchanged by the failed payout",
    );
}

/// Pin the concrete classification this host produces, so the set-shaped
/// assertions above cannot quietly become the only contract.
///
/// Native test host: `Error(Context, InvalidAction)` is returned as a value by
/// `try_transfer`, so `token_transfer` reports [`Error::TokenTransferFailed`].
/// Under WASM the same error traps and the same condition is reported as
/// [`Error::TokenMissing`] — see the module docs and `test::token_errors`.
#[test]
fn a_destroyed_token_is_classified_by_this_host_as_transfer_failed() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);
    destroy_stream_token(&h, id);

    let err = typed_error(h.client.try_withdraw(&id, &None), "withdraw");
    assert_eq!(
        err,
        Error::TokenTransferFailed,
        "the native host returns the host error as a value, which \
         token_transfer classifies as a failed transfer",
    );

    // The error a client decodes is Fluxora's own discriminant, never the
    // host's — that is the point of classifying instead of forwarding.
    assert_eq!(err as u32, 25);
}

/// A reverted withdraw must emit zero stream-contract events: a `Withdrawn`
/// event would tell an indexer the recipient was paid when nothing moved.
#[test]
fn a_failed_withdraw_against_a_destroyed_token_emits_no_events() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);
    destroy_stream_token(&h, id);

    assert_eq!(stream_events(&h), 0, "setup must leave no events pending");

    let _ = h.client.try_withdraw(&id, &None);
    assert_eq!(
        stream_events(&h),
        0,
        "the token failure must roll back the whole invocation, \
         including the Withdrawn event",
    );
}

/// The failure is confined to the payout route. The view layer still answers
/// what an off-chain client needs — how much is owed, and in what state the
/// stream is — which is what makes the error actionable rather than a black
/// hole.
#[test]
fn the_view_layer_still_answers_for_a_stream_with_a_destroyed_token() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(25 * DAY);
    destroy_stream_token(&h, id);

    assert!(h.client.stream_exists(&id));
    assert_eq!(h.client.vested_of(&id), 250 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 250 * ONE);
    assert_eq!(h.client.refundable_of(&id), 750 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Active);
}

/// The failure is recoverable, and this is the strongest evidence that it
/// rolled back *completely*: once the stream points at a live token again, the
/// full outstanding amount pays out. A half-applied `withdrawn` increment would
/// show up here as a shortfall.
#[test]
fn the_pending_payout_survives_restoring_the_token() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);

    destroy_stream_token(&h, id);
    let owed = h.client.withdrawable_of(&id);
    let err = typed_error(h.client.try_withdraw(&id, &None), "withdraw");
    assert!(is_token_error(err));

    restore_stream_token(&h, id);

    let paid = h.client.withdraw(&id, &None);
    assert_eq!(paid, owed, "the retry must pay everything still owed");
    assert_eq!(h.balance(&h.recipient), owed);
    assert_eq!(h.get(id).withdrawn, owed);
    h.assert_pool_exact();
}

/// The size of the request does not change the outcome: a partial withdrawal
/// against a destroyed token fails exactly as the full one does, and the
/// stream's accounting stays at zero.
#[test]
fn a_partial_withdrawal_request_fails_the_same_way() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);
    destroy_stream_token(&h, id);

    let before = h.get(id);
    for requested in [ONE, 100 * ONE, h.client.withdrawable_of(&id)] {
        let err = typed_error(
            h.client.try_withdraw(&id, &Some(requested)),
            "partial withdraw",
        );
        assert!(is_token_error(err));
    }

    assert_eq!(h.get(id), before);
    assert_eq!(h.balance(&h.recipient), 0);
}

/// `batch_withdraw` is all-or-nothing, and that must hold when it is one
/// member's token that is gone: the healthy streams in the same batch must not
/// be paid either, because a partial application of a batch the contract
/// reports as failed is exactly what the all-or-nothing rule forbids.
#[test]
fn batch_withdraw_pays_nobody_when_one_streams_token_is_destroyed() {
    let h = Harness::new();
    let doomed = h.create_simple(1_000 * ONE, 100 * DAY);
    let healthy = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);

    destroy_stream_token(&h, doomed);

    let doomed_before = h.get(doomed);
    let healthy_before = h.get(healthy);
    let recipient_before = h.balance(&h.recipient);
    let pool_before = h.pool();

    let err = typed_error(
        h.client
            .try_batch_withdraw(&h.recipient, &h.ids(&[healthy, doomed])),
        "batch_withdraw",
    );
    assert!(is_token_error(err));

    assert_eq!(
        h.get(healthy),
        healthy_before,
        "the healthy stream in a failed batch must not be paid",
    );
    assert_eq!(h.get(doomed), doomed_before);
    assert_eq!(h.balance(&h.recipient), recipient_before);
    assert_eq!(h.pool(), pool_before);
    assert_eq!(stream_events(&h), 0, "a failed batch must emit nothing");
}

/// The refund leg of `cancel` shares the payout route, so it fails closed the
/// same way — and the rollback matters more here than anywhere: an unbounded
/// `Cancelled` write would settle a stream (collapsing its schedule, freezing
/// its deposit) on the strength of a refund that never happened.
#[test]
fn cancels_refund_leg_fails_closed_on_a_destroyed_token() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);
    destroy_stream_token(&h, id);

    let before = h.get(id);
    let sender_before = h.balance(&h.sender);
    assert!(h.client.refundable_of(&id) > 0, "a refund must be due");

    let err = typed_error(h.client.try_cancel(&id), "cancel");
    assert!(is_token_error(err));

    assert_eq!(
        h.get(id).status,
        StreamStatus::Active,
        "a cancel whose refund failed must not settle the stream",
    );
    assert_eq!(h.get(id), before, "the schedule must be untouched");
    assert_eq!(h.balance(&h.sender), sender_before, "no refund may move");
    assert_eq!(stream_events(&h), 0);
}
