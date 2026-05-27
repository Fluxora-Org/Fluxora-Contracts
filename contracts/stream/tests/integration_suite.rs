use fluxora_stream::{FluxoraStream, FluxoraStreamClient, CreateStreamParams, CreateStreamRelativeParams};
use soroban_sdk::{testutils::{Address as _, Events}, vec, Address, Env, token::Client as TokenClient};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
    fn setup(mock_auth: bool) -> Self {
        let env = Env::default();
        if mock_auth {
            env.mock_all_auths();
        }

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin).address();
        let token = TokenClient::new(&env, &token_id);
        
        let admin = Address::generate(&env);
        let sender = Address::generate(&env);

        client.init(&token_id, &admin);

        Self { env, client, sender, token }
    }
}

#[test]
fn test_create_streams_empty_batch_semantics() {
    let ctx = TestContext::setup(true);

    let balance_before = ctx.token.balance(&ctx.sender);
    let count_before = ctx.client.get_stream_count();
    let events_before = ctx.env.events().all().len();

    // Call with empty vector
    let result = ctx.client.create_streams(&ctx.sender, &vec![&ctx.env]);

    assert_eq!(result.len(), 0);
    assert_eq!(ctx.token.balance(&ctx.sender), balance_before);
    assert_eq!(ctx.client.get_stream_count(), count_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
fn test_create_streams_relative_empty_batch_semantics() {
    let ctx = TestContext::setup(true);

    let balance_before = ctx.token.balance(&ctx.sender);
    let count_before = ctx.client.get_stream_count();
    let events_before = ctx.env.events().all().len();

    // Call with empty vector
    let result = ctx.client.create_streams_relative(&ctx.sender, &vec![&ctx.env]);

    assert_eq!(result.len(), 0);
    assert_eq!(ctx.token.balance(&ctx.sender), balance_before);
    assert_eq!(ctx.client.get_stream_count(), count_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}

#[test]
#[should_panic]
fn test_create_streams_empty_batch_unauthorized() {
    let ctx = TestContext::setup(false);
    // This should panic because sender hasn't authorized the call
    ctx.client.create_streams(&ctx.sender, &vec![&ctx.env]);
}

#[test]
#[should_panic]
fn test_create_streams_relative_empty_batch_unauthorized() {
    let ctx = TestContext::setup(false);
    // This should panic because sender hasn't authorized the call
    ctx.client.create_streams_relative(&ctx.sender, &vec![&ctx.env]);
}
