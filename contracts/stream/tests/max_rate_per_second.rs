#![cfg(test)]
extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, RateCapEnforced, RateUpdated,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::Client as TokenClient,
    vec, Address, Env,
};

struct TestContext {
    env: Env,
    client: FluxoraStreamClient<'static>,
    admin: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'static>,
}

impl TestContext {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let token = TokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        // Mint tokens to sender
        token.mint(&sender, &1_000_000_000);

        Self {
            env,
            client,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn create_stream(&self, rate_per_second: i128) -> Result<u64, ContractError> {
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &1000,
            &rate_per_second,
            &0,
            &0,
            &1000,
            &0,
            &None,
            &None,
        )
    }
}

#[test]
fn test_set_max_rate_per_second_admin_only() {
    let ctx = TestContext::setup();

    // Admin can set max rate
    let result = ctx.client.set_max_rate_per_second(&1000);
    assert!(result.is_ok());

    // Non-admin cannot set max rate
    let non_admin = Address::generate(&ctx.env);
    ctx.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &non_admin,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &ctx.client.address,
            fn_name: "set_max_rate_per_second",
            args: (1000i128,).into_val(&ctx.env),
            sub_invokes: &[],
        },
    }]);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client.set_max_rate_per_second(&1000);
    }));
    assert!(
        result.is_err(),
        "Non-admin should not be able to set max rate"
    );
}

#[test]
fn test_set_max_rate_per_second_invalid_params() {
    let ctx = TestContext::setup();

    // Zero rate should fail
    let result = ctx.client.try_set_max_rate_per_second(&0);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Negative rate should fail
    let result = ctx.client.try_set_max_rate_per_second(&-1);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_create_stream_respects_max_rate() {
    let ctx = TestContext::setup();

    // Set max rate to 100
    ctx.client.set_max_rate_per_second(&100);

    // Creating stream with rate <= max should succeed
    let result = ctx.create_stream(100);
    assert!(result.is_ok());

    let result = ctx.create_stream(50);
    assert!(result.is_ok());

    // Creating stream with rate > max should fail
    let result = ctx.client.try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &101, // Exceeds max rate of 100
        &0,
        &0,
        &1000,
        &0,
        &None,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_update_rate_per_second_respects_max_rate() {
    let ctx = TestContext::setup();

    // Create stream with initial rate
    let stream_id = ctx.create_stream(50).unwrap();

    // Set max rate to 100
    ctx.client.set_max_rate_per_second(&100);

    // Update to rate <= max should succeed
    let result = ctx.client.update_rate_per_second(&stream_id, &100);
    assert!(result.is_ok());

    // Update to rate > max should fail and emit RateCapEnforced event
    let events_before = ctx.env.events().all().len();
    let result = ctx.client.try_update_rate_per_second(&stream_id, &101);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Verify RateCapEnforced event was emitted
    let events = ctx.env.events().all();
    assert_eq!(events.len(), events_before + 1);

    let rate_cap_event = events
        .iter()
        .find(|e| e.0 == (symbol_short!("rate_cap"), stream_id));
    assert!(
        rate_cap_event.is_some(),
        "RateCapEnforced event must be emitted"
    );

    if let Some(event) = rate_cap_event {
        let rate_cap_enforced = RateCapEnforced::try_from_val(&ctx.env, &event.1)
            .expect("Event data must deserialize to RateCapEnforced");
        assert_eq!(rate_cap_enforced.stream_id, stream_id);
        assert_eq!(rate_cap_enforced.attempted_rate, 101);
        assert_eq!(rate_cap_enforced.max_rate_per_second, 100);
    }
}

#[test]
fn test_default_max_rate_is_unlimited() {
    let ctx = TestContext::setup();

    // Without setting max rate, very high rates should be allowed
    let high_rate = i128::MAX / 2; // Use half of max to avoid overflow in duration calc
    let result = ctx.client.try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &high_rate, // Use rate as deposit to ensure sufficient deposit
        &high_rate,
        &0,
        &0,
        &1, // 1 second duration to avoid overflow
        &0,
        &None,
        &None,
    );
    assert!(result.is_ok(), "High rates should be allowed by default");
}

#[test]
fn test_max_rate_applies_to_all_create_functions() {
    let ctx = TestContext::setup();

    // Set max rate to 100
    ctx.client.set_max_rate_per_second(&100);

    // Test create_streams
    let params = vec![
        &ctx.env,
        fluxora_stream::CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000,
            rate_per_second: 101, // Exceeds max
            start_time: 0,
            cliff_time: 0,
            end_time: 1000,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
        },
    ];

    let result = ctx.client.try_create_streams(&ctx.sender, &params);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Test create_stream_relative
    let relative_params = fluxora_stream::CreateStreamRelativeParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: 1000,
        rate_per_second: 101, // Exceeds max
        start_delay: 0,
        cliff_delay: 0,
        duration: 1000,
        withdraw_dust_threshold: Some(0),
        memo: None,
        metadata: None,
    };

    let result = ctx.client.try_create_stream_relative(&ctx.sender, &relative_params);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_max_rate_boundary_conditions() {
    let ctx = TestContext::setup();

    // Test with max rate = 1
    ctx.client.set_max_rate_per_second(&1);

    // Rate = 1 should succeed
    let result = ctx.create_stream(1);
    assert!(result.is_ok());

    // Rate = 2 should fail
    let result = ctx.client.try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &2,
        &0,
        &0,
        &1000,
        &0,
        &None,
        &None,
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Test with max rate = i128::MAX
    ctx.client.set_max_rate_per_second(&i128::MAX);

    // Very high rate should succeed
    let high_rate = i128::MAX / 2;
    let result = ctx.client.try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &high_rate,
        &high_rate,
        &0,
        &0,
        &1,
        &0,
        &None,
        &None,
    );
    assert!(result.is_ok());
}

#[test]
fn test_rate_cap_does_not_affect_existing_streams() {
    let ctx = TestContext::setup();

    // Create stream with high rate before setting cap
    let stream_id = ctx.create_stream(1000).unwrap();

    // Set lower max rate
    ctx.client.set_max_rate_per_second(&100);

    // Existing stream should still be queryable and functional
    let stream = ctx.client.get_stream_state(&stream_id);
    assert_eq!(stream.rate_per_second, 1000);

    // But updates to higher rates should be blocked
    let result = ctx.client.try_update_rate_per_second(&stream_id, &1001);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Updates within the cap should work
    let result = ctx.client.update_rate_per_second(&stream_id, &1100); // Still higher than old rate
    assert!(result.is_ok());
}

#[test]
fn test_rate_cap_with_arithmetic_overflow_protection() {
    let ctx = TestContext::setup();

    // Set a reasonable max rate
    ctx.client.set_max_rate_per_second(&1_000_000);

    // Try to create stream that would cause overflow in duration calculation
    // even though rate is within cap
    let result = ctx.client.try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000,
        &1_000_000,
        &0,
        &0,
        &i64::MAX as u64, // Very long duration
        &0,
        &None,
        &None,
    );
    
    // Should fail with InvalidParams (overflow or rate cap violation)
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

#[test]
fn test_multiple_rate_cap_enforced_events() {
    let ctx = TestContext::setup();

    // Create two streams
    let stream_id1 = ctx.create_stream(50).unwrap();
    let stream_id2 = ctx.create_stream(60).unwrap();

    // Set max rate
    ctx.client.set_max_rate_per_second(&100);

    let events_before = ctx.env.events().all().len();

    // Try to update both streams beyond the cap
    let _ = ctx.client.try_update_rate_per_second(&stream_id1, &101);
    let _ = ctx.client.try_update_rate_per_second(&stream_id2, &102);

    // Should have two RateCapEnforced events
    let events = ctx.env.events().all();
    let rate_cap_events: Vec<_> = events
        .iter()
        .filter(|e| e.0 .0 == symbol_short!("rate_cap"))
        .collect();

    assert_eq!(rate_cap_events.len(), 2);

    // Verify event details
    for (i, event) in rate_cap_events.iter().enumerate() {
        let rate_cap_enforced = RateCapEnforced::try_from_val(&ctx.env, &event.1)
            .expect("Event data must deserialize to RateCapEnforced");
        assert_eq!(rate_cap_enforced.max_rate_per_second, 100);
        assert_eq!(rate_cap_enforced.attempted_rate, 101 + i as i128);
    }
}
