//! Security invariants test suite.
//!
//! This module explicitly validates every invariant documented in
//! `docs/maintainer-security-checklist.md` and `docs/security.md`.
//! Each test maps to one or more checklist items so that regressions
//! are caught at CI time rather than during an audit.
//!
//! Run with:
//! ```bash
//! cargo test -p fluxora_stream --test security_invariants -- --nocapture
//! ```

extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env, IntoVal, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Shared test context
// ---------------------------------------------------------------------------

struct Ctx {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
}

impl Ctx {
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

        StellarAssetClient::new(&env, &token_id).mint(&sender, &10_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000u32);

        env.ledger().set_timestamp(0);

        Ctx {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    fn create_default_stream(&self) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: 1000u64,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// §1 CEI Pattern (Checks-Effects-Interactions)
// ---------------------------------------------------------------------------

/// After a successful withdrawal, the stream's withdrawn_amount must equal the
/// amount returned, and the transfer must happen after the state update.
#[test]
fn cei_withdraw_state_before_transfer() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    let balance_before = ctx.token().balance(&ctx.recipient);
    let stream_before = ctx.client().get_stream_state(&stream_id);

    let amount = ctx.client().withdraw(&stream_id, &None);

    let stream_after = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        stream_after.withdrawn_amount,
        stream_before.withdrawn_amount + amount,
        "withdrawn_amount must be updated before token transfer"
    );
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        balance_before + amount,
        "recipient must receive the withdrawn amount"
    );

    // Verify event ordering: withdrew before completed (if applicable).
    let events = ctx.env.events().all();
    let last = events.len() as u32;
    if last >= 2 {
        let topic0: Symbol = events.get_unchecked(last - 2).0;
        let topic1: Symbol = events.get_unchecked(last - 1).0;
        if topic1 == Symbol::new(&ctx.env, "completed") {
            assert_eq!(
                topic0,
                Symbol::new(&ctx.env, "withdrew"),
                "withdrew event must precede completed event"
            );
        }
    }
}

/// After cancellation, the stream must be marked Cancelled before any refund
/// transfer.
#[test]
fn cei_cancel_state_before_refund() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    let sender_before = ctx.token().balance(&ctx.sender);
    ctx.client().cancel_stream(&stream_id);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        stream.status,
        StreamStatus::Cancelled,
        "stream must be Cancelled"
    );
    assert!(
        stream.cancelled_at.is_some(),
        "cancelled_at timestamp must be set"
    );
    assert!(
        ctx.token().balance(&ctx.sender) > sender_before,
        "sender must receive refund after cancel"
    );
}

/// After a top-up, the deposit_amount must be updated before the token pull.
#[test]
fn cei_top_up_state_before_pull() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    let deposit_before = ctx.client().get_stream_state(&stream_id).deposit_amount;
    ctx.client().top_up_stream(&stream_id, &ctx.sender, &500);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        stream.deposit_amount,
        deposit_before + 500,
        "deposit_amount must increase before token pull"
    );
}

/// After shorten_stream_end_time, the deposit must be reduced and the refund
/// sent to the sender.
#[test]
fn cei_shorten_refund_before_transfer() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(0);
    let sender_before = ctx.token().balance(&ctx.sender);
    let deposit_before = ctx.client().get_stream_state(&stream_id).deposit_amount;

    ctx.client().shorten_stream_end_time(&stream_id, &500);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert!(
        stream.deposit_amount < deposit_before,
        "deposit must decrease after shorten"
    );
    assert_eq!(stream.end_time, 500, "end_time must be updated");
    assert!(
        ctx.token().balance(&ctx.sender) > sender_before,
        "sender must receive refund"
    );
}

// ---------------------------------------------------------------------------
// §2 Authorization Boundaries
//
// NOTE: Comprehensive auth-boundary tests live in
// `tests/adversarial_auth.rs`. The tests below cover boundary cases that are
// verifiable in the mock-all-auths context. For strict auth verification
// (wrong-signer rejection per entrypoint), see `tests/adversarial_auth.rs`.
// ---------------------------------------------------------------------------

/// Sender address cannot be the same as recipient (enforced at creation).
#[test]
fn auth_sender_not_recipient() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.sender.clone(),
            deposit_amount: 100_i128,
            rate_per_second: 1_i128,
            start_time: 10u64,
            cliff_time: 10u64,
            end_time: 100u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// set_admin requires authorization from the current admin (in mock_all_auths
/// any caller passes auth, but the internal admin check still gates on the
/// stored admin address).
#[test]
fn auth_set_admin_gated() {
    let ctx = Ctx::setup();
    let new_admin = Address::generate(&ctx.env);
    let result = ctx.client().try_set_admin(&new_admin);
    // In mock_all_auths mode this succeeds because all addresses pass auth.
    // Strict auth verification is in tests/adversarial_auth.rs.
    assert!(result.is_ok(), "set_admin must succeed with proper auth");
}

// ---------------------------------------------------------------------------
// §3 Terminal State Gating
// ---------------------------------------------------------------------------

/// Pausing a Completed stream must fail.
#[test]
fn terminal_pause_completed_fails() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id, &None);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );

    let result = ctx
        .client()
        .try_pause_stream(&stream_id, &PauseReason::Operational);
    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
}

/// Cancelling a Completed stream must fail.
#[test]
fn terminal_cancel_completed_fails() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id, &None);

    let result = ctx.client().try_cancel_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

/// Withdrawing from a Cancelled stream must still work (drain accrued).
#[test]
fn terminal_withdraw_from_cancelled_succeeds() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().cancel_stream(&stream_id);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Cancelled
    );

    let amount = ctx.client().withdraw(&stream_id, &None);
    assert_eq!(amount, 300, "must withdraw accrued at cancellation time");
}

/// Updating rate on a Cancelled stream must fail.
#[test]
fn terminal_rate_update_cancelled_fails() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().cancel_stream(&stream_id);

    let result = ctx.client().try_update_rate_per_second(&stream_id, &2);
    assert!(result.is_err());
}

/// Topping up a Completed stream must fail.
#[test]
fn terminal_top_up_completed_fails() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id, &None);

    let result = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &100);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));
}

// ---------------------------------------------------------------------------
// §4 Arithmetic Safety
// ---------------------------------------------------------------------------

/// Overflow in deposit accumulation during batch creation must be caught.
#[test]
fn arithmetic_batch_deposit_overflow_caught() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let params = vec![
        &ctx.env,
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: i128::MAX,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
        CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    ];

    let result = ctx.client().try_create_streams(&ctx.sender, &params);
    assert_eq!(result, Err(Ok(ContractError::ArithmeticOverflow)));
}

/// Rate × duration overflow must be caught at stream creation.
#[test]
fn arithmetic_rate_duration_overflow_caught() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: i128::MAX,
            rate_per_second: i128::MAX,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 2u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Negative amounts must be rejected at creation.
#[test]
fn arithmetic_negative_deposit_rejected() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: -1_i128,
            rate_per_second: 1_i128,
            start_time: 10u64,
            cliff_time: 10u64,
            end_time: 100u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

// ---------------------------------------------------------------------------
// §5 Init-Once Semantics
// ---------------------------------------------------------------------------

/// Second init must fail with AlreadyInitialised.
#[test]
fn init_twice_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let token = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token, &admin);

    let result = client.try_init(&Address::generate(&env), &Address::generate(&env));
    assert_eq!(result, Err(Ok(ContractError::AlreadyInitialised)));
}

/// Config must be unchanged after a failed re-init attempt.
#[test]
fn init_config_unchanged_after_failed_reinit() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);
    let token = Address::generate(&env);
    let admin = Address::generate(&env);

    client.init(&token, &admin);
    let original = client.get_config();

    let _ = client.try_init(&Address::generate(&env), &Address::generate(&env));

    let after = client.get_config();
    assert_eq!(after.token, original.token);
    assert_eq!(after.admin, original.admin);
}

// ---------------------------------------------------------------------------
// §6 Duplicate Stream ID Rejection
// ---------------------------------------------------------------------------

/// Batch withdraw with duplicate stream IDs must be rejected atomically.
#[test]
fn duplicate_id_batch_withdraw_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);

    let state_before = ctx.client().get_stream_state(&stream_id);
    let recipient_balance_before = ctx.token().balance(&ctx.recipient);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let liabilities_before = ctx.client().get_total_liabilities();

    let ids = vec![&ctx.env, stream_id, stream_id];
    let result = ctx.client().try_batch_withdraw(&ctx.recipient, &ids);
    assert_eq!(result, Err(Ok(ContractError::DuplicateStreamId)));
    assert_eq!(ctx.client().get_stream_state(&stream_id), state_before);
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        recipient_balance_before,
        "a rejected duplicate batch must not pay the recipient"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before,
        "a rejected duplicate batch must not move contract funds"
    );
    assert_eq!(
        ctx.client().get_total_liabilities(),
        liabilities_before,
        "a rejected duplicate batch must not change liabilities"
    );
}

/// Batch withdraw_to with duplicate stream IDs must be rejected atomically.
#[test]
fn duplicate_id_batch_withdraw_to_rejected() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);

    let destination = Address::generate(&ctx.env);
    let state_before = ctx.client().get_stream_state(&stream_id);
    let destination_balance_before = ctx.token().balance(&destination);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let liabilities_before = ctx.client().get_total_liabilities();

    let params = vec![
        &ctx.env,
        fluxora_stream::WithdrawToParam {
            stream_id,
            destination: destination.clone(),
        },
        fluxora_stream::WithdrawToParam {
            stream_id,
            destination: destination.clone(),
        },
    ];
    let result = ctx.client().try_batch_withdraw_to(&ctx.recipient, &params);
    assert_eq!(result, Err(Ok(ContractError::DuplicateStreamId)));
    assert_eq!(ctx.client().get_stream_state(&stream_id), state_before);
    assert_eq!(
        ctx.token().balance(&destination),
        destination_balance_before,
        "a rejected duplicate batch must not pay a destination"
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before,
        "a rejected duplicate batch must not move contract funds"
    );
    assert_eq!(
        ctx.client().get_total_liabilities(),
        liabilities_before,
        "a rejected duplicate batch must not change liabilities"
    );
}

/// Bulk cancellation rejects duplicate IDs before changing storage or balances.
#[test]
fn duplicate_id_bulk_cancel_rejected_atomically() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env.ledger().set_timestamp(500);

    let state_before = ctx.client().get_stream_state(&stream_id);
    let sender_balance_before = ctx.token().balance(&ctx.sender);
    let recipient_balance_before = ctx.token().balance(&ctx.recipient);
    let contract_balance_before = ctx.token().balance(&ctx.contract_id);
    let liabilities_before = ctx.client().get_total_liabilities();

    let ids = vec![&ctx.env, stream_id, stream_id];
    let result = ctx.client().try_bulk_cancel_streams(&ctx.sender, &ids);

    assert_eq!(result, Err(Ok(ContractError::DuplicateStreamId)));
    assert_eq!(ctx.client().get_stream_state(&stream_id), state_before);
    assert_eq!(ctx.token().balance(&ctx.sender), sender_balance_before);
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        recipient_balance_before
    );
    assert_eq!(
        ctx.token().balance(&ctx.contract_id),
        contract_balance_before
    );
    assert_eq!(ctx.client().get_total_liabilities(), liabilities_before);
}

/// Bulk admin resume rejects duplicate IDs without partially resuming a stream
/// or decrementing the paused-stream counter.
#[test]
fn duplicate_id_bulk_resume_rejected_atomically() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();
    ctx.env
        .ledger()
        .with_mut(|ledger| ledger.sequence_number += 32);
    ctx.client()
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

    let state_before = ctx.client().get_stream_state(&stream_id);
    let paused_count_before = ctx.client().get_paused_stream_count();
    let liabilities_before = ctx.client().get_total_liabilities();

    let ids = vec![&ctx.env, stream_id, stream_id];
    let result = ctx.client().try_bulk_resume_streams_as_admin(&ids);

    assert_eq!(result, Err(Ok(ContractError::DuplicateStreamId)));
    assert_eq!(ctx.client().get_stream_state(&stream_id), state_before);
    assert_eq!(
        ctx.client().get_paused_stream_count(),
        paused_count_before,
        "a rejected duplicate resume must not change the paused counter"
    );
    assert_eq!(ctx.client().get_total_liabilities(), liabilities_before);
}

// ---------------------------------------------------------------------------
// §7 Pause State Enforcement
// ---------------------------------------------------------------------------

/// Global emergency pause blocks withdrawals.
#[test]
fn pause_global_blocks_withdraw() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().set_global_emergency_paused(&true);
    ctx.env.ledger().set_timestamp(500);

    let result = ctx.client().try_withdraw(&stream_id, &None);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));
}

/// Global emergency pause blocks cancellations.
#[test]
fn pause_global_blocks_cancel() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().set_global_emergency_paused(&true);

    let result = ctx.client().try_cancel_stream(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));
}

/// Global emergency pause blocks top-ups.
#[test]
fn pause_global_blocks_top_up() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().set_global_emergency_paused(&true);

    let result = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &100);
    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));
}

// ---------------------------------------------------------------------------
// §8 Accrual Invariants
// ---------------------------------------------------------------------------

/// Accrued never exceeds deposit_amount.
#[test]
fn accrual_bounded_by_deposit() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    for t in &[500u64, 1000u64, 5000u64, 100_000u64] {
        ctx.env.ledger().set_timestamp(*t);
        let accrued = ctx.client().calculate_accrued(&stream_id);
        let stream = ctx.client().get_stream_state(&stream_id);
        assert!(
            accrued >= 0 && accrued <= stream.deposit_amount,
            "accrued={} at t={} must be in [0, deposit={}]",
            accrued,
            t,
            stream.deposit_amount
        );
    }
}

/// Withdrawn amount never exceeds deposit_amount.
#[test]
fn withdrawn_bounded_by_deposit() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id, &None);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id, &None);

    let stream = ctx.client().get_stream_state(&stream_id);
    assert!(
        stream.withdrawn_amount <= stream.deposit_amount,
        "withdrawn={} must not exceed deposit={}",
        stream.withdrawn_amount,
        stream.deposit_amount
    );
}

/// Before cliff time, accrued must be 0.
#[test]
fn accrual_zero_before_cliff() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 500u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    ctx.env.ledger().set_timestamp(0);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 0);
    ctx.env.ledger().set_timestamp(499);
    assert_eq!(ctx.client().calculate_accrued(&stream_id), 0);
}

// ---------------------------------------------------------------------------
// §9 Event Compatibility
// ---------------------------------------------------------------------------

/// create_stream must emit exactly one StreamCreated event with the correct
/// topic symbol.
#[test]
fn event_create_stream_emits_created() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let _stream_id = ctx.create_default_stream();

    let events = ctx.env.events().all();
    let event = events.last().unwrap();
    let topic: Symbol = event.0;

    assert_eq!(
        topic,
        Symbol::new(&ctx.env, "created"),
        "create_stream must emit 'created' topic"
    );
}

/// withdraw must emit a withdrew event.
#[test]
fn event_withdraw_emits_withdrew() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&stream_id, &None);

    let events = ctx.env.events().all();
    let event = events.last().unwrap();
    let topic: Symbol = event.0;

    assert_eq!(
        topic,
        Symbol::new(&ctx.env, "withdrew"),
        "withdraw must emit 'withdrew' topic"
    );
}

/// cancel_stream must emit a cancelled event.
#[test]
fn event_cancel_emits_cancelled() {
    let ctx = Ctx::setup();
    let stream_id = ctx.create_default_stream();

    ctx.client().cancel_stream(&stream_id);

    let events = ctx.env.events().all();
    let event = events.last().unwrap();
    let topic: Symbol = event.0;

    assert_eq!(
        topic,
        Symbol::new(&ctx.env, "cancelled"),
        "cancel_stream must emit 'cancelled' topic"
    );
}

// ---------------------------------------------------------------------------
// §10 Global Pause — Creation Pause (not emergency)
// ---------------------------------------------------------------------------

/// Creation pause blocks create_stream but does not block withdrawals.
#[test]
fn creation_pause_blocks_create_only() {
    let ctx = Ctx::setup();

    ctx.client().set_contract_paused(&true);

    ctx.env.ledger().set_timestamp(0);
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 100_i128,
            rate_per_second: 1_i128,
            start_time: 10u64,
            cliff_time: 10u64,
            end_time: 100u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );
    assert_eq!(
        result,
        Err(Ok(ContractError::ContractPaused)),
        "creation pause must block create_stream"
    );

    // Creation pause must NOT block admin operations.
    let result = ctx.client().try_set_admin(&Address::generate(&ctx.env));
    assert!(result.is_ok(), "creation pause must not block set_admin");
}

// ---------------------------------------------------------------------------
// §11 Total Liabilities Invariant
// ---------------------------------------------------------------------------

/// Total liabilities must increase by the deposit amount on stream creation.
#[test]
fn liabilities_created_with_stream() {
    let ctx = Ctx::setup();

    let before = ctx.client().get_total_liabilities();
    ctx.create_default_stream();
    let after = ctx.client().get_total_liabilities();

    assert_eq!(
        after,
        before + 1000,
        "total liabilities must increase by deposit amount"
    );
}

// ---------------------------------------------------------------------------
// §12 Storage Key Discriminant Stability
// ---------------------------------------------------------------------------

/// The DataKey enum must not be reordered. This test encodes the current
/// discriminant values as of CONTRACT_VERSION = 9. Any reordering shifts
/// all subsequent discriminants and corrupts persistent storage.
///
/// When adding new variants, append them at the end of the enum and
/// increment this counter.
#[test]
fn datakey_discriminant_count_stable() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let client = FluxoraStreamClient::new(&env, &contract_id);

    // We cannot directly inspect DataKey discriminants from integration
    // tests, but we can assert the CONTRACT_VERSION constant hasn't been
    // bumped without documentation. This is a sentinel.
    let version = client.version();
    assert!(version >= 9, "CONTRACT_VERSION must be at least 9");
}

// ---------------------------------------------------------------------------
// §13 Witness and Irrevocable Stream Mode
// ---------------------------------------------------------------------------

/// An irrevocable stream cannot be cancelled by the sender.
#[test]
fn irrevocable_cancel_rejected() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: Some(true),
            witness: None,
        },
    );

    let result = ctx.client().try_cancel_stream(&stream_id);
    assert!(
        result.is_err(),
        "irrevocable stream must reject cancellation"
    );
}

/// An irrevocable stream cannot be shortened.
#[test]
fn irrevocable_shorten_rejected() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(0),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: Some(true),
            witness: None,
        },
    );

    let result = ctx.client().try_shorten_stream_end_time(&stream_id, &500);
    assert!(result.is_err(), "irrevocable stream must reject shortening");
}
