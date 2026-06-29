//! State-machine coverage for the two-step recipient rotation flow:
//! propose (`update_recipient`) → accept (`accept_recipient_update`) or
//! cancel (`cancel_recipient_update`).

extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, IntoVal,
};

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
}

impl<'a> Ctx<'a> {
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
            sender,
            recipient,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

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
            &0i128,
            &None,
        )
    }

    fn propose_recipient_update(&self, stream_id: u64, new_recipient: &Address) {
        self.env.mock_auths(&[MockAuth {
            address: &self.sender,
            invoke: &MockAuthInvoke {
                contract: &self.contract_id,
                fn_name: "update_recipient",
                args: (stream_id, new_recipient.clone()).into_val(&self.env),
                sub_invokes: &[],
            },
        }]);
        self.client()
            .update_recipient(&stream_id, new_recipient);
    }
}

/// `accept_recipient_update` without a pending proposal must fail closed.
#[test]
fn accept_without_pending_errors() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "accept_recipient_update",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_accept_recipient_update(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// `cancel_recipient_update` clears the pending update and allows re-propose.
#[test]
fn cancel_clears_pending_and_allows_repropose() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let first = Address::generate(&ctx.env);
    let second = Address::generate(&ctx.env);

    ctx.propose_recipient_update(stream_id, &first);

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
    assert!(ctx
        .client()
        .get_pending_recipient_update(&stream_id)
        .is_none());

    ctx.propose_recipient_update(stream_id, &second);
    let pending = ctx
        .client()
        .get_pending_recipient_update(&stream_id)
        .unwrap();
    assert_eq!(pending.proposed_recipient, second);
}

/// Acceptance moves the stream between recipient indexes (old removed, new added).
#[test]
fn acceptance_updates_recipient_indexes() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let new_recipient = Address::generate(&ctx.env);

    let before_old = ctx.client().get_recipient_streams(&ctx.recipient);
    assert!(before_old.contains(stream_id));
    assert!(!ctx.client().get_recipient_streams(&new_recipient).contains(stream_id));

    ctx.propose_recipient_update(stream_id, &new_recipient);

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

    assert!(!ctx.client().get_recipient_streams(&ctx.recipient).contains(stream_id));
    assert!(ctx.client().get_recipient_streams(&new_recipient).contains(stream_id));

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.recipient, new_recipient);
}

/// Only the current recipient may accept; only the sender may cancel.
#[test]
fn auth_enforced_for_accept_and_cancel() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let new_recipient = Address::generate(&ctx.env);
    let stranger = Address::generate(&ctx.env);

    ctx.propose_recipient_update(stream_id, &new_recipient);

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "accept_recipient_update",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    assert!(ctx.client().try_accept_recipient_update(&stream_id).is_err());

    ctx.env.mock_auths(&[MockAuth {
        address: &stranger,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_recipient_update",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    assert!(ctx.client().try_cancel_recipient_update(&stream_id).is_err());
}

/// A second accept after completion must fail because pending state was cleared.
#[test]
fn double_accept_errors() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();
    let new_recipient = Address::generate(&ctx.env);

    ctx.propose_recipient_update(stream_id, &new_recipient);

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

    let second = ctx.client().try_accept_recipient_update(&stream_id);
    assert_eq!(second, Err(Ok(ContractError::InvalidState)));
}

/// Cancelling without a pending update must fail closed.
#[test]
fn cancel_without_pending_errors() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream();

    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.sender,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "cancel_recipient_update",
            args: (stream_id,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let result = ctx.client().try_cancel_recipient_update(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}
