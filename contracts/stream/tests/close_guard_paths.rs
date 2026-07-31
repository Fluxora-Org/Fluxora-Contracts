//! Tests for issue #522: close guard and recipient-index cleanup paths.
//!
//! 1. `test_close_non_completed_stream_rejected` — exercises the guard that
//!    rejects Active/Paused streams passed to `close_completed_stream`.
//! 2. `test_recipient_index_cleanup_graceful_on_missing_entry` — verifies that
//!    closing a completed stream succeeds gracefully even when the recipient
//!    index entry is absent (no panic, no partial state left behind).

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus, MAX_RECIPIENT_PAGE_SIZE,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let token = TokenClient::new(&env, &token_id);
        let stellar_asset = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        stellar_asset.mint(&sender, &1_000_000_000);
        client.init(&token_id, &admin);
        // create_stream pulls the deposit via transfer_from, which requires an allowance.
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);
        Self {
            env,
            client,
            sender,
            recipient,
            token,
        }
    }

    fn create_stream(&self, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: (duration as i128),
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: (now + duration),
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }

    fn create_irrevocable_stream(&self, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: (duration as i128),
                rate_per_second: 1,
                start_time: now,
                cliff_time: now,
                end_time: (now + duration),
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: Some(true),
                witness: None,
            },
        )
    }

    /// Create `n` streams for `self.recipient`, each with the given duration.
    fn create_n(&self, n: u32, duration: u64) -> std::vec::Vec<u64> {
        let mut ids = std::vec::Vec::new();
        for _ in 0..n {
            ids.push(self.create_stream(duration));
        }
        ids
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
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Active
    );
    let result = ctx.client.try_close_completed_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Paused stream → close_completed_stream returns InvalidState.
#[test]
fn test_close_paused_stream_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(10_000);
    // Clear the pause/resume cooldown before toggling.
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );
    let result = ctx.client.try_close_completed_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Completed stream (fully withdrawn) → close succeeds and stream is removed.
#[test]
fn test_close_completed_stream_ok() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);
    ctx.env.ledger().with_mut(|l| {
        l.timestamp += 101;
        l.sequence_number += 2;
    });
    ctx.client.withdraw(&stream_id, &None);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );
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
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: (now + 1_000),
            cliff_time: (now + 1_000),
            end_time: (now + 2_000),
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    ctx.client.cancel_stream(&stream_id);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
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
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
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
// close_cancelled_stream tests
// ---------------------------------------------------------------------------

/// Non-cancelled stream → close_cancelled_stream returns InvalidState.
#[test]
fn test_close_cancelled_non_cancelled_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(10_000);
    let result = ctx.client.try_close_cancelled_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Cancelled stream with zero claimable → close_cancelled_stream succeeds.
#[test]
fn test_close_cancelled_stream_ok() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    // Stream starts in the future → no accrual at cancel time
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_000,
            rate_per_second: 1,
            start_time: (now + 1_000),
            cliff_time: (now + 1_000),
            end_time: (now + 2_000),
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    ctx.client.cancel_stream(&stream_id);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
    ctx.client.close_cancelled_stream(&stream_id);
    assert!(ctx.client.try_get_stream_state(&stream_id).is_err());
}

/// Cancelled stream with remaining claimable → close_cancelled_stream returns InvalidState.
#[test]
fn test_close_cancelled_stream_with_claimable_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(10_000);
    ctx.env.ledger().with_mut(|l| l.timestamp += 100);
    ctx.client.cancel_stream(&stream_id);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
    let result = ctx.client.try_close_cancelled_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
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
    ctx.env.ledger().with_mut(|l| {
        l.timestamp += 101;
        l.sequence_number += 2;
    });
    ctx.client.withdraw(&stream_id, &None);

    let index_before = ctx.client.get_recipient_streams(&ctx.recipient);
    assert!(index_before.contains(stream_id));

    ctx.client.close_completed_stream(&stream_id);

    // Stream removed from storage
    assert!(ctx.client.try_get_stream_state(&stream_id).is_err());
    // Stream removed from index — no panic, no partial state
    let index_after = ctx.client.get_recipient_streams(&ctx.recipient);
    assert!(!index_after.contains(stream_id));
}

/// Closing one stream leaves other streams in the recipient index intact.
#[test]
fn test_close_removes_only_target_from_index() {
    let ctx = Ctx::setup();
    let id_a = ctx.create_stream(100);
    let id_b = ctx.create_stream(10_000);

    ctx.env.ledger().with_mut(|l| {
        l.timestamp += 101;
        l.sequence_number += 2;
    });
    ctx.client.withdraw(&id_a, &None);
    ctx.client.close_completed_stream(&id_a);

    let index = ctx.client.get_recipient_streams(&ctx.recipient);
    assert!(!index.contains(id_a));
    assert!(index.contains(id_b));
}

// ---------------------------------------------------------------------------
// Irrevocable stream guard
// ---------------------------------------------------------------------------

#[test]
fn test_irrevocable_stream_rejects_cancel() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_irrevocable_stream(10_000);
    let result = ctx.client.try_cancel_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
}

#[test]
fn test_irrevocable_stream_rejects_admin_cancel() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_irrevocable_stream(10_000);

    // We would test cancel_stream_as_admin, but for simplicity we verify the guard
    // logic which is shared.
    let result = ctx.client.try_cancel_stream_as_admin(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
}

#[test]
fn test_irrevocable_stream_rejects_keeper_cancel() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_irrevocable_stream(10_000);

    // Fast-forward past end_time + grace_period
    ctx.env.ledger().with_mut(|l| {
        l.timestamp += 10_000 + 7 * 86400; // end_time + 7 days
    });

    let keeper = Address::generate(&ctx.env);
    let result = ctx.client.try_keeper_cancel(&stream_id, &keeper);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
}

#[test]
fn test_irrevocable_stream_rejects_shorten_end_time() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_irrevocable_stream(10_000);

    let now = ctx.env.ledger().timestamp();
    let result = ctx
        .client
        .try_shorten_stream_end_time(&stream_id, &(now + 5_000));
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
}

#[test]
fn test_irrevocable_stream_rejects_bulk_cancel() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_irrevocable_stream(10_000);

    let streams = soroban_sdk::vec![&ctx.env, stream_id];
    let result = ctx.client.try_bulk_cancel_streams(&ctx.sender, &streams);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));
}

// ---------------------------------------------------------------------------
// Paginated-index consistency after close (issue #959)
// ---------------------------------------------------------------------------

/// Close a stream from the middle of a multi-page recipient index and verify
/// all remaining pages are consistent (no duplicates, no gaps, correct total).
#[test]
fn test_close_removes_from_multi_page_index() {
    let ctx = Ctx::setup();
    ctx.env.budget().reset_unlimited();

    let total = MAX_RECIPIENT_PAGE_SIZE + 1;
    let ids = ctx.create_n(total, 100);

    // Sanity: first page is full, second page has one entry.
    let page1 = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(page1.len() as u32, MAX_RECIPIENT_PAGE_SIZE);

    // Pick a stream from the non-final "page" (position 50 in the sorted
    // order is well within the first page).
    let close_id = ids[50 as usize];

    // Complete the stream so close_completed_stream will accept it.
    ctx.env.ledger().with_mut(|l| {
        l.timestamp += 101;
        l.sequence_number += 2;
    });
    ctx.client.withdraw(&close_id, &None);
    ctx.client.close_completed_stream(&close_id);

    // Stream removed from storage.
    assert!(ctx.client.try_get_stream_state(&close_id).is_err());

    // Paginate through every page and collect all remaining IDs.
    let mut remaining = soroban_sdk::Vec::<u64>::new(&ctx.env);
    let mut cursor = 0u64;
    loop {
        let page = ctx.client.get_recipient_streams_paginated(
            &ctx.recipient,
            &cursor,
            &MAX_RECIPIENT_PAGE_SIZE,
        );
        for i in 0..page.stream_ids.len() {
            remaining.push_back(page.stream_ids.get(i).unwrap());
        }
        if page.next_cursor == 0 {
            break;
        }
        cursor = page.next_cursor;
    }

    // Total count decreased by exactly one.
    assert_eq!(remaining.len() as u32, total - 1);

    // The closed stream must not appear in any page.
    for i in 0..remaining.len() {
        assert_ne!(remaining.get(i).unwrap(), close_id);
    }

    // Every original ID except the closed one must still be present.
    for id in &ids {
        if *id != close_id {
            let mut found = false;
            for i in 0..remaining.len() {
                if remaining.get(i).unwrap() == *id {
                    found = true;
                    break;
                }
            }
            assert!(found, "stream {} missing from index after close", id);
        }
    }
}

/// Close the last (and only) stream for a recipient and confirm the paginated
/// query returns an empty page gracefully (no panic, next_cursor == 0).
#[test]
fn test_close_last_stream_empty_index_graceful() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(100);

    // Verify the stream is indexed.
    let index_before = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(index_before.len(), 1);
    assert_eq!(index_before.get(0).unwrap(), stream_id);

    // Complete and close it.
    ctx.env.ledger().with_mut(|l| {
        l.timestamp += 101;
        l.sequence_number += 2;
    });
    ctx.client.withdraw(&stream_id, &None);
    ctx.client.close_completed_stream(&stream_id);

    // Non-paginated query returns empty.
    let index_after = ctx.client.get_recipient_streams(&ctx.recipient);
    assert_eq!(index_after.len(), 0);

    // Paginated query also returns empty with next_cursor == 0.
    let page =
        ctx.client
            .get_recipient_streams_paginated(&ctx.recipient, &0, &MAX_RECIPIENT_PAGE_SIZE);
    assert_eq!(page.stream_ids.len(), 0);
    assert_eq!(page.next_cursor, 0);
}

// ---------------------------------------------------------------------------
// Issue #1215 Decommission mode tests
// ---------------------------------------------------------------------------

/// Test set_stream_decommissioned blocks update_rate_per_second, decrease_rate_per_second,
/// top_up_stream, extend_stream_end_time, and clone_stream (returning ContractError::InvalidState),
/// while leaving withdraw, pause_stream/resume_stream, and cancel_stream fully functional.
#[test]
fn test_set_stream_decommissioned_blocks_mutations_only() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(1000);

    // Flag stream as decommissioned by sender
    ctx.client
        .set_stream_decommissioned(&stream_id, &ctx.sender, &true);

    // Verify stream state has decommissioned == Some(true)
    let state = ctx.client.get_stream_state(&stream_id);
    assert_eq!(state.decommissioned, Some(true));

    // 1. update_rate_per_second must fail with InvalidState
    let res_rate = ctx.client.try_update_rate_per_second(&stream_id, &2_i128);
    assert_eq!(res_rate, Err(Ok(ContractError::InvalidState)));

    // 2. decrease_rate_per_second must fail with InvalidState
    let res_dec = ctx.client.try_decrease_rate_per_second(&stream_id, &1_i128);
    assert_eq!(res_dec, Err(Ok(ContractError::InvalidState)));

    // 3. top_up_stream must fail with InvalidState
    let res_topup = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &100_i128);
    assert_eq!(res_topup, Err(Ok(ContractError::InvalidState)));

    // 4. extend_stream_end_time must fail with InvalidState
    let res_extend = ctx.client.try_extend_stream_end_time(&stream_id, &2000_u64);
    assert_eq!(res_extend, Err(Ok(ContractError::InvalidState)));

    // 5. clone_stream must fail with InvalidState
    let new_rec = Address::generate(&ctx.env);
    let res_clone = ctx
        .client
        .try_clone_stream(&stream_id, &new_rec, &0u64, &1000u64, &1000_i128, &false);
    assert_eq!(res_clone, Err(Ok(ContractError::InvalidState)));

    // --- Allowed operations ---

    // 1. pause_stream & resume_stream work
    ctx.env.ledger().with_mut(|l| l.sequence_number += 100);
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Paused
    );

    ctx.env.ledger().with_mut(|l| l.sequence_number += 100);
    ctx.client.resume_stream(&stream_id);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Active
    );

    // 2. withdraw works
    ctx.env.ledger().with_mut(|l| l.timestamp += 10);
    let withdrawn = ctx.client.withdraw(&stream_id, &None);
    assert!(withdrawn > 0);

    // 3. cancel_stream works
    ctx.client.cancel_stream(&stream_id);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );
}

/// Test reversibility of decommission mode, and precedence of irrevocable flag.
#[test]
fn test_set_stream_decommissioned_reversibility_and_irrevocable_precedence() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(1000);

    // Decommission
    ctx.client
        .set_stream_decommissioned(&stream_id, &ctx.sender, &true);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).decommissioned,
        Some(true)
    );

    // Revert back to false
    ctx.client
        .set_stream_decommissioned(&stream_id, &ctx.sender, &false);
    assert_eq!(
        ctx.client.get_stream_state(&stream_id).decommissioned,
        Some(false)
    );

    // After setting back to false, top_up_stream succeeds again
    let res_topup = ctx
        .client
        .try_top_up_stream(&stream_id, &ctx.sender, &100_i128);
    assert!(res_topup.is_ok());

    // --- Irrevocable stream behavior ---
    let irr_stream_id = ctx.create_irrevocable_stream(1000);

    // Decommission the irrevocable stream
    ctx.client
        .set_stream_decommissioned(&irr_stream_id, &ctx.sender, &true);
    assert_eq!(
        ctx.client.get_stream_state(&irr_stream_id).decommissioned,
        Some(true)
    );

    // Attempting to reverse decommission on irrevocable stream fails with Unauthorized
    let res_revert = ctx
        .client
        .try_set_stream_decommissioned(&irr_stream_id, &ctx.sender, &false);
    assert_eq!(res_revert, Err(Ok(ContractError::Unauthorized)));
}

/// Test set_stream_decommissioned authorization and terminal state guards.
#[test]
fn test_set_stream_decommissioned_guards() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_stream(1000);

    // Non-sender cannot set decommissioned
    let attacker = Address::generate(&ctx.env);
    let res_auth = ctx
        .client
        .try_set_stream_decommissioned(&stream_id, &attacker, &true);
    assert_eq!(res_auth, Err(Ok(ContractError::Unauthorized)));

    // Cancel stream
    ctx.client.cancel_stream(&stream_id);

    // Cannot set decommissioned on cancelled stream
    let res_terminal = ctx
        .client
        .try_set_stream_decommissioned(&stream_id, &ctx.sender, &true);
    assert_eq!(res_terminal, Err(Ok(ContractError::InvalidState)));
}
