//! Fuzz and property-based tests for the Fluxora factory policy wrapper.
//!
//! Asserts that exactly the documented rejection conditions hold (iff properties),
//! and no allowed in-policy input is wrongly rejected.

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    factory: FluxoraFactoryClient<'a>,
    sender: Address,
    _token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup(cap: i128, min_duration: u64) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let stream_cid = env.register_contract(None, FluxoraStream);
        let stream_client = FluxoraStreamClient::new(&env, &stream_cid);

        let factory_cid = env.register_contract(None, FluxoraFactory);
        let factory = FluxoraFactoryClient::new(&env, &factory_cid);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);

        stellar_asset.mint(&sender, &1_000_000_000_i128);
        stream_client.init(&token_id, &admin);
        token.approve(&sender, &stream_cid, &i128::MAX, &100_000);

        factory.init(&admin, &stream_cid, &cap, &min_duration);

        Self {
            env,
            factory,
            sender,
            _token: token,
        }
    }
}

proptest! {
    #[test]
    fn prop_factory_policy_enforced(
        cap in 100i128..1_000_000i128,
        min_duration in 10u64..3600u64,
        deposit_amount in 1i128..2_000_000i128,
        duration in 1u64..7200u64,
        is_allowlisted in proptest::bool::ANY,
    ) {
        let ctx = Ctx::setup(cap, min_duration);
        let recipient = Address::generate(&ctx.env);

        if is_allowlisted {
            ctx.factory.set_allowlist(&recipient, &true);
        }

        let start_time = ctx.env.ledger().timestamp();
        let end_time = start_time + duration;

        // Invoke factory's create_stream entrypoint
        let result = ctx.factory.try_create_stream(
            &ctx.sender,
            &CreateStreamParams {
                recipient: recipient.clone(),
                deposit_amount,
                rate_per_second: 1i128,
                start_time,
                cliff_time: start_time,
                end_time,
                withdraw_dust_threshold: Some(0i128),
                memo: None,
                metadata: None,
                kind: fluxora_stream::StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        );

        // Property 1: RecipientNotAllowlisted iff !is_allowlisted
        let expect_not_allowlisted = !is_allowlisted;
        if expect_not_allowlisted {
            assert_eq!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
        } else {
            assert_ne!(result, Err(Ok(FactoryError::RecipientNotAllowlisted)));
        }

        // Property 2: DepositExceedsCap iff deposit_amount > cap
        if is_allowlisted {
            let expect_exceeds_cap = deposit_amount > cap;
            if expect_exceeds_cap {
                assert_eq!(result, Err(Ok(FactoryError::DepositExceedsCap)));
            } else {
                assert_ne!(result, Err(Ok(FactoryError::DepositExceedsCap)));
            }
        }

        // Property 3: InvalidTimeRange iff start_time >= end_time
        if is_allowlisted && deposit_amount <= cap {
            let expect_invalid_time = start_time >= end_time;
            if expect_invalid_time {
                assert_eq!(result, Err(Ok(FactoryError::InvalidTimeRange)));
            } else {
                assert_ne!(result, Err(Ok(FactoryError::InvalidTimeRange)));
            }
        }

        // Property 4: DurationTooShort iff duration < min_duration
        if is_allowlisted && deposit_amount <= cap && start_time < end_time {
            let expect_too_short = duration < min_duration;
            if expect_too_short {
                assert_eq!(result, Err(Ok(FactoryError::DurationTooShort)));
            } else {
                assert_ne!(result, Err(Ok(FactoryError::DurationTooShort)));
            }
        }

        // Property 5: No allowlisted in-policy input is wrongly rejected
        if is_allowlisted && deposit_amount <= cap && start_time < end_time && duration >= min_duration {
            assert!(result.is_ok(), "Allowed input was wrongly rejected: {:?}", result);
        }
    }
}
