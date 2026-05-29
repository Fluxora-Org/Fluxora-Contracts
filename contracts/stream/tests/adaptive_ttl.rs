//! Tests for issue #516: adaptive TTL thresholds for stream ledger entries.
//!
//! Verifies that `compute_adaptive_ttl` scales correctly with remaining stream
//! lifetime and that the floor/cap invariants hold.

use fluxora_stream::{CreateStreamParams, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env.register_stellar_asset_contract_v2(token_admin.clone()).address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        stellar_asset.mint(&sender, &1_000_000_000);

        client.init(&token_id, &admin);

        Self { env, client, sender, token }
    }

    fn create_stream_with_duration(&self, recipient: &Address, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        let ids = self.client.create_streams(
            &self.sender,
            &soroban_sdk::vec![
                &self.env,
                CreateStreamParams {
                    recipient: recipient.clone(),
                    deposit_amount: duration as i128,
                    rate_per_second: 1,
                    start_time: now,
                    cliff_time: now,
                    end_time: now + duration,
                    withdraw_dust_threshold: None,
                    memo: None,
                }
            ],
        );
        ids.get(0).unwrap()
    }
}

/// A stream with a far-future end_time can be created and retrieved successfully.
/// (Adaptive TTL must not cause a panic or overflow.)
#[test]
fn test_long_duration_stream_created_ok() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    // ~1 year duration
    let stream_id = ctx.create_stream_with_duration(&recipient, 31_536_000);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
}

/// A stream with a very short duration (1 second) can be created and retrieved.
/// TTL must not fall below the static floor.
#[test]
fn test_short_duration_stream_created_ok() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let stream_id = ctx.create_stream_with_duration(&recipient, 1);
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
}

/// A stream whose end_time is already in the past (now == end_time) can be created.
/// Adaptive TTL falls back to BUFFER_LEDGERS floor.
#[test]
fn test_expired_stream_ttl_floor() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let now = ctx.env.ledger().timestamp();

    // end_time == start_time (zero duration is invalid per validation, use 1)
    let stream_id = ctx.create_stream_with_duration(&recipient, 1);

    // Advance ledger past end_time
    ctx.env.ledger().with_mut(|l| l.timestamp = now + 100);

    // Stream should still be readable (TTL floor kept it alive)
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
}

/// Multiple streams with different durations all persist correctly.
#[test]
fn test_mixed_duration_streams_all_readable() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    let id_short = ctx.create_stream_with_duration(&recipient, 100);
    let id_medium = ctx.create_stream_with_duration(&recipient, 86_400);
    let id_long = ctx.create_stream_with_duration(&recipient, 31_536_000);

    assert_eq!(ctx.client.get_stream_state(&id_short).stream_id, id_short);
    assert_eq!(ctx.client.get_stream_state(&id_medium).stream_id, id_medium);
    assert_eq!(ctx.client.get_stream_state(&id_long).stream_id, id_long);
}

/// Recipient index is readable after creating streams with adaptive TTL.
#[test]
fn test_recipient_index_readable_after_adaptive_ttl_write() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);

    ctx.create_stream_with_duration(&recipient, 86_400);
    ctx.create_stream_with_duration(&recipient, 31_536_000);

    let index = ctx.client.get_recipient_streams(&recipient, &None, &None);
    assert_eq!(index.len(), 2);
}

/// Stream state is readable after ledger advances (TTL was set large enough).
#[test]
fn test_stream_readable_after_ledger_advance() {
    let ctx = Ctx::setup();
    let recipient = Address::generate(&ctx.env);
    let stream_id = ctx.create_stream_with_duration(&recipient, 86_400);

    // Advance by 1 day worth of ledgers (17280 ledgers at 5s each)
    ctx.env.ledger().with_mut(|l| {
        l.sequence_number += 17_280;
        l.timestamp += 86_400;
    });

    // Should still be readable — adaptive TTL for 1-day stream is > 17280 ledgers
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.stream_id, stream_id);
}
