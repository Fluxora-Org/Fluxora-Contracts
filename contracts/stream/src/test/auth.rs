//! Stage 2 — authorization.
//!
//! Two complementary techniques are used, because each alone is weak:
//!
//! * **Positive:** run under `mock_all_auths` and inspect `env.auths()` to
//!   confirm `require_auth` was invoked on the *expected* address. A missing
//!   `require_auth` would sail past a permissive mock, but it cannot fake an
//!   entry in the auth snapshot.
//! * **Negative:** run with `mock_auths(&[])` so no authorization exists at
//!   all, and confirm the call fails. This deliberately avoids hardcoding
//!   sub-invocation trees, which drift with every signature change and turn
//!   into false failures rather than real coverage.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use super::common::*;

/// The address whose `require_auth` the last invocation actually demanded.
fn required_auth(env: &Env) -> Address {
    let auths = env.auths();
    assert!(!auths.is_empty(), "call required no authorization at all");
    auths[0].0.clone()
}

/// Drop all mocked authorization. Subsequent calls must fail unless they are
/// genuinely permissionless.
fn revoke_all_auths(env: &Env) {
    env.mock_auths(&[]);
}

// --- Sender-authorized operations -----------------------------------------

#[test]
fn create_requires_the_senders_authorization() {
    let h = Harness::new();
    h.create_simple(1_000 * ONE, 100 * DAY);
    assert_eq!(required_auth(&h.env), h.sender);
}

#[test]
fn cancel_pause_resume_and_top_up_require_the_sender() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.top_up(&id, &(100 * ONE));
    assert_eq!(required_auth(&h.env), h.sender, "top_up");

    h.client.pause(&id);
    assert_eq!(required_auth(&h.env), h.sender, "pause");

    h.client.resume(&id);
    assert_eq!(required_auth(&h.env), h.sender, "resume");

    h.client.cancel(&id);
    assert_eq!(required_auth(&h.env), h.sender, "cancel");
}

// --- Recipient-authorized operations --------------------------------------

#[test]
fn withdraw_and_transfer_require_the_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.withdraw(&id, &None);
    assert_eq!(required_auth(&h.env), h.recipient, "withdraw");

    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(required_auth(&h.env), h.recipient, "transfer_recipient");
}

/// After a transfer, authority follows the stream: the new recipient can
/// withdraw and the old one cannot.
#[test]
fn authority_follows_the_recipient_after_a_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.transfer_recipient(&id, &h.other);
    h.advance(10 * DAY);

    h.client.withdraw(&id, &None);
    assert_eq!(required_auth(&h.env), h.other);
}

#[test]
fn batch_withdraw_requires_the_recipient_once() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b]));
    assert_eq!(required_auth(&h.env), h.recipient);
}

/// A batch may not be used to drain someone else's streams by naming yourself
/// as the recipient.
#[test]
fn batch_withdraw_rejects_streams_belonging_to_someone_else() {
    let h = Harness::new();
    let mine = h.create_simple(100 * ONE, 100 * DAY);
    let theirs = h.client.create_stream(
        &h.sender,
        &h.other,
        &h.token,
        &(100 * ONE),
        &h.now(),
        &(h.now() + 100 * DAY),
        &h.now(),
        &true,
        &true,
        &true,
    );
    h.advance(10 * DAY);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[mine, theirs]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, crate::Error::Unauthorized);

    // The whole batch rolled back — no partial drain.
    assert_eq!(h.balance(&h.recipient), 0);
    h.assert_pool_exact();
}

// --- Negative: no authorization at all ------------------------------------

#[test]
#[should_panic(expected = "Unauthorized")]
fn withdraw_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    h.client.withdraw(&id, &None);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn cancel_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    h.client.cancel(&id);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn pause_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    revoke_all_auths(&h.env);
    h.client.pause(&id);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn top_up_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    revoke_all_auths(&h.env);
    h.client.top_up(&id, &(100 * ONE));
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn transfer_recipient_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    revoke_all_auths(&h.env);
    h.client.transfer_recipient(&id, &h.other);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn create_fails_without_authorization() {
    let h = Harness::new();
    revoke_all_auths(&h.env);
    h.create_simple(1_000 * ONE, 100 * DAY);
}

// --- Permissionless by design ---------------------------------------------

/// TTL extension is deliberately unauthenticated: a recipient's claim must
/// never depend on the sender's continued goodwill, and a keeper should not
/// need anyone's permission to pay rent.
#[test]
fn extend_stream_ttl_needs_no_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    revoke_all_auths(&h.env);
    let ledgers = h.client.extend_stream_ttl(&id);
    assert!(ledgers > 0);
    assert!(h.env.auths().is_empty(), "should have required no auth");
}

#[test]
fn batch_extend_ttl_needs_no_authorization() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);

    revoke_all_auths(&h.env);
    assert_eq!(h.client.batch_extend_ttl(&h.ids(&[a, b])), 2);
}

/// Views must be readable by anyone, including with no auth context at all.
#[test]
fn views_need_no_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    assert_eq!(h.client.vested_of(&id), 100 * ONE);
    assert_eq!(h.client.withdrawable_of(&id), 100 * ONE);
    assert_eq!(h.client.refundable_of(&id), 900 * ONE);
    assert_eq!(h.client.stream_count(), 1);
    assert!(h.client.stream_exists(&id));
    let _ = h.client.get_stream(&id);
}

/// Smart accounts (custom `__check_auth`) must work everywhere a classic
/// keypair does. A treasury wrapping `create_stream` in a policy contract that
/// caps spend per period is a headline use case.
#[test]
fn smart_account_addresses_work_as_sender_and_recipient() {
    let h = Harness::new();

    // A contract-typed address stands in for a smart account; under
    // `mock_all_auths` its `__check_auth` is satisfied the same way a
    // keypair's signature would be.
    let smart_sender = Address::generate(&h.env);
    let smart_recipient = Address::generate(&h.env);
    h.token_admin.mint(&smart_sender, &(1_000 * ONE));

    let start = h.now();
    let id = h.client.create_stream(
        &smart_sender,
        &smart_recipient,
        &h.token,
        &(500 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );
    assert_eq!(required_auth(&h.env), smart_sender);

    h.advance(50 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(required_auth(&h.env), smart_recipient);
    assert_eq!(h.balance(&smart_recipient), 250 * ONE);
    h.assert_pool_exact();
}
