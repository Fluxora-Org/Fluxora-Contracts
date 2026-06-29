//! Regression tests for issue #726 — `create_streams` must honor `CreationPaused`.

extern crate std;

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

const MAX_DEPOSIT: i128 = 10_000_000;
const MIN_DURATION: u64 = 86_400;
const DEPOSIT_AMOUNT: i128 = 200_000;
const RATE_PER_SECOND: i128 = 1;
const STREAM_DURATION: u64 = 200_000;
const SENDER_FUNDING: i128 = 1_000_000_000;
const LEDGER_TIMESTAMP: u64 = 1_000_000_000;

struct Ctx<'a> {
    env: Env,
    factory: FluxoraFactoryClient<'a>,
    sender: Address,
    recipient: Address,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(LEDGER_TIMESTAMP);

        let stream_contract_id = env.register_contract(None, FluxoraStream);
        let factory_contract_id = env.register_contract(None, FluxoraFactory);

        let stream = FluxoraStreamClient::new(&env, &stream_contract_id);
        let factory = FluxoraFactoryClient::new(&env, &factory_contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        stellar_asset.mint(&sender, &SENDER_FUNDING);

        stream.init(&token_id, &admin);
        factory.init(&admin, &stream_contract_id, &MAX_DEPOSIT, &MIN_DURATION);
        factory.set_allowlist(&recipient, &true);

        Self {
            env,
            factory,
            sender,
            recipient,
        }
    }

    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }
}

#[test]
fn test_create_streams_batch_paused_enforcement() {
    let ctx = Ctx::setup();
    let now = ctx.now();

    assert!(!ctx.factory.is_factory_paused());

    ctx.factory.set_factory_paused(&true);
    assert!(ctx.factory.is_factory_paused());

    let mut streams = soroban_sdk::Vec::new(&ctx.env);
    streams.push_back(fluxora_stream::CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: DEPOSIT_AMOUNT,
        rate_per_second: RATE_PER_SECOND,
        start_time: now,
        cliff_time: now,
        end_time: now + STREAM_DURATION,
        withdraw_dust_threshold: Some(0),
        memo: None,
        kind: fluxora_stream::StreamKind::Linear,
    });

    let result_non_empty = ctx.factory.try_create_streams(&ctx.sender, &streams);
    assert_eq!(result_non_empty, Err(Ok(FactoryError::CreationPaused)));

    let empty_streams = soroban_sdk::Vec::new(&ctx.env);
    let result_empty = ctx.factory.try_create_streams(&ctx.sender, &empty_streams);
    assert_eq!(result_empty, Err(Ok(FactoryError::CreationPaused)));

    ctx.factory.set_factory_paused(&false);
    assert!(!ctx.factory.is_factory_paused());

    // After resume, policy checks must run (not the pause gate). Removing allowlist
    // should yield RecipientNotAllowlisted rather than CreationPaused.
    ctx.factory.set_allowlist(&ctx.recipient, &false);
    let result_resumed = ctx.factory.try_create_streams(&ctx.sender, &streams);
    assert_eq!(
        result_resumed,
        Err(Ok(FactoryError::RecipientNotAllowlisted)),
        "expected post-resume call to pass pause gate and fail allowlist check"
    );
}

#[test]
fn test_create_streams_succeeds_when_factory_not_paused() {
    let ctx = Ctx::setup();
    let now = ctx.now();

    assert!(!ctx.factory.is_factory_paused());

    let mut streams = soroban_sdk::Vec::new(&ctx.env);
    streams.push_back(fluxora_stream::CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: DEPOSIT_AMOUNT,
        rate_per_second: RATE_PER_SECOND,
        start_time: now,
        cliff_time: now,
        end_time: now + STREAM_DURATION,
        withdraw_dust_threshold: Some(0),
        memo: None,
        kind: fluxora_stream::StreamKind::Linear,
    });

    // Prove the pause gate is not blocking: an allowlist violation means policy
    // evaluation ran after the pause check.
    ctx.factory.set_allowlist(&ctx.recipient, &false);
    let result = ctx.factory.try_create_streams(&ctx.sender, &streams);
    assert_eq!(
        result,
        Err(Ok(FactoryError::RecipientNotAllowlisted)),
        "expected unpaused factory to reach allowlist validation"
    );
}
