//! End-to-end integration test for the event-driven governance execution pattern.
//!
//! Tests that an executor stub, reading `ProposalExecuted` events from the
//! governance contract, can decode the target and calldata and apply a
//! parameter change to the stream contract.
//!
//! # Scenario
//!
//! 1. Deploy governance (3 signers, threshold 2) and stream contracts.
//! 2. Set the stream contract's admin to the governance contract address.
//! 3. Propose a `StreamSetMaxRate(5_000)` change on the stream.
//! 4. Approve twice to reach quorum.
//! 5. Assert the rate cap is unchanged before the timelock elapses.
//! 6. Advance past the timelock and execute.
//! 7. Assert the rate cap has changed to 5_000.
//! 8. Verify a cancelled/expired proposal yields no executor action.
//! 9. Governance-triggered `global_resume` and atomic `bulk_resume_streams_as_admin`
//!    mixed-batch partial-failure (see `docs/global-resume.md`).
//!
//! # Security notes
//!
//! The parameter change is impossible without the full
//! quorum + timelock + execute path completing. A malformed bulk-resume batch
//! cannot bypass governance authorization.

extern crate std;

use fluxora_governance::{
    CallData, FluxoraGovernance, FluxoraGovernanceClient, GovernanceError, ProposalExecuted,
};
use fluxora_stream::{
    ContractError, DataKey, FluxoraStream, FluxoraStreamClient, PauseReason, StreamKind,
    StreamStatus,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec,
    xdr::FromXdr,
    Address, Bytes, Env, Symbol, TryFromVal, Val, Vec as SdkVec,
};

// ---------------------------------------------------------------------------
// Constants (mirrored from governance lib.rs)
// ---------------------------------------------------------------------------

const TIMELOCK: u64 = 172_800;
const MAX_AGE: u64 = 2_592_000;
const BASE_TIMESTAMP: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// Executor Stub
// ---------------------------------------------------------------------------

/// Test-only executor stub that reads `ProposalExecuted` events and dispatches
/// the encoded operation to the target contract.
///
/// This simulates the event-driven operational pattern: an off-chain executor
/// monitors the governance contract's events, decodes the proposal intent,
/// and applies the change directly to the stream contract using the
/// authority record established by the governance proposal.
struct ExecutorStub;

impl ExecutorStub {
    /// Scan events emitted by `governance_id`, find the first
    /// `ProposalExecuted` event matching `proposal_id`, decode the
    /// `CallData` from the calldata bytes, and invoke it on the
    /// target contract.
    fn process_event(env: &Env, governance_id: &Address, proposal_id: u32) {
        let executed = Self::find_executed_event(env, governance_id, proposal_id)
            .expect("ProposalExecuted event must exist");

        let op = CallData::from_xdr(env, &executed.calldata)
            .expect("calldata must decode to a known CallData variant");

        match op {
            CallData::StreamSetMaxRate(max_rate) => {
                let stream_client = FluxoraStreamClient::new(env, &executed.target);
                stream_client.set_max_rate_per_second(&max_rate);
            }
            CallData::StreamGlobalResume => {
                let stream_client = FluxoraStreamClient::new(env, &executed.target);
                stream_client.global_resume();
            }
            CallData::StreamBulkResumeAsAdmin(stream_ids) => {
                let stream_client = FluxoraStreamClient::new(env, &executed.target);
                stream_client.bulk_resume_streams_as_admin(&stream_ids);
            }
            _ => {
                panic!("ExecutorStub: unexpected calldata variant {:?}", op);
            }
        }
    }

    /// Return `true` if a `ProposalExecuted` event for `proposal_id` was
    /// emitted by `governance_id`.
    fn has_executed_event(env: &Env, governance_id: &Address, proposal_id: u32) -> bool {
        Self::find_executed_event(env, governance_id, proposal_id).is_some()
    }

    fn find_executed_event(
        env: &Env,
        governance_id: &Address,
        proposal_id: u32,
    ) -> Option<ProposalExecuted> {
        let events = env.events().all();
        for i in (0..events.len()).rev() {
            let (addr, topics, data) = events.get(i).unwrap();
            if addr != *governance_id {
                continue;
            }
            let topic_vec: SdkVec<Val> = topics;
            if topic_vec.len() < 2 {
                continue;
            }

            let topic0 = Symbol::try_from_val(env, &topic_vec.get(0).unwrap())
                .expect("first topic must be a Symbol");
            if topic0 != symbol_short!("executed") {
                continue;
            }

            let raw_id: Val = topic_vec.get(1).unwrap();
            let event_id: u32 = raw_id.try_into().expect("second topic must be u32");
            if event_id != proposal_id {
                continue;
            }

            let executed: ProposalExecuted =
                ProposalExecuted::try_from_val(env, &data).expect("event data is ProposalExecuted");
            return Some(executed);
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct E2EContext {
    env: Env,
    governance_id: Address,
    stream_id: Address,
    admin: Address,
    signer_a: Address,
    signer_b: Address,
    signer_c: Address,
    sender: Address,
    recipient: Address,
    gov_client: FluxoraGovernanceClient<'static>,
    stream_client: FluxoraStreamClient<'static>,
}

impl E2EContext {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(BASE_TIMESTAMP);

        // ---- Deploy governance ----
        let governance_id = env.register_contract(None, FluxoraGovernance);
        let admin = Address::generate(&env);
        let signer_a = Address::generate(&env);
        let signer_b = Address::generate(&env);
        let signer_c = Address::generate(&env);

        let gov_client = FluxoraGovernanceClient::new(&env, &governance_id);
        gov_client.init(
            &admin,
            &vec![&env, signer_a.clone(), signer_b.clone(), signer_c.clone()],
            &2u32,
        );

        // ---- Deploy stream contract ----
        let stream_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let token_asset = StellarAssetClient::new(&env, &token_id);

        let stream_client = FluxoraStreamClient::new(&env, &stream_id);
        // The stream's admin is set to the governance contract, so the
        // governance contract can successfully call admin entrypoints
        // via dispatch_call during execute.
        stream_client.init(&token_id, &governance_id);

        // Mint tokens for stream creation
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        token_asset.mint(&sender, &1_000_000_000);
        let token = soroban_sdk::token::Client::new(&env, &token_id);
        token.approve(&sender, &stream_id, &i128::MAX, &100_000);

        E2EContext {
            env,
            governance_id,
            stream_id,
            admin,
            signer_a,
            signer_b,
            signer_c,
            sender,
            recipient,
            gov_client,
            stream_client,
        }
    }

    /// Read the current `max_rate_per_second` from the stream contract's
    /// instance storage. Returns `i128::MAX` when no cap has been set
    /// (the default).
    fn current_max_rate(&self) -> i128 {
        self.env.as_contract(&self.stream_id, || {
            self.env
                .storage()
                .instance()
                .get(&DataKey::MaxRatePerSecond)
                .unwrap_or(i128::MAX)
        })
    }

    /// XDR-encode a `StreamSetMaxRate` operation as proposal calldata.
    fn encode_set_max_rate(&self, rate: i128) -> Bytes {
        use soroban_sdk::xdr::ToXdr;
        CallData::StreamSetMaxRate(rate).to_xdr(&self.env)
    }

    fn encode_global_resume(&self) -> Bytes {
        use soroban_sdk::xdr::ToXdr;
        CallData::StreamGlobalResume.to_xdr(&self.env)
    }

    fn encode_bulk_resume(&self, stream_ids: SdkVec<u64>) -> Bytes {
        use soroban_sdk::xdr::ToXdr;
        CallData::StreamBulkResumeAsAdmin(stream_ids).to_xdr(&self.env)
    }

    /// Advance the ledger timestamp past the timelock relative to *now*
    /// so the most recently quorum'd proposal becomes executable.
    fn advance_past_timelock(&self) {
        let now = self.env.ledger().timestamp();
        self.env.ledger().set_timestamp(now + TIMELOCK + 1);
    }

    /// Advance past max age so the proposal expires.
    fn advance_past_max_age(&self) {
        self.env
            .ledger()
            .set_timestamp(BASE_TIMESTAMP + MAX_AGE + 1);
    }

    /// Clear the pause/resume cooldown (`MIN_PAUSE_INTERVAL_LEDGERS`).
    fn clear_pause_cooldown(&self) {
        self.env
            .ledger()
            .with_mut(|ledger| ledger.sequence_number += 32);
    }

    fn create_stream(&self, duration: u64) -> u64 {
        let now = self.env.ledger().timestamp();
        // Duration must outlive multiple governance timelock advances used in
        // these e2e flows; otherwise is_terminal_state treats the stream as
        // past end_time and resume fails with StreamTerminalState.
        let duration = duration.max(TIMELOCK * 3);
        self.stream_client.create_stream(
            &self.sender,
            &self.recipient,
            &(duration as i128),
            &1,
            &now,
            &now,
            &(now + duration),
            &0,
            &None,
            &StreamKind::Linear,
        )
    }

    /// Propose → approve to quorum → wait timelock → execute.
    fn propose_approve_execute(&self, calldata: &Bytes) -> u32 {
        let proposal_id = self
            .gov_client
            .propose(&self.signer_a, &self.stream_id, calldata);
        self.gov_client.approve(&self.signer_a, &proposal_id);
        self.gov_client.approve(&self.signer_b, &proposal_id);
        self.advance_past_timelock();
        let executor = Address::generate(&self.env);
        self.gov_client.execute(&executor, &proposal_id);
        proposal_id
    }
}

// ---------------------------------------------------------------------------
// Full end-to-end happy path
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_propose_approve_timelock_execute_changes_max_rate() {
    let ctx = E2EContext::setup();
    let calldata = ctx.encode_set_max_rate(5_000);

    // ---- Propose ----
    let proposal_id = ctx
        .gov_client
        .propose(&ctx.signer_a, &ctx.stream_id, &calldata);
    assert_eq!(proposal_id, 0u32);

    // Max rate should still be the default before quorum.
    assert_eq!(ctx.current_max_rate(), i128::MAX);

    // ---- Approve to quorum ----
    ctx.gov_client.approve(&ctx.signer_a, &proposal_id);
    ctx.gov_client.approve(&ctx.signer_b, &proposal_id);

    // Max rate should still be the default before timelock elapses.
    assert_eq!(ctx.current_max_rate(), i128::MAX);

    // ---- Execute before timelock is blocked ----
    let executor = Address::generate(&ctx.env);
    let early_result = ctx.gov_client.try_execute(&executor, &proposal_id);
    assert_eq!(early_result, Err(Ok(GovernanceError::TimelockNotElapsed)));
    assert_eq!(ctx.current_max_rate(), i128::MAX);

    // ---- Wait for timelock ----
    ctx.advance_past_timelock();

    // ---- Execute (governance dispatches the call to the stream contract) ----
    ctx.gov_client.execute(&executor, &proposal_id);

    // ---- Assert stream parameter changed ----
    assert_eq!(ctx.current_max_rate(), 5_000);

    // ---- Executor stub reads the event and verifies the flow ----
    let has_event = ExecutorStub::has_executed_event(&ctx.env, &ctx.governance_id, proposal_id);
    assert!(has_event, "ProposalExecuted event must be present");

    // The executor stub can also independently dispatch the decoded
    // operation, demonstrating the event-driven pattern.
    ExecutorStub::process_event(&ctx.env, &ctx.governance_id, proposal_id);

    // No error — the stub successfully decoded the event and applied
    // the change (idempotent: already at 5_000).
    assert_eq!(ctx.current_max_rate(), 5_000);

    // Verify the Proposal contains executed = true
    let proposal = ctx.gov_client.get_proposal(&proposal_id);
    assert!(proposal.executed);
}

// ---------------------------------------------------------------------------
// Pre-quorum execute is blocked
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_execute_without_quorum_is_blocked() {
    let ctx = E2EContext::setup();
    let calldata = ctx.encode_set_max_rate(5_000);

    let proposal_id = ctx
        .gov_client
        .propose(&ctx.signer_a, &ctx.stream_id, &calldata);

    // Only one approval (threshold = 2)
    ctx.gov_client.approve(&ctx.signer_a, &proposal_id);

    ctx.advance_past_timelock();

    let executor = Address::generate(&ctx.env);
    let result = ctx.gov_client.try_execute(&executor, &proposal_id);
    assert_eq!(result, Err(Ok(GovernanceError::QuorumNotReached)));

    // Max rate unchanged
    assert_eq!(ctx.current_max_rate(), i128::MAX);

    // No ProposalExecuted event was emitted
    assert!(
        !ExecutorStub::has_executed_event(&ctx.env, &ctx.governance_id, proposal_id),
        "No ProposalExecuted event should exist for a failed execution"
    );
}

// ---------------------------------------------------------------------------
// Pre-timelock execute is blocked
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_execute_before_timelock_is_blocked() {
    let ctx = E2EContext::setup();
    let calldata = ctx.encode_set_max_rate(5_000);

    let proposal_id = ctx
        .gov_client
        .propose(&ctx.signer_a, &ctx.stream_id, &calldata);

    ctx.gov_client.approve(&ctx.signer_a, &proposal_id);
    ctx.gov_client.approve(&ctx.signer_b, &proposal_id);

    // Advance only partway through the timelock
    ctx.env
        .ledger()
        .set_timestamp(BASE_TIMESTAMP + TIMELOCK - 1);

    let executor = Address::generate(&ctx.env);
    let result = ctx.gov_client.try_execute(&executor, &proposal_id);
    assert_eq!(result, Err(Ok(GovernanceError::TimelockNotElapsed)));

    // Max rate unchanged
    assert_eq!(ctx.current_max_rate(), i128::MAX);

    // No ProposalExecuted event was emitted
    assert!(
        !ExecutorStub::has_executed_event(&ctx.env, &ctx.governance_id, proposal_id),
        "No ProposalExecuted event should exist for a failed execution"
    );
}

// ---------------------------------------------------------------------------
// Cancelled proposal yields no executor action
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_cancelled_proposal_yields_no_action() {
    let ctx = E2EContext::setup();
    let calldata = ctx.encode_set_max_rate(5_000);

    let proposal_id = ctx
        .gov_client
        .propose(&ctx.signer_a, &ctx.stream_id, &calldata);

    // Cancel before any approvals
    ctx.gov_client.cancel_proposal(&ctx.signer_a, &proposal_id);

    // Attempt to execute should fail
    ctx.advance_past_timelock();
    let executor = Address::generate(&ctx.env);
    let result = ctx.gov_client.try_execute(&executor, &proposal_id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));

    // Max rate unchanged
    assert_eq!(ctx.current_max_rate(), i128::MAX);

    // No ProposalExecuted event was emitted
    assert!(
        !ExecutorStub::has_executed_event(&ctx.env, &ctx.governance_id, proposal_id),
        "No ProposalExecuted event should exist for a cancelled proposal"
    );
}

// ---------------------------------------------------------------------------
// Expired proposal yields no executor action
// ---------------------------------------------------------------------------

#[test]
fn test_e2e_expired_proposal_yields_no_action() {
    let ctx = E2EContext::setup();
    let calldata = ctx.encode_set_max_rate(5_000);

    let proposal_id = ctx
        .gov_client
        .propose(&ctx.signer_a, &ctx.stream_id, &calldata);

    ctx.gov_client.approve(&ctx.signer_a, &proposal_id);
    ctx.gov_client.approve(&ctx.signer_b, &proposal_id);

    // Advance past max age
    ctx.advance_past_max_age();

    let executor = Address::generate(&ctx.env);
    let result = ctx.gov_client.try_execute(&executor, &proposal_id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalExpired)));

    // Max rate unchanged
    assert_eq!(ctx.current_max_rate(), i128::MAX);

    // No ProposalExecuted event was emitted
    assert!(
        !ExecutorStub::has_executed_event(&ctx.env, &ctx.governance_id, proposal_id),
        "No ProposalExecuted event should exist for an expired proposal"
    );
}

// ---------------------------------------------------------------------------
// Global resume + mixed-batch bulk resume (atomic partial-failure)
// ---------------------------------------------------------------------------

/// Governance-triggered `global_resume` clears the emergency pause; a subsequent
/// mixed `StreamBulkResumeAsAdmin` batch that includes one cancelled stream
/// fails atomically — no paused stream is resumed.
#[test]
fn test_e2e_global_resume_mixed_batch_partial_failure_is_atomic() {
    let ctx = E2EContext::setup();

    // Create three streams; pause two; cancel one (the non-resumable target).
    let paused_a = ctx.create_stream(1_000);
    let cancelled = ctx.create_stream(1_000);
    let paused_b = ctx.create_stream(1_000);

    ctx.clear_pause_cooldown();
    ctx.stream_client
        .pause_stream_as_admin(&paused_a, &PauseReason::Emergency);
    ctx.clear_pause_cooldown();
    ctx.stream_client
        .pause_stream_as_admin(&paused_b, &PauseReason::Emergency);
    ctx.stream_client.cancel_stream_as_admin(&cancelled);

    assert_eq!(
        ctx.stream_client.get_stream_state(&paused_a).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.stream_client.get_stream_state(&paused_b).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.stream_client.get_stream_state(&cancelled).status,
        StreamStatus::Cancelled
    );

    // Incident: engage global emergency pause, then clear it via governance.
    ctx.stream_client.set_global_emergency_paused(&true);
    assert!(ctx.stream_client.get_global_emergency_paused());

    let resume_proposal = ctx.propose_approve_execute(&ctx.encode_global_resume());
    assert!(
        ExecutorStub::has_executed_event(&ctx.env, &ctx.governance_id, resume_proposal),
        "global_resume ProposalExecuted event must be present"
    );
    assert!(
        !ctx.stream_client.get_global_emergency_paused(),
        "global_resume must clear the emergency pause flag"
    );

    // Mixed batch: two resumable paused streams + one cancelled.
    ctx.clear_pause_cooldown();
    let batch = vec![&ctx.env, paused_a, cancelled, paused_b];
    let calldata = ctx.encode_bulk_resume(batch);

    let proposal_id = ctx
        .gov_client
        .propose(&ctx.signer_a, &ctx.stream_id, &calldata);
    ctx.gov_client.approve(&ctx.signer_a, &proposal_id);
    ctx.gov_client.approve(&ctx.signer_b, &proposal_id);
    ctx.advance_past_timelock();

    let executor = Address::generate(&ctx.env);
    let result = ctx.gov_client.try_execute(&executor, &proposal_id);

    // Dispatch traps on StreamTerminalState → whole execute reverts.
    assert!(
        result.is_err(),
        "mixed batch with a cancelled stream must fail, got {:?}",
        result
    );

    // Proposal must not be marked executed (tx reverted including CEI write).
    let proposal = ctx.gov_client.get_proposal(&proposal_id);
    assert!(
        !proposal.executed,
        "failed bulk resume must not leave proposal executed"
    );
    assert!(
        !ExecutorStub::has_executed_event(&ctx.env, &ctx.governance_id, proposal_id),
        "no ProposalExecuted event on atomic bulk-resume failure"
    );

    // Atomic: previously-paused streams stay Paused; cancelled stays Cancelled.
    assert_eq!(
        ctx.stream_client.get_stream_state(&paused_a).status,
        StreamStatus::Paused,
        "paused_a must not be partially resumed"
    );
    assert_eq!(
        ctx.stream_client.get_stream_state(&paused_b).status,
        StreamStatus::Paused,
        "paused_b must not be partially resumed"
    );
    assert_eq!(
        ctx.stream_client.get_stream_state(&cancelled).status,
        StreamStatus::Cancelled
    );
}

/// Even when the bulk-resume batch is malformed (includes a non-resumable
/// stream), governance quorum + timelock are still required — there is no
/// auth bypass via a bad batch.
#[test]
fn test_e2e_bulk_resume_partial_failure_still_requires_governance_auth() {
    let ctx = E2EContext::setup();

    let paused = ctx.create_stream(1_000);
    let cancelled = ctx.create_stream(1_000);
    ctx.clear_pause_cooldown();
    ctx.stream_client
        .pause_stream_as_admin(&paused, &PauseReason::Emergency);
    ctx.stream_client.cancel_stream_as_admin(&cancelled);

    let batch = vec![&ctx.env, paused, cancelled];
    let calldata = ctx.encode_bulk_resume(batch);

    let proposal_id = ctx
        .gov_client
        .propose(&ctx.signer_a, &ctx.stream_id, &calldata);

    // Pre-quorum: only one approval — execute must be blocked at governance.
    ctx.gov_client.approve(&ctx.signer_a, &proposal_id);
    ctx.advance_past_timelock();

    let executor = Address::generate(&ctx.env);
    let result = ctx.gov_client.try_execute(&executor, &proposal_id);
    assert_eq!(result, Err(Ok(GovernanceError::QuorumNotReached)));

    assert_eq!(
        ctx.stream_client.get_stream_state(&paused).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.stream_client.get_stream_state(&cancelled).status,
        StreamStatus::Cancelled
    );
    assert!(
        !ExecutorStub::has_executed_event(&ctx.env, &ctx.governance_id, proposal_id),
        "malformed batch must not bypass quorum"
    );
}

/// Happy-path control: after `global_resume`, an all-paused bulk resume via
/// governance succeeds and activates every target.
#[test]
fn test_e2e_global_resume_then_bulk_resume_all_paused_succeeds() {
    let ctx = E2EContext::setup();

    let a = ctx.create_stream(1_000);
    let b = ctx.create_stream(1_000);
    ctx.clear_pause_cooldown();
    ctx.stream_client
        .pause_stream_as_admin(&a, &PauseReason::Emergency);
    ctx.clear_pause_cooldown();
    ctx.stream_client
        .pause_stream_as_admin(&b, &PauseReason::Emergency);

    ctx.stream_client.set_global_emergency_paused(&true);
    ctx.propose_approve_execute(&ctx.encode_global_resume());
    assert!(!ctx.stream_client.get_global_emergency_paused());

    ctx.clear_pause_cooldown();
    let batch = vec![&ctx.env, a, b];
    let proposal_id = ctx.propose_approve_execute(&ctx.encode_bulk_resume(batch));

    assert!(ExecutorStub::has_executed_event(
        &ctx.env,
        &ctx.governance_id,
        proposal_id
    ));
    assert_eq!(
        ctx.stream_client.get_stream_state(&a).status,
        StreamStatus::Active
    );
    assert_eq!(
        ctx.stream_client.get_stream_state(&b).status,
        StreamStatus::Active
    );
}

/// Direct contract-level assertion of atomic mixed-batch semantics (mirrors
/// docs/global-resume.md without the governance wrapper).
#[test]
fn test_bulk_resume_as_admin_mixed_batch_atomic_no_partial() {
    let ctx = E2EContext::setup();

    let paused_a = ctx.create_stream(1_000);
    let cancelled = ctx.create_stream(1_000);
    let paused_b = ctx.create_stream(1_000);

    ctx.clear_pause_cooldown();
    ctx.stream_client
        .pause_stream_as_admin(&paused_a, &PauseReason::Emergency);
    ctx.clear_pause_cooldown();
    ctx.stream_client
        .pause_stream_as_admin(&paused_b, &PauseReason::Emergency);
    ctx.stream_client.cancel_stream_as_admin(&cancelled);

    ctx.clear_pause_cooldown();
    let result = ctx
        .stream_client
        .try_bulk_resume_streams_as_admin(&vec![&ctx.env, paused_a, cancelled, paused_b]);

    assert_eq!(result, Err(Ok(ContractError::StreamTerminalState)));
    assert_eq!(
        ctx.stream_client.get_stream_state(&paused_a).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.stream_client.get_stream_state(&paused_b).status,
        StreamStatus::Paused
    );
    assert_eq!(
        ctx.stream_client.get_stream_state(&cancelled).status,
        StreamStatus::Cancelled
    );
}
