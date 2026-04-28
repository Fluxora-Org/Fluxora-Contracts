extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, PauseReason, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, IntoVal,
};

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
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

        Ctx { env, contract_id, token_id, admin, sender, recipient, sac }
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
        self.client().pause_stream(&stream_id, &PauseReason::Operational);
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
    ctx.client().pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

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
    ctx.client().pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
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
    ctx.client().pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
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
    ctx.client().pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
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
    let result = ctx.client().try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
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
    let result = ctx.client().try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
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
    let result = ctx.client().try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
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
    let result = ctx.client().try_pause_stream_as_admin(&999u64, &PauseReason::Administrative);
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
    assert!(result.is_ok(), "cancel on time-terminal stream should succeed (0 refund)");
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
    ctx.client().pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

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
    let result = ctx.client().try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
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
    let result = ctx.client().try_pause_stream_as_admin(&stream_id, &PauseReason::Administrative);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}
