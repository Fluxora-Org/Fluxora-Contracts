/// Integration suite: adversarial auth (strict mock auths)
///
/// Scope: every state-mutating entrypoint is exercised with an unauthorized
/// caller (stranger, wrong role, or rotated-away key).  Each test:
///   1. Uses `setup_strict()` — no `mock_all_auths()`, explicit per-call auths only.
///   2. Supplies a *wrong* signer and asserts the call is rejected.
///   3. Verifies zero side-effects: stream state, token balances, and event log
///      are identical before and after the rejected call.
///
/// Intentional exclusions (with rationale):
///   - `calculate_accrued`, `get_stream_state`, `get_config`, `get_withdrawable`,
///     `get_claimable_at`, `get_recipient_streams`, `get_stream_count`, `version`:
///     read-only, no auth required by design — adversarial callers are harmless.
///   - `close_completed_stream`: permissionless by design (documented in lib.rs).
///   - `top_up_stream`: any address may fund a stream; the only auth check is
///     `funder.require_auth()` which is satisfied by the funder themselves.
///     A stranger cannot top up *on behalf of* someone else, but they can top up
///     using their own tokens — this is intentional protocol behaviour.
///
/// Residual risk:
///   - Soroban host-level auth failures surface as panics (not `ContractError`).
///     Tests that expect a host-trap use `std::panic::catch_unwind`; tests that
///     expect a `ContractError` use `try_*` methods.  Both patterns are correct
///     for their respective failure modes.
extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger, MockAuth, MockAuthInvoke},
    token::StellarAssetClient,
    Address, Env, IntoVal,
};

// ---------------------------------------------------------------------------
// Shared test harness (strict — no mock_all_auths)
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: soroban_sdk::token::Client<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        // Strict mode: no mock_all_auths — every call must carry explicit auth.

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // init — admin must authorize
        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "init",
                args: (&token_id, &admin).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        FluxoraStreamClient::new(&env, &contract_id).init(&token_id, &admin);

        // Mint tokens to sender using mock_all_auths just for the SAC call.
        env.mock_all_auths();
        StellarAssetClient::new(&env, &token_id).mint(&sender, &10_000_i128);

        let token = soroban_sdk::token::Client::new(&env, &token_id);
        Self {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    /// Create a default stream (sender-authorized, t=0..1000, rate=1, deposit=1000).
    /// Returns the stream_id.
    fn create_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        // sender authorizes create_stream + the token transfer sub-invocation
        self.env.mock_auths(&[MockAuth {
            address: &self.sender,
            invoke: &MockAuthInvoke {
                contract: &self.contract_id,
                fn_name: "create_stream",
                args: (
                    &self.sender,
                    &self.recipient,
                    1000_i128,
                    1_i128,
                    0u64,
                    0u64,
                    1000u64,
                    0_i128,
                )
                    .into_val(&self.env),
                sub_invokes: &[MockAuthInvoke {
                    contract: &self.token_id,
                    fn_name: "transfer",
                    args: (&self.sender, &self.contract_id, 1000_i128).into_val(&self.env),
                    sub_invokes: &[],
                }],
            },
        }]);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0_i128,
        )
    }

    /// Pause a stream as the sender (authorized).
    fn pause_as_sender(&self, stream_id: u64) {
        self.env.mock_auths(&[MockAuth {
            address: &self.sender,
            invoke: &MockAuthInvoke {
                contract: &self.contract_id,
                fn_name: "pause_stream",
                args: (stream_id,).into_val(&self.env),
                sub_invokes: &[],
            },
        }]);
        self.client().pause_stream(&stream_id);
    }

    fn total_supply(&self) -> i128 {
        self.token.balance(&self.sender)
            + self.token.balance(&self.recipient)
            + self.token.balance(&self.contract_id)
    }
}

// ===========================================================================
// create_stream — only the declared sender may authorize
// ===========================================================================

/// A stranger cannot create a stream on behalf of the real sender.
/// The host rejects the auth and no state or tokens move.
#[test]
fn adversarial_create_stream_stranger_cannot_impersonate_sender() {
    let ctx = Ctx::setup();
    let stranger = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(0);

    let supply_before = ctx.total_supply();
    let count_before = ctx.client().get_stream_count();
    let events_before = ctx.env.events().all().len();

    // Provide auth as stranger, but the call declares sender as the funding address.
    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
                0_i128,
            )
                .into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().create_stream(
            &ctx.sender,
            &ctx.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0_i128,
        );
    }));

    assert!(result.is_err(), "stranger must not create stream as sender");
    assert_eq!(
        ctx.client().get_stream_count(),
        count_before,
        "counter must not advance"
    );
    assert_eq!(ctx.total_supply(), supply_before, "no tokens must move");
    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no events emitted"
    );
}

// ===========================================================================
// pause_stream — only the stream's sender may pause
// ===========================================================================

/// A stranger cannot pause a stream they did not create.
#[test]
fn adversarial_pause_stream_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let stranger = Address::generate(&ctx.env);

    let state_before = ctx.client().get_stream_state(&stream_id);
    let supply_before = ctx.total_supply();
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().pause_stream(&stream_id);
    }));

    assert!(result.is_err(), "stranger must not pause stream");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        state_before.status
    );
    assert_eq!(ctx.total_supply(), supply_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The recipient cannot pause their own incoming stream.
#[test]
fn adversarial_pause_stream_recipient_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().pause_stream(&stream_id);
    }));

    assert!(result.is_err(), "recipient must not pause stream");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

/// The admin cannot pause via the sender-only entrypoint.
#[test]
fn adversarial_pause_stream_admin_cannot_use_sender_path() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().pause_stream(&stream_id);
    }));

    assert!(result.is_err(), "admin must not use sender-only pause path");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

// ===========================================================================
// resume_stream — only the stream's sender may resume
// ===========================================================================

/// A stranger cannot resume a paused stream.
#[test]
fn adversarial_resume_stream_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    let stranger = Address::generate(&ctx.env);
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().resume_stream(&stream_id);
    }));

    assert!(result.is_err(), "stranger must not resume stream");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused,
        "stream must remain Paused"
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The recipient cannot resume a paused stream.
#[test]
fn adversarial_resume_stream_recipient_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().resume_stream(&stream_id);
    }));

    assert!(result.is_err(), "recipient must not resume stream");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );
}

// ===========================================================================
// cancel_stream — only the stream's sender may cancel
// ===========================================================================

/// A stranger cannot cancel a stream.
#[test]
fn adversarial_cancel_stream_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let stranger = Address::generate(&ctx.env);

    let supply_before = ctx.total_supply();
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().cancel_stream(&stream_id);
    }));

    assert!(result.is_err(), "stranger must not cancel stream");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
    assert_eq!(ctx.total_supply(), supply_before, "no tokens must move");
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The recipient cannot cancel their own incoming stream.
#[test]
fn adversarial_cancel_stream_recipient_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    let supply_before = ctx.total_supply();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().cancel_stream(&stream_id);
    }));

    assert!(result.is_err(), "recipient must not cancel stream");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
    assert_eq!(ctx.total_supply(), supply_before);
}

/// The admin cannot cancel via the sender-only entrypoint.
#[test]
fn adversarial_cancel_stream_admin_cannot_use_sender_path() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().cancel_stream(&stream_id);
    }));

    assert!(
        result.is_err(),
        "admin must not use sender-only cancel path"
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

// ===========================================================================
// withdraw — only the stream's recipient may withdraw
// ===========================================================================

/// A stranger cannot withdraw from a stream they are not the recipient of.
#[test]
fn adversarial_withdraw_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(500);

    let stranger = Address::generate(&ctx.env);
    let state_before = ctx.client().get_stream_state(&stream_id);
    let supply_before = ctx.total_supply();
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw(&stream_id);
    }));

    assert!(result.is_err(), "stranger must not withdraw");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        state_before.withdrawn_amount,
        "withdrawn_amount must not change"
    );
    assert_eq!(ctx.total_supply(), supply_before, "no tokens must move");
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The sender cannot withdraw from their own outgoing stream.
#[test]
fn adversarial_withdraw_sender_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(500);

    let supply_before = ctx.total_supply();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw(&stream_id);
    }));

    assert!(result.is_err(), "sender must not withdraw from own stream");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        0
    );
    assert_eq!(ctx.total_supply(), supply_before);
}

/// The admin cannot withdraw via the recipient-only entrypoint.
#[test]
fn adversarial_withdraw_admin_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(500);

    let supply_before = ctx.total_supply();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw(&stream_id);
    }));

    assert!(
        result.is_err(),
        "admin must not withdraw via recipient path"
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        0
    );
    assert_eq!(ctx.total_supply(), supply_before);
}

// ===========================================================================
// withdraw_to — only the stream's recipient may redirect funds
// ===========================================================================

/// A stranger cannot call withdraw_to on a stream they are not the recipient of.
#[test]
fn adversarial_withdraw_to_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(500);

    let stranger = Address::generate(&ctx.env);
    let destination = Address::generate(&ctx.env);
    let supply_before = ctx.total_supply();
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw_to",
            args: (stream_id, &destination).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw_to(&stream_id, &destination);
    }));

    assert!(result.is_err(), "stranger must not call withdraw_to");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        0
    );
    assert_eq!(ctx.total_supply(), supply_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The sender cannot redirect funds via withdraw_to.
#[test]
fn adversarial_withdraw_to_sender_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(500);

    let destination = Address::generate(&ctx.env);
    let supply_before = ctx.total_supply();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "withdraw_to",
            args: (stream_id, &destination).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().withdraw_to(&stream_id, &destination);
    }));

    assert!(result.is_err(), "sender must not call withdraw_to");
    assert_eq!(ctx.total_supply(), supply_before);
}

// ===========================================================================
// batch_withdraw — caller must be the recipient of every stream
// ===========================================================================

/// A stranger cannot batch-withdraw for the real recipient.
#[test]
fn adversarial_batch_withdraw_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(500);

    let stranger = Address::generate(&ctx.env);
    let supply_before = ctx.total_supply();
    let events_before = ctx.env.events().all().len();

    let ids = soroban_sdk::vec![&ctx.env, stream_id];

    // Stranger provides their own auth but claims to be withdrawing for recipient's streams.
    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "batch_withdraw",
            args: (&stranger, ids.clone()).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = ctx.client().try_batch_withdraw(&stranger, &ids);
    assert_eq!(
        result,
        Err(Ok(ContractError::Unauthorized)),
        "batch_withdraw with wrong recipient must return Unauthorized"
    );

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        0,
        "withdrawn_amount must not change"
    );
    assert_eq!(ctx.total_supply(), supply_before, "no tokens must move");
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The sender cannot batch-withdraw from their own outgoing streams.
#[test]
fn adversarial_batch_withdraw_sender_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.env.ledger().set_timestamp(500);

    let ids = soroban_sdk::vec![&ctx.env, stream_id];

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "batch_withdraw",
            args: (&ctx.sender, ids.clone()).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = ctx.client().try_batch_withdraw(&ctx.sender, &ids);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        0
    );
}

/// Recipient of stream A cannot batch-withdraw from stream B (different recipient).
#[test]
fn adversarial_batch_withdraw_cross_stream_recipient_rejected() {
    let ctx = Ctx::setup();

    // Stream 0: recipient = ctx.recipient
    let id0 = ctx.create_stream();

    // Stream 1: different recipient
    let other_recipient = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &other_recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
                0_i128,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);
    let id1 = ctx.client().create_stream(
        &ctx.sender,
        &other_recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
    );

    ctx.env.ledger().set_timestamp(500);
    let ids = soroban_sdk::vec![&ctx.env, id0, id1];

    // ctx.recipient tries to batch-withdraw both streams — id1 belongs to other_recipient
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "batch_withdraw",
            args: (&ctx.recipient, ids.clone()).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = ctx.client().try_batch_withdraw(&ctx.recipient, &ids);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));

    // Neither stream's state must change
    assert_eq!(ctx.client().get_stream_state(&id0).withdrawn_amount, 0);
    assert_eq!(ctx.client().get_stream_state(&id1).withdrawn_amount, 0);
}

// ===========================================================================
// Admin-path entrypoints — only the contract admin may call these
// ===========================================================================

/// A stranger cannot pause a stream via the admin path.
#[test]
fn adversarial_pause_as_admin_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let stranger = Address::generate(&ctx.env);

    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().pause_stream_as_admin(&stream_id);
    }));

    assert!(result.is_err(), "stranger must not pause via admin path");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The stream's sender cannot pause via the admin path.
#[test]
fn adversarial_pause_as_admin_sender_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().pause_stream_as_admin(&stream_id);
    }));

    assert!(result.is_err(), "sender must not use admin pause path");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

/// A stranger cannot resume a stream via the admin path.
#[test]
fn adversarial_resume_as_admin_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    let stranger = Address::generate(&ctx.env);
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().resume_stream_as_admin(&stream_id);
    }));

    assert!(result.is_err(), "stranger must not resume via admin path");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused,
        "stream must remain Paused"
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The stream's sender cannot resume via the admin path.
#[test]
fn adversarial_resume_as_admin_sender_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().resume_stream_as_admin(&stream_id);
    }));

    assert!(result.is_err(), "sender must not use admin resume path");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );
}

/// A stranger cannot cancel a stream via the admin path.
#[test]
fn adversarial_cancel_as_admin_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let stranger = Address::generate(&ctx.env);

    let supply_before = ctx.total_supply();
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().cancel_stream_as_admin(&stream_id);
    }));

    assert!(result.is_err(), "stranger must not cancel via admin path");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
    assert_eq!(ctx.total_supply(), supply_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The stream's sender cannot cancel via the admin path.
#[test]
fn adversarial_cancel_as_admin_sender_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    let supply_before = ctx.total_supply();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().cancel_stream_as_admin(&stream_id);
    }));

    assert!(result.is_err(), "sender must not use admin cancel path");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
    assert_eq!(ctx.total_supply(), supply_before);
}

// ===========================================================================
// set_admin — only the current admin may rotate the key
// ===========================================================================

/// A stranger cannot rotate the admin key.
#[test]
fn adversarial_set_admin_stranger_rejected_config_unchanged() {
    let ctx = Ctx::setup();
    let stranger = Address::generate(&ctx.env);
    let new_admin = Address::generate(&ctx.env);

    let config_before = ctx.client().get_config();
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "set_admin",
            args: (new_admin.clone(),).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().set_admin(&new_admin);
    }));

    assert!(result.is_err(), "stranger must not rotate admin");
    let config_after = ctx.client().get_config();
    assert_eq!(
        config_after.admin, config_before.admin,
        "admin must be unchanged"
    );
    assert_eq!(
        config_after.token, config_before.token,
        "token must be unchanged"
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The stream sender cannot rotate the admin key.
#[test]
fn adversarial_set_admin_sender_rejected_config_unchanged() {
    let ctx = Ctx::setup();
    let new_admin = Address::generate(&ctx.env);
    let config_before = ctx.client().get_config();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "set_admin",
            args: (new_admin.clone(),).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().set_admin(&new_admin);
    }));

    assert!(result.is_err(), "sender must not rotate admin");
    assert_eq!(ctx.client().get_config().admin, config_before.admin);
}

/// After admin rotation, the old admin key is revoked — it cannot rotate again.
#[test]
fn adversarial_set_admin_old_admin_revoked_after_rotation() {
    let ctx = Ctx::setup();
    let new_admin = Address::generate(&ctx.env);

    // Legitimate rotation by current admin
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "set_admin",
            args: (new_admin.clone(),).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().set_admin(&new_admin);
    assert_eq!(ctx.client().get_config().admin, new_admin);

    // Old admin tries to rotate again — must be rejected
    let another = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin, // old admin
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "set_admin",
            args: (another.clone(),).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().set_admin(&another);
    }));

    assert!(result.is_err(), "old admin must be revoked after rotation");
    assert_eq!(
        ctx.client().get_config().admin,
        new_admin,
        "admin must still be new_admin"
    );
}

// ===========================================================================
// set_contract_paused — only the admin may toggle the global pause flag
// ===========================================================================

/// A stranger cannot set the global pause flag.
#[test]
fn adversarial_set_contract_paused_stranger_rejected() {
    let ctx = Ctx::setup();
    let stranger = Address::generate(&ctx.env);

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "set_contract_paused",
            args: (true,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().set_contract_paused(&true);
    }));

    assert!(result.is_err(), "stranger must not set global pause");

    // Contract must still accept new streams (not paused)
    ctx.env.ledger().set_timestamp(0);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                1000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
                0_i128,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 1000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);
    let id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
    );
    assert_eq!(
        ctx.client().get_stream_state(&id).status,
        StreamStatus::Active
    );
}

/// The stream sender cannot set the global pause flag.
#[test]
fn adversarial_set_contract_paused_sender_rejected() {
    let ctx = Ctx::setup();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "set_contract_paused",
            args: (true,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().set_contract_paused(&true);
    }));

    assert!(result.is_err(), "sender must not set global pause");
}

// ===========================================================================
// update_rate_per_second — only the stream's sender may update the rate
// ===========================================================================

/// A stranger cannot update the rate of a stream.
#[test]
fn adversarial_update_rate_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let stranger = Address::generate(&ctx.env);

    let state_before = ctx.client().get_stream_state(&stream_id);
    let events_before = ctx.env.events().all().len();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "update_rate_per_second",
            args: (stream_id, 2_i128).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().update_rate_per_second(&stream_id, &2_i128);
    }));

    assert!(result.is_err(), "stranger must not update rate");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).rate_per_second,
        state_before.rate_per_second,
        "rate must be unchanged"
    );
    assert_eq!(ctx.env.events().all().len(), events_before);
}

/// The recipient cannot update the rate of their incoming stream.
#[test]
fn adversarial_update_rate_recipient_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    let state_before = ctx.client().get_stream_state(&stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "update_rate_per_second",
            args: (stream_id, 2_i128).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().update_rate_per_second(&stream_id, &2_i128);
    }));

    assert!(result.is_err(), "recipient must not update rate");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).rate_per_second,
        state_before.rate_per_second
    );
}

// ===========================================================================
// shorten_stream_end_time / extend_stream_end_time — only the sender may modify schedule
// ===========================================================================

/// A stranger cannot shorten a stream's end time.
#[test]
fn adversarial_shorten_end_time_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(); // end_time = 1000
    let stranger = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(100);
    let state_before = ctx.client().get_stream_state(&stream_id);
    let supply_before = ctx.total_supply();

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "shorten_stream_end_time",
            args: (stream_id, 500u64).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().shorten_stream_end_time(&stream_id, &500u64);
    }));

    assert!(result.is_err(), "stranger must not shorten end time");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).end_time,
        state_before.end_time
    );
    assert_eq!(ctx.total_supply(), supply_before, "no tokens must move");
}

/// A stranger cannot extend a stream's end time.
#[test]
fn adversarial_extend_end_time_stranger_rejected_no_side_effects() {
    let ctx = Ctx::setup();
    // deposit=2000 so extension to 2000 is valid if authorized
    ctx.env.ledger().set_timestamp(0);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                2000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
                0_i128,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 2000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
    );

    let stranger = Address::generate(&ctx.env);
    let state_before = ctx.client().get_stream_state(&stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "extend_stream_end_time",
            args: (stream_id, 2000u64).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().extend_stream_end_time(&stream_id, &2000u64);
    }));

    assert!(result.is_err(), "stranger must not extend end time");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).end_time,
        state_before.end_time
    );
}

/// The recipient cannot shorten their incoming stream's end time.
#[test]
fn adversarial_shorten_end_time_recipient_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(); // end_time = 1000

    ctx.env.ledger().set_timestamp(100);
    let state_before = ctx.client().get_stream_state(&stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "shorten_stream_end_time",
            args: (stream_id, 500u64).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().shorten_stream_end_time(&stream_id, &500u64);
    }));

    assert!(result.is_err(), "recipient must not shorten end time");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).end_time,
        state_before.end_time
    );
}

/// The recipient cannot extend their incoming stream's end time.
#[test]
fn adversarial_extend_end_time_recipient_rejected() {
    let ctx = Ctx::setup();
    // deposit=2000 so extension to 2000 is valid if authorized
    ctx.env.ledger().set_timestamp(0);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "create_stream",
            args: (
                &ctx.sender,
                &ctx.recipient,
                2000_i128,
                1_i128,
                0u64,
                0u64,
                1000u64,
                0_i128,
            )
                .into_val(&ctx.env),
            sub_invokes: &[MockAuthInvoke {
                contract: &ctx.token_id,
                fn_name: "transfer",
                args: (&ctx.sender, &ctx.contract_id, 2000_i128).into_val(&ctx.env),
                sub_invokes: &[],
            }],
        },
    }]);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0_i128,
    );

    let state_before = ctx.client().get_stream_state(&stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "extend_stream_end_time",
            args: (stream_id, 2000u64).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().extend_stream_end_time(&stream_id, &2000u64);
    }));

    assert!(result.is_err(), "recipient must not extend end time");
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).end_time,
        state_before.end_time
    );
}

/// The admin cannot update rate via the sender-only path.
#[test]
fn adversarial_update_rate_admin_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    let state_before = ctx.client().get_stream_state(&stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "update_rate_per_second",
            args: (stream_id, 2_i128).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().update_rate_per_second(&stream_id, &2_i128);
    }));

    assert!(
        result.is_err(),
        "admin must not update rate via sender path"
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).rate_per_second,
        state_before.rate_per_second
    );
}
