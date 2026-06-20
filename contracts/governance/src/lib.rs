#![no_std]
#![allow(clippy::too_many_arguments)]

use soroban_sdk::{contract, contractimpl, contracttype, contracterror, symbol_short, Address, Bytes, Env, Vec};

// ---------------------------------------------------------------------------
// Governance constants
// ---------------------------------------------------------------------------

/// Minimum number of co-signer approvals required before a proposal can execute.
/// Default: 2-of-N (quorum of 2).
const GOVERNANCE_QUORUM: u32 = 2;

/// Seconds a proposal must remain unexecuted after reaching quorum before it can
/// be executed. Default: 48 hours.
const GOVERNANCE_TIMELOCK_SECONDS: u64 = 172_800;

/// Maximum number of co-signers the governance contract supports.
const MAX_SIGNERS: u32 = 20;

/// Maximum byte length for proposal calldata payload.
const MAX_CALLDATA_BYTES: u32 = 4_096;

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
    /// Signer is already registered in the co-signer set.
    DuplicateSigner = 12,
}

/// Storage keys for the governance contract.
#[contracttype]
pub enum DataKey {
    /// Admin address (instance storage).
    Admin,
    /// Registered co-signers list (instance storage).
    Signers,
    /// Monotonic proposal ID counter (instance storage).
    NextProposalId,
    /// Persistent record for a proposal (persistent storage, keyed by ID).
    Proposal(u32),
    /// Ledger timestamp at which a proposal first reached quorum (persistent).
    QuorumReachedAt(u32),
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
    env.storage().persistent().set(&DataKey::Proposal(id), proposal);
    bump_proposal(env, id);
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct FluxoraGovernance;

#[contractimpl]
impl FluxoraGovernance {
    /// Initialise the governance contract with an admin and a list of co-signers.
    ///
    /// # Parameters
    /// - `admin`: Address that can add/remove signers and reset governance state.
    /// - `signers`: Initial list of co-signers eligible to approve proposals.
    ///   Must not exceed `MAX_SIGNERS` and must not contain duplicates.
    ///
    /// # Errors
    /// - `AlreadyInitialized`: Contract has already been initialised.
    /// - `TooManySigners`: Provided signer list exceeds `MAX_SIGNERS`.
    /// - `DuplicateSigner`: Provided signer list contains the same address twice.
    pub fn init(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
    ) -> Result<(), GovernanceError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(GovernanceError::AlreadyInitialized);
        }
        if signers.len() > MAX_SIGNERS {
            return Err(GovernanceError::TooManySigners);
        }
        Self::require_unique_signers(&signers)?;

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Signers, &signers);
        env.storage().instance().set(&DataKey::NextProposalId, &0u32);

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
        if Self::is_signer(&signers, &signer) {
            return Err(GovernanceError::DuplicateSigner);
        }
        if signers.len() >= MAX_SIGNERS {
            return Err(GovernanceError::TooManySigners);
        }
        signers.push_back(signer);
        env.storage().instance().set(&DataKey::Signers, &signers);
        bump_instance(&env);
        Ok(())
    }

    /// Remove a co-signer from the governance set.
    ///
    /// # Authorization
    /// - Requires admin signature.
    pub fn remove_signer(env: Env, signer: Address) -> Result<(), GovernanceError> {
        get_admin(&env)?.require_auth();
        let mut signers = get_signers(&env)?;
        let mut idx: Option<u32> = None;
        for i in 0..signers.len() {
            if signers.get(i).unwrap() == signer {
                idx = Some(i);
                break;
            }
        }
        if let Some(i) = idx {
            signers.remove(i);
            env.storage().instance().set(&DataKey::Signers, &signers);
            bump_instance(&env);
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
        let signers = get_signers(&env)?;
        if !Self::is_signer(&signers, &proposer) {
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
    /// first reaches `GOVERNANCE_QUORUM`, the timelock clock starts.
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
    pub fn approve(
        env: Env,
        approver: Address,
        proposal_id: u32,
    ) -> Result<(), GovernanceError> {
        approver.require_auth();

        let signers = get_signers(&env)?;
        if !Self::is_signer(&signers, &approver) {
            return Err(GovernanceError::NotASigner);
        }

        let mut proposal = load_proposal(&env, proposal_id)?;

        if proposal.executed {
            return Err(GovernanceError::AlreadyExecuted);
        }

        // Prevent duplicate approvals.
        for i in 0..proposal.approvals.len() {
            if proposal.approvals.get(i).unwrap() == approver {
                return Err(GovernanceError::AlreadyApproved);
            }
        }

        proposal.approvals.push_back(approver.clone());
        let approval_count = proposal.approvals.len();

        save_proposal(&env, proposal_id, &proposal);
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("approved"), proposal_id),
            ProposalApproved {
                proposal_id,
                approver,
                approval_count,
            },
        );

        // Record the timestamp at which quorum was first reached so the timelock
        // can be measured from that moment.
        if approval_count == GOVERNANCE_QUORUM {
            let now = env.ledger().timestamp();
            let executable_after = now + GOVERNANCE_TIMELOCK_SECONDS;
            env.storage()
                .persistent()
                .set(&DataKey::QuorumReachedAt(proposal_id), &now);
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
    /// - `QuorumNotReached`: Approval count < `GOVERNANCE_QUORUM`.
    /// - `TimelockNotElapsed`: Less than `GOVERNANCE_TIMELOCK_SECONDS` have passed
    ///   since quorum was reached.
    pub fn execute(
        env: Env,
        executor: Address,
        proposal_id: u32,
    ) -> Result<(), GovernanceError> {
        executor.require_auth();

        let mut proposal = load_proposal(&env, proposal_id)?;

        if proposal.executed {
            return Err(GovernanceError::AlreadyExecuted);
        }

        if proposal.approvals.len() < GOVERNANCE_QUORUM {
            return Err(GovernanceError::QuorumNotReached);
        }

        // Verify timelock has elapsed from the moment quorum was reached.
        let quorum_at: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::QuorumReachedAt(proposal_id))
            .ok_or(GovernanceError::QuorumNotReached)?;

        let now = env.ledger().timestamp();
        if now < quorum_at + GOVERNANCE_TIMELOCK_SECONDS {
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

    // -----------------------------------------------------------------------
    // Query entrypoints
    // -----------------------------------------------------------------------

    /// Read a proposal by ID.
    pub fn get_proposal(env: Env, proposal_id: u32) -> Result<Proposal, GovernanceError> {
        load_proposal(&env, proposal_id)
    }

    /// Return the list of registered co-signers.
    pub fn get_signers(env: Env) -> Result<Vec<Address>, GovernanceError> {
        get_signers(&env)
    }

    /// Return the governance quorum constant.
    pub fn quorum(_env: Env) -> u32 {
        GOVERNANCE_QUORUM
    }

    /// Return the timelock duration in seconds.
    pub fn timelock_seconds(_env: Env) -> u64 {
        GOVERNANCE_TIMELOCK_SECONDS
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn is_signer(signers: &Vec<Address>, addr: &Address) -> bool {
        for i in 0..signers.len() {
            if &signers.get(i).unwrap() == addr {
                return true;
            }
        }
        false
    }

    fn require_unique_signers(signers: &Vec<Address>) -> Result<(), GovernanceError> {
        for i in 0..signers.len() {
            let signer = signers.get(i).unwrap();
            for j in (i + 1)..signers.len() {
                if signers.get(j).unwrap() == signer {
                    return Err(GovernanceError::DuplicateSigner);
                }
            }
        }
        Ok(())
    }
}
