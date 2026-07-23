//! Comprehensive tests for `clone_stream`.
//!
//! Covers:
//! - Happy-path cloning from Completed, Cancelled, Active, and Paused source streams
//! - Inherited fields: rate_per_second, cliff offset, withdraw_dust_threshold, memo
//! - Authorization: sender succeeds, recipient rejected, third-party rejected
//! - CliffOnly guard: force=false rejects sentinel threshold, force=true allows it
//! - Parameter validation: start_time in past, insufficient deposit, overflow
//! - Event emission: "created" + "cloned" events with correct payloads
//! - Global pause blocks cloning
//! - Source stream not found
//! - Cliff offset arithmetic: zero cliff, mid-stream cliff, cliff == end
//! - Token balance invariants after clone
//! - Multiple sequential clones (recurring payroll pattern)
extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, PauseReason, StreamCloned, StreamCreated,
    StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, IntoVal, Symbol, TryFromVal,
};

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    #[allow(dead_code)]
    sac: StellarAssetClient<'a>,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &100_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Ctx {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            sac,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    /// Create a standard stream: 1000 tokens, rate=1/s, 0..1000s, no cliff.
    fn create_default_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
        )
    }

    /// Create a stream with a cliff at t=500.
    fn create_cliff_stream(&self) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &500u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
        )
    }
}

// ---------------------------------------------------------------------------
// Happy-path: clone from Completed source
// ---------------------------------------------------------------------------

/// Cloning a Completed stream produces a new Active stream with the same rate.
#[test]
fn clone_from_completed_stream_succeeds() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    // Complete the source stream.
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);
    assert_eq!(
        ctx.client().get_stream_state(&source_id).status,
        StreamStatus::Completed
    );

    // Clone it for the next period.
    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(new_state.status, StreamStatus::Active);
    assert_eq!(new_state.rate_per_second, 1);
    assert_eq!(new_state.start_time, 1000);
    assert_eq!(new_state.end_time, 2000);
    assert_eq!(new_state.deposit_amount, 1000);
    assert_eq!(new_state.withdrawn_amount, 0);
    assert_eq!(new_state.sender, ctx.sender);
    assert_eq!(new_state.recipient, ctx.recipient);
}

/// Cloning a Cancelled stream succeeds.
#[test]
fn clone_from_cancelled_stream_succeeds() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(400);
    ctx.client().cancel_stream(&source_id);
    assert_eq!(
        ctx.client().get_stream_state(&source_id).status,
        StreamStatus::Cancelled
    );

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(new_state.status, StreamStatus::Active);
    assert_eq!(new_state.rate_per_second, 1);
}

/// Cloning an Active stream is allowed (pre-scheduling next period).
#[test]
fn clone_from_active_stream_succeeds() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(500);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    // Source stream is still Active and unaffected.
    assert_eq!(
        ctx.client().get_stream_state(&source_id).status,
        StreamStatus::Active
    );
    // New stream is also Active.
    assert_eq!(
        ctx.client().get_stream_state(&new_id).status,
        StreamStatus::Active
    );
}

/// Cloning a Paused stream is allowed.
#[test]
fn clone_from_paused_stream_succeeds() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(300);
    ctx.client()
        .pause_stream(&source_id, &PauseReason::Operational);
    assert_eq!(
        ctx.client().get_stream_state(&source_id).status,
        StreamStatus::Paused
    );

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(
        ctx.client().get_stream_state(&new_id).status,
        StreamStatus::Active
    );
    // Source stream remains Paused.
    assert_eq!(
        ctx.client().get_stream_state(&source_id).status,
        StreamStatus::Paused
    );
}

// ---------------------------------------------------------------------------
// Inherited fields
// ---------------------------------------------------------------------------

/// rate_per_second is copied verbatim from the source stream.
#[test]
fn clone_inherits_rate_per_second() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &5000_i128,
        &5_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &5000_i128,
        &false,
    );

    assert_eq!(ctx.client().get_stream_state(&new_id).rate_per_second, 5);
}

/// Cliff offset is preserved: new_cliff = new_start + (source_cliff - source_start).
#[test]
fn clone_preserves_cliff_offset() {
    let ctx = Ctx::setup();
    // Source: start=0, cliff=500, end=1000 → cliff_offset = 500.
    let source_id = ctx.create_cliff_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    // Clone: new_start=2000, expected new_cliff = 2000 + 500 = 2500.
    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &2000u64,
        &3000u64,
        &1000_i128,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(new_state.start_time, 2000);
    assert_eq!(new_state.cliff_time, 2500, "cliff offset must be preserved");
    assert_eq!(new_state.end_time, 3000);
}

/// Zero cliff (cliff == start) is preserved as zero offset.
#[test]
fn clone_preserves_zero_cliff_offset() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream(); // cliff == start == 0

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &2000u64,
        &3000u64,
        &1000_i128,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(
        new_state.cliff_time, new_state.start_time,
        "zero cliff offset must be preserved"
    );
}

/// withdraw_dust_threshold is copied verbatim.
#[test]
fn clone_inherits_withdraw_dust_threshold() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &100_i128,
        &None, // dust threshold = 100
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(
        ctx.client()
            .get_stream_state(&new_id)
            .withdraw_dust_threshold,
        100
    );
}

/// Memo is copied verbatim.
#[test]
fn clone_inherits_memo() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);
    let memo = Some(soroban_sdk::Bytes::from_slice(&ctx.env, b"payroll-jan"));
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &memo,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert!(new_state.memo.is_some(), "memo must be inherited");
    let expected = soroban_sdk::Bytes::from_slice(&ctx.env, b"payroll-jan");
    assert_eq!(new_state.memo.unwrap(), expected);
}

/// Clone with a different recipient works correctly.
#[test]
fn clone_with_different_recipient() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let new_recipient = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &new_recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(new_state.recipient, new_recipient);
    assert_ne!(new_state.recipient, ctx.recipient);
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

/// Source stream sender can clone (positive auth test, strict mode).
#[test]
fn clone_sender_authorized_strict() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.mock_all_auths();
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &10_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    env.ledger().set_timestamp(0);
    let source_id = client.create_stream(
        &sender,
        &recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    env.ledger().set_timestamp(1000);
    client.withdraw(&source_id);

    // Strict: only sender auth provided.
    env.mock_auths(&[MockAuth {
        address: &sender,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "clone_stream",
            args: (&source_id, &recipient, 1000u64, 2000u64, 1000_i128, false).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    let new_id = client.clone_stream(
        &source_id, &recipient, &1000u64, &2000u64, &1000_i128, &false,
    );
    assert_eq!(
        client.get_stream_state(&new_id).status,
        StreamStatus::Active
    );
}

/// Recipient cannot clone a stream they receive (strict mode).
#[test]
#[should_panic]
fn clone_recipient_unauthorized() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    env.mock_all_auths();
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &10_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    env.ledger().set_timestamp(0);
    let source_id = client.create_stream(
        &sender,
        &recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    env.ledger().set_timestamp(1000);
    client.withdraw(&source_id);

    // Recipient tries to clone — must panic (auth failure).
    env.mock_auths(&[MockAuth {
        address: &recipient,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "clone_stream",
            args: (&source_id, &recipient, 1000u64, 2000u64, 1000_i128, false).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.clone_stream(
        &source_id, &recipient, &1000u64, &2000u64, &1000_i128, &false,
    );
}

/// Third party cannot clone a stream they have no relation to (strict mode).
#[test]
#[should_panic]
fn clone_third_party_unauthorized() {
    let env = Env::default();
    let contract_id = env.register_contract(None, FluxoraStream);
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let attacker = Address::generate(&env);

    env.mock_all_auths();
    let client = FluxoraStreamClient::new(&env, &contract_id);
    client.init(&token_id, &admin);

    let sac = StellarAssetClient::new(&env, &token_id);
    sac.mint(&sender, &10_000_i128);
    TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

    env.ledger().set_timestamp(0);
    let source_id = client.create_stream(
        &sender,
        &recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    env.ledger().set_timestamp(1000);
    client.withdraw(&source_id);

    // Attacker tries to clone — must panic.
    env.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "clone_stream",
            args: (&source_id, &recipient, 1000u64, 2000u64, 1000_i128, false).into_val(&env),
            sub_invokes: &[],
        },
    }]);

    client.clone_stream(
        &source_id, &recipient, &1000u64, &2000u64, &1000_i128, &false,
    );
}

// ---------------------------------------------------------------------------
// CliffOnly guard (force flag)
// ---------------------------------------------------------------------------

/// force=false rejects a source stream with withdraw_dust_threshold == i128::MAX.
#[test]
fn clone_cliff_only_sentinel_rejected_without_force() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);
    // Create a stream with the CliffOnly sentinel threshold.
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &i128::MAX,
        &None, // sentinel
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false, // force=false → must reject
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// force=true allows cloning a source stream with the CliffOnly sentinel threshold.
#[test]
fn clone_cliff_only_sentinel_allowed_with_force() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &i128::MAX,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &true, // force=true → allowed
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(new_state.withdraw_dust_threshold, i128::MAX);
    assert_eq!(new_state.status, StreamStatus::Active);
}

/// Normal streams (threshold != i128::MAX) are unaffected by the force flag.
#[test]
fn clone_normal_stream_force_false_succeeds() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    // force=false on a normal stream must succeed.
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(
        ctx.client().get_stream_state(&new_id).status,
        StreamStatus::Active
    );
}

// ---------------------------------------------------------------------------
// Parameter validation
// ---------------------------------------------------------------------------

/// Source stream not found returns StreamNotFound.
#[test]
fn clone_source_not_found() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let result =
        ctx.client()
            .try_clone_stream(&999u64, &ctx.recipient, &0u64, &1000u64, &1000_i128, &false);

    assert_eq!(result, Err(Ok(ContractError::StreamNotFound)));
}

/// start_time in the past returns StartTimeInPast.
#[test]
fn clone_start_time_in_past_rejected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    // Ledger is at t=1000; start_time=500 is in the past.
    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &500u64,
        &1500u64, // start_time < now
        &1000_i128,
        &false,
    );

    assert_eq!(result, Err(Ok(ContractError::StartTimeInPast)));
}

/// Insufficient deposit returns InsufficientDeposit.
#[test]
fn clone_insufficient_deposit_rejected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream(); // rate=1/s

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    // rate=1, duration=1000s → need 1000 tokens; deposit=500 is insufficient.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &500_i128,
        &false, // too small
    );

    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));
}

/// Deposit exactly equal to rate * duration is valid (boundary).
#[test]
fn clone_deposit_exactly_covers_duration() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream(); // rate=1/s

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    // rate=1, duration=1000 → exactly 1000 tokens needed.
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(ctx.client().get_stream_state(&new_id).deposit_amount, 1000);
}

/// Deposit greater than required is accepted (excess stays in contract).
#[test]
fn clone_deposit_above_required_accepted() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &2000_i128,
        &false, // excess deposit
    );

    assert_eq!(ctx.client().get_stream_state(&new_id).deposit_amount, 2000);
}

/// sender == new_recipient is rejected (InvalidParams).
#[test]
fn clone_sender_equals_new_recipient_rejected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    // new_recipient == sender → invalid.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.sender, // same as source.sender
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));
}

/// Global pause blocks clone_stream.
#[test]
fn clone_blocked_when_globally_paused() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.client().set_global_emergency_paused(&true);

    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));
}

/// Creation pause blocks clone_stream.
#[test]
fn clone_blocked_when_creation_paused() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.client().set_contract_paused(&true);

    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(result, Err(Ok(ContractError::ContractPaused)));
}

// ---------------------------------------------------------------------------
// Event emission
// ---------------------------------------------------------------------------

/// clone_stream emits both a "created" event and a "cloned" event.
#[test]
fn clone_emits_created_and_cloned_events() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let events_before = ctx.env.events().all().len();

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let events = ctx.env.events().all();
    let mut saw_created = false;
    let mut saw_cloned = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        let topic0 = Symbol::from_val(&ctx.env, &event.1.get(0).unwrap());
        if topic0 == Symbol::new(&ctx.env, "created") {
            let payload = StreamCreated::try_from_val(&ctx.env, &event.2).unwrap();
            assert_eq!(payload.stream_id, new_id);
            assert_eq!(payload.sender, ctx.sender);
            assert_eq!(payload.recipient, ctx.recipient);
            assert_eq!(payload.deposit_amount, 1000);
            assert_eq!(payload.rate_per_second, 1);
            saw_created = true;
        }
        if topic0 == Symbol::new(&ctx.env, "cloned") {
            let payload = StreamCloned::try_from_val(&ctx.env, &event.2).unwrap();
            assert_eq!(payload.new_stream_id, new_id);
            assert_eq!(payload.source_stream_id, source_id);
            assert_eq!(payload.sender, ctx.sender);
            assert_eq!(payload.recipient, ctx.recipient);
            assert_eq!(payload.deposit_amount, 1000);
            assert_eq!(payload.rate_per_second, 1);
            assert_eq!(payload.start_time, 1000);
            assert_eq!(payload.end_time, 2000);
            saw_cloned = true;
        }
    }

    assert!(saw_created, "\"created\" event must be emitted");
    assert!(saw_cloned, "\"cloned\" event must be emitted");
}

/// "cloned" event carries the correct source_stream_id for indexer correlation.
#[test]
fn clone_event_carries_correct_source_stream_id() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let events_before = ctx.env.events().all().len();

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let events = ctx.env.events().all();
    let mut found = false;
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        let topic0 = Symbol::from_val(&ctx.env, &event.1.get(0).unwrap());
        if topic0 == Symbol::new(&ctx.env, "cloned") {
            let payload = StreamCloned::try_from_val(&ctx.env, &event.2).unwrap();
            assert_eq!(payload.source_stream_id, source_id);
            assert_eq!(payload.new_stream_id, new_id);
            found = true;
        }
    }
    assert!(
        found,
        "\"cloned\" event must be emitted with correct source_stream_id"
    );
}

/// No events are emitted when clone_stream fails (e.g. insufficient deposit).
#[test]
fn clone_no_events_on_failure() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let events_before = ctx.env.events().all().len();

    ctx.env.ledger().set_timestamp(1000);
    let _ = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1_i128,
        &false, // insufficient deposit
    );

    assert_eq!(
        ctx.env.events().all().len(),
        events_before,
        "no events must be emitted on failed clone"
    );
}

// ---------------------------------------------------------------------------
// Token balance invariants
// ---------------------------------------------------------------------------

/// Sender's balance decreases by exactly the deposit amount on clone.
#[test]
fn clone_sender_balance_decreases_by_deposit() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let sender_before = ctx.token.balance(&ctx.sender);
    let contract_before = ctx.token.balance(&ctx.contract_id);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(ctx.token.balance(&ctx.sender), sender_before - 1000);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_before + 1000);
}

/// Recipient balance is unchanged immediately after clone (no auto-withdrawal).
#[test]
fn clone_recipient_balance_unchanged_immediately() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let recipient_before = ctx.token.balance(&ctx.recipient);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(ctx.token.balance(&ctx.recipient), recipient_before);
}

/// After clone, recipient can withdraw accrued tokens from the new stream.
#[test]
fn clone_recipient_can_withdraw_from_new_stream() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    // Advance to t=1500 (500s into new stream).
    ctx.env.ledger().set_timestamp(1500);
    let withdrawn = ctx.client().withdraw(&new_id);
    assert_eq!(withdrawn, 500);
    assert_eq!(ctx.token.balance(&ctx.recipient), 500);
}

/// Source stream's state is completely unaffected by cloning.
#[test]
fn clone_does_not_mutate_source_stream() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let source_state_before = ctx.client().get_stream_state(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let source_state_after = ctx.client().get_stream_state(&source_id);
    assert_eq!(source_state_after.status, source_state_before.status);
    assert_eq!(
        source_state_after.withdrawn_amount,
        source_state_before.withdrawn_amount
    );
    assert_eq!(
        source_state_after.deposit_amount,
        source_state_before.deposit_amount
    );
}

// ---------------------------------------------------------------------------
// Recurring payroll pattern (multiple sequential clones)
// ---------------------------------------------------------------------------

/// Three sequential monthly clones produce independent streams with correct IDs.
#[test]
fn clone_recurring_payroll_three_months() {
    let ctx = Ctx::setup();

    // Month 1: 0..1000s, rate=1, deposit=1000.
    ctx.env.ledger().set_timestamp(0);
    let m1_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&m1_id);

    // Month 2: clone from month 1.
    ctx.env.ledger().set_timestamp(1000);
    let m2_id = ctx.client().clone_stream(
        &m1_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    ctx.env.ledger().set_timestamp(2000);
    ctx.client().withdraw(&m2_id);

    // Month 3: clone from month 2.
    ctx.env.ledger().set_timestamp(2000);
    let m3_id = ctx.client().clone_stream(
        &m2_id,
        &ctx.recipient,
        &2000u64,
        &3000u64,
        &1000_i128,
        &false,
    );

    // All three IDs are distinct and sequential.
    assert_ne!(m1_id, m2_id);
    assert_ne!(m2_id, m3_id);
    assert_eq!(m2_id, m1_id + 1);
    assert_eq!(m3_id, m2_id + 1);

    // Month 3 stream has correct parameters.
    let m3_state = ctx.client().get_stream_state(&m3_id);
    assert_eq!(m3_state.rate_per_second, 1);
    assert_eq!(m3_state.start_time, 2000);
    assert_eq!(m3_state.end_time, 3000);
    assert_eq!(m3_state.status, StreamStatus::Active);
}

/// Cloning preserves the cliff offset across multiple generations.
#[test]
fn clone_cliff_offset_preserved_across_generations() {
    let ctx = Ctx::setup();
    // Source: start=0, cliff=500, end=1000 → cliff_offset=500.
    let source_id = ctx.create_cliff_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    // Gen 2: start=1000, expected cliff=1500.
    ctx.env.ledger().set_timestamp(1000);
    let gen2_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );
    let gen2 = ctx.client().get_stream_state(&gen2_id);
    assert_eq!(gen2.cliff_time, 1500);

    ctx.env.ledger().set_timestamp(2000);
    ctx.client().withdraw(&gen2_id);

    // Gen 3: start=2000, expected cliff=2500.
    ctx.env.ledger().set_timestamp(2000);
    let gen3_id = ctx.client().clone_stream(
        &gen2_id,
        &ctx.recipient,
        &2000u64,
        &3000u64,
        &1000_i128,
        &false,
    );
    let gen3 = ctx.client().get_stream_state(&gen3_id);
    assert_eq!(gen3.cliff_time, 2500);
}

/// Stream count increments correctly with each clone.
#[test]
fn clone_increments_stream_count() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();
    assert_eq!(ctx.client().get_stream_count(), 1);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );
    assert_eq!(ctx.client().get_stream_count(), 2);

    ctx.env.ledger().set_timestamp(2000);
    ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &2000u64,
        &3000u64,
        &1000_i128,
        &false,
    );
    assert_eq!(ctx.client().get_stream_count(), 3);
}

/// New stream appears in recipient's stream index after clone.
#[test]
fn clone_new_stream_appears_in_recipient_index() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let index_before = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(index_before.len(), 1);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let index_after = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(index_after.len(), 2);
    assert!(index_after.contains(&new_id));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

/// Cliff == end_time in source: new cliff is clamped to new end_time.
#[test]
fn clone_cliff_equals_end_in_source() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);
    // cliff == end_time (degenerate but valid).
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &1000u64,
        &1000u64, // cliff == end
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    // cliff_offset = 1000 - 0 = 1000. new_cliff = 1000 + 1000 = 2000 == new_end.
    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(new_state.cliff_time, 2000);
    assert_eq!(new_state.end_time, 2000);
}

/// Cloning with a different deposit amount (larger) works correctly.
#[test]
fn clone_with_larger_deposit_for_raise() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream(); // rate=1, deposit=1000

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    // "Raise": same rate but larger deposit (excess stays in contract).
    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &5000_i128,
        &false, // 5x deposit
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(new_state.deposit_amount, 5000);
    assert_eq!(new_state.rate_per_second, 1); // rate unchanged
}

/// Cloning a stream with no memo produces a new stream with no memo.
#[test]
fn clone_no_memo_produces_no_memo() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream(); // no memo

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert!(ctx.client().get_stream_state(&new_id).memo.is_none());
}

/// Cloning produces a stream with withdrawn_amount = 0 regardless of source.
#[test]
fn clone_new_stream_has_zero_withdrawn_amount() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    // Partially withdraw from source.
    ctx.env.ledger().set_timestamp(600);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(ctx.client().get_stream_state(&new_id).withdrawn_amount, 0);
}

/// Cloning a stream with a high rate and large deposit works without overflow.
#[test]
fn clone_high_rate_large_deposit_no_overflow() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    let rate: i128 = 1_000_000;
    let duration: u64 = 1_000;
    let deposit: i128 = rate * duration as i128; // 1_000_000_000

    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &(deposit * 2));

    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit,
        &rate,
        &0u64,
        &0u64,
        &duration,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(duration);
    ctx.client().withdraw(&source_id);

    ctx.env.ledger().set_timestamp(duration);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &duration,
        &(duration * 2),
        &deposit,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(new_state.rate_per_second, rate);
    assert_eq!(new_state.deposit_amount, deposit);
    assert_eq!(new_state.status, StreamStatus::Active);
}

// ---------------------------------------------------------------------------
// Clone-override parameter-validation matrix
//
// Security invariant: clone_stream re-runs validate_stream_params with the
// *caller-supplied* override values.  Every invalid value that create_stream
// would reject must also be rejected via the clone path, with no ability to
// sneak an invalid stream through.
//
// For every rejected-clone case the test additionally asserts that the source
// stream is left completely unmodified (status, deposit_amount,
// withdrawn_amount, rate_per_second, start_time, end_time, cliff_time).
//
// Unique stream_id and recipient-index update are verified in the positive
// cases at the bottom of this section.
// ---------------------------------------------------------------------------

// ── Helper: snapshot a stream's immutable identity for "source unaffected" checks ──

/// Lightweight snapshot of the fields that must be immutable when a clone is
/// rejected.  We compare before/after to assert the source stream is untouched.
struct StreamSnapshot {
    status: StreamStatus,
    deposit_amount: i128,
    withdrawn_amount: i128,
    rate_per_second: i128,
    start_time: u64,
    cliff_time: u64,
    end_time: u64,
}

impl StreamSnapshot {
    fn capture(ctx: &Ctx<'_>, stream_id: u64) -> Self {
        let s = ctx.client().get_stream_state(&stream_id);
        Self {
            status: s.status,
            deposit_amount: s.deposit_amount,
            withdrawn_amount: s.withdrawn_amount,
            rate_per_second: s.rate_per_second,
            start_time: s.start_time,
            cliff_time: s.cliff_time,
            end_time: s.end_time,
        }
    }

    fn assert_unchanged(&self, ctx: &Ctx<'_>, stream_id: u64) {
        let s = ctx.client().get_stream_state(&stream_id);
        assert_eq!(s.status, self.status, "source status must be unchanged");
        assert_eq!(
            s.deposit_amount, self.deposit_amount,
            "source deposit must be unchanged"
        );
        assert_eq!(
            s.withdrawn_amount, self.withdrawn_amount,
            "source withdrawn_amount must be unchanged"
        );
        assert_eq!(
            s.rate_per_second, self.rate_per_second,
            "source rate must be unchanged"
        );
        assert_eq!(
            s.start_time, self.start_time,
            "source start_time must be unchanged"
        );
        assert_eq!(
            s.cliff_time, self.cliff_time,
            "source cliff_time must be unchanged"
        );
        assert_eq!(
            s.end_time, self.end_time,
            "source end_time must be unchanged"
        );
    }
}

// ── 1. end_time == start_time (boundary: equality means zero-duration stream) ──

/// clone with end_time == start_time must be rejected (start_time >= end_time).
/// Security: the clone path cannot be used to create a zero-duration stream that
/// create_stream would block.
#[test]
fn clone_override_end_time_equals_start_time_rejected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    // start_time == end_time — must be rejected with InvalidParams.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &2000u64,
        &2000u64, // end == start
        &1000_i128,
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "end_time == start_time must return InvalidParams"
    );
    snap.assert_unchanged(&ctx, source_id);
}

// ── 2. end_time < start_time ──

/// clone with end_time strictly less than start_time must be rejected.
#[test]
fn clone_override_end_time_before_start_time_rejected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &2000u64,
        &1999u64, // end < start
        &1000_i128,
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "end_time < start_time must return InvalidParams"
    );
    snap.assert_unchanged(&ctx, source_id);
}

// ── 3. start_time in the past ──

/// clone with start_time already passed must return StartTimeInPast.
/// (Existing positive test covers the happy path; this asserts source unaffected.)
#[test]
fn clone_override_start_in_past_source_unaffected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1500);
    // start_time=500 < current_time=1500 → StartTimeInPast.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &500u64,
        &2000u64,
        &1500_i128,
        &false,
    );

    assert_eq!(result, Err(Ok(ContractError::StartTimeInPast)));
    snap.assert_unchanged(&ctx, source_id);
}

// ── 4. deposit == 0 ──

/// clone with zero deposit must be rejected (deposit_amount <= 0).
/// Security: create_stream rejects zero deposit; the clone path must too.
#[test]
fn clone_override_zero_deposit_rejected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &0_i128, // zero deposit
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "zero deposit must return InvalidParams"
    );
    snap.assert_unchanged(&ctx, source_id);
}

// ── 5. deposit < 0 ──

/// clone with negative deposit must be rejected.
/// Security: negative deposits could produce negative token flows if not caught.
#[test]
fn clone_override_negative_deposit_rejected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &-1_i128, // negative deposit
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "negative deposit must return InvalidParams"
    );
    snap.assert_unchanged(&ctx, source_id);
}

// ── 6. deposit < rate * duration (InsufficientDeposit) ──

/// clone with deposit that doesn't cover the full accrual must be rejected.
/// Source unaffected is re-verified explicitly here as part of the matrix.
#[test]
fn clone_override_deposit_below_total_streamable_rejected() {
    let ctx = Ctx::setup();
    // Source: rate=10/s, duration=1000s → must deposit ≥ 10_000.
    ctx.env.ledger().set_timestamp(0);
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &100_000_i128);
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000_i128,
        &10_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    // rate=10, duration=1000 → need 10_000; deposit=9_999 is one short.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &9_999_i128,
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InsufficientDeposit)),
        "deposit < rate * duration must return InsufficientDeposit"
    );
    snap.assert_unchanged(&ctx, source_id);
}

// ── 7. rate_per_second > MAX_RATE_PER_SECOND (governance cap) ──

/// When governance lowers MAX_RATE_PER_SECOND, cloning a stream whose inherited
/// rate now exceeds the cap must be rejected.
///
/// Security: a stream created before the cap was lowered cannot be used as a
/// template to perpetuate an above-cap rate via the clone path.
#[test]
fn clone_override_rate_exceeds_governance_cap_rejected() {
    let ctx = Ctx::setup();

    // Create a source stream with rate=100/s while cap is effectively i128::MAX.
    ctx.env.ledger().set_timestamp(0);
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &1_000_000_i128);
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_000_i128,
        &100_i128, // rate = 100/s
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    // Governance lowers the cap to 50/s — source rate (100) now exceeds it.
    ctx.client().set_max_rate_per_second(&50_i128);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    // Inherited rate=100 > cap=50 → must be rejected.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &100_000_i128,
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "inherited rate exceeding governance cap must return InvalidParams"
    );
    snap.assert_unchanged(&ctx, source_id);
}

/// When the governance cap is at exactly the source rate, the clone is allowed
/// (boundary: rate == cap is valid).
#[test]
fn clone_override_rate_at_exact_governance_cap_allowed() {
    let ctx = Ctx::setup();

    ctx.env.ledger().set_timestamp(0);
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&ctx.sender, &1_000_000_i128);
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &100_000_i128,
        &100_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    // Cap set exactly to source rate — should be allowed.
    ctx.client().set_max_rate_per_second(&100_i128);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &100_000_i128,
        &false,
    );

    assert_eq!(
        ctx.client().get_stream_state(&new_id).rate_per_second,
        100,
        "rate == cap must succeed"
    );
}

// ── 8. computed cliff_time > end_time ──

/// When the inherited cliff_offset pushes new_cliff past new_end_time, the
/// clone must be rejected (cliff_time > end_time violates the time constraint).
///
/// Security: the cliff-offset arithmetic in clone_stream could silently
/// produce an out-of-range cliff if the new window is shorter than the source.
/// validate_stream_params must catch this.
#[test]
fn clone_override_computed_cliff_exceeds_end_time_rejected() {
    let ctx = Ctx::setup();

    // Source: start=0, cliff=800, end=1000 → cliff_offset=800.
    ctx.env.ledger().set_timestamp(0);
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &800u64, // cliff at 80% into the stream
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    // New window: start=2000, end=2500 (duration=500).
    // new_cliff = 2000 + 800 = 2800 > end=2500 → must be rejected.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &2000u64,
        &2500u64, // shorter window than the cliff offset
        &500_i128,
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "computed cliff > end_time must return InvalidParams"
    );
    snap.assert_unchanged(&ctx, source_id);
}

// ── 9. new_recipient == source.sender ──

/// clone with new_recipient equal to the source sender must be rejected
/// (sender == recipient is forbidden by validate_stream_params).
/// Source unaffected is verified explicitly as part of the matrix.
#[test]
fn clone_override_recipient_equals_sender_source_unaffected() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.sender, // same as source.sender
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "new_recipient == sender must return InvalidParams"
    );
    snap.assert_unchanged(&ctx, source_id);
}

// ── 10. Multiple invalid overrides in the same call ──

/// When both start_time >= end_time AND deposit is zero, the first
/// validation check encountered returns the appropriate error.  The exact
/// error order matches validate_stream_params (deposit checked before times).
#[test]
fn clone_override_multiple_invalid_params_deposit_checked_first() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    // Both deposit=0 (invalid) and end_time==start_time (invalid).
    // deposit<=0 is checked before time constraints in validate_stream_params.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &2000u64,
        &2000u64, // end == start
        &0_i128,  // zero deposit
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::InvalidParams)),
        "multiple invalid params: deposit checked first"
    );
    snap.assert_unchanged(&ctx, source_id);
}

// ── 11. Unique stream_id — clone always produces a fresh ID ──

/// Each successful clone receives a strictly increasing, unique stream_id.
/// Security: stream IDs must never alias existing streams.
#[test]
fn clone_override_each_clone_gets_unique_stream_id() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    // Clone #1.
    ctx.env.ledger().set_timestamp(1000);
    let clone1_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    ctx.env.ledger().set_timestamp(2000);
    ctx.client().withdraw(&clone1_id);

    // Clone #2 (chained from clone #1).
    ctx.env.ledger().set_timestamp(2000);
    let clone2_id = ctx.client().clone_stream(
        &clone1_id,
        &ctx.recipient,
        &2000u64,
        &3000u64,
        &1000_i128,
        &false,
    );

    // Clone #3 (chained from clone #2).
    ctx.env.ledger().set_timestamp(3000);
    ctx.client().withdraw(&clone2_id);
    ctx.env.ledger().set_timestamp(3000);
    let clone3_id = ctx.client().clone_stream(
        &clone2_id,
        &ctx.recipient,
        &3000u64,
        &4000u64,
        &1000_i128,
        &false,
    );

    // All IDs are distinct.
    assert_ne!(source_id, clone1_id, "clone1 must differ from source");
    assert_ne!(clone1_id, clone2_id, "clone2 must differ from clone1");
    assert_ne!(clone2_id, clone3_id, "clone3 must differ from clone2");
    assert_ne!(source_id, clone3_id, "clone3 must differ from source");

    // IDs are monotonically increasing (each call increments the counter).
    assert!(clone1_id > source_id, "stream IDs must increase");
    assert!(clone2_id > clone1_id, "stream IDs must increase");
    assert!(clone3_id > clone2_id, "stream IDs must increase");
}

// ── 12. Recipient index updated on successful clone to a distinct recipient ──

/// When a clone targets a *different* recipient, that recipient's stream index
/// is updated to include the new stream_id.  The original recipient's index is
/// unchanged, and the source stream is unaffected.
#[test]
fn clone_override_recipient_index_updated_for_new_recipient() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream(); // recipient = ctx.recipient

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let original_recipient_index = ctx.client().get_recipient_streams(&ctx.recipient);
    let new_recipient = Address::generate(&ctx.env);
    let new_recipient_index_before = ctx.client().get_recipient_streams(&new_recipient);

    ctx.env.ledger().set_timestamp(1000);
    let new_id = ctx.client().clone_stream(
        &source_id,
        &new_recipient, // different recipient
        &1000u64,
        &2000u64,
        &1000_i128,
        &false,
    );

    // New recipient's index now contains the cloned stream.
    let new_recipient_index_after = ctx.client().get_recipient_streams(&new_recipient);
    assert_eq!(
        new_recipient_index_after.len(),
        new_recipient_index_before.len() + 1,
        "new recipient index must grow by 1"
    );
    assert!(
        new_recipient_index_after.contains(&new_id),
        "cloned stream_id must appear in new recipient's index"
    );

    // Original recipient's index is unchanged.
    let original_recipient_index_after = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(
        original_recipient_index_after.len(),
        original_recipient_index.len(),
        "original recipient index must not change"
    );

    // Source stream is unaffected.
    let source_after = ctx.client().get_stream_state(&source_id);
    assert_eq!(source_after.recipient, ctx.recipient);
    assert_eq!(source_after.deposit_amount, 1000);
}

// ── 13. Rejected clone does NOT update recipient index ──

/// When a clone is rejected (e.g. insufficient deposit), the recipient's stream
/// index must remain unchanged — no partial state must be written.
#[test]
fn clone_override_rejected_clone_does_not_update_recipient_index() {
    let ctx = Ctx::setup();
    let source_id = ctx.create_default_stream();

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let new_recipient = Address::generate(&ctx.env);
    let index_before = ctx.client().get_recipient_streams(&new_recipient);

    ctx.env.ledger().set_timestamp(1000);
    // Insufficient deposit — clone must fail.
    let result = ctx.client().try_clone_stream(
        &source_id,
        &new_recipient,
        &1000u64,
        &2000u64,
        &1_i128, // way below rate * duration
        &false,
    );

    assert_eq!(result, Err(Ok(ContractError::InsufficientDeposit)));

    let index_after = ctx.client().get_recipient_streams(&new_recipient);
    assert_eq!(
        index_after.len(),
        index_before.len(),
        "rejected clone must not update recipient index"
    );
}

// ── 14. CliffOnly source: rate must remain 0; clone validates accordingly ──

/// A CliffOnly source stream has rate=0.  Cloning it with force=true must
/// succeed, and the cloned stream must also have rate=0 (inherited).
/// This confirms that CliffOnly validation parity is maintained.
#[test]
fn clone_override_cliff_only_inherits_zero_rate() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create a CliffOnly source (rate forced to 0 by create_stream internally).
    // The 9-arg form used in this file maps to the contract's create_stream;
    // StreamKind::CliffOnly is passed as the kind argument.
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &0_i128,  // rate (ignored / forced to 0 for CliffOnly)
        &0u64,    // start
        &500u64,  // cliff
        &1000u64, // end
        &0_i128,  // dust threshold
        &None,
        &StreamKind::CliffOnly,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    ctx.env.ledger().set_timestamp(1000);
    // force=true required because CliffOnly has withdraw_dust_threshold handling.
    // For a plain CliffOnly stream (threshold != i128::MAX), force=false should work.
    let new_id = ctx.client().clone_stream(
        &source_id,
        &ctx.recipient,
        &2000u64,
        &3000u64,
        &1000_i128,
        &false,
    );

    let new_state = ctx.client().get_stream_state(&new_id);
    assert_eq!(
        new_state.rate_per_second, 0,
        "CliffOnly clone must inherit rate=0"
    );

    snap.assert_unchanged(&ctx, source_id);
}

// ── 15. Arithmetic overflow in cliff-offset computation ──

/// If start_time is near u64::MAX and the source cliff_offset is large,
/// the addition overflows.  clone_stream must return ArithmeticOverflow
/// rather than producing a silent wrap-around cliff value.
///
/// Security: overflow in cliff computation could silently create a stream
/// with an absurdly small or invalid cliff, bypassing the time-range check.
#[test]
fn clone_override_cliff_offset_overflow_rejected() {
    let ctx = Ctx::setup();

    // Source with a non-trivial cliff offset (500s).
    ctx.env.ledger().set_timestamp(0);
    let source_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &500u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&source_id);

    let snap = StreamSnapshot::capture(&ctx, source_id);

    // Set ledger time so that u64::MAX - 100 is a "future" start_time.
    // cliff_offset=500; new_cliff = (u64::MAX - 100) + 500 overflows u64.
    let overflow_start = u64::MAX - 100;
    // We don't actually set the ledger to overflow_start (that would make
    // start_time appear to be in the past).  Instead, push ledger just below.
    ctx.env.ledger().set_timestamp(overflow_start - 1);

    let result = ctx.client().try_clone_stream(
        &source_id,
        &ctx.recipient,
        &overflow_start,
        &u64::MAX, // end_time, irrelevant if cliff overflows first
        &1000_i128,
        &false,
    );

    assert_eq!(
        result,
        Err(Ok(ContractError::ArithmeticOverflow)),
        "cliff offset overflow must return ArithmeticOverflow"
    );
    snap.assert_unchanged(&ctx, source_id);
}
