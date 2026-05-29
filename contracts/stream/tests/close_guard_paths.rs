//! Tests for issue #522: close guard and recipient-index cleanup paths.
//!
//! 1. `test_close_non_completed_stream_rejected` — exercises the guard that
//!    rejects Active/Paused streams passed to `close_completed_stream`.
//! 2. `test_recipient_index_cleanup_graceful_on_missing_entry` — verifies that
//!    closing a completed stream succeeds gracefully even when the recipient
//!    index entry is absent (no panic, no partial state left behind).

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
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
        let recipient = Address::generate(&env);
        stellar_asset.mint(&sender, &1_000_000_000);
        client.init(&token_id, &admin);
        Self { env, client, sender, recipient, token }
    }

    fn create_stream(&self, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender, &self.recipient,
            &(duration as i128), &1,
            &now, &now, &(now + duration),
            &0, &None,
        )
    }
}

// ---------------------------------------------------------------------------
// Close guard — rejects non-terminal streams
// ---------------------------------------------------------------------------

/// Active stream → close_completed_stream returns InvalidState (guard fires).
#[test]
fn test_close_non_completed_stream_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(10_000);
    assert_eq!(ctx.client.get_stream_state(&stream_id).status, StreamStatus::Active);
    let result = ctx.client.try_close_completed_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Paused stream → close_completed_stream returns InvalidState.
#[test]
fn test_close_paused_stream_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(10_000);
    ctx.client.pause_stream(&stream_id);
    assert_eq!(ctx.client.get_stream_state(&stream_id).status, StreamStatus::Paused);
    let result = ctx.client.try_close_completed_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Completed stream (fully withdrawn) → close succeeds and stream is removed.
#[test]
fn test_close_completed_stream_ok() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);
    ctx.env.ledger().with_mut(|l| l.timestamp += 101);
    ctx.client.withdraw(&stream_id);
    assert_eq!(ctx.client.get_stream_state(&stream_id).status, StreamStatus::Completed);
    ctx.client.close_completed_stream(&stream_id);
    assert!(ctx.client.try_get_stream_state(&stream_id).is_err());
}

/// Cancelled stream with zero claimable → close succeeds.
#[test]
fn test_close_cancelled_zero_claimable_ok() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    // Stream starts in the future → no accrual at cancel time
    let stream_id = ctx.client.create_stream(
        &ctx.sender, &ctx.recipient,
        &1_000, &1,
        &(now + 1_000), &(now + 1_000), &(now + 2_000),
        &0, &None,
    );
    ctx.client.cancel_stream(&stream_id);
    assert_eq!(ctx.client.get_stream_state(&stream_id).status, StreamStatus::Cancelled);
    ctx.client.close_completed_stream(&stream_id);
    assert!(ctx.client.try_get_stream_state(&stream_id).is_err());
}

/// Cancelled stream with remaining claimable → close returns InvalidState.
#[test]
fn test_close_cancelled_with_claimable_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(10_000);
    ctx.env.ledger().with_mut(|l| l.timestamp += 100);
    ctx.client.cancel_stream(&stream_id);
    assert_eq!(ctx.client.get_stream_state(&stream_id).status, StreamStatus::Cancelled);
    let result = ctx.client.try_close_completed_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Non-existent stream → StreamNotFound.
#[test]
fn test_close_nonexistent_stream() {
    let ctx = Ctx::setup();
    let result = ctx.client.try_close_completed_stream(&9999);
    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

// ---------------------------------------------------------------------------
// Recipient-index cleanup path
// ---------------------------------------------------------------------------

/// `close_completed_stream` removes the stream from the recipient index.
/// `remove_stream_from_recipient_index` silently skips missing entries (no panic),
/// which is the correct graceful behavior for a permissionless cleanup function.
#[test]
fn test_recipient_index_cleanup_graceful_on_missing_entry() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);
    ctx.env.ledger().with_mut(|l| l.timestamp += 101);
    ctx.client.withdraw(&stream_id);

    let index_before = ctx.client.get_recipient_streams(&ctx.recipient, &None, &None);
    assert!(index_before.contains(&stream_id));

    ctx.client.close_completed_stream(&stream_id);

    // Stream removed from storage
    assert!(ctx.client.try_get_stream_state(&stream_id).is_err());
    // Stream removed from index — no panic, no partial state
    let index_after = ctx.client.get_recipient_streams(&ctx.recipient, &None, &None);
    assert!(!index_after.contains(&stream_id));
}

/// Closing one stream leaves other streams in the recipient index intact.
#[test]
fn test_close_removes_only_target_from_index() {
    let ctx = Ctx::setup();
    let id_a = ctx.create_stream(100);
    let id_b = ctx.create_stream(10_000);

    ctx.env.ledger().with_mut(|l| l.timestamp += 101);
    ctx.client.withdraw(&id_a);
    ctx.client.close_completed_stream(&id_a);

    let index = ctx.client.get_recipient_streams(&ctx.recipient, &None, &None);
    assert!(!index.contains(&id_a));
    assert!(index.contains(&id_b));
}
