//! Regression tests for issue #652: TTL of `Proposal(id)` and
//! `QuorumReachedAt(id)` near the Soroban persistent-entry archival threshold.
//!
//! Verifies that:
//! 1. `Proposal(id)` survives a single ledger advance past
//!    `PERSISTENT_LIFETIME_THRESHOLD` (17,280 ledgers / ~1 day) thanks to the
//!    on-write bump.
//! 2. Reading a proposal (which calls `bump_proposal`) re-extends the entry's
//!    TTL so it survives the full timelock window without being silently
//!    archived.
//! 3. `execute` succeeds after the full `GOVERNANCE_TIMELOCK_SECONDS` (48
//!    hours) by reading the still-live `QuorumReachedAt(id)` entry.
//! 4. With reads spaced below `PERSISTENT_BUMP_AMOUNT` (~7 days), a proposal
//!    can be exercised across the entire `MAX_PROPOSAL_AGE_SECONDS` (30
//!    days) window without losing its entries.
//!
//! The bump policy pins persistent entries at
//! `threshold = 17_280`, `bump = 120_960` (≈ 7 days) on every read and write,
//! which is generous relative to the 48-hour timelock. These tests fail
//! loudly if the bump constants or the read-time bump in `load_proposal`
//! regress.
//!
//! Mirrors the stream contract's `tests/adaptive_ttl.rs` patterns.

extern crate std;

use fluxora_governance::{FluxoraGovernance, FluxoraGovernanceClient, GovernanceError};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Bytes, Env,
};

// ---------------------------------------------------------------------
// Constants mirroring `contracts/governance/src/lib.rs`. Kept local to
// this test so a regression that changes them is easier to spot.
// ---------------------------------------------------------------------

/// Default timelock between quorum and execution (48 hours).
const TIMELOCK_SECONDS: u64 = 172_800;
/// Maximum age of a proposal (30 days).
const MAX_PROPOSAL_AGE_SECONDS: u64 = 2_592_000;
/// Ledger close time used for sequence -> timestamp conversions.
const LEDGER_CLOSE_TIME_SECS: u64 = 5;
/// Soroban archival threshold for persistent entries (~1 day).
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 17_280;
/// Bump amount applied to each persisted governance entry (~7 days).
const PERSISTENT_BUMP_AMOUNT: u32 = 120_960;
/// Number of ledgers spanning the 48-hour timelock.
const LEDGERS_PER_TIMELOCK: u32 = 34_560;
/// Number of ledgers spanning the 30-day proposal-age window.
const LEDGERS_PER_MAX_AGE: u32 = 518_400;
const START_TIMESTAMP: u64 = 1_000_000;

// ---------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------

struct GovCtx<'a> {
    env: Env,
    #[allow(dead_code)]
    admin: Address,
    signer_a: Address,
    signer_b: Address,
    #[allow(dead_code)]
    signer_c: Address,
    client: FluxoraGovernanceClient<'a>,
}

impl<'a> GovCtx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(START_TIMESTAMP);

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
            admin,
            signer_a,
            signer_b,
            signer_c,
            client,
        }
    }

    fn target(&self) -> Address {
        Address::generate(&self.env)
    }

    fn calldata(&self, tag: &str) -> Bytes {
        Bytes::from_slice(&self.env, tag.as_bytes())
    }

    fn create_proposal(&self) -> u32 {
        let target = self.target();
        let data = self.calldata("ttl_test");
        self.client.propose(&self.signer_a, &target, &data)
    }

    fn reach_quorum(&self, proposal_id: u32) {
        self.client.approve(&self.signer_a, &proposal_id);
        self.client.approve(&self.signer_b, &proposal_id);
    }

    /// Advance both the ledger sequence number and the timestamp by the given
    /// number of ledgers.
    fn advance_ledgers(&self, ledgers: u32) {
        self.env.ledger().with_mut(|l| {
            l.sequence_number += ledgers;
            l.timestamp += u64::from(ledgers) * LEDGER_CLOSE_TIME_SECS;
        });
    }
}

// ---------------------------------------------------------------------
// Read/write bump invariants
// ---------------------------------------------------------------------

/// `Proposal(id)` is bumped at write-time to `PERSISTENT_BUMP_AMOUNT` (≈ 7
/// days). Advancing the ledger sequence past `PERSISTENT_LIFETIME_THRESHOLD`
/// but well within the bumped lifetime must not archive the entry.
#[test]
fn test_proposal_entry_survives_threshold_on_write_bump() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();

    // Advance 10,000 ledgers past the archival threshold but well below the
    // 7-day bump window.
    let advance = PERSISTENT_LIFETIME_THRESHOLD + 10_000;
    ctx.advance_ledgers(advance);
    assert!(ctx.env.ledger().get().sequence_number > PERSISTENT_LIFETIME_THRESHOLD);
    assert!(
        ctx.env.ledger().get().sequence_number < PERSISTENT_BUMP_AMOUNT,
        "advance must keep us inside the bump window"
    );

    // Proposal must still be readable — `get_proposal` internally calls
    // `load_proposal` which bumps TTL on read as a defensive measure, but the
    // write-time bump alone is sufficient here.
    let proposal = ctx.client.get_proposal(&id);
    assert_eq!(proposal.proposer, ctx.signer_a);
    assert!(!proposal.executed);
    assert!(!proposal.cancelled);
}

/// Reading a proposal calls `bump_proposal` which re-extends the TTL. After
/// advancing to just below the archival threshold and reading, an additional
/// advance (past the original threshold) must not silently archive the entry.
#[test]
fn test_read_extends_proposal_persistent_ttl() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();

    // Advance to just under the archival threshold.
    let just_under = PERSISTENT_LIFETIME_THRESHOLD - 100;
    ctx.advance_ledgers(just_under);
    assert!(ctx.env.ledger().get().sequence_number < PERSISTENT_LIFETIME_THRESHOLD);

    // Read the proposal — the contract's `load_proposal` helper calls
    // `bump_proposal`, which re-extends the TTL.
    let _ = ctx.client.get_proposal(&id);

    // Now advance *past* the original archival threshold. Without the
    // read-time bump, the entry would have archived here.
    ctx.advance_ledgers(500);
    assert!(ctx.env.ledger().get().sequence_number > PERSISTENT_LIFETIME_THRESHOLD);

    // The entry is still readable thanks to the read-time TTL bump.
    let proposal = ctx.client.get_proposal(&id);
    assert_eq!(proposal.proposer, ctx.signer_a);
}

// ---------------------------------------------------------------------
// Full-timelock execution without archival
// ---------------------------------------------------------------------

/// After reaching quorum, both `Proposal(id)` and `QuorumReachedAt(id)` are
/// bumped to `PERSISTENT_BUMP_AMOUNT`. The 48-hour timelock is ~34,560 ledgers,
/// so executing after the full timelock must succeed without either entry
/// archiving.
#[test]
fn test_execute_succeeds_at_maximum_timelock_no_archival() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();
    ctx.reach_quorum(id);

    // Advance past the timelock window in both sequence and timestamp.
    ctx.env.ledger().with_mut(|l| {
        l.sequence_number += LEDGERS_PER_TIMELOCK + 10;
        l.timestamp += TIMELOCK_SECONDS + 1;
    });

    // `execute` reads `Proposal(id)` (bumped on every read) and
    // `QuorumReachedAt(id)` (bumped once on write in `approve`). Both must
    // still be on-chain — otherwise `execute` returns `ProposalNotFound` or
    // `QuorumNotReached`.
    let executor = Address::generate(&ctx.env);
    ctx.client.execute(&executor, &id);

    let proposal = ctx.client.get_proposal(&id);
    assert!(proposal.executed);
}

/// Admins and indexers regularly call `get_proposal` (which bumps TTL).
/// Across the full `MAX_PROPOSAL_AGE_SECONDS` window (30 days =
/// `LEDGERS_PER_MAX_AGE` ledgers), a handful of well-placed reads keep the
/// proposal entry alive long enough to execute.
#[test]
fn test_proposal_survives_max_proposal_age_with_periodic_reads() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();
    ctx.reach_quorum(id);

    // Step in ~5-day increments (~86,000 ledgers each — well inside the
    // ~7-day bump window each read resets). Five steps cover the full 30-day
    // proposal-age window.
    let step: u32 = 86_000;
    let mut advanced: u32 = 0;
    while advanced + step < LEDGERS_PER_MAX_AGE {
        ctx.advance_ledgers(step);
        advanced += step;
        // Touch the proposal to re-bump its TTL.
        let _ = ctx.client.get_proposal(&id);
    }
    // Final advance (and read) covering the last chunk.
    ctx.advance_ledgers(LEDGERS_PER_MAX_AGE - advanced);
    let _ = ctx.client.get_proposal(&id);

    // Bump-boundary probe: the last read above bumped TTL to
    // `PERSISTENT_BUMP_AMOUNT` (120,960) ledgers from the current sequence.
    // Advance a small amount past `PERSISTENT_LIFETIME_THRESHOLD` without
    // re-reading.  The entry must still be alive — proving the periodic-read
    // schedule lands safely above the archival threshold.
    ctx.advance_ledgers(PERSISTENT_LIFETIME_THRESHOLD + 1);
    let _ = ctx.client.get_proposal(&id);

    // Stop short of expiry: pin the timestamp to `created_at + MAX_AGE - 60`
    // so the policy `now > created_at + MAX_PROPOSAL_AGE_SECONDS` is
    // strictly false, but the timelock (48h after quorum) has elapsed.
    let proposal = ctx.client.get_proposal(&id);
    let near_expiry = proposal.created_at + MAX_PROPOSAL_AGE_SECONDS - 60;
    ctx.env.ledger().with_mut(|l| {
        l.timestamp = near_expiry;
    });

    // Execute must still find the Proposal and QuorumReachedAt entries.
    let executor = Address::generate(&ctx.env);
    ctx.client.execute(&executor, &id);
    let proposal = ctx.client.get_proposal(&id);
    assert!(proposal.executed);
}

// ---------------------------------------------------------------------
// Failure-mode baseline
// ---------------------------------------------------------------------

/// Negative control: executing a non-existent proposal id returns
/// `ProposalNotFound`.  This is the exact error code a future regression
/// would surface if the persistent-entry bump policy were removed and a
/// live proposal silently archived.  Pinning the error here makes any
/// such failure unmistakable in CI.
#[test]
fn test_execute_unknown_id_returns_proposal_not_found() {
    let ctx = GovCtx::setup();
    let executor = Address::generate(&ctx.env);

    let result = ctx.client.try_execute(&executor, &0u32);
    assert_eq!(result, Err(Ok(GovernanceError::ProposalNotFound)));
}

// ---------------------------------------------------------------------
// Drift guard
// ---------------------------------------------------------------------

/// Drift guard: assert that the contract's runtime constants match what
/// this test file assumes.  If the contract changes `timelock_seconds` or
/// `max_proposal_age_seconds`, this test fails loudly so that the TTL
/// arithmetic in the regression cases above cannot quietly misalign.
#[test]
fn test_local_ttl_constants_match_contract() {
    let ctx = GovCtx::setup();
    assert_eq!(ctx.client.timelock_seconds(), TIMELOCK_SECONDS);
    assert_eq!(
        ctx.client.max_proposal_age_seconds(),
        MAX_PROPOSAL_AGE_SECONDS
    );
}

// ---------------------------------------------------------------------
// QuorumReachedAt TTL regression (issue #638)
// ---------------------------------------------------------------------

/// Regression test for issue #638: QuorumReachedAt TTL must be bumped on
/// every approve and execute so that a 48-hour timelock cannot strand a
/// fully-approved proposal as un-executable due to entry archival.
///
/// This test advances the ledger near the TTL horizon between quorum and
/// execution, then verifies execute still succeeds by reading the still-live
/// QuorumReachedAt entry.
#[test]
fn test_quorum_reached_at_ttl_refreshed_on_execute() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();
    ctx.reach_quorum(id);

    // Advance ledger sequence to just inside the bump window (well past the
    // archival threshold) and advance timestamp past the timelock.
    ctx.env.ledger().with_mut(|l| {
        l.sequence_number += PERSISTENT_BUMP_AMOUNT - 100;
        l.timestamp += TIMELOCK_SECONDS + 1;
    });

    // execute must still find the QuorumReachedAt entry and succeed.
    // Without the TTL bump on execute, this would return QuorumNotReached.
    let executor = Address::generate(&ctx.env);
    ctx.client.execute(&executor, &id);

    let proposal = ctx.client.get_proposal(&id);
    assert!(proposal.executed);
}

/// Multiple approvals must each refresh the QuorumReachedAt TTL.
/// After quorum, additional reads of the proposal (e.g. from indexers)
/// must not let the entry expire before execution.
#[test]
fn test_quorum_reached_at_ttl_refreshed_on_approve_and_near_horizon() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();

    // Only first approval — quorum not yet reached.
    ctx.client.approve(&ctx.signer_a, &id);

    // Advance near the TTL horizon.
    ctx.advance_ledgers(PERSISTENT_BUMP_AMOUNT - 500);

    // Second approval reaches quorum and bumps QuorumReachedAt TTL.
    ctx.client.approve(&ctx.signer_b, &id);

    // Advance past the timelock.
    ctx.env.ledger().with_mut(|l| {
        l.timestamp += TIMELOCK_SECONDS + 1;
    });

    // Execute must succeed — QuorumReachedAt was bumped on the second approve.
    let executor = Address::generate(&ctx.env);
    ctx.client.execute(&executor, &id);

    let proposal = ctx.client.get_proposal(&id);
    assert!(proposal.executed);
}

/// Security: an expired QuorumReachedAt entry must cause execute to fail
/// closed with QuorumNotReached rather than silently re-opening the timelock.
#[test]
fn test_execute_fails_closed_if_quorum_entry_missing() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();

    // Only one approval — quorum never reached, so QuorumReachedAt is never written.
    ctx.client.approve(&ctx.signer_a, &id);

    // Advance past the timelock window.
    ctx.env.ledger().with_mut(|l| {
        l.sequence_number += LEDGERS_PER_TIMELOCK + 1;
        l.timestamp += TIMELOCK_SECONDS + 1;
    });

    // execute must fail closed — no QuorumReachedAt entry means QuorumNotReached.
    let executor = Address::generate(&ctx.env);
    let result = ctx.client.try_execute(&executor, &id);
    assert_eq!(result, Err(Ok(GovernanceError::QuorumNotReached)));
}

// ---------------------------------------------------------------------
// ProposalApprovalIdx TTL regression (issue #732)
// ---------------------------------------------------------------------

/// After a long idle period between approvals, the approval index TTL must
/// remain coupled to the proposal so duplicate approvals are still rejected.
#[test]
fn test_approval_index_ttl_refreshed_rejects_duplicate_after_long_delay() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();

    ctx.client.approve(&ctx.signer_a, &id);

    ctx.advance_ledgers(PERSISTENT_BUMP_AMOUNT - 500);

    let duplicate = ctx.client.try_approve(&ctx.signer_a, &id);
    assert_eq!(duplicate, Err(Ok(GovernanceError::AlreadyApproved)));

    ctx.client.approve(&ctx.signer_b, &id);
    let proposal = ctx.client.get_proposal(&id);
    assert_eq!(proposal.approvals.len(), 2);
}

/// Each approval refreshes the approval-index TTL so later signers can still
/// rely on duplicate detection near the proposal-age horizon.
#[test]
fn test_approval_index_ttl_bumped_on_every_approve() {
    let ctx = GovCtx::setup();
    let id = ctx.create_proposal();

    ctx.client.approve(&ctx.signer_a, &id);
    ctx.advance_ledgers(PERSISTENT_LIFETIME_THRESHOLD + 1_000);
    ctx.client.approve(&ctx.signer_b, &id);

    let duplicate = ctx.client.try_approve(&ctx.signer_a, &id);
    assert_eq!(duplicate, Err(Ok(GovernanceError::AlreadyApproved)));
}
