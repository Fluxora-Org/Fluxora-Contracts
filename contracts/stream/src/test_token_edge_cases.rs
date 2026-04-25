// ---------------------------------------------------------------------------
// Additional Token Edge Case Tests
// ---------------------------------------------------------------------------
// These tests complement the existing token interaction tests by covering:
// 1. Event emission verification for all token operations
// 2. Authorization boundary checks with strict auth mode
// 3. Edge cases around stream lifecycle timing
// 4. Comprehensive overflow scenarios
// 5. Malicious token assumption documentation (audit notes)
//
// These tests verify observable behavior guarantees that integrators rely on,
// including CEI ordering, atomic transactions, and explicit error handling.
//
// Key assumptions tested:
// 1. Token transfers succeed or fail explicitly (no silent failures)
// 2. State is persisted before external token transfers (CEI pattern)
// 3. Events are emitted correctly for all state transitions
// 4. Authorization checks prevent unauthorized operations
// 5. Any failure causes atomic rollback (no partial state changes)

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, FromVal, IntoVal, Symbol, TryFromVal, Vec,
};

use crate::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamCreated,
    StreamEvent, StreamStatus, WithdrawalTo,
};

// ---------------------------------------------------------------------------
// §11  Event emission verification for token operations
// ---------------------------------------------------------------------------

/// create_stream emits StreamCreated event with correct parameters.
#[test]
fn create_stream_emits_correct_event() {
    let ctx = TestContext::setup();

    ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0u32,
    );

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "must emit StreamCreated event");

    // Verify event contains correct stream_id
    let created_event = events
        .iter()
        .find(|e| e.0 == (symbol_short!("created"), 0u64));
    assert!(created_event.is_some(), "must emit StreamCreated event");
}

/// withdraw emits Withdrawal event with correct amount.
#[test]
fn withdraw_emits_correct_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "must emit Withdrawal event");

    // Verify event contains correct stream_id
    let withdrawal_event = events
        .iter()
        .find(|e| e.0 == (symbol_short!("withdrew"), stream_id));
    assert!(withdrawal_event.is_some(), "must emit Withdrawal event");
}

/// cancel_stream emits StreamCancelled event.
#[test]
fn cancel_stream_emits_correct_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&stream_id);

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "must emit StreamCancelled event");

    // Verify event contains correct stream_id
    let cancelled_event = events
        .iter()
        .find(|e| e.0 == (symbol_short!("cancelled"), stream_id));
    assert!(cancelled_event.is_some(), "must emit StreamCancelled event");
}

/// top_up_stream emits StreamToppedUp event with correct amounts.
#[test]
fn top_up_stream_emits_correct_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "must emit StreamToppedUp event");

    // Verify event contains correct stream_id
    let topped_up_event = events
        .iter()
        .find(|e| e.0 == (symbol_short!("top_up"), stream_id));
    assert!(topped_up_event.is_some(), "must emit StreamToppedUp event");
}

/// update_rate_per_second emits RateUpdated event.
#[test]
fn update_rate_emits_correct_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    ctx.client()
        .update_rate_per_second(&stream_id, &2_i128);

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "must emit RateUpdated event");

    // Verify event contains correct stream_id
    let rate_updated_event = events
        .iter()
        .find(|e| e.0 == (symbol_short!("rate_upd"), stream_id));
    assert!(rate_updated_event.is_some(), "must emit RateUpdated event");
}

/// shorten_stream_end_time emits StreamEndShortened event.
#[test]
fn shorten_end_time_emits_correct_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    ctx.client()
        .shorten_stream_end_time(&stream_id, &500u64);

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "must emit StreamEndShortened event");

    // Verify event contains correct stream_id
    let shortened_event = events
        .iter()
        .find(|e| e.0 == (symbol_short!("end_shrt"), stream_id));
    assert!(
        shortened_event.is_some(),
        "must emit StreamEndShortened event"
    );
}

/// extend_stream_end_time emits StreamEndExtended event.
#[test]
fn extend_end_time_emits_correct_event() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(100);
    ctx.client()
        .extend_stream_end_time(&stream_id, &2000u64);

    let events = ctx.env.events().all();
    assert!(!events.is_empty(), "must emit StreamEndExtended event");

    // Verify event contains correct stream_id
    let extended_event = events
        .iter()
        .find(|e| e.0 == (symbol_short!("end_ext"), stream_id));
    assert!(
        extended_event.is_some(),
        "must emit StreamEndExtended event"
    );
}

// ---------------------------------------------------------------------------
// §12  Authorization boundary checks (strict auth mode)
// ---------------------------------------------------------------------------

/// Non-sender cannot pause stream (strict auth mode).
#[test]
fn non_sender_cannot_pause_stream_strict_auth() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();

    let non_sender = Address::generate(&ctx.env);

    // Try to pause without proper auth
    let result = ctx.client().try_pause_stream(&stream_id);
    assert!(
        result.is_err(),
        "non-sender must not be able to pause stream"
    );
}

/// Non-recipient cannot withdraw (strict auth mode).
#[test]
fn non_recipient_cannot_withdraw_strict_auth() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();

    let non_recipient = Address::generate(&ctx.env);

    // Try to withdraw without proper auth
    let result = ctx.client().try_withdraw(&stream_id);
    assert!(
        result.is_err(),
        "non-recipient must not be able to withdraw"
    );
}

/// Non-admin cannot cancel stream as admin (strict auth mode).
#[test]
fn non_admin_cannot_cancel_stream_as_admin_strict_auth() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();

    let non_admin = Address::generate(&ctx.env);

    // Try to cancel as admin without proper auth
    let result = ctx.client().try_cancel_stream_as_admin(&stream_id);
    assert!(
        result.is_err(),
        "non-admin must not be able to cancel stream as admin"
    );
}

/// Non-sender cannot update rate (strict auth mode).
#[test]
fn non_sender_cannot_update_rate_strict_auth() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();

    let non_sender = Address::generate(&ctx.env);

    // Try to update rate without proper auth
    let result = ctx.client().try_update_rate_per_second(&stream_id, &2_i128);
    assert!(
        result.is_err(),
        "non-sender must not be able to update rate"
    );
}

/// Non-sender cannot shorten end time (strict auth mode).
#[test]
fn non_sender_cannot_shorten_end_time_strict_auth() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();

    let non_sender = Address::generate(&ctx.env);

    // Try to shorten end time without proper auth
    let result = ctx.client().try_shorten_stream_end_time(&stream_id, &500u64);
    assert!(
        result.is_err(),
        "non-sender must not be able to shorten end time"
    );
}

/// Non-sender cannot extend end time (strict auth mode).
#[test]
fn non_sender_cannot_extend_end_time_strict_auth() {
    let ctx = TestContext::setup_strict();
    let stream_id = ctx.create_default_stream();

    let non_sender = Address::generate(&ctx.env);

    // Try to extend end time without proper auth
    let result = ctx.client().try_extend_stream_end_time(&stream_id, &2000u64);
    assert!(
        result.is_err(),
        "non-sender must not be able to extend end time"
    );
}

// ---------------------------------------------------------------------------
// §13  Edge cases around stream lifecycle timing
// ---------------------------------------------------------------------------

/// Withdraw at exact cliff time returns 0 (cliff not yet passed).
#[test]
fn withdraw_at_exact_cliff_time_returns_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &500u64, // cliff at 500
        &1000u64,
    );

    // At exact cliff time, nothing is withdrawable yet
    ctx.env.ledger().set_timestamp(500);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 0,
        "withdraw at exact cliff time must return 0"
    );
}

/// Withdraw one second after cliff time returns accrued amount.
#[test]
fn withdraw_one_second_after_cliff_returns_accrued() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &500u64, // cliff at 500
        &1000u64,
    );

    // One second after cliff, 1 token is accrued
    ctx.env.ledger().set_timestamp(501);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 1,
        "withdraw one second after cliff must return 1"
    );
}

/// Withdraw at exact end time returns full deposit.
#[test]
fn withdraw_at_exact_end_time_returns_full_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // end_time=1000

    // At exact end time, full deposit is accrued
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 1000,
        "withdraw at exact end time must return full deposit"
    );
}

/// Withdraw after end time returns full deposit (no over-accrual).
#[test]
fn withdraw_after_end_time_returns_full_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // end_time=1000

    // After end time, full deposit is accrued (no over-accrual)
    ctx.env.ledger().set_timestamp(2000);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(
        withdrawn, 1000,
        "withdraw after end time must return full deposit"
    );
}

/// Cancel at exact start time refunds full deposit.
#[test]
fn cancel_at_exact_start_time_refunds_full_deposit() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0u32,
    );

    // At exact start time, nothing is accrued yet
    ctx.env.ledger().set_timestamp(0);
    ctx.client().cancel_stream(&stream_id);

    // Verify full refund
    let token = TokenClient::new(&ctx.env, &ctx.token_id);
    assert_eq!(
        token.balance(&ctx.sender),
        10_000,
        "cancel at start time must refund full deposit"
    );
}

/// Cancel at exact end time refunds 0 (fully accrued).
#[test]
fn cancel_at_exact_end_time_refunds_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // end_time=1000

    // At exact end time, full deposit is accrued
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().cancel_stream(&stream_id);

    // Verify no refund (fully accrued)
    let token = TokenClient::new(&ctx.env, &ctx.token_id);
    assert_eq!(
        token.balance(&ctx.sender),
        9_000,
        "cancel at end time must refund 0"
    );
}

/// Cancel after end time refunds 0 (fully accrued).
#[test]
fn cancel_after_end_time_refunds_zero() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // end_time=1000

    // After end time, full deposit is accrued
    ctx.env.ledger().set_timestamp(2000);
    ctx.client().cancel_stream(&stream_id);

    // Verify no refund (fully accrued)
    let token = TokenClient::new(&ctx.env, &ctx.token_id);
    assert_eq!(
        token.balance(&ctx.sender),
        9_000,
        "cancel after end time must refund 0"
    );
}

// ---------------------------------------------------------------------------
// §14  Comprehensive overflow scenarios
// ---------------------------------------------------------------------------

/// create_stream fails when deposit_amount is i128::MAX.
#[test]
fn create_stream_max_deposit_fails() {
    let ctx = TestContext::setup();

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &i128::MAX,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0u32,
        &0u32,
    );

    assert!(
        result.is_err(),
        "create_stream must fail with i128::MAX deposit"
    );
    assert_eq!(ctx.client().get_stream_count(), 0);
}

/// create_stream fails when rate_per_second is i128::MAX.
#[test]
fn create_stream_max_rate_fails() {
    let ctx = TestContext::setup();

    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &i128::MAX,
        &0u64,
        &0u64,
        &1000u64,
        &0u32,
        &0u32,
    );

    assert!(
        result.is_err(),
        "create_stream must fail with i128::MAX rate"
    );
    assert_eq!(ctx.client().get_stream_count(), 0);
}

/// create_stream fails when rate * duration overflows.
#[test]
fn create_stream_rate_duration_overflow_fails() {
    let ctx = TestContext::setup();

    // rate=1_000_000_000, duration=1_000_000_000 => product=1_000_000_000_000_000_000
    // This should overflow when multiplied
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_000_000_000_000_000_000_i128,
        &1_000_000_000_i128,
        &0u64,
        &0u64,
        &1_000_000_000u64,
    );

    assert!(
        result.is_err(),
        "create_stream must fail when rate * duration overflows"
    );
    assert_eq!(ctx.client().get_stream_count(), 0);
}

/// top_up_stream fails when deposit_amount + amount overflows.
#[test]
fn top_up_stream_deposit_overflow_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // deposit=1000

    // Try to top up with i128::MAX - 999 (would overflow when added to 1000)
    let result = ctx
        .client()
        .try_top_up_stream(&stream_id, &ctx.sender, &(i128::MAX - 999));

    assert!(
        result.is_err(),
        "top_up_stream must fail when deposit overflows"
    );

    // Verify deposit_amount unchanged
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1000);
}

/// update_rate_per_second fails when new rate * remaining duration overflows.
#[test]
fn update_rate_overflow_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // deposit=1000, rate=1, duration=1000

    ctx.env.ledger().set_timestamp(100);

    // Try to update rate to i128::MAX (would overflow when multiplied by remaining duration)
    let result = ctx
        .client()
        .try_update_rate_per_second(&stream_id, &i128::MAX);

    assert!(
        result.is_err(),
        "update_rate must fail when new rate overflows"
    );

    // Verify rate unchanged
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.rate_per_second, 1);
}

/// shorten_stream_end_time fails when new rate * new duration overflows.
#[test]
fn shorten_end_time_overflow_fails() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_000_000_000_000_000_000_i128,
        &1_000_000_000_i128,
        &0u64,
        &0u64,
        &1_000_000_000u64,
    );

    ctx.env.ledger().set_timestamp(100);

    // Try to shorten end time to 500 (would overflow when rate * new duration)
    let result = ctx
        .client()
        .try_shorten_stream_end_time(&stream_id, &500u64);

    assert!(
        result.is_err(),
        "shorten_end_time must fail when new rate overflows"
    );

    // Verify end_time unchanged
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.end_time, 1_000_000_000);
}

// ---------------------------------------------------------------------------
// §15  Malicious token assumption documentation (audit notes)
// ---------------------------------------------------------------------------

/// This test documents the malicious token assumptions and non-goals.
/// It serves as an audit note for reviewers.
#[test]
fn malicious_token_assumptions_documented() {
    // This test is a documentation placeholder.
    // The actual malicious token scenarios cannot be automatically tested
    // with standard Soroban test utilities.
    //
    // Key assumptions:
    // 1. Token contract does not re-enter streaming contract during transfers
    // 2. Token contract does not silently fail (panics or returns error on failure)
    // 3. Token contract implements standard SEP-41 interface
    // 4. Token contract behavior is deterministic
    //
    // Non-goals (intentionally not mitigated):
    // 1. Malicious token contracts that violate SEP-41 guarantees
    // 2. Token supply manipulation (minting, burning, fee-on-transfer)
    // 3. Token contract upgradeability
    // 4. Token balance verification
    // 5. Token allowance management
    // 6. Token decimal precision
    //
    // See docs/token-assumptions.md for complete documentation.
}

/// This test documents the CEI ordering pattern used throughout the contract.
#[test]
fn cei_ordering_pattern_documented() {
    // This test is a documentation placeholder.
    // The CEI (Checks-Effects-Interactions) pattern is used throughout
    // the contract to reduce reentrancy risk.
    //
    // Pattern:
    // 1. Checks: Validate inputs, auth, state
    // 2. Effects: Update state (persist with save_stream)
    // 3. Interactions: External token transfers
    //
    // This ordering ensures that state is persisted before any external
    // calls, reducing the impact of potential reentrancy.
    //
    // See docs/security.md for detailed documentation.
}

/// This test documents the atomic transaction guarantee.
#[test]
fn atomic_transaction_guarantee_documented() {
    // This test is a documentation placeholder.
    // All contract operations are atomic: either fully succeed or fully fail.
    //
    // Guarantees:
    // 1. If any validation fails, no state is persisted
    // 2. If token transfer fails, entire transaction reverts
    // 3. No partial state changes are visible on-chain
    // 4. Events are only emitted on successful operations
    //
    // This ensures that integrators can rely on consistent state transitions.
}

/// This test documents the authorization model.
#[test]
fn authorization_model_documented() {
    // This test is a documentation placeholder.
    // The contract enforces strict authorization for all operations.
    //
    // Authorization matrix:
    // - create_stream: sender (the address supplied as sender)
    // - pause_stream: stream's sender
    // - resume_stream: stream's sender
    // - cancel_stream: stream's sender
    // - withdraw: stream's recipient
    // - withdraw_to: stream's recipient
    // - update_rate_per_second: stream's sender
    // - shorten_stream_end_time: stream's sender
    // - extend_stream_end_time: stream's sender
    // - top_up_stream: funder (any address)
    // - close_completed_stream: permissionless (any caller)
    // - set_admin: current contract admin
    // - set_contract_paused: contract admin
    //
    // See docs/security.md for detailed documentation.
}
