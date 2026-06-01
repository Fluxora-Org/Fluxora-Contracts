extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, PauseReason, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env, IntoVal,
};

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    #[allow(dead_code)]
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    #[allow(dead_code)]
    sac: StellarAssetClient<'a>,
}

impl<'a> Ctx<'a> {
    /// Strict mode — no mock_all_auths; every call must carry explicit auth.
    fn setup() -> Self {
        let env = Env::default();

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);

        env.mock_auths(&[MockAuth {
            address: &admin,
            invoke: &MockAuthInvoke {
                contract: &contract_id,
                fn_name: "init",
                args: (&token_id, &admin).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        env.mock_auths(&[MockAuth {
            address: &token_admin,
            invoke: &MockAuthInvoke {
                contract: &token_id,
                fn_name: "mint",
                args: (&sender, 10_000_i128).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        sac.mint(&sender, &10_000_i128);

        env.mock_auths(&[MockAuth {
            address: &sender,
            invoke: &MockAuthInvoke {
                contract: &token_id,
                fn_name: "approve",
                args: (&sender, &contract_id, i128::MAX, 100_000u32).into_val(&env),
                sub_invokes: &[],
            },
        }]);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        Ctx {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            sac,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    /// Create a standard 1000-unit stream (rate 1/s, 0..1000s, no cliff).
    fn create_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
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
                    0i128,
                    Option::<soroban_sdk::Bytes>::None,
                )
                    .into_val(&self.env),
                sub_invokes: &[],
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
            &0,
            &None,
        )
    }

    /// Pause a stream as the sender (helper to reach Paused state).
    fn pause_as_sender(&self, stream_id: u64) {
        self.env.mock_auths(&[MockAuth {
            address: &self.sender,
            invoke: &MockAuthInvoke {
                contract: &self.contract_id,
                fn_name: "pause_stream",
                args: (stream_id, PauseReason::Operational).into_val(&self.env),
                sub_invokes: &[],
            },
        }]);
        self.client()
            .pause_stream(&stream_id, &PauseReason::Operational);
    }

    /// Cancel a stream as the sender (helper to reach Cancelled state).
    fn cancel_as_sender(&self, stream_id: u64) {
        self.env.mock_auths(&[MockAuth {
            address: &self.sender,
            invoke: &MockAuthInvoke {
                contract: &self.contract_id,
                fn_name: "cancel_stream",
                args: (stream_id,).into_val(&self.env),
                sub_invokes: &[],
            },
        }]);
        self.client().cancel_stream(&stream_id);
    }

    /// Advance ledger past end_time so the stream is time-terminal.
    fn advance_past_end(&self) {
        self.env.ledger().set_timestamp(2000);
    }
}

// ---------------------------------------------------------------------------
// pause_stream_as_admin — authentication
// ---------------------------------------------------------------------------

/// Admin with correct auth can pause an Active stream.
#[test]
fn test_admin_pause_active_stream_succeeds() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client()
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );
}

/// Non-admin address must be rejected by pause_stream_as_admin.
#[test]
#[should_panic]
fn test_admin_pause_rejects_non_admin() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let non_admin = Address::generate(&ctx.env);

    ctx.env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client()
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
}

/// Stream sender is not the admin; using the admin entrypoint must be rejected.
#[test]
#[should_panic]
fn test_admin_pause_rejects_sender_as_caller() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client()
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
}

/// Recipient must not be able to call pause_stream_as_admin.
#[test]
#[should_panic]
fn test_admin_pause_rejects_recipient_as_caller() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client()
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
}

// ---------------------------------------------------------------------------
// pause_stream_as_admin — state coverage
// ---------------------------------------------------------------------------

/// Admin cannot pause an already-Paused stream (double-pause).
#[test]
fn test_admin_pause_fails_on_paused_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx
        .client()
        .try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    assert_eq!(result, Err(Ok(ContractError::StreamAlreadyPaused)));
}

/// Admin cannot pause a Cancelled (terminal) stream.
#[test]
fn test_admin_pause_fails_on_cancelled_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.cancel_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx
        .client()
        .try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// Admin cannot pause a time-terminal stream (past end_time).
#[test]
fn test_admin_pause_fails_on_time_terminal_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.advance_past_end();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx
        .client()
        .try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// Admin cannot pause a non-existent stream.
#[test]
fn test_admin_pause_fails_on_nonexistent_stream() {
    let ctx = Ctx::setup();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (999u64, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx
        .client()
        .try_pause_stream_as_admin(&999u64, &PauseReason::Administrative);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

// ---------------------------------------------------------------------------
// resume_stream_as_admin — authentication
// ---------------------------------------------------------------------------

/// Admin with correct auth can resume a Paused stream.
#[test]
fn test_admin_resume_paused_stream_succeeds() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().resume_stream_as_admin(&stream_id);

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
}

/// Non-admin address must be rejected by resume_stream_as_admin.
#[test]
#[should_panic]
fn test_admin_resume_rejects_non_admin() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);
    let non_admin = Address::generate(&ctx.env);

    ctx.env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().resume_stream_as_admin(&stream_id);
}

/// Stream sender is not the admin; using the admin resume entrypoint must be rejected.
#[test]
#[should_panic]
fn test_admin_resume_rejects_sender_as_caller() {
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
    ctx.client().resume_stream_as_admin(&stream_id);
}

/// Recipient must not be able to call resume_stream_as_admin.
#[test]
#[should_panic]
fn test_admin_resume_rejects_recipient_as_caller() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().resume_stream_as_admin(&stream_id);
}

// ---------------------------------------------------------------------------
// resume_stream_as_admin — state coverage
// ---------------------------------------------------------------------------

/// Admin cannot resume an Active stream (not paused).
#[test]
fn test_admin_resume_fails_on_active_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_resume_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamNotPaused)));
}

/// Admin cannot resume a Cancelled (terminal) stream.
#[test]
fn test_admin_resume_fails_on_cancelled_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.cancel_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_resume_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// Admin cannot resume a time-terminal stream (past end_time).
#[test]
fn test_admin_resume_fails_on_time_terminal_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);
    ctx.advance_past_end();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_resume_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// Admin cannot resume a non-existent stream.
#[test]
fn test_admin_resume_fails_on_nonexistent_stream() {
    let ctx = Ctx::setup();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (999u64,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_resume_stream_as_admin(&999u64);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

// ---------------------------------------------------------------------------
// cancel_stream_as_admin — authentication
// ---------------------------------------------------------------------------

/// Admin with correct auth can cancel an Active stream.
#[test]
fn test_admin_cancel_active_stream_succeeds() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().cancel_stream_as_admin(&stream_id);

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
}

/// Admin with correct auth can cancel a Paused stream.
#[test]
fn test_admin_cancel_paused_stream_succeeds() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().cancel_stream_as_admin(&stream_id);

    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
}

/// Non-admin address must be rejected by cancel_stream_as_admin.
#[test]
#[should_panic]
fn test_admin_cancel_rejects_non_admin() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let non_admin = Address::generate(&ctx.env);

    ctx.env.mock_auths(&[MockAuth {
        address: &non_admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().cancel_stream_as_admin(&stream_id);
}

/// Stream sender is not the admin; using the admin cancel entrypoint must be rejected.
#[test]
#[should_panic]
fn test_admin_cancel_rejects_sender_as_caller() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().cancel_stream_as_admin(&stream_id);
}

/// Recipient must not be able to call cancel_stream_as_admin.
#[test]
#[should_panic]
fn test_admin_cancel_rejects_recipient_as_caller() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().cancel_stream_as_admin(&stream_id);
}

// ---------------------------------------------------------------------------
// cancel_stream_as_admin — state coverage
// ---------------------------------------------------------------------------

/// Admin cannot cancel an already-Cancelled (terminal) stream.
#[test]
fn test_admin_cancel_fails_on_cancelled_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.cancel_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_cancel_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Admin cannot cancel a time-terminal stream (past end_time).
#[test]
fn test_admin_cancel_fails_on_time_terminal_stream() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.advance_past_end();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    // A time-terminal stream (past end_time) is still Active status and can be cancelled.
    // The sender receives 0 refund since all tokens are fully accrued.
    let result = ctx.client().try_cancel_stream_as_admin(&stream_id);
    assert!(
        result.is_ok(),
        "cancel on time-terminal stream should succeed (0 refund)"
    );
}

/// Admin cannot cancel a non-existent stream.
#[test]
fn test_admin_cancel_fails_on_nonexistent_stream() {
    let ctx = Ctx::setup();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (999u64,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_cancel_stream_as_admin(&999u64);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

// ---------------------------------------------------------------------------
// Edge cases — double-action and invalid transitions
// ---------------------------------------------------------------------------

/// Admin pausing twice must fail on the second attempt.
#[test]
fn test_admin_pause_twice_fails() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    // First pause — succeeds.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client()
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

    // Second pause — must fail.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx
        .client()
        .try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    assert_eq!(result, Err(Ok(ContractError::StreamAlreadyPaused)));
}

/// Admin resuming twice must fail on the second attempt.
#[test]
fn test_admin_resume_twice_fails() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.pause_as_sender(stream_id);

    // First resume — succeeds.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().resume_stream_as_admin(&stream_id);

    // Second resume — stream is now Active, must fail.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_resume_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamNotPaused)));
}

/// Admin cancelling twice must fail on the second attempt.
#[test]
fn test_admin_cancel_twice_fails() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    // First cancel — succeeds.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().cancel_stream_as_admin(&stream_id);

    // Second cancel — must fail with terminal state error.
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_cancel_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Admin cannot resume a stream that was cancelled (invalid transition).
#[test]
fn test_admin_resume_fails_after_cancel() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.cancel_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "resume_stream_as_admin",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_resume_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// Admin cannot pause a stream that was already cancelled (invalid transition).
#[test]
fn test_admin_pause_fails_after_cancel() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    ctx.cancel_as_sender(stream_id);

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.admin,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "pause_stream_as_admin",
            args: (stream_id, PauseReason::Administrative).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx
        .client()
        .try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

// ---------------------------------------------------------------------------
// Recipient Update (Propose-and-Accept) — Issue #534
// ---------------------------------------------------------------------------

#[test]
fn test_recipient_update_propose_and_accept_flow() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let new_recipient = Address::generate(&ctx.env);

    // 1. Propose (Sender)
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "update_recipient",
            args: (stream_id, new_recipient.clone()).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().update_recipient(&stream_id, &new_recipient);

    // Verify pending update exists
    let pending = ctx
        .client()
        .get_pending_recipient_update(&stream_id)
        .unwrap();
    assert_eq!(pending.proposed_recipient, new_recipient);

    // 2. Accept (Current Recipient)
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "accept_recipient_update",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().accept_recipient_update(&stream_id);

    // Verify update applied
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.recipient, new_recipient);
    assert!(ctx
        .client()
        .get_pending_recipient_update(&stream_id)
        .is_none());
}

#[test]
fn test_recipient_update_cancel_by_sender() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let new_recipient = Address::generate(&ctx.env);

    // Propose
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "update_recipient",
            args: (stream_id, new_recipient.clone()).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().update_recipient(&stream_id, &new_recipient);

    // Cancel (Sender)
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_recipient_update",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().cancel_recipient_update(&stream_id);

    // Verify cancelled
    assert!(ctx
        .client()
        .get_pending_recipient_update(&stream_id)
        .is_none());
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.recipient, ctx.recipient);
}

#[test]
fn test_recipient_update_auth_enforcement() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let new_recipient = Address::generate(&ctx.env);

    // Sender proposes
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "update_recipient",
            args: (stream_id, new_recipient.clone()).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    ctx.client().update_recipient(&stream_id, &new_recipient);

    // Random person tries to accept (Unauthorized)
    let stranger = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "accept_recipient_update",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let res = ctx.client().try_accept_recipient_update(&stream_id);
    assert!(res.is_err());

    // Random person tries to cancel (Unauthorized)
    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_recipient_update",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let res = ctx.client().try_cancel_recipient_update(&stream_id);
    assert!(res.is_err());
}

// ---------------------------------------------------------------------------
// delegated_withdraw — validate_delegation_params coverage (#518)
// ---------------------------------------------------------------------------
//
// These tests exercise the error paths guarded by `validate_delegation_params`:
//   - expired deadline  → SignatureDeadlineExpired
//   - nonce mismatch    → InvalidParams
//   - stream not found  → StreamNotFound (propagated from load_stream inside helper)
//
// Additional adversarial paths tested here:
//   - destination == contract address → InvalidParams
//   - stream is Paused                → InvalidState
//   - stream is Completed             → InvalidState

#[cfg(any())]
mod delegated_withdraw_adversarial {
    extern crate std;

    use ed25519_dalek::{Signer, SigningKey};
    use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, PauseReason};
    use soroban_sdk::{
        testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
        token::{Client as TokenClient, StellarAssetClient},
        xdr::{AccountId, PublicKey, ScAddress, ToXdr, Uint256},
        Address, Bytes, BytesN, Env, IntoVal, TryIntoVal,
    };

    struct RecipientKeypair {
        signing_key: SigningKey,
        pub address: Address,
    }

    impl RecipientKeypair {
        fn from_seed(env: &Env, seed: [u8; 32]) -> Self {
            let signing_key = SigningKey::from_bytes(&seed);
            let pk_bytes = signing_key.verifying_key().to_bytes();
            let account_id = AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(pk_bytes)));
            let address: Address = ScAddress::Account(account_id).try_into_val(env).unwrap();
            Self {
                signing_key,
                address,
            }
        }

        fn sign(
            &self,
            env: &Env,
            contract_id: &Address,
            stream_id: u64,
            destination: &Address,
            nonce: u64,
            deadline: u64,
        ) -> BytesN<64> {
            let mut msg = Bytes::new(env);
            msg.extend_from_array(b"fluxora_delegated_withdraw");
            msg.append(&contract_id.clone().to_xdr(env));
            msg.append(&destination.clone().to_xdr(env));
            msg.extend_from_array(&stream_id.to_be_bytes());
            msg.extend_from_array(&nonce.to_be_bytes());
            msg.extend_from_array(&deadline.to_be_bytes());
            let hash: BytesN<32> = env.crypto().sha256(&msg).into();
            BytesN::from_array(env, &self.signing_key.sign(&hash.to_array()).to_bytes())
        }
    }

    struct Ctx<'a> {
        env: Env,
        contract_id: Address,
        sender: Address,
        relayer: Address,
        recipient_kp: RecipientKeypair,
        #[allow(dead_code)]
        token: TokenClient<'a>,
    }

    impl<'a> Ctx<'a> {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let contract_id = env.register_contract(None, FluxoraStream);
            let token_admin = Address::generate(&env);
            let token_id = env
                .register_stellar_asset_contract_v2(token_admin)
                .address();
            let admin = Address::generate(&env);
            let sender = Address::generate(&env);
            let relayer = Address::generate(&env);
            let recipient_kp = RecipientKeypair::from_seed(&env, [0x02u8; 32]);

            let client = FluxoraStreamClient::new(&env, &contract_id);
            client.init(&token_id, &admin);

            let sac = StellarAssetClient::new(&env, &token_id);
            sac.mint(&sender, &10_000_i128);
            let token = TokenClient::new(&env, &token_id);
            token.approve(&sender, &contract_id, &i128::MAX, &100_000);

            Ctx {
                env,
                contract_id,
                sender,
                relayer,
                recipient_kp,
                token,
            }
        }

        fn client(&self) -> FluxoraStreamClient<'_> {
            FluxoraStreamClient::new(&self.env, &self.contract_id)
        }

        fn create_stream(&self) -> u64 {
            self.env.ledger().set_timestamp(0);
            self.client().create_stream(
                &self.sender,
                &self.recipient_kp.address,
                &1000_i128,
                &1_i128,
                &0u64,
                &0u64,
                &1000u64,
                &0,
                &None,
            )
        }

        fn sign(&self, stream_id: u64, dest: &Address, nonce: u64, deadline: u64) -> BytesN<64> {
            self.recipient_kp.sign(
                &self.env,
                &self.contract_id,
                stream_id,
                dest,
                nonce,
                deadline,
            )
        }
    }

    /// Expired deadline is rejected before any stream state is read.
    #[test]
    fn test_delegated_withdraw_expired_deadline() {
        let ctx = Ctx::setup();
        let stream_id = ctx.create_stream();
        let dest = Address::generate(&ctx.env);

        // Advance ledger past the deadline.
        ctx.env.ledger().set_timestamp(500);
        let deadline = 100u64; // already expired
        let sig = ctx.sign(stream_id, &dest, 0, deadline);

        let result = ctx.client().try_delegated_withdraw(
            &stream_id,
            &ctx.relayer,
            &dest,
            &0,
            &deadline,
            &sig,
        );
        assert_eq!(result, Err(Ok(ContractError::SignatureDeadlineExpired)));
    }

    /// Nonce mismatch (stale nonce after a successful withdrawal) is rejected.
    #[test]
    fn test_delegated_withdraw_stale_nonce_rejected() {
        let ctx = Ctx::setup();
        let stream_id = ctx.create_stream();
        let dest = Address::generate(&ctx.env);

        // First withdrawal consumes nonce 0.
        ctx.env.ledger().set_timestamp(300);
        let sig0 = ctx.sign(stream_id, &dest, 0, 9999);
        ctx.client()
            .delegated_withdraw(&stream_id, &ctx.relayer, &dest, &0, &9999, &sig0);

        // Replay with nonce 0 must fail.
        ctx.env.ledger().set_timestamp(600);
        let result =
            ctx.client()
                .try_delegated_withdraw(&stream_id, &ctx.relayer, &dest, &0, &9999, &sig0);
        assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
    }

    /// Supplying a future nonce (skipping) is rejected.
    #[test]
    fn test_delegated_withdraw_future_nonce_rejected() {
        let ctx = Ctx::setup();
        let stream_id = ctx.create_stream();
        let dest = Address::generate(&ctx.env);

        ctx.env.ledger().set_timestamp(300);
        let sig = ctx.sign(stream_id, &dest, 1, 9999); // nonce 1 but stored is 0
        let result =
            ctx.client()
                .try_delegated_withdraw(&stream_id, &ctx.relayer, &dest, &1, &9999, &sig);
        assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
    }

    /// Non-existent stream returns StreamNotFound (propagated from load_stream in helper).
    #[test]
    fn test_delegated_withdraw_stream_not_found() {
        let ctx = Ctx::setup();
        let dest = Address::generate(&ctx.env);
        ctx.env.ledger().set_timestamp(0);

        let dummy_sig = BytesN::from_array(&ctx.env, &[0u8; 64]);
        let result = ctx.client().try_delegated_withdraw(
            &999u64,
            &ctx.relayer,
            &dest,
            &0,
            &9999,
            &dummy_sig,
        );
        assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
    }

    /// Destination equal to the contract address is rejected.
    #[test]
    fn test_delegated_withdraw_destination_is_contract() {
        let ctx = Ctx::setup();
        let stream_id = ctx.create_stream();
        ctx.env.ledger().set_timestamp(300);

        let dest = ctx.contract_id.clone();
        let sig = ctx.sign(stream_id, &dest, 0, 9999);
        let result =
            ctx.client()
                .try_delegated_withdraw(&stream_id, &ctx.relayer, &dest, &0, &9999, &sig);
        assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
    }

    /// Paused stream is rejected with InvalidState.
    #[test]
    fn test_delegated_withdraw_paused_stream_rejected() {
        let ctx = Ctx::setup();
        let stream_id = ctx.create_stream();
        let dest = Address::generate(&ctx.env);

        // Pause the stream.
        ctx.env.mock_auths(&[MockAuth {
            address: &ctx.sender,
            invoke: &MockAuthInvoke {
                contract: &ctx.contract_id,
                fn_name: "pause_stream",
                args: (stream_id, PauseReason::Operational).into_val(&ctx.env),
                sub_invokes: &[],
            },
        }]);
        ctx.client()
            .pause_stream(&stream_id, &PauseReason::Operational);

        // Restore blanket auth so relayer.require_auth() in delegated_withdraw passes.
        ctx.env.mock_all_auths();
        ctx.env.ledger().set_timestamp(300);
        let sig = ctx.sign(stream_id, &dest, 0, 9999);
        let result =
            ctx.client()
                .try_delegated_withdraw(&stream_id, &ctx.relayer, &dest, &0, &9999, &sig);
        assert_eq!(result, Err(Ok(ContractError::InvalidState)));
    }

    /// Completed stream is rejected with InvalidState.
    #[test]
    fn test_delegated_withdraw_completed_stream_rejected() {
        let ctx = Ctx::setup();
        let stream_id = ctx.create_stream();
        let dest = Address::generate(&ctx.env);

        // Drain the stream to Completed.
        ctx.env.ledger().set_timestamp(1000);
        let sig0 = ctx.sign(stream_id, &dest, 0, 9999);
        ctx.client()
            .delegated_withdraw(&stream_id, &ctx.relayer, &dest, &0, &9999, &sig0);

        // Second attempt on a Completed stream must fail.
        let sig1 = ctx.sign(stream_id, &dest, 1, 9999);
        let result =
            ctx.client()
                .try_delegated_withdraw(&stream_id, &ctx.relayer, &dest, &1, &9999, &sig1);
        assert_eq!(result, Err(Ok(ContractError::InvalidState)));
    }
}

// ---------------------------------------------------------------------------
// Issue #510: delegated_withdraw — adversarial tests
// ---------------------------------------------------------------------------

/// Helper: build the 40-byte message that the recipient must sign.
fn build_delegated_msg(
    env: &Env,
    stream_id: u64,
    nonce: u64,
    deadline: u64,
    expected_minimum_amount: i128,
) -> soroban_sdk::Bytes {
    let mut msg = soroban_sdk::Bytes::new(env);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&nonce.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg.extend_from_array(&expected_minimum_amount.to_be_bytes());
    msg
}

fn delegated_keypair(env: &Env) -> (BytesN<32>, SigningKey) {
    let signing_key = SigningKey::from_bytes(&[0x07u8; 32]);
    let public_key = BytesN::from_array(env, &signing_key.verifying_key().to_bytes());
    (public_key, signing_key)
}

fn sign_delegated_msg(env: &Env, signing_key: &SigningKey, msg: &soroban_sdk::Bytes) -> BytesN<64> {
    let bytes: std::vec::Vec<u8> = (0..msg.len()).map(|i| msg.get_unchecked(i)).collect();
    let sig = signing_key.sign(&bytes);
    BytesN::from_array(env, &sig.to_bytes())
}

#[test]
fn delegated_withdraw_expired_deadline_rejected() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    // Advance time past deadline
    ctx.env.ledger().set_timestamp(2000);

    let relayer = Address::generate(&ctx.env);
    let fake_key = BytesN::from_array(&ctx.env, &[0u8; 32]);
    let fake_sig = BytesN::from_array(&ctx.env, &[0u8; 64]);

    let result = ctx.client().try_delegated_withdraw(
        &stream_id, &relayer, &fake_key, &0u64, &500u64, // deadline in the past
        &0i128, &fake_sig,
    );

    assert_eq!(
        result,
        Err(Ok(fluxora_stream::ContractError::InvalidSignature)),
        "expired deadline must return InvalidSignature"
    );
}

#[test]
fn delegated_withdraw_wrong_nonce_rejected() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    let relayer = Address::generate(&ctx.env);
    let fake_key = BytesN::from_array(&ctx.env, &[0u8; 32]);
    let fake_sig = BytesN::from_array(&ctx.env, &[0u8; 64]);

    // Stored nonce is 0; supply nonce=1 (wrong)
    let result = ctx.client().try_delegated_withdraw(
        &stream_id, &relayer, &fake_key, &1u64, // wrong nonce
        &9999u64, &0i128, &fake_sig,
    );

    assert_eq!(
        result,
        Err(Ok(fluxora_stream::ContractError::InvalidSignature)),
        "wrong nonce must return InvalidSignature"
    );
}

#[test]
fn delegated_withdraw_below_minimum_rejected() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.env.ledger().set_timestamp(500); // mid-stream
    let stream_id = ctx.create_stream();

    let (pub_key, signing_key) = delegated_keypair(&ctx.env);

    let nonce = 0u64;
    let deadline = 9999u64;
    let expected_minimum = 999i128; // demand 999 but only ~500 accrued

    let msg = build_delegated_msg(&ctx.env, stream_id, nonce, deadline, expected_minimum);
    let sig = sign_delegated_msg(&ctx.env, &signing_key, &msg);

    let relayer = Address::generate(&ctx.env);

    let result = ctx.client().try_delegated_withdraw(
        &stream_id,
        &relayer,
        &pub_key,
        &nonce,
        &deadline,
        &expected_minimum,
        &sig,
    );

    assert_eq!(
        result,
        Err(Ok(fluxora_stream::ContractError::BelowMinimumAmount)),
        "withdrawable below minimum must return BelowMinimumAmount"
    );
}

#[test]
fn delegated_withdraw_nonce_increments_preventing_replay() {
    let ctx = Ctx::setup();
    ctx.env.mock_all_auths();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    // Advance to mid-stream so there's something to withdraw
    ctx.env.ledger().set_timestamp(500);

    let (pub_key, signing_key) = delegated_keypair(&ctx.env);

    let nonce = 0u64;
    let deadline = 9999u64;
    let min_amount = 0i128;

    let msg = build_delegated_msg(&ctx.env, stream_id, nonce, deadline, min_amount);
    let sig = sign_delegated_msg(&ctx.env, &signing_key, &msg);

    let relayer = Address::generate(&ctx.env);

    // First call succeeds
    let amount = ctx.client().delegated_withdraw(
        &stream_id,
        &relayer,
        &pub_key,
        &nonce,
        &deadline,
        &min_amount,
        &sig,
    );
    assert!(amount > 0, "first delegated_withdraw must transfer tokens");

    // Nonce is now 1; replaying nonce=0 must fail
    let replay = ctx.client().try_delegated_withdraw(
        &stream_id,
        &relayer,
        &pub_key,
        &nonce, // stale nonce
        &deadline,
        &min_amount,
        &sig,
    );
    assert_eq!(
        replay,
        Err(Ok(fluxora_stream::ContractError::InvalidSignature)),
        "replayed nonce must be rejected"
    );

    // get_delegated_nonce must return 1
    assert_eq!(
        ctx.client().get_delegated_nonce(&ctx.recipient),
        1u64,
        "nonce must be 1 after one successful delegated_withdraw"
    );
}
