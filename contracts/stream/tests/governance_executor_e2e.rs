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
//!
//! # Security notes
//!
//! The parameter change is impossible without the full
//! quorum + timelock + execute path completing.

extern crate std;

use fluxora_governance::{
    CallData, FluxoraGovernance, FluxoraGovernanceClient, GovernanceError, ProposalExecuted,
};
use fluxora_stream::{DataKey, FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::StellarAssetClient,
    vec, xdr::FromXdr, Address, Bytes, Env, Symbol, TryFromVal, Val, Vec as SdkVec,
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
            _ => {
                panic!(
                    "ExecutorStub: unexpected calldata variant {:?}",
                    op
                );
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
        // governance contract can successfully call set_max_rate_per_second
        // via dispatch_call during execute.
        stream_client.init(&token_id, &governance_id);

        // Mint some tokens for potential stream creation
        let sender = Address::generate(&env);
        token_asset.mint(&sender, &1_000_000_000);

        E2EContext {
            env,
            governance_id,
            stream_id,
            admin,
            signer_a,
            signer_b,
            signer_c,
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

    /// Advance the ledger timestamp past the timelock so the proposal with
    /// `quorum_at` becomes executable.
    fn advance_past_timelock(&self) {
        self.env
            .ledger()
            .set_timestamp(BASE_TIMESTAMP + TIMELOCK + 1);
    }

    /// Advance past max age so the proposal expires.
    fn advance_past_max_age(&self) {
        self.env
            .ledger()
            .set_timestamp(BASE_TIMESTAMP + MAX_AGE + 1);
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
    let proposal_id =
        ctx.gov_client
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
    let early_result = ctx
        .gov_client
        .try_execute(&executor, &proposal_id);
    assert_eq!(
        early_result,
        Err(Ok(GovernanceError::TimelockNotElapsed))
    );
    assert_eq!(ctx.current_max_rate(), i128::MAX);

    // ---- Wait for timelock ----
    ctx.advance_past_timelock();

    // ---- Execute (governance dispatches the call to the stream contract) ----
    ctx.gov_client
        .execute(&executor, &proposal_id);

    // ---- Assert stream parameter changed ----
    assert_eq!(ctx.current_max_rate(), 5_000);

    // ---- Executor stub reads the event and verifies the flow ----
    let has_event = ExecutorStub::has_executed_event(
        &ctx.env,
        &ctx.governance_id,
        proposal_id,
    );
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
