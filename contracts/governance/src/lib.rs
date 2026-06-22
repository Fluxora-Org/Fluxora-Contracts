#![no_std]
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Bytes, Env, Map,
    Vec,
};

// ---------------------------------------------------------------------------
// Governance constants
// ---------------------------------------------------------------------------

/// Seconds a proposal must remain unexecuted after reaching quorum before it can
/// be executed. Default: 48 hours.
const GOVERNANCE_TIMELOCK_SECONDS: u64 = 172_800;

/// Maximum number of co-signers the governance contract supports.
const MAX_SIGNERS: u32 = 20;

/// Maximum byte length for proposal calldata payload.
const MAX_CALLDATA_BYTES: u32 = 4_096;

/// Maximum age in seconds for a proposal before it expires and becomes
/// non-executable. Default: 30 days.
const MAX_PROPOSAL_AGE_SECONDS: u64 = 2_592_000;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Persistent record of a governance proposal.
///
/// `calldata` is stored as an opaque `Bytes` payload whose interpretation is
/// left to the off-chain executor or to a typed adapter layer.  Storing the
/// payload on-chain provides a tamper-evident audit trail and enables indexers
/// to reconstruct the full proposal intent without any additional side-channel.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    /// Address that submitted the proposal.
    pub proposer: Address,
    /// Target contract whose parameters should be changed upon execution.
    pub target: Address,
    /// Opaque calldata encoding the intended function call and arguments.
    pub calldata: Bytes,
    /// List of co-signer addresses that have approved this proposal.
    pub approvals: Vec<Address>,
    /// Ledger timestamp at which the proposal was submitted.
    pub created_at: u64,
    /// True once `execute` has been called successfully.
    pub executed: bool,
    /// True once `cancel_proposal` has been called. Terminal — no further
    /// approvals or execution are allowed.
    pub cancelled: bool,
}

/// Error codes for the governance contract.
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GovernanceError {
    /// Contract has not been initialised.
    NotInitialized = 1,
    /// Contract is already initialised.
    AlreadyInitialized = 2,
    /// Caller is not the admin.
    Unauthorized = 3,
    /// Caller is not a registered co-signer.
    NotASigner = 4,
    /// Proposal with this ID does not exist.
    ProposalNotFound = 5,
    /// Proposal has already been executed.
    AlreadyExecuted = 6,
    /// Proposal has not yet accumulated the required number of approvals.
    QuorumNotReached = 7,
    /// Timelock period has not elapsed since quorum was first reached.
    TimelockNotElapsed = 8,
    /// Signer has already approved this proposal.
    AlreadyApproved = 9,
    /// Calldata exceeds MAX_CALLDATA_BYTES.
    CalldataTooLarge = 10,
    /// Signer list exceeds MAX_SIGNERS.
    TooManySigners = 11,
    /// Proposal has passed the max age window and can no longer be approved or executed.
    ProposalExpired = 12,
    /// Proposal has been cancelled and can no longer be approved or executed.
    ProposalCancelled = 13,
    /// Caller is not the proposer nor the admin of the contract.
    NotProposerOrAdmin = 14,
    /// Provided threshold is zero or exceeds signer count.
    InvalidThreshold = 15,
    /// Removing this signer would leave fewer signers than the required threshold.
    QuorumWouldBreak = 16,
    /// Signer is already registered in the co-signer set.
    DuplicateSigner = 17,
}

/// Storage keys for the governance contract.
#[contracttype]
pub enum DataKey {
    /// Admin address (instance storage).
    Admin,
    /// Registered co-signers list (instance storage).
    Signers,
    /// Minimum approval threshold (instance storage).
    Threshold,
    /// Monotonic proposal ID counter (instance storage).
    NextProposalId,
    /// Persistent record for a proposal (persistent storage, keyed by ID).
    Proposal(u32),
    /// Ledger timestamp at which a proposal first reached quorum (persistent).
    QuorumReachedAt(u32),
    /// Map-backed membership index for registered co-signers (instance storage).
    SignerIndex,
    /// Per-proposal approval membership index (persistent storage, keyed by ID).
    ProposalApprovals(u32),
}

// ---------------------------------------------------------------------------
// TTL constants (mirrors stream contract conventions)
// ---------------------------------------------------------------------------

const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;
const INSTANCE_BUMP_AMOUNT: u32 = 120_960;
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 17_280;
const PERSISTENT_BUMP_AMOUNT: u32 = 120_960;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Emitted when a new proposal is submitted.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProposalCreated {
    pub proposal_id: u32,
    pub proposer: Address,
    pub target: Address,
}

/// Records the timestamp and effective threshold when quorum was first reached.
/// Used to judge in-flight proposals against the threshold that was active at
/// quorum time, protecting against mid-flight threshold changes by the admin.
#[contracttype]
#[derive(Clone, Debug)]
pub struct QuorumInfo {
    pub reached_at: u64,
    pub threshold: u32,
}

/// Emitted when a co-signer approves a proposal.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProposalApproved {
    pub proposal_id: u32,
    pub approver: Address,
    pub approval_count: u32,
}

/// Emitted when quorum is first reached for a proposal, starting the timelock.
#[contracttype]
#[derive(Clone, Debug)]
pub struct QuorumReached {
    pub proposal_id: u32,
    pub quorum_reached_at: u64,
    pub executable_after: u64,
}

/// Emitted when a proposal is cancelled by the proposer or admin.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProposalCancelled {
    pub proposal_id: u32,
    pub canceller: Address,
}

/// Emitted when a proposal is executed after quorum and timelock.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProposalExecuted {
    pub proposal_id: u32,
    pub executor: Address,
    pub target: Address,
    pub calldata: Bytes,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn bump_proposal(env: &Env, id: u32) {
    env.storage().persistent().extend_ttl(
        &DataKey::Proposal(id),
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

fn get_admin(env: &Env) -> Result<Address, GovernanceError> {
    bump_instance(env);
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(GovernanceError::NotInitialized)
}

fn get_signers(env: &Env) -> Result<Vec<Address>, GovernanceError> {
    env.storage()
        .instance()
        .get(&DataKey::Signers)
        .ok_or(GovernanceError::NotInitialized)
}

fn build_signer_index(
    env: &Env,
    signers: &Vec<Address>,
) -> Result<Map<Address, bool>, GovernanceError> {
    let mut index = Map::new(env);
    for i in 0..signers.len() {
        if let Some(signer) = signers.get(i) {
            if index.contains_key(signer.clone()) {
                return Err(GovernanceError::DuplicateSigner);
            }
            index.set(signer, true);
        }
    }
    Ok(index)
}

fn get_signer_index(env: &Env) -> Result<Map<Address, bool>, GovernanceError> {
    bump_instance(env);
    if let Some(index) = env.storage().instance().get(&DataKey::SignerIndex) {
        return Ok(index);
    }

    let signers = get_signers(env)?;
    let index = build_signer_index(env, &signers)?;
    env.storage().instance().set(&DataKey::SignerIndex, &index);
    Ok(index)
}

fn save_signers_with_index(env: &Env, signers: &Vec<Address>, index: &Map<Address, bool>) {
    env.storage().instance().set(&DataKey::Signers, signers);
    env.storage().instance().set(&DataKey::SignerIndex, index);
    bump_instance(env);
}

fn get_threshold(env: &Env) -> Result<u32, GovernanceError> {
    env.storage()
        .instance()
        .get(&DataKey::Threshold)
        .ok_or(GovernanceError::NotInitialized)
}

fn read_next_proposal_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::NextProposalId)
        .unwrap_or(0u32)
}

fn increment_proposal_id(env: &Env) -> u32 {
    let id = read_next_proposal_id(env);
    env.storage()
        .instance()
        .set(&DataKey::NextProposalId, &(id + 1));
    id
}

fn load_proposal(env: &Env, id: u32) -> Result<Proposal, GovernanceError> {
    let proposal: Proposal = env
        .storage()
        .persistent()
        .get(&DataKey::Proposal(id))
        .ok_or(GovernanceError::ProposalNotFound)?;
    bump_proposal(env, id);
    Ok(proposal)
}

fn save_proposal(env: &Env, id: u32, proposal: &Proposal) {
    env.storage()
        .persistent()
        .set(&DataKey::Proposal(id), proposal);
    bump_proposal(env, id);
}

fn get_approval_index(env: &Env, id: u32, proposal: &Proposal) -> Map<Address, bool> {
    if let Some(index) = env
        .storage()
        .persistent()
        .get(&DataKey::ProposalApprovals(id))
    {
        bump_approval_index(env, id);
        return index;
    }

    let mut index = Map::new(env);
    for i in 0..proposal.approvals.len() {
        if let Some(approver) = proposal.approvals.get(i) {
            index.set(approver, true);
        }
    }
    env.storage()
        .persistent()
        .set(&DataKey::ProposalApprovals(id), &index);
    bump_approval_index(env, id);
    index
}

fn save_approval_index(env: &Env, id: u32, index: &Map<Address, bool>) {
    env.storage()
        .persistent()
        .set(&DataKey::ProposalApprovals(id), index);
    bump_approval_index(env, id);
}

fn bump_approval_index(env: &Env, id: u32) {
    env.storage().persistent().extend_ttl(
        &DataKey::ProposalApprovals(id),
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct FluxoraGovernance;

#[contractimpl]
impl FluxoraGovernance {
    /// Initialise the governance contract with an admin, a list of co-signers,
    /// and an approval threshold.
    ///
    /// # Parameters
    /// - `admin`: Address that can add/remove signers and reset governance state.
    /// - `signers`: Initial list of co-signers eligible to approve proposals.
    ///   Must not exceed `MAX_SIGNERS` and must not contain duplicates.
    /// - `threshold`: Minimum number of approvals required for a proposal to
    ///   execute.  Must satisfy `1 <= threshold <= signers.len()`.
    ///
    /// # Errors
    /// - `AlreadyInitialized`: Contract has already been initialised.
    /// - `TooManySigners`: Provided signer list exceeds `MAX_SIGNERS`.
    /// - `DuplicateSigner`: Provided signer list contains the same address twice.
    /// - `InvalidThreshold`: `threshold` is zero or exceeds the number of signers.
    pub fn init(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
        threshold: u32,
    ) -> Result<(), GovernanceError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(GovernanceError::AlreadyInitialized);
        }
        if signers.len() > MAX_SIGNERS {
            return Err(GovernanceError::TooManySigners);
        }
        let signer_index = build_signer_index(&env, &signers)?;
        if threshold == 0 || threshold > signers.len() {
            return Err(GovernanceError::InvalidThreshold);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Signers, &signers);
        env.storage()
            .instance()
            .set(&DataKey::SignerIndex, &signer_index);
        env.storage()
            .instance()
            .set(&DataKey::Threshold, &threshold);
        env.storage()
            .instance()
            .set(&DataKey::NextProposalId, &0u32);

        bump_instance(&env);
        Ok(())
    }

    /// Update the admin address.
    ///
    /// # Authorization
    /// - Requires admin signature.
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), GovernanceError> {
        get_admin(&env)?.require_auth();
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        bump_instance(&env);
        Ok(())
    }

    /// Add a co-signer to the governance set.
    ///
    /// The signer set is unique: an address may occupy at most one co-signer slot.
    ///
    /// # Authorization
    /// - Requires admin signature.
    ///
    /// # Errors
    /// - `TooManySigners`: Adding this signer would exceed `MAX_SIGNERS`.
    /// - `DuplicateSigner`: `signer` is already registered.
    pub fn add_signer(env: Env, signer: Address) -> Result<(), GovernanceError> {
        get_admin(&env)?.require_auth();
        let mut signers = get_signers(&env)?;
        let mut signer_index = get_signer_index(&env)?;
        if signer_index.contains_key(signer.clone()) {
            return Err(GovernanceError::DuplicateSigner);
        }
        if signers.len() >= MAX_SIGNERS {
            return Err(GovernanceError::TooManySigners);
        }
        signers.push_back(signer.clone());
        signer_index.set(signer, true);
        save_signers_with_index(&env, &signers, &signer_index);
        Ok(())
    }

    /// Remove a co-signer from the governance set.
    ///
    /// # Authorization
    /// - Requires admin signature.
    ///
    /// # Errors
    /// - `QuorumWouldBreak`: Removal would leave fewer signers than the required
    ///   threshold, making future proposals permanently unexecutable.
    pub fn remove_signer(env: Env, signer: Address) -> Result<(), GovernanceError> {
        get_admin(&env)?.require_auth();
        let mut signer_index = get_signer_index(&env)?;
        if !signer_index.contains_key(signer.clone()) {
            return Ok(());
        }

        let mut signers = get_signers(&env)?;
        let mut idx: Option<u32> = None;
        for i in 0..signers.len() {
            if signers.get(i).is_some_and(|candidate| candidate == signer) {
                idx = Some(i);
                break;
            }
        }
        if let Some(i) = idx {
            let threshold = get_threshold(&env)?;
            if signers.len() - 1 < threshold {
                return Err(GovernanceError::QuorumWouldBreak);
            }
            signers.remove(i);
            signer_index.remove(signer);
            save_signers_with_index(&env, &signers, &signer_index);
        }
        Ok(())
    }

    /// Submit a new governance proposal.
    ///
    /// Any registered co-signer may propose. The proposer does not automatically
    /// approve the proposal — they must call `approve` separately.
    ///
    /// # Parameters
    /// - `proposer`: The co-signer submitting the proposal.
    /// - `target`: The contract address to call when the proposal is executed.
    /// - `calldata`: Opaque bytes encoding the intended operation (stored for audit).
    ///
    /// # Returns
    /// - The proposal ID assigned to the new proposal (monotonically increasing u32).
    ///
    /// # Authorization
    /// - Requires `proposer.require_auth()`.
    ///
    /// # Errors
    /// - `NotASigner`: `proposer` is not in the registered signers list.
    /// - `CalldataTooLarge`: `calldata.len() > MAX_CALLDATA_BYTES`.
    pub fn propose(
        env: Env,
        proposer: Address,
        target: Address,
        calldata: Bytes,
    ) -> Result<u32, GovernanceError> {
        proposer.require_auth();

        // Verify proposer is a registered signer.
        let signer_index = get_signer_index(&env)?;
        if !signer_index.contains_key(proposer.clone()) {
            return Err(GovernanceError::NotASigner);
        }

        if calldata.len() > MAX_CALLDATA_BYTES {
            return Err(GovernanceError::CalldataTooLarge);
        }

        let id = increment_proposal_id(&env);
        let now = env.ledger().timestamp();

        let proposal = Proposal {
            proposer: proposer.clone(),
            target: target.clone(),
            calldata: calldata.clone(),
            approvals: Vec::new(&env),
            created_at: now,
            executed: false,
            cancelled: false,
        };

        save_proposal(&env, id, &proposal);
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("proposed"), id),
            ProposalCreated {
                proposal_id: id,
                proposer,
                target,
            },
        );

        Ok(id)
    }

    /// Approve a proposal as a registered co-signer.
    ///
    /// Each signer may approve at most once per proposal.  When the approval count
    /// first reaches the configured threshold, the timelock clock starts.
    ///
    /// # Parameters
    /// - `approver`: The co-signer casting their approval.
    /// - `proposal_id`: The proposal to approve.
    ///
    /// # Authorization
    /// - Requires `approver.require_auth()`.
    ///
    /// # Errors
    /// - `NotASigner`: `approver` is not in the registered signers list.
    /// - `ProposalNotFound`: No proposal with this ID.
    /// - `AlreadyExecuted`: Proposal has already been executed.
    /// - `AlreadyApproved`: This signer already approved this proposal.
    pub fn approve(env: Env, approver: Address, proposal_id: u32) -> Result<(), GovernanceError> {
        approver.require_auth();

        let signer_index = get_signer_index(&env)?;
        if !signer_index.contains_key(approver.clone()) {
            return Err(GovernanceError::NotASigner);
        }

        let mut proposal = load_proposal(&env, proposal_id)?;

        if proposal.cancelled {
            return Err(GovernanceError::ProposalCancelled);
        }
        if proposal.executed {
            return Err(GovernanceError::AlreadyExecuted);
        }
        if env.ledger().timestamp() > proposal.created_at + MAX_PROPOSAL_AGE_SECONDS {
            return Err(GovernanceError::ProposalExpired);
        }

        // Prevent duplicate approvals using the per-proposal membership index.
        let mut approval_index = get_approval_index(&env, proposal_id, &proposal);
        if approval_index.contains_key(approver.clone()) {
            return Err(GovernanceError::AlreadyApproved);
        }

        proposal.approvals.push_back(approver.clone());
        approval_index.set(approver.clone(), true);
        let approval_count = proposal.approvals.len();

        save_proposal(&env, proposal_id, &proposal);
        save_approval_index(&env, proposal_id, &approval_index);
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("approved"), proposal_id),
            ProposalApproved {
                proposal_id,
                approver,
                approval_count,
            },
        );

        // Record the timestamp and effective threshold at which quorum was first
        // reached.  Using the stored snapshot at execution time protects in-flight
        // proposals against mid-flight threshold changes by the admin.
        let threshold = get_threshold(&env)?;
        if approval_count == threshold {
            let now = env.ledger().timestamp();
            let executable_after = now + GOVERNANCE_TIMELOCK_SECONDS;
            let info = QuorumInfo {
                reached_at: now,
                threshold,
            };
            env.storage()
                .persistent()
                .set(&DataKey::QuorumReachedAt(proposal_id), &info);
            env.storage().persistent().extend_ttl(
                &DataKey::QuorumReachedAt(proposal_id),
                PERSISTENT_LIFETIME_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );

            env.events().publish(
                (symbol_short!("quorum"), proposal_id),
                QuorumReached {
                    proposal_id,
                    quorum_reached_at: now,
                    executable_after,
                },
            );
        }

        Ok(())
    }

    /// Execute a proposal that has reached quorum and passed the timelock.
    ///
    /// Marks the proposal as executed and emits `ProposalExecuted`.  The
    /// `target` address and `calldata` are included in the event so that
    /// off-chain executors or indexers can reconstruct and verify the call.
    ///
    /// # Parameters
    /// - `executor`: The address triggering execution (need not be a signer).
    /// - `proposal_id`: The proposal to execute.
    ///
    /// # Authorization
    /// - Requires `executor.require_auth()`.
    ///
    /// # Errors
    /// - `ProposalNotFound`: No proposal with this ID.
    /// - `AlreadyExecuted`: Proposal already executed.
    /// - `QuorumNotReached`: Approval count < threshold.
    /// - `TimelockNotElapsed`: Less than `GOVERNANCE_TIMELOCK_SECONDS` have passed
    ///   since quorum was reached.
    pub fn execute(env: Env, executor: Address, proposal_id: u32) -> Result<(), GovernanceError> {
        executor.require_auth();

        let mut proposal = load_proposal(&env, proposal_id)?;

        if proposal.cancelled {
            return Err(GovernanceError::ProposalCancelled);
        }
        if proposal.executed {
            return Err(GovernanceError::AlreadyExecuted);
        }
        if env.ledger().timestamp() > proposal.created_at + MAX_PROPOSAL_AGE_SECONDS {
            return Err(GovernanceError::ProposalExpired);
        }

        // Verify quorum was reached and use the recorded threshold (snapshot at
        // quorum time) so that in-flight proposals are immune to mid-flight
        // threshold changes.
        let quorum_info: QuorumInfo = env
            .storage()
            .persistent()
            .get(&DataKey::QuorumReachedAt(proposal_id))
            .ok_or(GovernanceError::QuorumNotReached)?;

        if proposal.approvals.len() < quorum_info.threshold {
            return Err(GovernanceError::QuorumNotReached);
        }

        // Verify timelock has elapsed from the moment quorum was reached.
        let now = env.ledger().timestamp();
        if now < quorum_info.reached_at + GOVERNANCE_TIMELOCK_SECONDS {
            return Err(GovernanceError::TimelockNotElapsed);
        }

        // CEI: mark as executed before emitting the event.
        proposal.executed = true;
        save_proposal(&env, proposal_id, &proposal);
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("executed"), proposal_id),
            ProposalExecuted {
                proposal_id,
                executor,
                target: proposal.target.clone(),
                calldata: proposal.calldata.clone(),
            },
        );

        Ok(())
    }

    /// Cancel a proposal, marking it as terminal so no further approvals or
    /// execution are possible.
    ///
    /// # Authorization
    /// - Requires `caller.require_auth()`.
    /// - `caller` must be the original `proposer` or the contract `admin`.
    ///
    /// # Parameters
    /// - `caller`: The address requesting cancellation.
    /// - `proposal_id`: The proposal to cancel.
    ///
    /// # Errors
    /// - `ProposalNotFound`: No proposal with this ID.
    /// - `AlreadyExecuted`: Proposal has already been executed.
    /// - `ProposalCancelled`: Proposal is already cancelled.
    /// - `NotProposerOrAdmin`: `caller` is neither the proposer nor the admin.
    pub fn cancel_proposal(
        env: Env,
        caller: Address,
        proposal_id: u32,
    ) -> Result<(), GovernanceError> {
        caller.require_auth();

        let mut proposal = load_proposal(&env, proposal_id)?;

        if proposal.executed {
            return Err(GovernanceError::AlreadyExecuted);
        }
        if proposal.cancelled {
            return Err(GovernanceError::ProposalCancelled);
        }

        // Only the original proposer or the admin may cancel.
        let admin = get_admin(&env)?;
        if caller != proposal.proposer && caller != admin {
            return Err(GovernanceError::NotProposerOrAdmin);
        }

        proposal.cancelled = true;
        save_proposal(&env, proposal_id, &proposal);
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("cancelled"), proposal_id),
            ProposalCancelled {
                proposal_id,
                canceller: caller,
            },
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Query entrypoints
    // -----------------------------------------------------------------------

    /// Read a proposal by ID.
    pub fn get_proposal(env: Env, proposal_id: u32) -> Result<Proposal, GovernanceError> {
        load_proposal(&env, proposal_id)
    }

    /// Return the number of proposals created so far.
    ///
    /// Proposal IDs are assigned densely starting at 0, so this is also the
    /// exclusive upper bound for enumerating proposals by ID.
    pub fn proposal_count(env: Env) -> u32 {
        bump_instance(&env);
        read_next_proposal_id(&env)
    }

    /// Return the list of registered co-signers.
    pub fn get_signers(env: Env) -> Result<Vec<Address>, GovernanceError> {
        get_signers(&env)
    }

    /// Return the effective approval threshold.
    pub fn quorum(env: Env) -> u32 {
        get_threshold(&env).unwrap_or(0)
    }

    /// Return the timelock duration in seconds.
    pub fn timelock_seconds(_env: Env) -> u64 {
        GOVERNANCE_TIMELOCK_SECONDS
    }

    /// Return the maximum proposal age in seconds before it expires.
    pub fn max_proposal_age_seconds(_env: Env) -> u64 {
        MAX_PROPOSAL_AGE_SECONDS
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{vec, Env};

    const TIMELOCK: u64 = 172_800;
    const MAX_AGE: u64 = 2_592_000;

    struct Ctx {
        env: Env,
        #[allow(dead_code)]
        contract_id: Address,
        admin: Address,
        signer_a: Address,
        signer_b: Address,
        #[allow(dead_code)]
        signer_c: Address,
        client: FluxoraGovernanceClient<'static>,
    }

    impl Ctx {
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

            Ctx {
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
    }

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    #[test]
    fn test_quorum_and_timelock_constants() {
        let ctx = Ctx::setup();
        assert_eq!(ctx.client.quorum(), 2);
        assert_eq!(ctx.client.timelock_seconds(), TIMELOCK);
    }

    #[test]
    fn test_max_proposal_age_constant() {
        let ctx = Ctx::setup();
        assert_eq!(ctx.client.max_proposal_age_seconds(), MAX_AGE);
    }

    // -----------------------------------------------------------------------
    // Threshold validation
    // -----------------------------------------------------------------------

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
        // 2 signers but threshold = 3
        let result = client.try_init(&admin, &vec![&env, signer_a, signer_b], &3u32);
        assert_eq!(result, Err(Ok(GovernanceError::InvalidThreshold)));
    }

    #[test]
    fn test_init_accepts_threshold_equal_to_signer_count() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let contract_id = env.register_contract(None, FluxoraGovernance);
        let admin = Address::generate(&env);
        let signer_a = Address::generate(&env);
        let signer_b = Address::generate(&env);
        let client = FluxoraGovernanceClient::new(&env, &contract_id);
        let result = client.try_init(&admin, &vec![&env, signer_a, signer_b], &2u32);
        assert!(result.is_ok());
        assert_eq!(client.quorum(), 2);
    }

    #[test]
    fn test_init_accepts_threshold_of_one() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);
        let contract_id = env.register_contract(None, FluxoraGovernance);
        let admin = Address::generate(&env);
        let signer = Address::generate(&env);
        let client = FluxoraGovernanceClient::new(&env, &contract_id);
        let result = client.try_init(&admin, &vec![&env, signer], &1u32);
        assert!(result.is_ok());
        assert_eq!(client.quorum(), 1);
    }

    // -----------------------------------------------------------------------
    // Quorum invariant on remove_signer
    // -----------------------------------------------------------------------

    #[test]
    fn test_remove_signer_down_to_threshold_succeeds() {
        let ctx = Ctx::setup(); // 3 signers, threshold=2
                                // After removing signer_c, we have 2 signers == threshold — should succeed.
        ctx.client.remove_signer(&ctx.signer_c);
        let signers = ctx.client.get_signers();
        assert_eq!(signers.len(), 2);
        // quorum still 2, which is <= signers.len() — invariant holds.
        assert_eq!(ctx.client.quorum(), 2);
    }

    #[test]
    fn test_remove_signer_below_threshold_errors() {
        let ctx = Ctx::setup(); // 3 signers, threshold=2
        ctx.client.remove_signer(&ctx.signer_c); // 2 signers left
                                                 // Trying to remove another signer would leave 1 < threshold=2
        let result = ctx.client.try_remove_signer(&ctx.signer_b);
        assert_eq!(result, Err(Ok(GovernanceError::QuorumWouldBreak)));
        // Verify signer set is unchanged.
        let signers = ctx.client.get_signers();
        assert_eq!(signers.len(), 2);
    }

    #[test]
    fn test_remove_signer_nonexistent_does_not_break_quorum() {
        let ctx = Ctx::setup(); // 3 signers, threshold=2
        let stranger = Address::generate(&ctx.env);
        // Removing a non-existent signer should be a no-op, not an error.
        let result = ctx.client.try_remove_signer(&stranger);
        assert!(result.is_ok());
        let signers = ctx.client.get_signers();
        assert_eq!(signers.len(), 3);
    }

    // -----------------------------------------------------------------------
    // Proposal creation
    // -----------------------------------------------------------------------

    #[test]
    fn test_propose_returns_incremental_ids() {
        let ctx = Ctx::setup();
        let id0 = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("a"));
        let id1 = ctx
            .client
            .propose(&ctx.signer_b, &ctx.dummy_target(), &ctx.calldata("b"));
        assert_eq!(id0, 0);
        assert_eq!(id1, 1);
    }

    #[test]
    fn test_propose_stores_proposal() {
        let ctx = Ctx::setup();
        let target = ctx.dummy_target();
        let data = ctx.calldata("set_cap:5000");
        let id = ctx.client.propose(&ctx.signer_a, &target, &data);
        let p = ctx.client.get_proposal(&id);
        assert_eq!(p.proposer, ctx.signer_a);
        assert_eq!(p.target, target);
        assert!(!p.executed);
        assert!(!p.cancelled);
        assert_eq!(p.approvals.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Cancellation
    // -----------------------------------------------------------------------

    #[test]
    fn test_cancel_by_proposer_succeeds() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.cancel_proposal(&ctx.signer_a, &id);
        let p = ctx.client.get_proposal(&id);
        assert!(p.cancelled);
    }

    #[test]
    fn test_cancel_by_admin_succeeds() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.cancel_proposal(&ctx.admin, &id);
        let p = ctx.client.get_proposal(&id);
        assert!(p.cancelled);
    }

    #[test]
    fn test_cancel_unauthorized_errors() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        let result = ctx.client.try_cancel_proposal(&ctx.signer_b, &id);
        assert_eq!(result, Err(Ok(GovernanceError::NotProposerOrAdmin)));
    }

    #[test]
    fn test_cancel_twice_errors() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.cancel_proposal(&ctx.signer_a, &id);
        let result = ctx.client.try_cancel_proposal(&ctx.signer_a, &id);
        assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
    }

    #[test]
    fn test_cancel_executed_proposal_errors() {
        let ctx = Ctx::setup();
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
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.cancel_proposal(&ctx.signer_a, &id);
        let result = ctx.client.try_approve(&ctx.signer_b, &id);
        assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
    }

    #[test]
    fn test_cancel_after_quorum_before_timelock() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        ctx.client.cancel_proposal(&ctx.signer_a, &id);
        let executor = Address::generate(&ctx.env);
        let result = ctx.client.try_execute(&executor, &id);
        assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
    }

    #[test]
    fn test_approve_after_cancel_errors() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.cancel_proposal(&ctx.signer_a, &id);
        let result = ctx.client.try_approve(&ctx.signer_b, &id);
        assert_eq!(result, Err(Ok(GovernanceError::ProposalCancelled)));
    }

    #[test]
    fn test_execute_after_cancel_errors() {
        let ctx = Ctx::setup();
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

    // -----------------------------------------------------------------------
    // Expiry
    // -----------------------------------------------------------------------

    #[test]
    fn test_approve_after_expiry_errors() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE + 1);
        let result = ctx.client.try_approve(&ctx.signer_b, &id);
        assert_eq!(result, Err(Ok(GovernanceError::ProposalExpired)));
    }

    #[test]
    fn test_execute_after_expiry_errors() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
        ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE + 1);
        let executor = Address::generate(&ctx.env);
        let result = ctx.client.try_execute(&executor, &id);
        assert_eq!(result, Err(Ok(GovernanceError::ProposalExpired)));
    }

    #[test]
    fn test_execute_at_expiry_boundary_succeeds() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        // Set timestamp to exactly the expiry boundary (created_at + MAX_AGE).
        // This is *not* past the boundary, so the proposal is not expired.
        // Since MAX_AGE >> TIMELOCK, the timelock has also elapsed.
        ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE);
        let executor = Address::generate(&ctx.env);
        let result = ctx.client.try_execute(&executor, &id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_expired_not_executable_even_with_quorum_and_timelock_met() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        ctx.env
            .ledger()
            .set_timestamp(1_000_000 + MAX_AGE + TIMELOCK + 100);
        let executor = Address::generate(&ctx.env);
        let result = ctx.client.try_execute(&executor, &id);
        assert_eq!(result, Err(Ok(GovernanceError::ProposalExpired)));
    }

    // -----------------------------------------------------------------------
    // Full happy path (regression)
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_governance_flow() {
        let ctx = Ctx::setup();
        let target = ctx.dummy_target();
        let calldata = ctx.calldata("set_cap:100000");
        let id = ctx.client.propose(&ctx.signer_a, &target, &calldata);
        assert_eq!(id, 0);

        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);

        let p = ctx.client.get_proposal(&id);
        assert_eq!(p.approvals.len(), 2);
        assert!(!p.executed);
        assert!(!p.cancelled);

        let executor = Address::generate(&ctx.env);
        let early = ctx.client.try_execute(&executor, &id);
        assert_eq!(early, Err(Ok(GovernanceError::TimelockNotElapsed)));

        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
        ctx.client.execute(&executor, &id);
        let p = ctx.client.get_proposal(&id);
        assert!(p.executed);
        assert_eq!(p.target, target);
    }

    #[test]
    fn test_execute_without_quorum_errors() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
        let executor = Address::generate(&ctx.env);
        let result = ctx.client.try_execute(&executor, &id);
        assert_eq!(result, Err(Ok(GovernanceError::QuorumNotReached)));
    }

    #[test]
    fn test_execute_twice_errors() {
        let ctx = Ctx::setup();
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
}
