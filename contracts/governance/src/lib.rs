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
    /// Governance arithmetic would overflow instead of producing a valid deadline or ID.
    ArithmeticOverflow = 18,
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
    /// Map<Address, bool> membership index for O(1) signer lookups (instance storage).
    SignerIndex,
    /// Per-proposal Map<Address, bool> for O(1) duplicate-approval detection (persistent).
    ProposalApprovalIdx(u32),
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

/// Emitted when the admin adds a new co-signer to the governance set.
///
/// Published by [`add_signer`](FluxoraGovernance::add_signer) after the signer
/// list has been persisted (CEI: state mutation precedes the event). Indexers
/// use this to reconstruct the live co-signer set from chain events alone.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SignerAdded {
    /// The address that was added to the co-signer set.
    pub signer: Address,
}

/// Emitted when the admin removes an existing co-signer from the governance set.
///
/// Published by [`remove_signer`](FluxoraGovernance::remove_signer) only when a
/// matching address was actually removed and the updated signer list persisted.
/// Removing an address that is not registered is a no-op and emits **no** event.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SignerRemoved {
    /// The address that was removed from the co-signer set.
    pub signer: Address,
}

/// Emitted when the admin address is rotated.
///
/// Published by [`set_admin`](FluxoraGovernance::set_admin) after the new admin
/// has been persisted (CEI: state mutation precedes the event). Carries both the
/// previous and new admin so indexers can reconstruct the full admin history.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AdminChanged {
    /// The admin address that was in effect before the rotation.
    pub old: Address,
    /// The admin address that is in effect after the rotation.
    pub new: Address,
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

/// Extends the TTL of the QuorumReachedAt entry so it outlives the timelock.
/// Called on every approve and execute to prevent archival before execution.
fn bump_quorum_ttl(env: &Env, id: u32) {
    if env
        .storage()
        .persistent()
        .has(&DataKey::QuorumReachedAt(id))
    {
        env.storage().persistent().extend_ttl(
            &DataKey::QuorumReachedAt(id),
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
}

fn get_signer_index(env: &Env) -> Result<Map<Address, bool>, GovernanceError> {
    env.storage()
        .instance()
        .get(&DataKey::SignerIndex)
        .ok_or(GovernanceError::NotInitialized)
}

fn save_signer_index(env: &Env, index: &Map<Address, bool>) {
    env.storage().instance().set(&DataKey::SignerIndex, index);
}

fn get_approval_index(env: &Env, proposal_id: u32) -> Map<Address, bool> {
    env.storage()
        .persistent()
        .get(&DataKey::ProposalApprovalIdx(proposal_id))
        .unwrap_or_else(|| Map::new(env))
}

fn save_approval_index(env: &Env, proposal_id: u32, index: &Map<Address, bool>) {
    env.storage()
        .persistent()
        .set(&DataKey::ProposalApprovalIdx(proposal_id), index);
    env.storage().persistent().extend_ttl(
        &DataKey::ProposalApprovalIdx(proposal_id),
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

fn checked_deadline(start: u64, seconds: u64) -> Result<u64, GovernanceError> {
    start
        .checked_add(seconds)
        .ok_or(GovernanceError::ArithmeticOverflow)
}

fn increment_proposal_id(env: &Env) -> Result<u32, GovernanceError> {
    let id = read_next_proposal_id(env);
    let next = id
        .checked_add(1)
        .ok_or(GovernanceError::ArithmeticOverflow)?;
    env.storage()
        .instance()
        .set(&DataKey::NextProposalId, &next);
    Ok(id)
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
        if threshold == 0 || threshold > signers.len() {
            return Err(GovernanceError::InvalidThreshold);
        }

        // Build Map index in a single O(n) pass; duplicates are detected via the map.
        let mut signer_index: Map<Address, bool> = Map::new(&env);
        for i in 0..signers.len() {
            let s = signers.get(i).unwrap();
            if signer_index.contains_key(s.clone()) {
                return Err(GovernanceError::DuplicateSigner);
            }
            signer_index.set(s, true);
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
        let old_admin = get_admin(&env)?;
        old_admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        bump_instance(&env);

        // CEI: the new admin is persisted before the event is emitted.
        env.events().publish(
            (symbol_short!("adm_chg"),),
            AdminChanged {
                old: old_admin,
                new: new_admin,
            },
        );

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

        // O(1) duplicate check via Map index.
        if signer_index.contains_key(signer.clone()) {
            return Err(GovernanceError::DuplicateSigner);
        }
        if signers.len() >= MAX_SIGNERS {
            return Err(GovernanceError::TooManySigners);
        }
        signers.push_back(signer.clone());
        signer_index.set(signer.clone(), true);
        env.storage().instance().set(&DataKey::Signers, &signers);
        save_signer_index(&env, &signer_index);
        bump_instance(&env);

        // CEI: the updated signer set is persisted before the event is emitted.
        env.events()
            .publish((symbol_short!("sgnr_add"),), SignerAdded { signer });

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

        // O(1) membership check — skip Vec scan entirely when address is absent.
        if !signer_index.contains_key(signer.clone()) {
            return Ok(());
        }

        let mut signers = get_signers(&env)?;
        let threshold = get_threshold(&env)?;
        if signers.len() - 1 < threshold {
            return Err(GovernanceError::QuorumWouldBreak);
        }

        // Scan Vec only to find the removal position (unavoidable for ordered Vec removal).
        for i in 0..signers.len() {
            if signers.get(i).is_some_and(|candidate| candidate == signer) {
                signers.remove(i);
                break;
            }
        }

        signer_index.remove(signer.clone());
        env.storage().instance().set(&DataKey::Signers, &signers);
        save_signer_index(&env, &signer_index);
        bump_instance(&env);

        // CEI: the updated signer set is persisted before the event is
        // emitted. Only reached when a matching signer was actually removed;
        // removing a non-existent address returned early above (silent no-op,
        // no event).
        env.events()
            .publish((symbol_short!("sgnr_rm"),), SignerRemoved { signer });
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
    /// - `ArithmeticOverflow`: proposal ID counter has reached `u32::MAX`.
    pub fn propose(
        env: Env,
        proposer: Address,
        target: Address,
        calldata: Bytes,
    ) -> Result<u32, GovernanceError> {
        proposer.require_auth();

        // O(1) signer membership check via Map index.
        if !Self::is_signer(&env, &proposer)? {
            return Err(GovernanceError::NotASigner);
        }

        if calldata.len() > MAX_CALLDATA_BYTES {
            return Err(GovernanceError::CalldataTooLarge);
        }

        let id = increment_proposal_id(&env)?;
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
    /// - `ArithmeticOverflow`: proposal age or quorum timelock deadline cannot be represented.
    pub fn approve(env: Env, approver: Address, proposal_id: u32) -> Result<(), GovernanceError> {
        approver.require_auth();

        // O(1) signer membership check via Map index.
        if !Self::is_signer(&env, &approver)? {
            return Err(GovernanceError::NotASigner);
        }

        let mut proposal = load_proposal(&env, proposal_id)?;

        if proposal.cancelled {
            return Err(GovernanceError::ProposalCancelled);
        }
        if proposal.executed {
            return Err(GovernanceError::AlreadyExecuted);
        }
        if env.ledger().timestamp()
            > checked_deadline(proposal.created_at, MAX_PROPOSAL_AGE_SECONDS)?
        {
            return Err(GovernanceError::ProposalExpired);
        }

        // O(1) duplicate-approval check via per-proposal Map index.
        let mut approval_idx = get_approval_index(&env, proposal_id);
        if approval_idx.contains_key(approver.clone()) {
            return Err(GovernanceError::AlreadyApproved);
        }

        proposal.approvals.push_back(approver.clone());
        approval_idx.set(approver.clone(), true);
        let approval_count = proposal.approvals.len();

        let threshold = get_threshold(&env)?;
        let quorum_reached = if approval_count == threshold {
            let now = env.ledger().timestamp();
            let executable_after = checked_deadline(now, GOVERNANCE_TIMELOCK_SECONDS)?;
            Some((now, executable_after))
        } else {
            None
        };

        save_proposal(&env, proposal_id, &proposal);
        save_approval_index(&env, proposal_id, &approval_idx);
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
        if let Some((now, executable_after)) = quorum_reached {
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
            bump_quorum_ttl(&env, proposal_id);

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
    /// - `ArithmeticOverflow`: proposal age or quorum timelock deadline cannot be represented.
    pub fn execute(env: Env, executor: Address, proposal_id: u32) -> Result<(), GovernanceError> {
        executor.require_auth();

        let mut proposal = load_proposal(&env, proposal_id)?;

        if proposal.cancelled {
            return Err(GovernanceError::ProposalCancelled);
        }
        if proposal.executed {
            return Err(GovernanceError::AlreadyExecuted);
        }
        if env.ledger().timestamp()
            > checked_deadline(proposal.created_at, MAX_PROPOSAL_AGE_SECONDS)?
        {
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
        bump_quorum_ttl(&env, proposal_id);

        if proposal.approvals.len() < quorum_info.threshold {
            return Err(GovernanceError::QuorumNotReached);
        }

        // Verify timelock has elapsed from the moment quorum was reached.
        let now = env.ledger().timestamp();
        let exec_after = Self::executable_after(&quorum_info)?;
        if now < exec_after {
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

    /// Return the stored `QuorumInfo` snapshot for a proposal, or `None` if
    /// quorum has not yet been reached.
    ///
    /// # Parameters
    /// - `proposal_id`: The proposal to query.
    ///
    /// # Returns
    /// - `Some(QuorumInfo { reached_at, threshold })` if quorum was reached.
    /// - `None` if quorum has not been reached (no approvals, below threshold,
    ///   or proposal does not exist).
    ///
    /// This is a pure read — no authorization required, no state mutation
    /// other than the standard TTL bump on the stored `QuorumInfo` entry.
    pub fn get_quorum_info(env: Env, proposal_id: u32) -> Option<QuorumInfo> {
        let info: Option<QuorumInfo> = env
            .storage()
            .persistent()
            .get(&DataKey::QuorumReachedAt(proposal_id));
        if info.is_some() {
            env.storage().persistent().extend_ttl(
                &DataKey::QuorumReachedAt(proposal_id),
                PERSISTENT_LIFETIME_THRESHOLD,
                PERSISTENT_BUMP_AMOUNT,
            );
        }
        info
    }

    /// Return `true` if the proposal is in an executable state **right now**.
    ///
    /// Mirrors the exact gating order used by [`execute`](Self::execute):
    ///
    /// 1. Proposal exists (`ProposalNotFound` otherwise).
    /// 2. Not cancelled.
    /// 3. Not already executed.
    /// 4. Not expired.
    /// 5. Quorum has been reached (approvals >= threshold snapshot).
    /// 6. Timelock has elapsed (`now >= executable_after`).
    ///
    /// # Parameters
    /// - `proposal_id`: The proposal to check.
    ///
    /// # Returns
    /// - `Ok(true)` iff all gates pass — the proposal can be executed now.
    /// - `Ok(false)` if any gate blocks execution (cancelled, executed,
    ///   expired, quorum not reached, timelock not elapsed).
    /// - `Err(GovernanceError::ProposalNotFound)` if the ID is unknown.
    /// - `Err(GovernanceError::ArithmeticOverflow)` if timelock arithmetic
    ///   overflows (should not happen under normal ledger conditions).
    ///
    /// This is a pure read — no authorization required, no state mutation
    /// beyond the TTL bumps already performed by [`load_proposal`] and
    /// [`get_quorum_info`].
    pub fn is_executable(env: Env, proposal_id: u32) -> Result<bool, GovernanceError> {
        let proposal = load_proposal(&env, proposal_id)?;

        if proposal.cancelled {
            return Ok(false);
        }
        if proposal.executed {
            return Ok(false);
        }
        if env.ledger().timestamp()
            > checked_deadline(proposal.created_at, MAX_PROPOSAL_AGE_SECONDS)?
        {
            return Ok(false);
        }

        let quorum_info: QuorumInfo = match env
            .storage()
            .persistent()
            .get(&DataKey::QuorumReachedAt(proposal_id))
        {
            Some(info) => {
                env.storage().persistent().extend_ttl(
                    &DataKey::QuorumReachedAt(proposal_id),
                    PERSISTENT_LIFETIME_THRESHOLD,
                    PERSISTENT_BUMP_AMOUNT,
                );
                info
            }
            None => return Ok(false),
        };

        if proposal.approvals.len() < quorum_info.threshold {
            return Ok(false);
        }

        let now = env.ledger().timestamp();
        let exec_after = Self::executable_after(&quorum_info)?;
        if now < exec_after {
            return Ok(false);
        }

        Ok(true)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Compute the ledger timestamp at which a proposal becomes executable,
    /// given its `QuorumInfo` snapshot.
    ///
    /// Returns `reached_at + GOVERNANCE_TIMELOCK_SECONDS`, or
    /// `ArithmeticOverflow` if the sum would overflow `u64`.
    fn executable_after(info: &QuorumInfo) -> Result<u64, GovernanceError> {
        checked_deadline(info.reached_at, GOVERNANCE_TIMELOCK_SECONDS)
    }

    /// O(1) signer membership check via the Map index stored in instance storage.
    fn is_signer(env: &Env, addr: &Address) -> Result<bool, GovernanceError> {
        let index = get_signer_index(env)?;
        Ok(index.contains_key(addr.clone()))
    }
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

    #[test]
    fn test_propose_returns_structured_error_when_proposal_id_counter_overflows() {
        let ctx = Ctx::setup();
        ctx.env.as_contract(&ctx.contract_id, || {
            ctx.env
                .storage()
                .instance()
                .set(&DataKey::NextProposalId, &u32::MAX);
        });

        let result = ctx.client.try_propose(
            &ctx.signer_a,
            &ctx.dummy_target(),
            &ctx.calldata("overflow"),
        );

        assert_eq!(result, Err(Ok(GovernanceError::ArithmeticOverflow)));
        ctx.env.as_contract(&ctx.contract_id, || {
            assert_eq!(read_next_proposal_id(&ctx.env), u32::MAX);
        });
    }

    #[test]
    fn test_approve_returns_structured_error_when_quorum_timelock_overflows() {
        let ctx = Ctx::setup();
        ctx.env.ledger().set_timestamp(u64::MAX - MAX_AGE);
        let id = ctx.client.propose(
            &ctx.signer_a,
            &ctx.dummy_target(),
            &ctx.calldata("timelock"),
        );

        ctx.client.approve(&ctx.signer_a, &id);
        ctx.env.ledger().set_timestamp(u64::MAX - TIMELOCK + 1);

        let result = ctx.client.try_approve(&ctx.signer_b, &id);

        assert_eq!(result, Err(Ok(GovernanceError::ArithmeticOverflow)));
    }

    #[test]
    fn test_execute_returns_structured_error_when_quorum_timelock_overflows() {
        let ctx = Ctx::setup();
        ctx.env.ledger().set_timestamp(u64::MAX - MAX_AGE);
        let id = ctx.client.propose(
            &ctx.signer_a,
            &ctx.dummy_target(),
            &ctx.calldata("timelock"),
        );
        let mut proposal = ctx.client.get_proposal(&id);
        proposal.approvals.push_back(ctx.signer_a.clone());
        proposal.approvals.push_back(ctx.signer_b.clone());
        ctx.env.as_contract(&ctx.contract_id, || {
            save_proposal(&ctx.env, id, &proposal);
            ctx.env.storage().persistent().set(
                &DataKey::QuorumReachedAt(id),
                &QuorumInfo {
                    reached_at: u64::MAX - TIMELOCK + 1,
                    threshold: 2,
                },
            );
        });
        ctx.env.ledger().set_timestamp(u64::MAX - 100);
        let executor = Address::generate(&ctx.env);

        let result = ctx.client.try_execute(&executor, &id);

        assert_eq!(result, Err(Ok(GovernanceError::ArithmeticOverflow)));
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

    // -----------------------------------------------------------------------
    // get_quorum_info
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_quorum_info_before_quorum() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        // No approvals yet — quorum not reached.
        assert!(ctx.client.get_quorum_info(&id).is_none());
    }

    #[test]
    fn test_get_quorum_info_below_threshold() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        // Only 1 approval — threshold is 2, quorum not reached.
        assert!(ctx.client.get_quorum_info(&id).is_none());
    }

    #[test]
    fn test_get_quorum_info_after_quorum() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        // First approval — below threshold.
        ctx.client.approve(&ctx.signer_a, &id);
        // Second approval — hits threshold, quorum reached at timestamp 1_000_000.
        ctx.client.approve(&ctx.signer_b, &id);

        let info = ctx
            .client
            .get_quorum_info(&id)
            .expect("should have quorum info");
        assert_eq!(info.reached_at, 1_000_000);
        assert_eq!(info.threshold, 2);
    }

    #[test]
    fn test_get_quorum_info_preserves_snapshot_threshold() {
        // Verify the snapshot threshold is independent of later threshold changes.
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);

        let info = ctx
            .client
            .get_quorum_info(&id)
            .expect("should have quorum info");
        assert_eq!(info.threshold, 2);

        // Remove signer_c — threshold stays 2, snapshot should still be 2.
        ctx.client.remove_signer(&ctx.signer_c);
        let info = ctx
            .client
            .get_quorum_info(&id)
            .expect("should still have quorum info");
        assert_eq!(info.threshold, 2);
    }

    #[test]
    fn test_get_quorum_info_none_for_nonexistent_proposal() {
        let ctx = Ctx::setup();
        // A valid ID that was never proposed; no QuorumInfo exists.
        assert!(ctx.client.get_quorum_info(&999).is_none());
    }

    #[test]
    fn test_get_quorum_info_none_after_execute() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
        let executor = Address::generate(&ctx.env);
        ctx.client.execute(&executor, &id);
        // QuorumInfo should still exist (execution does not delete it).
        let info = ctx
            .client
            .get_quorum_info(&id)
            .expect("should still have quorum info after execute");
        assert_eq!(info.reached_at, 1_000_000);
        assert_eq!(info.threshold, 2);
    }

    // -----------------------------------------------------------------------
    // is_executable
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_executable_nonexistent_proposal() {
        let ctx = Ctx::setup();
        let result = ctx.client.try_is_executable(&999);
        assert_eq!(result, Err(Ok(GovernanceError::ProposalNotFound)));
    }

    #[test]
    fn test_is_executable_pre_quorum() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        // No approvals yet.
        assert_eq!(ctx.client.is_executable(&id), false);
    }

    #[test]
    fn test_is_executable_below_threshold() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        // Only 1 approval — threshold 2 not met.
        assert_eq!(ctx.client.is_executable(&id), false);
    }

    #[test]
    fn test_is_executable_post_quorum_pre_timelock() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        // Timelock not yet elapsed (current time is still 1_000_000).
        assert_eq!(ctx.client.is_executable(&id), false);
    }

    #[test]
    fn test_is_executable_post_timelock() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
        assert_eq!(ctx.client.is_executable(&id), true);
    }

    #[test]
    fn test_is_executable_cancelled() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        ctx.client.cancel_proposal(&ctx.signer_a, &id);
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
        assert_eq!(ctx.client.is_executable(&id), false);
    }

    #[test]
    fn test_is_executable_executed() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
        let executor = Address::generate(&ctx.env);
        ctx.client.execute(&executor, &id);
        assert_eq!(ctx.client.is_executable(&id), false);
    }

    #[test]
    fn test_is_executable_expired() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE + 1);
        assert_eq!(ctx.client.is_executable(&id), false);
    }

    #[test]
    fn test_is_executable_at_timelock_boundary() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);

        // Exactly at reached_at + TIMELOCK — timelock has elapsed (now >= exec_after).
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK);
        assert_eq!(ctx.client.is_executable(&id), true);
    }

    #[test]
    fn test_is_executable_one_second_before_timelock() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);

        // One second before the timelock elapses — should NOT be executable.
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK - 1);
        assert_eq!(ctx.client.is_executable(&id), false);
    }

    #[test]
    fn test_is_executable_at_expiry_boundary() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);

        // Exactly at created_at + MAX_AGE — not past it, so not expired.
        // Since MAX_AGE >> TIMELOCK, the timelock has also elapsed.
        ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE);
        assert_eq!(ctx.client.is_executable(&id), true);
    }

    #[test]
    fn test_is_executable_one_second_before_expiry() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);

        // One second before expiry — still executable if timelock has elapsed.
        ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE - 1);
        assert_eq!(ctx.client.is_executable(&id), true);
    }

    #[test]
    fn test_is_executable_one_second_after_expiry() {
        let ctx = Ctx::setup();
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);

        // One second past expiry — not executable.
        ctx.env.ledger().set_timestamp(1_000_000 + MAX_AGE + 1);
        assert_eq!(ctx.client.is_executable(&id), false);
    }

    #[test]
    fn test_is_executable_agrees_with_execute_across_states() {
        let ctx = Ctx::setup();

        // --- Pre-quorum ---
        let id = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("x"));
        assert_eq!(ctx.client.is_executable(&id), false);
        let executor = Address::generate(&ctx.env);
        assert_eq!(
            ctx.client.try_execute(&executor, &id),
            Err(Ok(GovernanceError::QuorumNotReached))
        );

        // --- Post-quorum, pre-timelock ---
        ctx.client.approve(&ctx.signer_a, &id);
        ctx.client.approve(&ctx.signer_b, &id);
        assert_eq!(ctx.client.is_executable(&id), false);
        assert_eq!(
            ctx.client.try_execute(&executor, &id),
            Err(Ok(GovernanceError::TimelockNotElapsed))
        );

        // --- Post-timelock, executable ---
        ctx.env.ledger().set_timestamp(1_000_000 + TIMELOCK + 1);
        assert_eq!(ctx.client.is_executable(&id), true);
        assert!(ctx.client.try_execute(&executor, &id).is_ok());

        // --- Post-execution ---
        assert_eq!(ctx.client.is_executable(&id), false);
        assert_eq!(
            ctx.client.try_execute(&executor, &id),
            Err(Ok(GovernanceError::AlreadyExecuted))
        );

        // --- Cancelled proposal (fresh) ---
        let id2 = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("y"));
        ctx.client.approve(&ctx.signer_a, &id2);
        ctx.client.approve(&ctx.signer_b, &id2);
        ctx.client.cancel_proposal(&ctx.signer_a, &id2);
        assert_eq!(ctx.client.is_executable(&id2), false);
        assert_eq!(
            ctx.client.try_execute(&executor, &id2),
            Err(Ok(GovernanceError::ProposalCancelled))
        );

        // --- Expired proposal (fresh) ---
        let id3 = ctx
            .client
            .propose(&ctx.signer_a, &ctx.dummy_target(), &ctx.calldata("z"));
        ctx.client.approve(&ctx.signer_a, &id3);
        ctx.client.approve(&ctx.signer_b, &id3);
        ctx.env
            .ledger()
            .set_timestamp(1_000_000 + MAX_AGE + TIMELOCK + 100);
        assert_eq!(ctx.client.is_executable(&id3), false);
        assert_eq!(
            ctx.client.try_execute(&executor, &id3),
            Err(Ok(GovernanceError::ProposalExpired))
        );
    }
}
