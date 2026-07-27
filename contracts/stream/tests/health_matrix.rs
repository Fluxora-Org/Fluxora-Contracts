#![cfg(test)]

use fluxora_stream::{
    CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    MAX_RECIPIENT_PAGE_SIZE,
};
use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env};

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        let token_admin_client = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
        token_admin_client.mint(&sender, &1_000_000_000);

        Self {
            env,
            client,
            sender,
            recipient,
        }
    }
}

#[test]
fn test_health_matrix_active_fully_funded_before_cliff() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(50);
    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(!health.is_expired);
    assert_eq!(health.accrued_to_date, 0);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(950));
}

#[test]
fn test_health_matrix_active_underfunded_mid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(300);
    let health = ctx.client.get_stream_health(&stream_id);

    assert!(health.is_underfunded);
    assert!(!health.is_expired);
    assert_eq!(health.accrued_to_date, 600);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(200));
}

#[test]
fn test_health_matrix_paused_underfunded_mid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 2_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(300);
    ctx.client
        .pause_stream(&stream_id, &PauseReason::Operational);

    let health = ctx.client.get_stream_health(&stream_id);

    assert!(health.is_underfunded);
    assert!(!health.is_expired);
    assert_eq!(health.accrued_to_date, 600);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(200));
}

#[test]
fn test_health_matrix_expired_not_fully_withdrawn() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(1200);
    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(health.is_expired);
    assert_eq!(health.accrued_to_date, 1000);
    assert_eq!(health.remaining_deposit, 1000);
    assert_eq!(health.seconds_until_depletion, Some(0));
}

#[test]
fn test_health_matrix_completed_after_end() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(1200);
    ctx.client.withdraw(&stream_id);

    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(!health.is_expired);
    assert_eq!(health.accrued_to_date, 1000);
    assert_eq!(health.remaining_deposit, 0);
    assert_eq!(health.seconds_until_depletion, Some(0));
}

#[test]
fn test_health_matrix_cancelled_mid() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 100u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(500);
    ctx.client.cancel_stream(&stream_id);

    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(!health.is_expired);
    assert_eq!(health.accrued_to_date, 500);
    // Cancellation does not adjust deposit_amount in state, so remaining_deposit stays 1000 until withdraw.
    assert_eq!(health.remaining_deposit, 1000);
    // Seconds until depletion still returns the time remaining if it wasn't cancelled,
    // since the rate_per_second is unmodified.
    assert_eq!(health.seconds_until_depletion, Some(500));
}

#[test]
fn test_health_matrix_before_start() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // start_time=500, so ledger at t=0 is before the stream begins.
    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 500u64,
            cliff_time: 500u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(0);
    let health = ctx.client.get_stream_health(&stream_id);

    assert!(!health.is_underfunded);
    assert!(!health.is_expired);
    assert_eq!(health.accrued_to_date, 0);
    assert_eq!(health.remaining_deposit, 1000);
    // No accrual yet, so depletion timer is undefined -> None.
    assert_eq!(health.seconds_until_depletion, None);
}

// ────────────────────────────────────────────────────────────────────────────
// Portfolio-level tests: get_sender_portfolio_health
// ────────────────────────────────────────────────────────────────────────────

/// Helper: create a linear stream returning the stream_id.
fn create_linear(
    ctx: &TestContext,
    deposit: i128,
    rate: i128,
    cliff: u64,
    end: u64,
) -> u64 {
    ctx.client.create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: rate,
            start_time: 0u64,
            cliff_time: cliff,
            end_time: end,
            withdraw_dust_threshold: Some(0_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    )
}

#[test]
fn test_portfolio_health_no_streams() {
    let ctx = TestContext::setup();
    let other = Address::generate(&ctx.env);

    let page = ctx
        .client
        .get_sender_portfolio_health(&other, &0u64, &10);
    assert_eq!(page.underfunded_count, 0);
    assert_eq!(page.expired_count, 0);
    assert_eq!(page.healthy_count, 0);
    assert_eq!(page.next_cursor, 0u64);
    assert_eq!(page.stream_ids.len(), 0);
}

#[test]
fn test_portfolio_health_single_healthy() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let sid = create_linear(&ctx, 1000, 1, 0, 1000);

    ctx.env.ledger().set_timestamp(100);
    let page = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &100);

    assert_eq!(page.underfunded_count, 0);
    assert_eq!(page.expired_count, 0);
    assert_eq!(page.healthy_count, 1);
    assert_eq!(page.next_cursor, 0u64);
    assert!(page.stream_ids.iter().any(|id| id == sid));
}

#[test]
fn test_portfolio_health_single_underfunded() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    create_linear(&ctx, 1000, 2, 0, 1000);
    // at t=300, accrued = 600, deposit = 1000, need = 2×1000 = 2000 → underfunded
    ctx.env.ledger().set_timestamp(300);

    let page = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &100);
    assert_eq!(page.underfunded_count, 1);
    assert_eq!(page.expired_count, 0);
    assert_eq!(page.healthy_count, 0);
}

#[test]
fn test_portfolio_health_single_expired() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    create_linear(&ctx, 1000, 1, 0, 100);

    ctx.env.ledger().set_timestamp(200); // past end_time, not withdrawn → expired
    let page = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &100);
    assert_eq!(page.underfunded_count, 0);
    assert_eq!(page.expired_count, 1);
    assert_eq!(page.healthy_count, 0);
}

#[test]
fn test_portfolio_health_mixed_states() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // healthy: rate 1, end 10000
    create_linear(&ctx, 10_000, 1, 0, 10_000);
    // underfunded: rate 2, deposit 1000, end 1000 → needs 2000
    create_linear(&ctx, 1000, 2, 0, 1000);
    // expired: end 100
    create_linear(&ctx, 1000, 1, 0, 100);

    ctx.env.ledger().set_timestamp(200);
    let page = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &100);
    assert_eq!(page.underfunded_count, 1);
    assert_eq!(page.expired_count, 1);
    assert_eq!(page.healthy_count, 1);
    assert_eq!(page.stream_ids.len(), 3);
}

#[test]
fn test_portfolio_health_excludes_completed_and_cancelled() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let active = create_linear(&ctx, 10_000, 1, 0, 10_000);
    let to_cancel = create_linear(&ctx, 1000, 1, 0, 1000);

    ctx.env.ledger().set_timestamp(500);
    ctx.client.cancel_stream(&to_cancel);

    // Withdraw fully from the first stream to complete it
    ctx.env.ledger().set_timestamp(10_000);
    ctx.client.withdraw(&active);

    ctx.env.ledger().set_timestamp(10_100);
    let page = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &100);
    // Both terminal → excluded from all buckets
    assert_eq!(page.underfunded_count, 0);
    assert_eq!(page.expired_count, 0);
    assert_eq!(page.healthy_count, 0);
    // Terminal streams are still listed in stream_ids
    assert_eq!(page.stream_ids.len(), 2);
}

#[test]
fn test_portfolio_health_cursor_pagination() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create MAX+5 streams to force two pages
    for _ in 0..(MAX_RECIPIENT_PAGE_SIZE + 5) {
        create_linear(&ctx, 10_000, 1, 0, 10_000);
    }

    ctx.env.ledger().set_timestamp(100);
    let page1 = ctx.client.get_sender_portfolio_health(
        &ctx.sender,
        &0u64,
        &MAX_RECIPIENT_PAGE_SIZE,
    );
    assert_eq!(page1.stream_ids.len(), MAX_RECIPIENT_PAGE_SIZE as usize);
    assert_ne!(page1.next_cursor, 0u64, "must have more pages");

    let page2 = ctx.client.get_sender_portfolio_health(
        &ctx.sender,
        &page1.next_cursor,
        &MAX_RECIPIENT_PAGE_SIZE,
    );
    assert_eq!(page2.stream_ids.len(), 5);
    assert_eq!(page2.next_cursor, 0u64, "last page → next_cursor == 0");
}

#[test]
fn test_portfolio_health_limit_zero_uses_max() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    for _ in 0..5 {
        create_linear(&ctx, 10_000, 1, 0, 10_000);
    }

    ctx.env.ledger().set_timestamp(100);
    // limit=0 → treated as MAX_PAGE_SIZE
    let page = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &0);
    assert_eq!(page.stream_ids.len(), 5);
    assert_eq!(page.healthy_count, 5);
}

#[test]
fn test_portfolio_health_limit_clamped_to_max() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    for _ in 0..5 {
        create_linear(&ctx, 10_000, 1, 0, 10_000);
    }

    ctx.env.ledger().set_timestamp(100);
    // limit way above MAX → capped
    let page = ctx.client.get_sender_portfolio_health(
        &ctx.sender,
        &0u64,
        &(MAX_RECIPIENT_PAGE_SIZE * 3),
    );
    assert_eq!(page.stream_ids.len(), 5); // only 5 exist, so all returned
}

#[test]
fn test_portfolio_health_past_end_cursor_returns_empty_page() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    for _ in 0..3 {
        create_linear(&ctx, 1000, 1, 0, 1000);
    }

    ctx.env.ledger().set_timestamp(100);
    let page = ctx.client.get_sender_portfolio_health(
        &ctx.sender,
        &0xFFFF_FFFF_FFFF_FFFFu64,
        &100,
    );
    assert_eq!(page.stream_ids.len(), 0);
    assert_eq!(page.next_cursor, 0u64);
}

#[test]
fn test_portfolio_health_paused_stream_counts_as_healthy_when_funded() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let sid = create_linear(&ctx, 10_000, 1, 0, 10_000);
    ctx.client.pause_stream(&sid, &PauseReason::Operational);

    ctx.env.ledger().set_timestamp(100);
    let page = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &100);
    assert_eq!(page.healthy_count, 1);
    assert_eq!(page.underfunded_count, 0);
    assert_eq!(page.expired_count, 0);
}

#[test]
fn test_portfolio_health_paused_underfunded_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let sid = create_linear(&ctx, 1000, 2, 0, 1000);
    ctx.env.ledger().set_timestamp(300);
    ctx.client.pause_stream(&sid, &PauseReason::Operational);

    let page = ctx
        .client
        .get_sender_portfolio_health(&ctx.sender, &0u64, &100);
    assert_eq!(page.underfunded_count, 1);
    assert_eq!(page.healthy_count, 0);
}
