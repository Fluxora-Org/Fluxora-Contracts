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

use proptest::prelude::*;
use soroban_sdk::testutils::{Address as _, Events as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal, Val, Vec};

use super::common::*;
use crate::Stream;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallerRole {
    Sender,
    InitialRecipient,
    AlternateRecipient,
    Unrelated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthAction {
    Withdraw,
    BatchWithdraw,
    TransferRecipient,
    Pause,
    Resume,
    Cancel,
    TopUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthSnapshot {
    stream: Stream,
    sender_balance: i128,
    initial_recipient_balance: i128,
    alternate_recipient_balance: i128,
    unrelated_balance: i128,
    pool: i128,
    stream_count: u64,
}

impl CallerRole {
    fn address(self, h: &Harness, unrelated: &Address) -> Address {
        match self {
            CallerRole::Sender => h.sender.clone(),
            CallerRole::InitialRecipient => h.recipient.clone(),
            CallerRole::AlternateRecipient => h.other.clone(),
            CallerRole::Unrelated => unrelated.clone(),
        }
    }
}

impl AuthAction {
    fn expected_authorizer(self, stream: &Stream) -> Address {
        match self {
            AuthAction::Withdraw | AuthAction::BatchWithdraw | AuthAction::TransferRecipient => {
                stream.recipient.clone()
            }
            AuthAction::Pause | AuthAction::Resume | AuthAction::Cancel | AuthAction::TopUp => {
                stream.sender.clone()
            }
        }
    }

    fn transfer_target(self, h: &Harness, stream: &Stream) -> Address {
        if self != AuthAction::TransferRecipient {
            return h.other.clone();
        }
        if stream.recipient == h.other {
            h.recipient.clone()
        } else {
            h.other.clone()
        }
    }

    fn fn_name(self) -> &'static str {
        match self {
            AuthAction::Withdraw => "withdraw",
            AuthAction::BatchWithdraw => "batch_withdraw",
            AuthAction::TransferRecipient => "transfer_recipient",
            AuthAction::Pause => "pause",
            AuthAction::Resume => "resume",
            AuthAction::Cancel => "cancel",
            AuthAction::TopUp => "top_up",
        }
    }

    fn args(self, h: &Harness, stream_id: u64, stream: &Stream, caller: &Address) -> Vec<Val> {
        match self {
            AuthAction::Withdraw => (stream_id, None::<i128>).into_val(&h.env),
            AuthAction::BatchWithdraw => (caller, h.ids(&[stream_id])).into_val(&h.env),
            AuthAction::TransferRecipient => {
                (stream_id, self.transfer_target(h, stream)).into_val(&h.env)
            }
            AuthAction::Pause => (stream_id,).into_val(&h.env),
            AuthAction::Resume => (stream_id,).into_val(&h.env),
            AuthAction::Cancel => (stream_id,).into_val(&h.env),
            AuthAction::TopUp => (stream_id, 10 * ONE).into_val(&h.env),
        }
    }

    fn apply(self, h: &Harness, stream_id: u64, stream: &Stream, caller: &Address) -> bool {
        let invoke = MockAuthInvoke {
            contract: &h.contract_id,
            fn_name: self.fn_name(),
            args: self.args(h, stream_id, stream, caller),
            sub_invokes: &[],
        };
        let auth = MockAuth {
            address: caller,
            invoke: &invoke,
        };
        let auths = [auth];
        let client = h.client.mock_auths(&auths);

        match self {
            AuthAction::Withdraw => matches!(client.try_withdraw(&stream_id, &None), Ok(Ok(_))),
            AuthAction::BatchWithdraw => {
                matches!(
                    client.try_batch_withdraw(caller, &h.ids(&[stream_id])),
                    Ok(Ok(_))
                )
            }
            AuthAction::TransferRecipient => matches!(
                client.try_transfer_recipient(&stream_id, &self.transfer_target(h, stream)),
                Ok(Ok(_))
            ),
            AuthAction::Pause => matches!(client.try_pause(&stream_id), Ok(Ok(_))),
            AuthAction::Resume => matches!(client.try_resume(&stream_id), Ok(Ok(_))),
            AuthAction::Cancel => matches!(client.try_cancel(&stream_id), Ok(Ok(_))),
            AuthAction::TopUp => matches!(client.try_top_up(&stream_id, &(10 * ONE)), Ok(Ok(_))),
        }
    }
}

fn snapshot(h: &Harness, stream_id: u64, unrelated: &Address) -> AuthSnapshot {
    AuthSnapshot {
        stream: h.get(stream_id),
        sender_balance: h.balance(&h.sender),
        initial_recipient_balance: h.balance(&h.recipient),
        alternate_recipient_balance: h.balance(&h.other),
        unrelated_balance: h.balance(unrelated),
        pool: h.pool(),
        stream_count: h.client.stream_count(),
    }
}

// --- Sender-authorized operations -----------------------------------------

#[test]
fn create_requires_the_senders_authorization() {
    let h = Harness::new();
    h.create_simple(1_000 * ONE, 100 * DAY);
    assert_eq!(required_auth(&h.env), h.sender);
}

#[test]
fn cancel_pause_resume_top_up_and_transfer_require_the_sender() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.top_up(&id, &(100 * ONE));
    assert_eq!(required_auth(&h.env), h.sender, "top_up");

    h.client.pause(&id);
    assert_eq!(required_auth(&h.env), h.sender, "pause");

    h.client.resume(&id);
    assert_eq!(required_auth(&h.env), h.sender, "resume");

    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(required_auth(&h.env), h.sender, "transfer_recipient");

    h.client.cancel(&id);
    assert_eq!(required_auth(&h.env), h.sender, "cancel");
}

// --- Recipient-authorized operations --------------------------------------

#[test]
fn withdraw_requires_the_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.withdraw(&id, &None);
    assert_eq!(required_auth(&h.env), h.recipient, "withdraw");
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
fn batch_withdraw_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    h.client.batch_withdraw(&h.recipient, &h.ids(&[id]));
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

// --- Sender-only: top_up cannot be called by recipient or others ---------

/// Verify that a rejected top_up (due to authorization failure) is completely
/// side-effect free: tokens are not transferred and stream state is unchanged.
///
/// `require_auth` raises a host `Abort` trap in the test environment — it does
/// not return a typed `Error` — so we cannot use `try_top_up` to catch the
/// failure as a `Result`. Instead we use `catch_unwind` to absorb the panic,
/// then assert that nothing changed. The host uses `RefCell` borrows that are
/// fully released when the call stack unwinds, so the env remains usable for
/// state-verification reads after the caught panic.
#[test]
fn rejected_top_up_is_side_effect_free() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let stream_before = h.get(id);
    let pool_before = h.pool();
    let sender_balance_before = h.balance(&h.sender);

    // Attempt top_up without any authorization. The host aborts with
    // "Unauthorized" — catch it so we can inspect state afterwards.
    revoke_all_auths(&h.env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.client.top_up(&id, &(100 * ONE));
    }));
    assert!(result.is_err(), "expected top_up to be rejected");

    // Restore mocked auth so state-reading calls below don't also abort.
    h.env.mock_all_auths();

    // Verify no state changed after rejected top_up.
    let stream_after = h.get(id);
    assert_eq!(
        stream_before.deposited, stream_after.deposited,
        "deposited changed"
    );
    assert_eq!(
        stream_before.end_time, stream_after.end_time,
        "end_time changed"
    );
    assert_eq!(
        stream_before.withdrawn, stream_after.withdrawn,
        "withdrawn changed"
    );
    assert_eq!(stream_before.status, stream_after.status, "status changed");

    // Verify balances unchanged.
    assert_eq!(h.pool(), pool_before, "pool balance changed");
    assert_eq!(
        h.balance(&h.sender),
        sender_balance_before,
        "sender balance changed"
    );
    h.assert_pool_exact();
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

prop_compose! {
    fn caller_role_strategy()(n in 0u8..4) -> CallerRole {
        match n {
            0 => CallerRole::Sender,
            1 => CallerRole::InitialRecipient,
            2 => CallerRole::AlternateRecipient,
            _ => CallerRole::Unrelated,
        }
    }
}

prop_compose! {
    fn auth_action_strategy()(n in 0u8..7) -> AuthAction {
        match n {
            0 => AuthAction::Withdraw,
            1 => AuthAction::BatchWithdraw,
            2 => AuthAction::TransferRecipient,
            3 => AuthAction::Pause,
            4 => AuthAction::Resume,
            5 => AuthAction::Cancel,
            _ => AuthAction::TopUp,
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::default())]

    /// Stateful authorization model:
    ///
    /// * sender-authorized actions: `pause`, `resume`, `cancel`, `top_up`
    /// * recipient-authorized actions: `withdraw`, `batch_withdraw`,
    ///   `transfer_recipient`
    /// * after a transfer, "recipient" means the current recipient stored on
    ///   the stream, not the original recipient
    ///
    /// Any rejected call — whether rejected by host auth, by the batch
    /// recipient ownership check, or by a state boundary such as retrying
    /// `pause` — must leave stream state, token balances, stream count, and
    /// emitted contract events untouched.
    #[test]
    fn generated_caller_sequences_enforce_the_state_authorization_predicate(
        steps in prop::collection::vec(
            (
                caller_role_strategy(),
                auth_action_strategy(),
                0u64..20,
            ),
            1..32,
        )
    ) {
        let h = Harness::new();
        let unrelated = Address::generate(&h.env);
        let id = h.create_simple(1_000 * ONE, 100 * DAY);

        for (step, (role, action, days)) in steps.into_iter().enumerate() {
            h.advance(days * DAY);

            let before = snapshot(&h, id, &unrelated);
            let caller = role.address(&h, &unrelated);
            let expected = action.expected_authorizer(&before.stream);
            let caller_is_authorized = caller == expected;

            let accepted = action.apply(&h, id, &before.stream, &caller);

            if accepted {
                prop_assert!(
                    caller_is_authorized,
                    "step {}: {:?} accepted {:?}; expected authorizer was {:?}",
                    step,
                    action,
                    role,
                    expected,
                );
                let required = required_auth(&h.env);
                prop_assert_eq!(
                    required,
                    expected,
                    "step {}: {:?} accepted {:?} but required the wrong address",
                    step,
                    action,
                    role,
                );
            }

            if !accepted {
                let after = snapshot(&h, id, &unrelated);
                prop_assert_eq!(
                    after,
                    before,
                    "step {}: rejected {:?} by {:?} changed state or balances",
                    step,
                    action,
                    role,
                );
                prop_assert!(
                    h.env.events().all().events().is_empty(),
                    "step {}: rejected {:?} by {:?} emitted events",
                    step,
                    action,
                    role,
                );
            }

            h.assert_pool_exact();
        }
    }
}
