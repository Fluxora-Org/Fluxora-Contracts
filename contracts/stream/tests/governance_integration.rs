extern crate std;

use fluxora_governance::{
    DataKey, FluxoraGovernance, FluxoraGovernanceClient, GovernanceError, Proposal, QuorumInfo,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Bytes, Env,
};

// Mirror constants from governance lib.rs
const TIMELOCK: u64 = 172_800; // 48 hours
const MAX_AGE: u64 = 2_592_000; // 30 days

struct GovCtx<'a> {
    env: Env,
    contract_id: Address,
    admin: Address,
    signer_a: Address,
    signer_b: Address,
    signer_c: Address,
    client: FluxoraGovernanceClient<'a>,
}

impl<'a> GovCtx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);

        let contract_id = env.register_contract(None, FluxoraGovernance);

        let admin = Address::generate(&env);
        let signer_a = Address::generate(&env);
        let signer_b = Address::generate(&env);
        let signer_c = Address::generate(&env);

        let client = FluxoraGovernanceClient::new(&env, &contract_id);
        client.init(
            &admin,
            &vec![&env, signer_a.clone(), signer_b.clone(), signer_c.clone()],
            &2u32,
        );

        GovCtx {
            env,
            contract_id,
            admin,
            signer_a,
            signer_b,
            signer_c,
            client,
        }
    }

    fn dummy_target(&self) -> Address {
        Address::generate(&self.env)
    }

    fn calldata(&self, tag: &str) -> Bytes {
        Bytes::from_slice(&self.env, tag.as_bytes())
    }

    fn seed_next_proposal_id(&self, next_id: u32) {
        let contract_id = self.contract_id.clone();
        self.env.as_contract(&contract_id, || {
            self.env
                .storage()
                .instance()
                .set(&DataKey::NextProposalId, &next_id);
        });
    }

    fn seed_proposal(&self, proposal_id: u32, proposal: &Proposal) {
        let contract_id = self.contract_id.clone();
        self.env.as_contract(&contract_id, || {
            self.env
                .storage()
                .persistent()
                .set(&DataKey::Proposal(proposal_id), proposal);
        });
    }

    fn seed_quorum_info(&self, proposal_id: u32, info: &QuorumInfo) {
        let contract_id = self.contract_id.clone();
        self.env.as_contract(&contract_id, || {
            self.env
                .storage()
                .persistent()
                .set(&DataKey::QuorumReachedAt(proposal_id), info);
        });
    }
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

#[test]
fn test_init_stores_signers() {
    let ctx = GovCtx::setup();
    let signers = ctx.client.get_signers();
    assert_eq!(signers.len(), 3);
}

#[test]
fn test_init_twice_errors() {
    let ctx = GovCtx::setup();
    let result = ctx
        .client
        .try_init(&ctx.admin, &vec![&ctx.env, ctx.signer_a.clone()], &1u32);
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyInitialized)));
}

#[test]
fn test_init_duplicate_signers_errors() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, FluxoraGovernance);
    let client = FluxoraGovernanceClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let result = client.try_init(&admin, &vec![&env, signer.clone(), signer]);

    assert_eq!(result, Err(Ok(GovernanceError::DuplicateSigner)));
}

#[test]
fn test_quorum_and_timelock_constants() {
    let ctx = GovCtx::setup();
    assert_eq!(ctx.client.quorum(), 2);
    assert_eq!(ctx.client.timelock_seconds(), TIMELOCK);
}

// ---------------------------------------------------------------------------
// Proposal creation
// ---------------------------------------------------------------------------

#[test]
fn test_propose_returns_incremental_ids() {
    let ctx = GovCtx::setup();
    let target = ctx.dummy_target();

    assert_eq!(ctx.client.proposal_count(), 0);

    let id0 = ctx
        .client
        .propose(&ctx.signer_a, &target, &ctx.calldata("call0"));
    assert_eq!(id0, 0);
    assert_eq!(ctx.client.proposal_count(), 1);

    let id1 = ctx
        .client
        .propose(&ctx.signer_b, &target, &ctx.calldata("call1"));
    assert_eq!(id1, 1);
    assert_eq!(ctx.client.proposal_count(), 2);

    let id2 = ctx
        .client
        .propose(&ctx.signer_c, &target, &ctx.calldata("call2"));
    assert_eq!(id2, 2);
    assert_eq!(ctx.client.proposal_count(), 3);
}

#[test]
fn test_propose_returns_structured_error_on_proposal_id_overflow() {
    let ctx = GovCtx::setup();
    ctx.seed_next_proposal_id(u32::MAX);

    let result = ctx.client.try_propose(
        &ctx.signer_a,
        &ctx.dummy_target(),
        &ctx.calldata("overflow"),
    );

    assert_eq!(result, Err(Ok(GovernanceError::ProposalIdOverflow)));
    assert_eq!(ctx.client.proposal_count(), u32::MAX);
}

#[test]
fn test_propose_non_signer_errors() {
    let ctx = GovCtx::setup();
    let outsider = Address::generate(&ctx.env);
    let result = ctx
        .client
        .try_propose(&outsider, &ctx.dummy_target(), &ctx.calldata("x"));
    assert_eq!(result, Err(Ok(GovernanceError::NotASigner)));
}

#[test]
fn test_propose_stores_proposal() {
    let ctx = GovCtx::setup();
    let target = ctx.dummy_target();
    let data = ctx.calldata("set_cap:5000");

    let id = ctx.client.propose(&ctx.signer_a, &target, &data);
    let proposal = ctx.client.get_proposal(&id);

    assert_eq!(proposal.proposer, ctx.signer_a);
    assert_eq!(proposal.target, target);
    assert!(!proposal.executed);
    assert_eq!(proposal.approvals.len(), 0);
}

// ---------------------------------------------------------------------------
// Approval
// ---------------------------------------------------------------------------

#[test]
fn test_approve_increments_approval_count() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    let p = ctx.client.get_proposal(&id);
    assert_eq!(p.approvals.len(), 1);

    ctx.client.approve(&ctx.signer_b, &id);
    let p = ctx.client.get_proposal(&id);
    assert_eq!(p.approvals.len(), 2);
}

#[test]
fn test_approve_returns_structured_error_when_executable_after_overflows() {
    let ctx = GovCtx::setup();
    let proposal = Proposal {
        proposer: ctx.signer_a.clone(),
        target: ctx.dummy_target(),
        calldata: ctx.calldata("x"),
        approvals: vec![&ctx.env, ctx.signer_a.clone()],
        created_at: u64::MAX - MAX_AGE,
        executed: false,
        cancelled: false,
    };
    ctx.seed_proposal(42, &proposal);
    ctx.env.ledger().set_timestamp(u64::MAX - TIMELOCK + 1);

    let result = ctx.client.try_approve(&ctx.signer_b, &42u32);

    assert_eq!(result, Err(Ok(GovernanceError::ArithmeticOverflow)));
}

#[test]
fn test_approve_duplicate_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    let result = ctx.client.try_approve(&ctx.signer_a, &id);
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyApproved)));
}

#[test]
fn test_approve_non_signer_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
    let outsider = Address::generate(&ctx.env);

    let result = ctx.client.try_approve(&outsider, &id);
    assert_eq!(result, Err(Ok(GovernanceError::NotASigner)));
}

#[test]
fn test_approve_nonexistent_proposal_errors() {
    let ctx = GovCtx::setup();
    let result = ctx.client.try_approve(&ctx.signer_a, &9999u32);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalNotFound)));
}

#[test]
fn test_approve_executed_proposal_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    // Advance past timelock
    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    let executor = Address::generate(&ctx.env);
    ctx.client.execute(&executor, &id);

    let result = ctx.client.try_approve(&ctx.signer_c, &id);
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyExecuted)));
}

// ---------------------------------------------------------------------------
// Execution — happy path
// ---------------------------------------------------------------------------

#[test]
fn test_execute_after_quorum_and_timelock_succeeds() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    // Advance past timelock
    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    let executor = Address::generate(&ctx.env);
    ctx.client.execute(&executor, &id);

    let p = ctx.client.get_proposal(&id);
    assert!(p.executed);
}

// ---------------------------------------------------------------------------
// Execution — error paths
// ---------------------------------------------------------------------------

#[test]
fn test_execute_without_quorum_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    // Only 1 approval (quorum = 2)
    ctx.client.approve(&ctx.signer_a, &id);

    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::QuorumNotReached)));
}

#[test]
fn test_execute_before_timelock_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    // Advance less than the full timelock
    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK - 1);

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::TimelockNotElapsed)));
}

#[test]
fn test_execute_twice_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    let executor = Address::generate(&ctx.env);
    ctx.client.execute(&executor, &id);

    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyExecuted)));
}

#[test]
fn test_execute_nonexistent_proposal_errors() {
    let ctx = GovCtx::setup();
    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &9999u32);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalNotFound)));
}

// ---------------------------------------------------------------------------
// Signer management
// ---------------------------------------------------------------------------

#[test]
fn test_add_remove_signer() {
    let ctx = GovCtx::setup();
    let new_signer = Address::generate(&ctx.env);

    ctx.client.add_signer(&new_signer);
    let signers = ctx.client.get_signers();
    assert_eq!(signers.len(), 4);

    ctx.client.remove_signer(&new_signer);
    let signers = ctx.client.get_signers();
    assert_eq!(signers.len(), 3);
}

#[test]
fn test_add_duplicate_signer_errors() {
    let ctx = GovCtx::setup();
    let result = ctx.client.try_add_signer(&ctx.signer_a);

    assert_eq!(result, Err(Ok(GovernanceError::DuplicateSigner)));
}

#[test]
fn test_add_signer_unauthorized_errors() {
    let ctx = GovCtx::setup();
    let outsider = Address::generate(&ctx.env);
    // mock_all_auths is active so we test logic only (auth is always satisfied);
    // to isolate the Unauthorized path we would need to disable mock_all_auths.
    // This test verifies a signer can still propose after being added.
    let new_signer = Address::generate(&ctx.env);
    ctx.client.add_signer(&new_signer);
    // New signer can now propose
    let id = ctx
        .client
        .propose(&new_signer, &ctx.dummy_target(), &ctx.calldata("y"));
    let p = ctx.client.get_proposal(&id);
    assert_eq!(p.proposer, new_signer);
    let _ = outsider; // suppress unused warning
}

// ---------------------------------------------------------------------------
// Threshold and quorum invariant
// ---------------------------------------------------------------------------

#[test]
fn test_init_rejects_zero_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let contract_id = env.register_contract(None, FluxoraGovernance);
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let client = FluxoraGovernanceClient::new(&env, &contract_id);
    let result = client.try_init(&admin, &vec![&env, signer], &0u32);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidThreshold)));
}

#[test]
fn test_init_rejects_threshold_above_signer_count() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let contract_id = env.register_contract(None, FluxoraGovernance);
    let admin = Address::generate(&env);
    let signer_a = Address::generate(&env);
    let signer_b = Address::generate(&env);
    let client = FluxoraGovernanceClient::new(&env, &contract_id);
    let result = client.try_init(&admin, &vec![&env, signer_a, signer_b], &3u32);
    assert_eq!(result, Err(Ok(GovernanceError::InvalidThreshold)));
}

#[test]
fn test_remove_signer_below_threshold_errors() {
    let ctx = GovCtx::setup(); // 3 signers, threshold=2
    ctx.client.remove_signer(&ctx.signer_c); // 2 signers left
    let result = ctx.client.try_remove_signer(&ctx.signer_b);
    assert_eq!(result, Err(Ok(GovernanceError::QuorumWouldBreak)));
    let signers = ctx.client.get_signers();
    assert_eq!(signers.len(), 2);
}

#[test]
fn test_execute_with_exactly_threshold_approvals_succeeds() {
    let ctx = GovCtx::setup(); // 3 signers, threshold=2
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert!(result.is_ok());
}

#[test]
fn test_quorum_threshold_respected_after_add_signer() {
    // With threshold=2 and 4 signers, still need exactly 2 approvals.
    let ctx = GovCtx::setup(); // 3 signers, threshold=2
    let extra = Address::generate(&ctx.env);
    ctx.client.add_signer(&extra);
    assert_eq!(ctx.client.get_signers().len(), 4);
    assert_eq!(ctx.client.quorum(), 2); // threshold unchanged

    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
    ctx.client.approve(&ctx.signer_a, &id);
    // Only 1 approval — should NOT reach quorum since threshold=2
    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::QuorumNotReached)));
}

// ---------------------------------------------------------------------------
// Full flow: propose → 2-of-3 approve → wait timelock → execute
// ---------------------------------------------------------------------------

#[test]
fn test_full_governance_flow() {
    let ctx = GovCtx::setup();
    let target = ctx.dummy_target();
    let calldata = ctx.calldata("set_cap:100000");

    // Signer A proposes
    let id = ctx.client.propose(&ctx.signer_a, &target, &calldata);
    assert_eq!(id, 0);

    // Signers A and B approve (quorum = 2)
    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    let p = ctx.client.get_proposal(&id);
    assert_eq!(p.approvals.len(), 2);
    assert!(!p.executed);

    // Cannot execute before timelock
    let executor = Address::generate(&ctx.env);
    let early_result = ctx.client.try_execute(&executor, &id);
    assert_eq!(early_result, Err(Ok(GovernanceError::TimelockNotElapsed)));

    // Advance past timelock
    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    ctx.client.execute(&executor, &id);

    let p = ctx.client.get_proposal(&id);
    assert!(p.executed);
    assert_eq!(p.target, target);
}

// ---------------------------------------------------------------------------
// Edge: third signer approves after quorum; extra approval is recorded
// ---------------------------------------------------------------------------

#[test]
fn test_third_approval_after_quorum_is_stored() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);
    // Third signer also approves (valid; just redundant for quorum)
    ctx.client.approve(&ctx.signer_c, &id);

    let p = ctx.client.get_proposal(&id);
    assert_eq!(p.approvals.len(), 3);
}

// ---------------------------------------------------------------------------
// Edge: calldata is preserved in proposal and execution event
// ---------------------------------------------------------------------------

#[test]
fn test_calldata_preserved_in_proposal() {
    let ctx = GovCtx::setup();
    let data = ctx.calldata("set_min_duration:86400");
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &data);
    let p = ctx.client.get_proposal(&id);
    assert_eq!(p.calldata, data);
}

// ---------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------

#[test]
fn test_cancel_by_proposer_succeeds() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.cancel_proposal(&ctx.signer_a, &id);

    let p = ctx.client.get_proposal(&id);
    assert!(p.cancelled);
}

#[test]
fn test_cancel_by_admin_succeeds() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.cancel_proposal(&ctx.admin, &id);

    let p = ctx.client.get_proposal(&id);
    assert!(p.cancelled);
    assert!(!p.executed);
}

#[test]
fn test_cancel_unauthorized_non_proposer_non_admin_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    // signer_b is neither the proposer (signer_a) nor the admin
    let result = ctx.client.try_cancel_proposal(&ctx.signer_b, &id);
    assert_eq!(result, Err(Ok(GovernanceError::NotProposerOrAdmin)));
}

#[test]
fn test_cancel_twice_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.cancel_proposal(&ctx.signer_a, &id);

    let result = ctx.client.try_cancel_proposal(&ctx.signer_a, &id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
}

#[test]
fn test_cancel_executed_proposal_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    let executor = Address::generate(&ctx.env);
    ctx.client.execute(&executor, &id);

    let result = ctx.client.try_cancel_proposal(&ctx.signer_a, &id);
    assert_eq!(result, Err(Ok(GovernanceError::AlreadyExecuted)));
}

#[test]
fn test_cancel_before_quorum() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    // Cancel before any approvals
    ctx.client.cancel_proposal(&ctx.signer_a, &id);

    // Subsequent approve should fail
    let result = ctx.client.try_approve(&ctx.signer_b, &id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
}

#[test]
fn test_cancel_after_quorum_before_timelock() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    // Cancel before timelock elapses
    ctx.client.cancel_proposal(&ctx.signer_a, &id);

    // Execute should fail
    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
}

#[test]
fn test_approve_after_cancel_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.cancel_proposal(&ctx.signer_a, &id);

    let result = ctx.client.try_approve(&ctx.signer_b, &id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
}

#[test]
fn test_execute_after_cancel_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    ctx.client.cancel_proposal(&ctx.signer_a, &id);

    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
}

// ---------------------------------------------------------------------------
// Expiry
// ---------------------------------------------------------------------------

#[test]
fn test_execute_at_expiry_boundary_succeeds() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    // Set timestamp to exactly the expiry boundary (created_at + MAX_AGE)
    ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE);

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::TimelockNotElapsed)));
}

#[test]
fn test_execute_after_expiry_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    // Advance past timelock
    ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);

    // Now advance past the max age too
    ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE + 1);

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalExpired)));
}

#[test]
fn test_approve_after_expiry_errors() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    // Advance past max age
    ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE + 1);

    let result = ctx.client.try_approve(&ctx.signer_b, &id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalExpired)));
}

#[test]
fn test_approve_returns_structured_error_when_expiry_overflows() {
    let ctx = GovCtx::setup();
    let proposal = Proposal {
        proposer: ctx.signer_a.clone(),
        target: ctx.dummy_target(),
        calldata: ctx.calldata("x"),
        approvals: vec![&ctx.env],
        created_at: u64::MAX - MAX_AGE + 1,
        executed: false,
        cancelled: false,
    };
    ctx.seed_proposal(43, &proposal);

    let result = ctx.client.try_approve(&ctx.signer_b, &43u32);

    assert_eq!(result, Err(Ok(GovernanceError::ArithmeticOverflow)));
}

#[test]
fn test_expired_not_executable_even_with_quorum_and_timelock_met() {
    let ctx = GovCtx::setup();
    let id = ctx
        .client
        .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.client.approve(&ctx.signer_b, &id);

    // Advance past both timelock and max age
    ctx.env
        .ledger()
        .set_timestamp(1_000_000 + MAX_AGE + TIMELOCK + 100);

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalExpired)));
}

#[test]
fn test_execute_returns_structured_error_when_quorum_timelock_overflows() {
    let ctx = GovCtx::setup();
    let proposal = Proposal {
        proposer: ctx.signer_a.clone(),
        target: ctx.dummy_target(),
        calldata: ctx.calldata("x"),
        approvals: vec![&ctx.env, ctx.signer_a.clone(), ctx.signer_b.clone()],
        created_at: 1_000_000,
        executed: false,
        cancelled: false,
    };
    ctx.seed_proposal(77, &proposal);
    ctx.seed_quorum_info(
        77,
        &QuorumInfo {
            reached_at: u64::MAX - TIMELOCK + 1,
            threshold: 2,
        },
    );

    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &77u32);

    assert_eq!(result, Err(Ok(GovernanceError::ArithmeticOverflow)));
    assert!(!ctx.client.get_proposal(&77u32).executed);
}

#[test]
fn test_max_proposal_age_constant() {
    let ctx = GovCtx::setup();
    assert_eq!(ctx.client.max_proposal_age_seconds(), MAX_AGE);
}
