extern crate std;

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient};
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
    _token: TokenClient<'a>,
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
            _token: token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn create_stream(&self) -> u64 {
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0_i128,
            &None,
            &fluxora_stream::StreamKind::Linear,
        )
    }
}

#[test]
fn equal_ledger_timestamp_is_accepted() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 500);

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 500);
}

#[test]
fn retrograde_get_withdrawable_returns_clock_regression() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(500);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 500);

    ctx.env.ledger().set_timestamp(499);
    let result = ctx.client().try_get_withdrawable(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::ClockRegression)));
}

#[test]
fn retrograde_withdraw_returns_clock_regression_before_state_changes() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(600);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 600);

    ctx.env.ledger().set_timestamp(500);
    let result = ctx.client().try_withdraw(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::ClockRegression)));

    ctx.env.ledger().set_timestamp(600);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 600);
}

#[test]
fn forward_progress_after_regression_attempt_is_still_accepted() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.create_stream();

    ctx.env.ledger().set_timestamp(300);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 300);

    ctx.env.ledger().set_timestamp(299);
    assert_eq!(
        ctx.client().try_get_withdrawable(&stream_id),
        Err(Ok(ContractError::ClockRegression))
    );

    ctx.env.ledger().set_timestamp(301);
    assert_eq!(ctx.client().get_withdrawable(&stream_id), 301);
}
