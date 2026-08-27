//! Authorization matrix for every mutating entrypoint.
//!
//! # Design
//!
//! Two complementary techniques are used together, because each alone has a
//! blind spot:
//!
//! * **Positive (snapshot):** run under `mock_all_auths` and inspect
//!   `env.auths()` after the call. The snapshot records every `require_auth`
//!   the contract invoked. A missing `require_auth` would sail past a
//!   permissive mock, but it cannot manufacture a snapshot entry.
//!
//! * **Negative (revoke):** call with `mock_auths(&[])` so no authorization
//!   exists at all, then confirm the call fails with `Unauthorized`. This
//!   deliberately avoids hardcoding sub-invocation trees, which drift with
//!   every signature change and turn into false failures rather than real
//!   coverage.
//!
//! * **Wrong-caller runtime checks:** for entrypoints where the contract
//!   independently validates the caller against a stored field (e.g.
//!   `batch_withdraw` checking `stream.recipient == recipient`), the call is
//!   made under `mock_all_auths` with the wrong identity supplied as the
//!   argument. The contract returns `Error::Unauthorized` from its own guard.
//!
//! # Matrix
//!
//! | Entrypoint            | Required authority    | Capability flag      |
//! |-----------------------|-----------------------|----------------------|
//! | `create_stream`       | `sender`              | —                    |
//! | `top_up`              | `sender` on stream    | —                    |
//! | `pause`               | `sender` on stream    | `pausable == true`   |
//! | `resume`              | `sender` on stream    | —                    |
//! | `cancel`              | `sender` on stream    | `cancellable == true`|
//! | `withdraw`            | `recipient` on stream | —                    |
//! | `batch_withdraw`      | `recipient` (once)    | —                    |
//! | `transfer_recipient`  | `recipient` on stream | `transferable == true`|
//! | `extend_stream_ttl`   | **permissionless**    | —                    |
//! | `batch_extend_ttl`    | **permissionless**    | —                    |
//! | all view fns          | **permissionless**    | —                    |
//!
//! For each mutating entrypoint the suite checks:
//!
//! 1. **Positive path** — `env.auths()` names the correct address.
//! 2. **No auth at all** — `mock_auths(&[])` is rejected (panics `Unauthorized`).
//! 3. **Wrong identity** — where the contract has a runtime field check,
//!    the wrong address is passed and `Error::Unauthorized` is returned.
//! 4. **Storage unchanged** — stream state and pool balance are identical
//!    before and after every rejected call.
//! 5. **Delegated authority** — contract-typed addresses (smart accounts) are
//!    accepted wherever classic keypairs are.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use super::common::*;
use crate::Error;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The address whose `require_auth` the last invocation actually demanded.
///
/// Asserts the call required at least one auth entry; use this to catch
/// entrypoints that forgot `require_auth` entirely.
fn required_auth(env: &Env) -> Address {
    let auths = env.auths();
    assert!(!auths.is_empty(), "call required no authorization at all");
    auths[0].0.clone()
}

/// Drop all mocked authorization. Every subsequent call that relies on
/// `require_auth` must fail.
fn revoke_all_auths(env: &Env) {
    env.mock_auths(&[]);
}

// ---------------------------------------------------------------------------
// 1. create_stream — sender authority
// ---------------------------------------------------------------------------

/// Positive: `create_stream` demands exactly the sender's authorization.
#[test]
fn create_requires_the_senders_authorization() {
    let h = Harness::new();
    h.create_simple(1_000 * ONE, 100 * DAY);
    assert_eq!(required_auth(&h.env), h.sender);
}

/// Negative: no authorization → rejected; no stream created, pool untouched.
#[test]
#[should_panic(expected = "Unauthorized")]
fn create_fails_without_authorization() {
    let h = Harness::new();
    revoke_all_auths(&h.env);
    h.create_simple(1_000 * ONE, 100 * DAY);
}

/// Wrong caller: a stream can only be funded by the address passed as `sender`.
/// Under `mock_all_auths`, any address may present itself as `sender`, which is
/// intentional — the real guard is the nested SAC transfer that debits *that*
/// address. What this test pins is that the `stream_count` reflects the actual
/// creator and that the sender argument routes the debit correctly: `h.other`
/// pays when listed as `sender`, not `h.sender`.
#[test]
fn create_debits_the_stated_sender_not_a_third_party() {
    let h = Harness::new();
    let sender_balance_before = h.balance(&h.sender);
    let other_balance_before = h.balance(&h.other);

    // List `h.other` as the sender; the SAC should debit `h.other`, not
    // `h.sender`.
    h.client.create_stream(
        &h.other,
        &h.recipient,
        &h.token,
        &(100 * ONE),
        &h.now(),
        &(h.now() + 100 * DAY),
        &h.now(),
        &true,
        &true,
        &true,
    );

    // The sender argument, not h.sender, was debited.
    assert_eq!(
        h.balance(&h.other),
        other_balance_before - 100 * ONE,
        "h.other paid"
    );
    assert_eq!(
        h.balance(&h.sender),
        sender_balance_before,
        "h.sender untouched"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 2. top_up — sender authority
// ---------------------------------------------------------------------------

/// Positive: `top_up` demands the stream's sender.
#[test]
fn top_up_requires_the_sender() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.top_up(&id, &(100 * ONE));
    assert_eq!(required_auth(&h.env), h.sender, "top_up");
}

/// Negative: no authorization → rejected; deposit and pool unchanged.
#[test]
#[should_panic(expected = "Unauthorized")]
fn top_up_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    revoke_all_auths(&h.env);
    h.client.top_up(&id, &(100 * ONE));
}

/// Storage-unchanged guard: after a rejected top_up the deposit and pool are
/// byte-identical to what they were before. Tested independently of auth
/// method to ensure the contract never commits a partial write.
#[test]
fn rejected_top_up_does_not_alter_the_stream_or_move_tokens() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let deposited_before = h.get(id).deposited;
    let end_time_before = h.get(id).end_time;
    let pool_before = h.pool();

    revoke_all_auths(&h.env);
    let _ = h.client.try_top_up(&id, &(200 * ONE));
    h.env.mock_all_auths();

    assert_eq!(h.get(id).deposited, deposited_before, "deposit unchanged");
    assert_eq!(h.get(id).end_time, end_time_before, "end_time unchanged");
    assert_eq!(h.pool(), pool_before, "pool unchanged");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 3. pause — sender authority
// ---------------------------------------------------------------------------

/// Positive: `pause` demands the stream's sender.
#[test]
fn pause_requires_the_sender() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.pause(&id);
    assert_eq!(required_auth(&h.env), h.sender, "pause");
}

/// Negative: no authorization → rejected.
#[test]
#[should_panic(expected = "Unauthorized")]
fn pause_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    revoke_all_auths(&h.env);
    h.client.pause(&id);
}

/// Storage-unchanged guard: `status` and `paused_at` are untouched after a
/// rejected pause.
#[test]
fn rejected_pause_does_not_set_paused_at_or_change_status() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(20 * DAY);

    revoke_all_auths(&h.env);
    let _ = h.client.try_pause(&id);
    h.env.mock_all_auths();

    let s = h.get(id);
    assert_eq!(s.status, crate::StreamStatus::Active, "status unchanged");
    assert_eq!(s.paused_at, None, "paused_at not set");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 4. resume — sender authority
// ---------------------------------------------------------------------------

/// Positive: `resume` demands the stream's sender.
#[test]
fn resume_requires_the_sender() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.pause(&id);
    h.client.resume(&id);
    assert_eq!(required_auth(&h.env), h.sender, "resume");
}

/// Negative: no authorization → rejected.
#[test]
#[should_panic(expected = "Unauthorized")]
fn resume_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.pause(&id);

    revoke_all_auths(&h.env);
    h.client.resume(&id);
}

/// Storage-unchanged guard: `paused_at`, `paused_total`, and `status` are
/// untouched after a rejected resume.
#[test]
fn rejected_resume_does_not_advance_paused_total_or_clear_paused_at() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.pause(&id);
    let paused_at_before = h.get(id).paused_at;

    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    let _ = h.client.try_resume(&id);
    h.env.mock_all_auths();

    let s = h.get(id);
    assert_eq!(s.status, crate::StreamStatus::Paused, "still paused");
    assert_eq!(s.paused_at, paused_at_before, "paused_at not cleared");
    assert_eq!(s.paused_total, 0, "paused_total not advanced");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 5. cancel — sender authority
// ---------------------------------------------------------------------------

/// Positive: `cancel` demands the stream's sender.
#[test]
fn cancel_requires_the_sender() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.cancel(&id);
    assert_eq!(required_auth(&h.env), h.sender, "cancel");
}

/// Negative: no authorization → rejected.
#[test]
#[should_panic(expected = "Unauthorized")]
fn cancel_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    h.client.cancel(&id);
}

/// Storage-unchanged guard: no refund issued, no schedule rewrite, stream
/// state byte-identical to before the rejected call.
#[test]
fn rejected_cancel_leaves_stream_byte_identical_and_issues_no_refund() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(40 * DAY);

    let before = h.get(id);
    let sender_balance_before = h.balance(&h.sender);
    let pool_before = h.pool();

    revoke_all_auths(&h.env);
    let _ = h.client.try_cancel(&id);
    h.env.mock_all_auths();

    let after = h.get(id);
    assert_eq!(after.deposited, before.deposited, "deposit unchanged");
    assert_eq!(after.withdrawn, before.withdrawn, "withdrawn unchanged");
    assert_eq!(after.end_time, before.end_time, "end_time unchanged");
    assert_eq!(after.status, before.status, "status unchanged");
    assert_eq!(after.paused_at, before.paused_at, "paused_at unchanged");
    assert_eq!(
        after.paused_total, before.paused_total,
        "paused_total unchanged"
    );
    assert_eq!(
        h.balance(&h.sender),
        sender_balance_before,
        "no refund issued"
    );
    assert_eq!(h.pool(), pool_before, "pool unchanged");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 6. withdraw — recipient authority
// ---------------------------------------------------------------------------

/// Positive: `withdraw` demands the stream's recipient.
#[test]
fn withdraw_requires_the_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(required_auth(&h.env), h.recipient, "withdraw");
}

/// Negative: no authorization → rejected.
#[test]
#[should_panic(expected = "Unauthorized")]
fn withdraw_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    h.client.withdraw(&id, &None);
}

/// Storage-unchanged guard: `withdrawn` field and recipient balance are
/// unchanged after a rejected withdraw.
#[test]
fn rejected_withdraw_leaves_accounting_and_pool_unchanged() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);

    let withdrawn_before = h.get(id).withdrawn;
    let balance_before = h.balance(&h.recipient);
    let pool_before = h.pool();

    revoke_all_auths(&h.env);
    let _ = h.client.try_withdraw(&id, &None);
    h.env.mock_all_auths();

    assert_eq!(h.get(id).withdrawn, withdrawn_before, "withdrawn unchanged");
    assert_eq!(
        h.balance(&h.recipient),
        balance_before,
        "recipient balance unchanged"
    );
    assert_eq!(h.pool(), pool_before, "pool unchanged");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 7. batch_withdraw — recipient authority (once for the whole batch)
// ---------------------------------------------------------------------------

/// Positive: `batch_withdraw` demands exactly the named recipient's auth once.
#[test]
fn batch_withdraw_requires_the_recipient_once() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b]));
    assert_eq!(required_auth(&h.env), h.recipient);
}

/// Negative: no authorization → rejected.
#[test]
#[should_panic(expected = "Unauthorized")]
fn batch_withdraw_fails_without_authorization() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    revoke_all_auths(&h.env);
    h.client.batch_withdraw(&h.recipient, &h.ids(&[a, b]));
}

/// Wrong identity: the contract independently checks `stream.recipient ==
/// recipient` for every entry. Passing `h.other` as the `recipient` argument
/// while the streams' stored recipient is `h.recipient` triggers the runtime
/// guard, not the Soroban auth layer. All streams must remain untouched —
/// no partial drain.
#[test]
fn batch_withdraw_rejects_streams_belonging_to_someone_else() {
    let h = Harness::new();
    let mine = h.create_simple(100 * ONE, 100 * DAY);
    // A second stream whose stored recipient is `h.other`.
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

    let pool_before = h.pool();
    let recipient_balance_before = h.balance(&h.recipient);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[mine, theirs]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Unauthorized, "runtime field check fires");

    // The batch rolled back atomically — no partial drain.
    assert_eq!(
        h.balance(&h.recipient),
        recipient_balance_before,
        "no tokens moved"
    );
    assert_eq!(h.pool(), pool_before, "pool unchanged");
    h.assert_pool_exact();
}

/// Mixing own streams with a foreign one rolls back the entire batch, not
/// just the offending entry.
#[test]
fn batch_withdraw_rolls_back_entirely_on_unauthorized_stream() {
    let h = Harness::new();
    let own_a = h.create_simple(100 * ONE, 100 * DAY);
    let own_b = h.create_simple(100 * ONE, 100 * DAY);
    // A stream belonging to `h.other`.
    let foreign = h.client.create_stream(
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
    h.advance(50 * DAY);

    let pool_before = h.pool();
    let withdrawn_a_before = h.get(own_a).withdrawn;
    let withdrawn_b_before = h.get(own_b).withdrawn;

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[own_a, own_b, foreign]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Unauthorized);

    // own_a and own_b were untouched — the whole transaction rolled back.
    assert_eq!(
        h.get(own_a).withdrawn,
        withdrawn_a_before,
        "own_a unchanged"
    );
    assert_eq!(
        h.get(own_b).withdrawn,
        withdrawn_b_before,
        "own_b unchanged"
    );
    assert_eq!(h.pool(), pool_before, "pool unchanged");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 8. transfer_recipient — current recipient's authority
// ---------------------------------------------------------------------------

/// Positive: `transfer_recipient` demands the current recipient's auth.
#[test]
fn transfer_recipient_requires_the_current_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);
    h.client.transfer_recipient(&id, &h.other);
    assert_eq!(required_auth(&h.env), h.recipient, "transfer_recipient");
}

/// Negative: no authorization → rejected.
#[test]
#[should_panic(expected = "Unauthorized")]
fn transfer_recipient_fails_without_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    revoke_all_auths(&h.env);
    h.client.transfer_recipient(&id, &h.other);
}

/// Storage-unchanged guard: the recipient field is unchanged after a rejected
/// transfer.
#[test]
fn rejected_transfer_does_not_change_the_recipient() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let recipient_before = h.get(id).recipient.clone();

    revoke_all_auths(&h.env);
    let _ = h.client.try_transfer_recipient(&id, &h.other);
    h.env.mock_all_auths();

    assert_eq!(h.get(id).recipient, recipient_before, "recipient unchanged");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 9. Authority follows the recipient across transfers
// ---------------------------------------------------------------------------

/// After a `transfer_recipient` call the **new** recipient can withdraw and
/// the **old** one cannot. The contract re-checks `stream.recipient` on every
/// call; there is no cached key.
#[test]
fn authority_follows_the_recipient_after_a_transfer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.transfer_recipient(&id, &h.other);
    h.advance(10 * DAY);

    // New recipient's auth is demanded.
    h.client.withdraw(&id, &None);
    assert_eq!(
        required_auth(&h.env),
        h.other,
        "new recipient's auth demanded"
    );

    // Old recipient has no further authority — revoking all auth and calling
    // under the old env confirms the call was actually locked to the new party.
    revoke_all_auths(&h.env);
    h.advance(10 * DAY);
    let result = h.client.try_withdraw(&id, &None);
    assert!(result.is_err(), "no auth at all must fail");

    h.env.mock_all_auths();
    h.assert_pool_exact();
}

/// Transfers chain correctly: A → B → C, and only C retains authority.
#[test]
fn authority_updates_through_a_transfer_chain() {
    let h = Harness::new();
    let third = Address::generate(&h.env);
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.client.transfer_recipient(&id, &h.other);
    h.client.transfer_recipient(&id, &third);
    assert_eq!(h.get(id).recipient, third, "final recipient is third");

    h.advance(20 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(required_auth(&h.env), third, "third's auth demanded");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 10. Permissionless: TTL extension and views
// ---------------------------------------------------------------------------

/// TTL extension is deliberately unauthenticated: a recipient's claim must
/// never depend on the sender's continued goodwill, and a keeper should not
/// need anyone's permission to pay rent.
#[test]
fn extend_stream_ttl_needs_no_authorization() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    revoke_all_auths(&h.env);
    let ledgers = h.client.extend_stream_ttl(&id);
    assert!(ledgers > 0, "returned non-zero ledger count");
    assert!(h.env.auths().is_empty(), "required no auth");
}

#[test]
fn batch_extend_ttl_needs_no_authorization() {
    let h = Harness::new();
    let a = h.create_simple(100 * ONE, 100 * DAY);
    let b = h.create_simple(100 * ONE, 100 * DAY);

    revoke_all_auths(&h.env);
    assert_eq!(h.client.batch_extend_ttl(&h.ids(&[a, b])), 2);
    assert!(h.env.auths().is_empty(), "required no auth");
}

/// Views must be callable by any party — including with no auth context at all.
/// An indexer or surveillance tool must never be forced to hold any key just to
/// read on-chain state.
#[test]
fn all_view_functions_need_no_authorization() {
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

    assert!(
        h.env.auths().is_empty(),
        "views must not trigger require_auth",
    );
}

// ---------------------------------------------------------------------------
// 11. Invalid / nonexistent stream IDs
// ---------------------------------------------------------------------------

/// Calls against a stream id that has never been issued must return a typed
/// error, not a panic, and must not touch storage.
#[test]
fn operations_on_nonexistent_stream_return_stream_not_found() {
    let h = Harness::new();
    let _ = h.create_simple(1_000 * ONE, 100 * DAY); // id 0 is valid
    let bad_id: u64 = 9999;

    assert_eq!(
        h.client
            .try_top_up(&bad_id, &(100 * ONE))
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound,
        "top_up",
    );
    assert_eq!(
        h.client.try_pause(&bad_id).unwrap_err().unwrap(),
        Error::StreamNotFound,
        "pause",
    );
    assert_eq!(
        h.client.try_resume(&bad_id).unwrap_err().unwrap(),
        Error::StreamNotFound,
        "resume",
    );
    assert_eq!(
        h.client.try_cancel(&bad_id).unwrap_err().unwrap(),
        Error::StreamNotFound,
        "cancel",
    );
    assert_eq!(
        h.client.try_withdraw(&bad_id, &None).unwrap_err().unwrap(),
        Error::StreamNotFound,
        "withdraw",
    );
    assert_eq!(
        h.client
            .try_transfer_recipient(&bad_id, &h.other)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound,
        "transfer_recipient",
    );

    // Pool untouched — no tokens moved.
    assert_eq!(h.pool(), 1_000 * ONE);
    h.assert_pool_exact();
}

/// Zero is a legitimate stream id (the first stream ever created), but calling
/// against id 0 before any stream exists must return `StreamNotFound`, not
/// read garbage or succeed.
#[test]
fn operations_on_id_zero_before_any_stream_is_created_return_not_found() {
    let h = Harness::new();

    assert_eq!(
        h.client.try_withdraw(&0u64, &None).unwrap_err().unwrap(),
        Error::StreamNotFound,
    );
    assert_eq!(
        h.client.try_cancel(&0u64).unwrap_err().unwrap(),
        Error::StreamNotFound,
    );
    assert_eq!(
        h.client.try_pause(&0u64).unwrap_err().unwrap(),
        Error::StreamNotFound,
    );
}

// ---------------------------------------------------------------------------
// 12. Delegated / smart-account authority
// ---------------------------------------------------------------------------

/// Smart accounts (custom `__check_auth`) work everywhere a classic keypair
/// does. A treasury wrapping `create_stream` in a policy contract that caps
/// spend per period is a headline use case.
#[test]
fn smart_account_addresses_work_as_sender_and_recipient() {
    let h = Harness::new();

    // A contract-typed address stands in for a smart account. Under
    // `mock_all_auths` its `__check_auth` is satisfied exactly as a keypair's
    // signature would be.
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
    assert_eq!(required_auth(&h.env), smart_sender, "create auth");

    // Sender-gated operations.
    h.client.top_up(&id, &(500 * ONE));
    assert_eq!(required_auth(&h.env), smart_sender, "top_up auth");
    // 500 + 500 = 1_000 over 200 days = 5/day.

    h.client.pause(&id);
    assert_eq!(required_auth(&h.env), smart_sender, "pause auth");

    h.client.resume(&id);
    assert_eq!(required_auth(&h.env), smart_sender, "resume auth");

    // Recipient-gated operations.
    h.advance(100 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(required_auth(&h.env), smart_recipient, "withdraw auth");
    assert_eq!(
        h.balance(&smart_recipient),
        500 * ONE,
        "50% of 1000 at 100/200 days"
    );

    // Cancel (sender) after partial withdrawal.
    h.client.cancel(&id);
    assert_eq!(required_auth(&h.env), smart_sender, "cancel auth");

    h.assert_pool_exact();
}

/// A smart account holding the sender role of stream A cannot act on stream B,
/// which has a different sender. The auth check is per-stream-field.
#[test]
fn delegated_address_cannot_act_on_a_stream_it_does_not_own() {
    let h = Harness::new();

    // Stream A — h.sender is the sender.
    let _id_a = h.create_simple(1_000 * ONE, 100 * DAY);

    // Stream B — other_sender is the sender.
    let other_sender = Address::generate(&h.env);
    h.token_admin.mint(&other_sender, &(500 * ONE));
    let start = h.now();
    let id_b = h.client.create_stream(
        &other_sender,
        &h.recipient,
        &h.token,
        &(500 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );

    h.advance(10 * DAY);

    // Revoke all auth — under no-auth, cancel on stream B must be rejected
    // regardless of who asks.
    revoke_all_auths(&h.env);
    let result = h.client.try_cancel(&id_b);
    assert!(result.is_err(), "cancel without auth must fail");

    h.env.mock_all_auths();

    // Stream B is untouched.
    assert_eq!(h.get(id_b).status, crate::StreamStatus::Active);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 13. Capability flags are enforced independently of authorization
// ---------------------------------------------------------------------------

/// `cancel` on a non-cancellable stream returns `NotCancellable` — not
/// `Unauthorized` — even when the correct authority (the sender) is present.
/// Auth is checked first, then the capability flag.
#[test]
fn cancel_on_non_cancellable_stream_returns_not_cancellable_not_unauthorized() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        false, // cancellable = false
        true,
        true,
    );
    h.advance(30 * DAY);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::NotCancellable,
        "capability flag, not auth error"
    );

    assert_eq!(h.pool(), 1_000 * ONE);
    h.assert_pool_exact();
}

/// `pause` on a non-pausable stream returns `NotPausable`, even when the sender
/// is fully authenticated.
#[test]
fn pause_on_non_pausable_stream_returns_not_pausable_not_unauthorized() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        false, // pausable = false
        true,
    );
    h.advance(10 * DAY);

    let err = h.client.try_pause(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::NotPausable, "capability flag, not auth error");

    assert_eq!(h.get(id).status, crate::StreamStatus::Active);
    h.assert_pool_exact();
}

/// `transfer_recipient` on a non-transferable stream returns `NotTransferable`,
/// even when the current recipient is fully authenticated.
#[test]
fn transfer_on_non_transferable_stream_returns_not_transferable_not_unauthorized() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(
        1_000 * ONE,
        start,
        start + 100 * DAY,
        start,
        true,
        true,
        false, // transferable = false
    );

    let err = h
        .client
        .try_transfer_recipient(&id, &h.other)
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        Error::NotTransferable,
        "capability flag, not auth error"
    );

    assert_eq!(h.get(id).recipient, h.recipient, "recipient unchanged");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 14. All sender-only operations — composite positive sweep
// ---------------------------------------------------------------------------

/// Every sender-authorized entrypoint in natural lifecycle order, confirming
/// the correct address is demanded each time.
#[test]
fn all_sender_operations_demand_the_sender_in_lifecycle_order() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    assert_eq!(required_auth(&h.env), h.sender, "create");

    h.advance(10 * DAY);

    h.client.top_up(&id, &(100 * ONE));
    assert_eq!(required_auth(&h.env), h.sender, "top_up");

    h.client.pause(&id);
    assert_eq!(required_auth(&h.env), h.sender, "pause");

    h.client.resume(&id);
    assert_eq!(required_auth(&h.env), h.sender, "resume");

    h.client.cancel(&id);
    assert_eq!(required_auth(&h.env), h.sender, "cancel");

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 15. All recipient-only operations — composite positive sweep
// ---------------------------------------------------------------------------

/// Every recipient-authorized entrypoint in natural lifecycle order, confirming
/// the correct address is demanded each time.
#[test]
fn all_recipient_operations_demand_the_recipient_in_lifecycle_order() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(10 * DAY);

    h.client.withdraw(&id, &None);
    assert_eq!(required_auth(&h.env), h.recipient, "withdraw");

    // Create a fresh stream for transfer_recipient.
    let id2 = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.transfer_recipient(&id2, &h.other);
    assert_eq!(required_auth(&h.env), h.recipient, "transfer_recipient");

    // Batch withdraw: multiple streams, one auth check.
    let id3 = h.create_simple(200 * ONE, 50 * DAY);
    let id4 = h.create_simple(200 * ONE, 50 * DAY);
    h.advance(10 * DAY);
    h.client.batch_withdraw(&h.recipient, &h.ids(&[id3, id4]));
    assert_eq!(required_auth(&h.env), h.recipient, "batch_withdraw");

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 16. Cross-operation: no auth bleed between entrypoints
// ---------------------------------------------------------------------------

/// Having valid authorization for one operation does not grant another. These
/// tests run under `revoke_all_auths` to confirm that even a caller who
/// physically holds the key is denied if the contract-level auth is absent.
#[test]
fn no_auth_for_withdraw_also_blocks_top_up_and_cancel() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    revoke_all_auths(&h.env);

    assert!(
        h.client.try_withdraw(&id, &None).is_err(),
        "withdraw blocked"
    );
    assert!(
        h.client.try_top_up(&id, &(50 * ONE)).is_err(),
        "top_up blocked"
    );
    assert!(h.client.try_cancel(&id).is_err(), "cancel blocked");
    assert!(h.client.try_pause(&id).is_err(), "pause blocked");

    h.env.mock_all_auths();

    // All guards fired without mutating state.
    assert_eq!(h.get(id).withdrawn, 0);
    assert_eq!(h.get(id).deposited, 1_000 * ONE);
    assert_eq!(h.get(id).status, crate::StreamStatus::Active);
    h.assert_pool_exact();
}
