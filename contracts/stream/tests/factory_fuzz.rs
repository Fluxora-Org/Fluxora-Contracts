//! Fuzz and property-based tests for the Fluxora factory policy wrapper.
//!
//! Asserts that exactly the documented rejection conditions hold (iff properties),
//! and no allowed in-policy input is wrongly rejected.

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::Address as _,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    factory: FluxoraFactoryClient<'a>,
    #[allow(dead_code)]
    stream: FluxoraStreamClient<'a>,
    #[allow(dead_code)]
    admin: Address,
    sender: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup(max_deposit: i128, min_duration: u64) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        // Deploy stream contract
        let stream_id = env.register_contract(None, FluxoraStream);
        let stream = FluxoraStreamClient::new(&env, &stream_id);

        // Deploy factory contract
        let factory_id = env.register_contract(None, FluxoraFactory);
        let factory = FluxoraFactoryClient::new(&env, &factory_id);

        // Token setup
        let token_admin = Address::generate(&env);
        let token_contract_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_contract_id);
        let stellar_asset = StellarAssetClient::new(&env, &token_contract_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        // Mint ample tokens to the sender
        stellar_asset.mint(&sender, &1_000_000_000_000);

        // Init stream contract
        stream.init(&token_contract_id, &stream_id);

        // Approve stream contract to pull tokens from sender (avoiding InsufficientBalance/Allowance)
        token.approve(&sender, &stream_id, &1_000_000_000_000, &9999);

        // Init factory with specified policy values
        factory.init(&admin, &stream_id, &max_deposit, &min_duration);

        Self {
            env,
            factory,
            stream,
            admin,
            sender,
            token,
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property-based test asserting that exactly the documented rejection conditions
    /// hold on the factory create_stream policy checks, and no valid input is rejected.
    #[test]
    fn test_create_stream_policy_properties(
        is_allowlisted in any::<bool>(),
        deposit_amount in 1_000i128..15_000i128,
        duration in 10u64..500u64,
        start_time in 1_000u64..10_000u64,
    ) {
        let cap = 10_000i128;
        let min_duration = 100u64;
        let end_time = start_time + duration;

        // Perform setup and optionally allowlist the recipient
        let ctx = Ctx::setup(cap, min_duration);
        let recipient = Address::generate(&ctx.env);

        if is_allowlisted {
            ctx.factory.set_allowlist(&recipient, &true);
        }

        // Invoke factory's create_stream entrypoint
        let result = ctx.factory.try_create_stream(
            &ctx.sender,
            &recipient,
            &deposit_amount,
            &1i128, // rate_per_second
            &start_time,
            &start_time, // cliff_time == start_time
            &end_time,
            &0i128, // withdraw_dust_threshold
            &fluxora_stream::StreamKind::Linear,
            &None,
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
