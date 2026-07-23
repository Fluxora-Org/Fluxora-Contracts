extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, StreamKind, WithdrawToParam,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
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
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        TestContext {
            env,
            contract_id,
            sender,
            recipient,
            token,
        }
    }

    fn setup_strict() -> Self {
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
        use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
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

        TestContext {
            env: env.clone(),
            contract_id,
            sender,
            recipient,
            token: TokenClient::new(&env, &token_id),
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }
}

#[test]
fn test_batch_withdraw_to_requires_recipient_auth() {
    let ctx = TestContext::setup_strict();
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};

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
                0i128,
                Option::<soroban_sdk::Bytes>::None,
                StreamKind::Linear,
            )
                .into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    let destination = Address::generate(&ctx.env);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id,
            destination: destination.clone(),
        },
    ];

    let batch_args = (&ctx.recipient, withdrawals.clone()).into_val(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "batch_withdraw_to",
            args: batch_args,
            sub_invokes: &[],
        },
    }]);
    let results = ctx.client().batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().amount, 500);
    assert_eq!(ctx.token.balance(&destination), 500);
}

#[test]
fn test_batch_withdraw_to_mixed_recipients_reverts_atomically() {
    let ctx = TestContext::setup_strict();
    use soroban_sdk::{testutils::MockAuth, testutils::MockAuthInvoke, IntoVal};
    let other_recipient = Address::generate(&ctx.env);

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
                0i128,
                Option::<soroban_sdk::Bytes>::None,
                StreamKind::Linear,
            )
                .into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let stream_id_a = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );
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
                0i128,
                Option::<soroban_sdk::Bytes>::None,
                StreamKind::Linear,
            )
                .into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);
    let stream_id_b = ctx.client().create_stream(
        &ctx.sender,
        &other_recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id: stream_id_a,
            destination: Address::generate(&ctx.env),
        },
        WithdrawToParam {
            stream_id: stream_id_b,
            destination: Address::generate(&ctx.env),
        },
    ];

    let batch_args = (&ctx.recipient, withdrawals.clone()).into_val(&ctx.env);
    ctx.env.mock_auths(&[MockAuth {
        address: &ctx.recipient,
        invoke: &MockAuthInvoke {
            contract: &ctx.contract_id,
            fn_name: "batch_withdraw_to",
            args: batch_args,
            sub_invokes: &[],
        },
    }]);

    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
    assert_eq!(
        ctx.client().get_stream_state(&stream_id_a).withdrawn_amount,
        0
    );
    assert_eq!(
        ctx.client().get_stream_state(&stream_id_b).withdrawn_amount,
        0
    );
}

#[test]
fn test_batch_withdraw_to_duplicate_destinations_aggregate_transfers() {
    let ctx = TestContext::setup();
    let stream_id_a = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );
    let stream_id_b = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    let destination = Address::generate(&ctx.env);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id: stream_id_a,
            destination: destination.clone(),
        },
        WithdrawToParam {
            stream_id: stream_id_b,
            destination: destination.clone(),
        },
    ];

    let results = ctx.client().batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().amount, 500);
    assert_eq!(results.get(1).unwrap().amount, 500);
    assert_eq!(ctx.token.balance(&destination), 1000);
}

#[test]
fn test_batch_withdraw_to_rejects_contract_destination() {
    let ctx = TestContext::setup();
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(500);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id,
            destination: ctx.contract_id.clone(),
        },
    ];

    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        0
    );
    assert_eq!(ctx.token.balance(&ctx.contract_id), 1000);
}

#[test]
fn test_batch_withdraw_to_mixed_dust_blocked_non_dust_blocked() {
    let ctx = TestContext::setup();

    // Create stream with high dust threshold (will be dust-blocked)
    let stream_id_dust = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &500, // High dust threshold
        &None,
        &StreamKind::Linear,
    );

    // Create stream with zero dust threshold (will not be dust-blocked)
    let stream_id_no_dust = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0, // No dust threshold
        &None,
        &StreamKind::Linear,
    );

    // Advance time to 100: withdrawable = 100 for both streams
    ctx.env.ledger().set_timestamp(100);

    let destination = Address::generate(&ctx.env);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id: stream_id_dust,
            destination: destination.clone(),
        },
        WithdrawToParam {
            stream_id: stream_id_no_dust,
            destination: destination.clone(),
        },
    ];

    let results = ctx.client().batch_withdraw_to(&ctx.recipient, &withdrawals);

    // Call should succeed overall
    assert_eq!(results.len(), 2);

    // Dust-blocked entry should return 0
    assert_eq!(results.get(0).unwrap().amount, 0);

    // Non-dust-blocked entry should return correct withdrawable amount
    assert_eq!(results.get(1).unwrap().amount, 100);

    // Destination should receive only the non-dust-blocked amount
    assert_eq!(ctx.token.balance(&destination), 100);

    // Dust-blocked stream should have no state change
    let dust_stream_state = ctx.client().get_stream_state(&stream_id_dust);
    assert_eq!(dust_stream_state.withdrawn_amount, 0);

    // Non-dust-blocked stream should have state updated
    let no_dust_stream_state = ctx.client().get_stream_state(&stream_id_no_dust);
    assert_eq!(no_dust_stream_state.withdrawn_amount, 100);
}

#[test]
fn test_batch_withdraw_to_all_dust_blocked() {
    let ctx = TestContext::setup();

    // Create multiple streams with high dust thresholds (all will be dust-blocked)
    let stream_id_dust_1 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &500, // High dust threshold
        &None,
        &StreamKind::Linear,
    );

    let stream_id_dust_2 = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &500, // High dust threshold
        &None,
        &StreamKind::Linear,
    );

    // Advance time to 100: withdrawable = 100 for both streams (below threshold of 500)
    ctx.env.ledger().set_timestamp(100);

    let destination = Address::generate(&ctx.env);
    let withdrawals = soroban_sdk::vec![
        &ctx.env,
        WithdrawToParam {
            stream_id: stream_id_dust_1,
            destination: destination.clone(),
        },
        WithdrawToParam {
            stream_id: stream_id_dust_2,
            destination: destination.clone(),
        },
    ];

    // Call should succeed as a no-op (not error)
    let results = ctx.client().batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 2);

    // All entries should return 0
    assert_eq!(results.get(0).unwrap().amount, 0);
    assert_eq!(results.get(1).unwrap().amount, 0);

    // Destination should receive nothing
    assert_eq!(ctx.token.balance(&destination), 0);

    // All streams should have no state change
    let dust_stream_state_1 = ctx.client().get_stream_state(&stream_id_dust_1);
    assert_eq!(dust_stream_state_1.withdrawn_amount, 0);

    let dust_stream_state_2 = ctx.client().get_stream_state(&stream_id_dust_2);
    assert_eq!(dust_stream_state_2.withdrawn_amount, 0);
}
