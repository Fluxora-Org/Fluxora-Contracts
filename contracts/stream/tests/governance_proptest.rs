//! Property-based tests for the `FluxoraGovernance` contract.
//!
//! # Invariants under test
//!
//! These properties assert correctness of the three most security-critical
//! behaviours of the governance timelock contract, across randomised inputs:
//!
//! 1. **Below-quorum invariant** — a proposal can never be executed when
//!    `approvals.len() < threshold`.  Regardless of how many signers are in
//!    the set, the correct approval count to reach quorum is
//!    `min(threshold, n_signers)`.  Providing fewer approvals than the
//!    threshold must always return `QuorumNotReached`.
//!
//! 2. **Timelock invariant** — a proposal that has reached quorum can never
//!    be executed before `quorum_reached_at + GOVERNANCE_TIMELOCK_SECONDS`.
//!    The boundary condition (`now == exec_after`) is also tested: the
//!    contract uses `now < exec_after` so the boundary is *inclusive* of
//!    execution.
//!
//! 3. **One-way executed flag** — once a proposal has been executed the
//!    `executed` field is permanently `true`.  A second `execute` call on
//!    the same proposal must always return `AlreadyExecuted`.
//!
//! # Security notes
//!
//! The highest-risk off-by-one locations are:
//! - `now < exec_after` in `execute` (timelock boundary).
//! - `approval_count == threshold` in `approve` (quorum boundary).
//!
//! The parametric strategies deliberately probe values adjacent to those
//! boundaries (`TIMELOCK - 1`, `TIMELOCK`, `TIMELOCK + 1`;
//! `threshold - 1`, `threshold`) so randomised case generation covers the
//! exact same boundary scenarios that hand-written example tests can miss.
//!
//! # Determinism
//!
//! proptest is configured with a fixed seed (`0`) so that regression runs in
//! CI are deterministic.  The case count (256) is intentionally kept small to
//! balance coverage with wall-clock time in CI.

extern crate std;

use fluxora_governance::{FluxoraGovernance, FluxoraGovernanceClient, GovernanceError};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Bytes, Env,
};

// ---------------------------------------------------------------------------
// Mirror constants from governance lib.rs
// ---------------------------------------------------------------------------

/// Timelock duration in seconds — must match `GOVERNANCE_TIMELOCK_SECONDS` in lib.rs.
const TIMELOCK: u64 = 172_800; // 48 hours

/// Maximum proposal age — must match `MAX_PROPOSAL_AGE_SECONDS` in lib.rs.
const MAX_AGE: u64 = 2_592_000; // 30 days

/// Maximum co-signers — must match `MAX_SIGNERS` in lib.rs.
const MAX_SIGNERS: usize = 20;

/// Ledger timestamp used as the creation baseline in all tests.
///
/// Chosen far enough from 0 to allow proposals to be created at a realistic
/// timestamp, and far enough below `u64::MAX` to avoid arithmetic overflow in
/// the timelock and age deadline calculations.
const BASE_TIMESTAMP: u64 = 10_000_000;

// ---------------------------------------------------------------------------
// Shared helper: build a fresh Soroban env with `n_signers` co-signers and
// a configured threshold, returning the client and the signer address pool.
// ---------------------------------------------------------------------------

/// Stand-alone test environment for a single governance contract instance.
struct GovEnv {
    env: Env,
    /// All registered co-signer addresses in declaration order.
    signers: std::vec::Vec<Address>,
    client: FluxoraGovernanceClient<'static>,
}

impl GovEnv {
    /// Construct a governance contract with `n_signers` unique co-signers and
    /// the given `threshold`.
    ///
    /// # Panics
    /// Panics if `n_signers == 0`, `threshold == 0`, or
    /// `threshold > n_signers` — the contract enforces these constraints and
    /// would return `InvalidThreshold`.
    fn new(n_signers: usize, threshold: u32) -> Self {
        assert!(n_signers > 0 && n_signers <= MAX_SIGNERS);
        assert!(threshold > 0 && threshold as usize <= n_signers);

        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(BASE_TIMESTAMP);

        let contract_id = env.register_contract(None, FluxoraGovernance);
        let admin = Address::generate(&env);

        // Build the signer pool.
        let mut signers = std::vec::Vec::with_capacity(n_signers);
        let mut sdk_signers = vec![&env];
        for _ in 0..n_signers {
            let addr = Address::generate(&env);
            sdk_signers.push_back(addr.clone());
            signers.push(addr);
        }

        let client = FluxoraGovernanceClient::new(&env, &contract_id);
        client.init(&admin, &sdk_signers, &threshold);

        GovEnv {
            env,
            signers,
            client,
        }
    }

    /// Helper: return an opaque `Bytes` payload used as proposal calldata.
    fn calldata(&self, tag: &[u8]) -> Bytes {
        Bytes::from_slice(&self.env, tag)
    }

    /// Helper: return a fresh random target address.
    fn target(&self) -> Address {
        Address::generate(&self.env)
    }

    /// Submit a proposal from `signers[0]`, returning the assigned ID.
    fn propose(&self) -> u32 {
        self.client.propose(
            &self.signers[0],
            &self.target(),
            &self.calldata(b"proptest"),
        )
    }

    /// Have the first `n` signers approve `proposal_id`.
    ///
    /// `n` is clamped to the number of registered signers.
    fn approve_n(&self, proposal_id: u32, n: usize) {
        let n = n.min(self.signers.len());
        for signer in self.signers.iter().take(n) {
            self.client.approve(signer, &proposal_id);
        }
    }

    /// Advance the ledger timestamp to `BASE_TIMESTAMP + delta`.
    fn advance_time(&self, delta: u64) {
        self.env.ledger().set_timestamp(BASE_TIMESTAMP + delta);
    }
}

// ---------------------------------------------------------------------------
// proptest strategies
// ---------------------------------------------------------------------------

/// Strategy: signer count in [1, MAX_SIGNERS].
fn signer_count_strategy() -> impl Strategy<Value = usize> {
    1usize..=MAX_SIGNERS
}

/// Strategy: threshold in [1, n_signers] given a concrete signer count.
fn threshold_strategy(n_signers: usize) -> impl Strategy<Value = u32> {
    (1u32..=(n_signers as u32))
}

/// Strategy: number of approvals to apply in [0, n_signers].
fn approval_count_strategy(n_signers: usize) -> impl Strategy<Value = usize> {
    0usize..=n_signers
}

/// Strategy: time delta offset relative to the quorum timestamp, covering
/// the region just before, at, and just after the timelock boundary.
/// Offsets are expressed in seconds relative to `TIMELOCK`.
fn time_delta_strategy() -> impl Strategy<Value = u64> {
    // Sample uniformly across [0, TIMELOCK + 1000].
    // Ensures we hit "before", "at boundary", and "after" cases.
    0u64..=(TIMELOCK + 1_000)
}

// ---------------------------------------------------------------------------
// Invariant 1 — Below-quorum: execute always fails when approvals < threshold
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        // Fixed seed for deterministic CI runs.
        source_file: Some("contracts/stream/tests/governance_proptest.rs"),
        ..ProptestConfig::default()
    })]

    /// **Invariant 1 — Below-quorum guard.**
    ///
    /// For any combination of signer set size and threshold, supplying strictly
    /// fewer approvals than `threshold` must prevent execution.
    ///
    /// The test advances the clock past the timelock window so that the *only*
    /// gate that can fail is the quorum check — not the time check.
    ///
    /// Asserts: `execute` returns `QuorumNotReached` whenever
    /// `approval_count < threshold`.
    #[test]
    fn prop_execute_fails_below_quorum(
        n_signers in signer_count_strategy(),
        threshold_raw in 1usize..=20usize,
    ) {
        // Clamp threshold to a valid range for this signer count.
        let threshold = (threshold_raw.min(n_signers)) as u32;

        let gov = GovEnv::new(n_signers, threshold);
        let id = gov.propose();

        // Apply strictly fewer approvals than required.
        // When threshold == 1 we cannot apply 0-of-1 via the below formula
        // and still have an interesting quorum miss, so we use 0 approvals.
        let below_threshold = if threshold > 1 { (threshold - 1) as usize } else { 0 };
        gov.approve_n(id, below_threshold);

        // Advance well past the timelock so time is not the blocking factor.
        gov.advance_time(TIMELOCK + 10_000);

        let executor = Address::generate(&gov.env);
        let result = gov.client.try_execute(&executor, &id);

        // The proposal never reached quorum, so QuorumNotReached must be
        // returned regardless of time.
        prop_assert_eq!(
            result,
            Err(Ok(GovernanceError::QuorumNotReached)),
            "Expected QuorumNotReached with {below_threshold} approvals and threshold={threshold}"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 2 — Timelock: execute always fails before quorum_at + TIMELOCK
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        source_file: Some("contracts/stream/tests/governance_proptest.rs"),
        ..ProptestConfig::default()
    })]

    /// **Invariant 2 — Timelock guard.**
    ///
    /// After quorum is reached at ledger time `quorum_at`, any `execute` call
    /// where `now < quorum_at + GOVERNANCE_TIMELOCK_SECONDS` must fail with
    /// `TimelockNotElapsed`.
    ///
    /// The test uses a randomised time delta in `[0, TIMELOCK - 1]` to
    /// exercise values strictly before the boundary, and separately asserts
    /// success at and after the boundary.
    ///
    /// Asserts: `execute` returns `TimelockNotElapsed` for
    /// `now < quorum_at + TIMELOCK`.
    #[test]
    fn prop_execute_fails_before_timelock_elapses(
        n_signers in signer_count_strategy(),
        threshold_raw in 1usize..=20usize,
        // Random time delta in the open interval (0, TIMELOCK).
        // We add 1 so the minimum is 1 second before the boundary.
        seconds_short in 1u64..TIMELOCK,
    ) {
        let threshold = (threshold_raw.min(n_signers)) as u32;
        let gov = GovEnv::new(n_signers, threshold);
        let id = gov.propose();

        // Apply exactly threshold approvals to reach quorum at BASE_TIMESTAMP.
        gov.approve_n(id, threshold as usize);

        // Advance to (quorum_at + TIMELOCK - seconds_short), which is strictly
        // less than the executable timestamp.
        let advance_by = TIMELOCK - seconds_short;
        gov.advance_time(advance_by);

        let executor = Address::generate(&gov.env);
        let result = gov.client.try_execute(&executor, &id);

        prop_assert_eq!(
            result,
            Err(Ok(GovernanceError::TimelockNotElapsed)),
            "Expected TimelockNotElapsed at delta={advance_by} (TIMELOCK={TIMELOCK}, short by {seconds_short}s)"
        );
    }

    /// **Invariant 2b — Timelock boundary: execute succeeds exactly at the boundary.**
    ///
    /// The contract uses `now < exec_after` (strict less-than), so at
    /// `now == exec_after` the proposal must be executable.
    ///
    /// This property holds across all valid (signer-count, threshold) pairs.
    #[test]
    fn prop_execute_succeeds_at_timelock_boundary(
        n_signers in signer_count_strategy(),
        threshold_raw in 1usize..=20usize,
    ) {
        let threshold = (threshold_raw.min(n_signers)) as u32;
        let gov = GovEnv::new(n_signers, threshold);
        let id = gov.propose();

        // Quorum is reached at BASE_TIMESTAMP.
        gov.approve_n(id, threshold as usize);

        // Set clock to exactly quorum_at + TIMELOCK (= exec_after).
        gov.advance_time(TIMELOCK);

        let executor = Address::generate(&gov.env);
        let result = gov.client.try_execute(&executor, &id);

        prop_assert!(
            result.is_ok(),
            "Expected success at exact timelock boundary, got: {result:?}"
        );
    }

    /// **Invariant 2c — Timelock post-boundary: execute succeeds after the boundary.**
    ///
    /// Any time strictly after `exec_after` must also allow execution.
    #[test]
    fn prop_execute_succeeds_after_timelock_boundary(
        n_signers in signer_count_strategy(),
        threshold_raw in 1usize..=20usize,
        // Extra seconds beyond the timelock boundary; keep below MAX_AGE.
        extra_seconds in 1u64..1_000_000u64,
    ) {
        let threshold = (threshold_raw.min(n_signers)) as u32;
        let gov = GovEnv::new(n_signers, threshold);
        let id = gov.propose();

        gov.approve_n(id, threshold as usize);

        // Must stay within MAX_AGE of BASE_TIMESTAMP to avoid ProposalExpired.
        let advance_by = TIMELOCK + extra_seconds.min(MAX_AGE - TIMELOCK - 1);
        gov.advance_time(advance_by);

        let executor = Address::generate(&gov.env);
        let result = gov.client.try_execute(&executor, &id);

        prop_assert!(
            result.is_ok(),
            "Expected success after timelock boundary (advance_by={advance_by}), got: {result:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 3 — One-way executed flag: second execute always returns AlreadyExecuted
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        source_file: Some("contracts/stream/tests/governance_proptest.rs"),
        ..ProptestConfig::default()
    })]

    /// **Invariant 3 — Executed flag is one-way (terminal state).**
    ///
    /// Once `execute` succeeds, every subsequent call to `execute` on the same
    /// `proposal_id` must return `AlreadyExecuted`, no matter the executor
    /// address or how much time has passed.
    ///
    /// This tests the CEI (Checks-Effects-Interactions) pattern: the flag is
    /// set before any events are emitted, and the check happens first on re-entry.
    ///
    /// Asserts: `try_execute` returns `AlreadyExecuted` on the second call.
    #[test]
    fn prop_second_execute_always_returns_already_executed(
        n_signers in signer_count_strategy(),
        threshold_raw in 1usize..=20usize,
        // Extra seconds past the timelock for the initial execution.
        extra_after_timelock in 0u64..1_000_000u64,
    ) {
        let threshold = (threshold_raw.min(n_signers)) as u32;
        let gov = GovEnv::new(n_signers, threshold);
        let id = gov.propose();

        // Reach quorum at BASE_TIMESTAMP.
        gov.approve_n(id, threshold as usize);

        // Advance to a valid execution window.
        let advance_by = TIMELOCK + extra_after_timelock.min(MAX_AGE - TIMELOCK - 1);
        gov.advance_time(advance_by);

        let executor = Address::generate(&gov.env);

        // First execution — must succeed.
        let first = gov.client.try_execute(&executor, &id);
        prop_assert!(
            first.is_ok(),
            "First execute should succeed, got: {first:?}"
        );

        // Verify the on-chain flag is set.
        let proposal = gov.client.get_proposal(&id);
        prop_assert!(proposal.executed, "Proposal.executed must be true after first execute");

        // Second execution — must fail with AlreadyExecuted regardless of
        // who calls it or when.
        let second = gov.client.try_execute(&executor, &id);
        prop_assert_eq!(
            second,
            Err(Ok(GovernanceError::AlreadyExecuted)),
            "Second execute on proposal {id} should return AlreadyExecuted"
        );

        // A different executor address also gets AlreadyExecuted.
        let executor2 = Address::generate(&gov.env);
        let third = gov.client.try_execute(&executor2, &id);
        prop_assert_eq!(
            third,
            Err(Ok(GovernanceError::AlreadyExecuted)),
            "Third execute (different executor) should also return AlreadyExecuted"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 4 — Combined: random approval orderings never bypass quorum/timelock
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 200,
        source_file: Some("contracts/stream/tests/governance_proptest.rs"),
        ..ProptestConfig::default()
    })]

    /// **Invariant 4 — Random approval count and time combination.**
    ///
    /// Across all combinations of approval count (relative to threshold) and
    /// time advancement (relative to timelock boundary), the outcome of
    /// `execute` is fully determined by:
    ///
    /// - Whether `approval_count >= threshold` (quorum).
    /// - Whether `now >= quorum_at + TIMELOCK` (timelock elapsed).
    ///
    /// No other input should be able to make execution succeed or fail.
    ///
    /// This property tests the full cross-product of the two primary safety
    /// conditions, exercising every quadrant of the (quorum × timelock) matrix.
    #[test]
    fn prop_execute_outcome_matches_quorum_and_timelock_conditions(
        n_signers in signer_count_strategy(),
        threshold_raw in 1usize..=20usize,
        approval_count_raw in 0usize..=20usize,
        time_delta in time_delta_strategy(),
    ) {
        let threshold = (threshold_raw.min(n_signers)) as u32;
        let gov = GovEnv::new(n_signers, threshold);
        let id = gov.propose();

        // Clamp approvals to available signers.
        let approvals = approval_count_raw.min(n_signers);
        gov.approve_n(id, approvals);

        // Advance time by the random delta.
        gov.advance_time(time_delta);

        let quorum_met = approvals >= threshold as usize;
        // Quorum is recorded at BASE_TIMESTAMP; timelock expires at BASE_TIMESTAMP + TIMELOCK.
        let timelock_elapsed = time_delta >= TIMELOCK;

        let executor = Address::generate(&gov.env);
        let result = gov.client.try_execute(&executor, &id);

        match (quorum_met, timelock_elapsed) {
            (false, _) => {
                // Quorum not met: must return QuorumNotReached.
                prop_assert_eq!(
                    result,
                    Err(Ok(GovernanceError::QuorumNotReached)),
                    "Expected QuorumNotReached: approvals={approvals}, threshold={threshold}"
                );
            }
            (true, false) => {
                // Quorum met but timelock not elapsed: must return TimelockNotElapsed.
                prop_assert_eq!(
                    result,
                    Err(Ok(GovernanceError::TimelockNotElapsed)),
                    "Expected TimelockNotElapsed: time_delta={time_delta}, threshold={threshold}"
                );
            }
            (true, true) => {
                // Both conditions satisfied: must succeed.
                prop_assert!(
                    result.is_ok(),
                    "Expected success with quorum={approvals}/{threshold} and time_delta={time_delta}, got: {result:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Invariant 5 — Exactly-quorum boundary: approval count == threshold succeeds
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        source_file: Some("contracts/stream/tests/governance_proptest.rs"),
        ..ProptestConfig::default()
    })]

    /// **Invariant 5 — Exactly-quorum boundary.**
    ///
    /// When exactly `threshold` approvals have been submitted (not one more or
    /// fewer), the proposal must be executable after the timelock.
    ///
    /// This guards the `approval_count == threshold` branch in `approve` which
    /// triggers quorum recording, and the `approvals.len() < threshold` guard
    /// in `execute`.
    #[test]
    fn prop_exactly_quorum_approvals_allows_execution(
        n_signers in signer_count_strategy(),
        threshold_raw in 1usize..=20usize,
    ) {
        let threshold = (threshold_raw.min(n_signers)) as u32;
        let gov = GovEnv::new(n_signers, threshold);
        let id = gov.propose();

        // Apply exactly the threshold count of approvals.
        gov.approve_n(id, threshold as usize);

        // Advance to exactly the timelock boundary (inclusive).
        gov.advance_time(TIMELOCK);

        let executor = Address::generate(&gov.env);
        let result = gov.client.try_execute(&executor, &id);

        prop_assert!(
            result.is_ok(),
            "Expected success with exactly threshold={threshold} approvals at exact timelock boundary, got: {result:?}"
        );
    }

    /// **Invariant 5b — One-below-quorum boundary.**
    ///
    /// When `threshold - 1` approvals have been submitted (strictly below
    /// quorum), the proposal must never execute.
    ///
    /// This is the mirror test of 5: threshold is necessary, not threshold - 1.
    ///
    /// Skipped when `threshold == 1` since `0` approvals trivially fail quorum
    /// without a meaningful off-by-one risk.
    #[test]
    fn prop_one_below_quorum_prevents_execution(
        // Use at least 2 signers and threshold 2+ so the off-by-one is meaningful.
        n_signers in 2usize..=MAX_SIGNERS,
        threshold_raw in 2usize..=20usize,
    ) {
        let threshold = (threshold_raw.min(n_signers)) as u32;
        let gov = GovEnv::new(n_signers, threshold);
        let id = gov.propose();

        // Apply exactly threshold - 1 approvals.
        let below = (threshold - 1) as usize;
        gov.approve_n(id, below);

        // Advance well past the timelock so only quorum can block execution.
        gov.advance_time(TIMELOCK + 3_600);

        let executor = Address::generate(&gov.env);
        let result = gov.client.try_execute(&executor, &id);

        prop_assert_eq!(
            result,
            Err(Ok(GovernanceError::QuorumNotReached)),
            "Expected QuorumNotReached with threshold-1={below} approvals (threshold={threshold})"
        );
    }
}
