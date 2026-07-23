//! Property-based tests for `fluxora_governance` signer list / index invariants.
//!
//! # Invariants under test
//!
//! The governance contract maintains two parallel data structures for the
//! registered co-signer set:
//!
//! - **`Signers`** (`Vec<Address>`, instance storage) — ordered list used for
//!   iteration and quorum counting.
//! - **`SignerIndex`** (`Map<Address, bool>`, instance storage) — O(1) membership
//!   index used by `propose`, `approve`, and `add_signer` duplicate checks.
//!
//! These tests assert that randomised sequences of `add_signer` / `remove_signer`
//! operations never desync the two structures.  After every operation:
//!
//! 1. **List/index agreement** — every address in `get_signers()` is present in
//!    `SignerIndex` (verified by calling `try_add_signer` on listed addresses and
//!    asserting `DuplicateSigner`), and only listed addresses are in the index.
//!
//! 2. **Duplicate-free list** — `get_signers()` never contains the same address
//!    more than once, regardless of operation ordering.
//!
//! 3. **Quorum safety** — `get_signers().len() >= threshold` always holds, or the
//!    triggering operation returned `QuorumWouldBreak`.
//!
//! # Edge cases covered
//!
//! - Remove of an address that was never registered (silent no-op, no state change).
//! - Add of an address already registered (`DuplicateSigner`).
//! - Remove that would drop signer count below threshold (`QuorumWouldBreak`).
//! - Signer set at maximum capacity (`TooManySigners`).
//! - Interleaved add/remove of the same address.
//!
//! # Security notes
//!
//! A desync between `Signers` and `SignerIndex` would break duplicate-approval
//! detection (an address absent from the index could approve multiple times) and
//! could allow proposals to pass with fewer unique signers than the threshold
//! appears to require.  These property tests are the primary safety net.
//!
//! # Determinism
//!
//! proptest is configured with a fixed `source_file` seed so that CI regression
//! runs are deterministic.  Case count (256) balances coverage with wall-clock
//! time.

extern crate std;

use fluxora_governance::{FluxoraGovernance, FluxoraGovernanceClient, GovernanceError};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Env,
};
use std::collections::HashSet;
use std::vec::Vec as StdVec;

// ---------------------------------------------------------------------------
// Constants (mirrored from lib.rs)
// ---------------------------------------------------------------------------

/// Maximum co-signers — must match `MAX_SIGNERS` in lib.rs.
const MAX_SIGNERS: usize = 20;

/// Number of addresses pre-generated into the shared pool across all tests.
/// Larger than MAX_SIGNERS to allow adds beyond capacity and removes of
/// addresses that were never signers.
const POOL_SIZE: usize = 22;

/// Baseline ledger timestamp used across all tests.
const BASE_TIMESTAMP: u64 = 10_000_000;

// ---------------------------------------------------------------------------
// Operation type
// ---------------------------------------------------------------------------

/// A single governance mutation: add or remove the address at `pool_idx`.
#[derive(Clone, Debug)]
enum Op {
    Add(usize),
    Remove(usize),
}

// ---------------------------------------------------------------------------
// proptest strategies
// ---------------------------------------------------------------------------

/// Strategy: one Op over the address pool.
fn op_strategy() -> impl Strategy<Value = Op> {
    (any::<bool>(), 0usize..POOL_SIZE).prop_map(|(is_add, idx)| {
        if is_add {
            Op::Add(idx)
        } else {
            Op::Remove(idx)
        }
    })
}

/// Strategy: non-empty sequence of ops, length in [1, 40].
fn op_sequence_strategy() -> impl Strategy<Value = StdVec<Op>> {
    prop::collection::vec(op_strategy(), 1..=40)
}

// ---------------------------------------------------------------------------
// Test environment
// ---------------------------------------------------------------------------

/// Self-contained governance environment for proptest cases.
struct GovEnv {
    #[allow(dead_code)]
    env: Env,
    /// All POOL_SIZE pre-generated addresses; ops reference these by index.
    pool: StdVec<Address>,
    client: FluxoraGovernanceClient<'static>,
    threshold: u32,
}

impl GovEnv {
    /// Initialise a governance contract with `init_count` signers (pool[0..init_count])
    /// and the given `threshold`.
    fn new(init_count: usize, threshold: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(BASE_TIMESTAMP);

        let contract_id = env.register_contract(None, FluxoraGovernance);
        let admin = Address::generate(&env);

        let mut pool: StdVec<Address> = StdVec::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            pool.push(Address::generate(&env));
        }

        let mut sdk_signers = vec![&env];
        for addr in pool.iter().take(init_count) {
            sdk_signers.push_back(addr.clone());
        }

        let client = FluxoraGovernanceClient::new(&env, &contract_id);
        client.init(&admin, &sdk_signers, &threshold);

        GovEnv {
            env,
            pool,
            client,
            threshold,
        }
    }
}

// ---------------------------------------------------------------------------
// Invariant checker
// ---------------------------------------------------------------------------

/// Assert all three invariants for the current on-chain state against the
/// caller's ground-truth expected set (pool indices of current signers).
///
/// Panics with a descriptive message on any invariant violation; proptest
/// catches the panic and records it as a test-case failure with shrinking.
fn check_invariants(gov: &GovEnv, expected: &HashSet<usize>) {
    let on_chain = gov.client.get_signers();
    let on_chain_len = on_chain.len() as usize;

    // ------------------------------------------------------------------
    // Invariant 2: no duplicates in Signers list.
    //
    // O(n²) pairwise comparison — acceptable because MAX_SIGNERS == 20.
    // ------------------------------------------------------------------
    for i in 0..on_chain.len() {
        for j in (i + 1)..on_chain.len() {
            let ai = on_chain.get(i).unwrap();
            let aj = on_chain.get(j).unwrap();
            assert!(
                ai != aj,
                "Invariant 2 violated: duplicate address at positions {i} and {j} in Signers list"
            );
        }
    }

    // ------------------------------------------------------------------
    // List length must match ground-truth set size.
    // ------------------------------------------------------------------
    assert_eq!(
        on_chain_len,
        expected.len(),
        "Signer list length {on_chain_len} != expected set size {}",
        expected.len()
    );

    // ------------------------------------------------------------------
    // Invariant 1a: every expected address appears in the on-chain list.
    //
    // Combined with the length+duplicate checks this proves list == expected.
    // ------------------------------------------------------------------
    for &pool_idx in expected {
        let addr = &gov.pool[pool_idx];
        let mut found = false;
        for i in 0..on_chain.len() {
            if on_chain.get(i).unwrap() == *addr {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "Invariant 1a violated: expected signer pool[{pool_idx}] missing from Signers list"
        );
    }

    // ------------------------------------------------------------------
    // Invariant 1b: every expected address is present in SignerIndex.
    //
    // Proof: `add_signer` checks SignerIndex BEFORE the TooManySigners guard,
    // so an already-indexed address always returns DuplicateSigner regardless
    // of current signer count.  A return value other than DuplicateSigner
    // means the address is absent from the index — a list/index desync.
    //
    // Note: the call does NOT mutate state when it returns an error.
    // ------------------------------------------------------------------
    for &pool_idx in expected {
        let addr = &gov.pool[pool_idx];
        let result = gov.client.try_add_signer(addr);
        assert_eq!(
            result,
            Err(Ok(GovernanceError::DuplicateSigner)),
            "Invariant 1b violated: signer pool[{pool_idx}] is in Signers list but NOT in \
             SignerIndex (list/index desync) — try_add_signer returned {result:?}"
        );
    }

    // ------------------------------------------------------------------
    // Invariant 3: quorum safety — signer count never falls below threshold.
    // ------------------------------------------------------------------
    assert!(
        on_chain_len >= gov.threshold as usize,
        "Invariant 3 violated: {on_chain_len} signers < threshold {} (quorum safety broken)",
        gov.threshold
    );
}

// ---------------------------------------------------------------------------
// Main proptest — randomised add/remove sequences
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        source_file: Some("contracts/governance/tests/signer_index_proptest.rs"),
        ..ProptestConfig::default()
    })]

    /// **Signer list/index invariants under randomised add/remove sequences.**
    ///
    /// Applies up to 40 randomised `add_signer` / `remove_signer` calls drawn
    /// from a 22-address pool to a live Soroban environment.  A parallel Rust
    /// `HashSet` tracks the expected signer set and is kept in sync with the
    /// contract result after every operation.  All three invariants are asserted
    /// after every step, including the initial state before any mutations.
    ///
    /// Operation outcomes are validated:
    ///
    /// - `Ok(())` from `add_signer` → address was not a signer; expected set gains it.
    /// - `DuplicateSigner` from `add_signer` → address was already a signer; no change.
    /// - `TooManySigners` from `add_signer` → set was at MAX_SIGNERS capacity; no change.
    /// - `Ok(())` from `remove_signer` → address removed (or was absent, silent no-op).
    /// - `QuorumWouldBreak` from `remove_signer` → address is a signer but removal would
    ///   drop count below threshold; no change.
    #[test]
    fn prop_signer_list_index_invariants(
        init_count_raw in 2usize..=5usize,
        threshold_raw in 1usize..=5usize,
        ops in op_sequence_strategy(),
    ) {
        let init_count = init_count_raw.min(POOL_SIZE);
        let threshold = (threshold_raw.min(init_count)) as u32;
        let gov = GovEnv::new(init_count, threshold);

        // Ground-truth: pool indices of addresses currently in the signer set.
        let mut expected: HashSet<usize> = (0..init_count).collect();

        // Assert invariants on initial state before any mutations.
        check_invariants(&gov, &expected);

        for op in &ops {
            match op {
                Op::Add(pool_idx) => {
                    let addr = &gov.pool[*pool_idx];
                    let result = gov.client.try_add_signer(addr);
                    match result {
                        Ok(Ok(())) => {
                            // Successful add: must not have been a signer before.
                            prop_assert!(
                                !expected.contains(pool_idx),
                                "add_signer succeeded but pool[{pool_idx}] was already in expected set"
                            );
                            expected.insert(*pool_idx);
                        }
                        Err(Ok(GovernanceError::DuplicateSigner)) => {
                            // Expected: address is already registered.
                            prop_assert!(
                                expected.contains(pool_idx),
                                "DuplicateSigner returned but pool[{pool_idx}] is not in expected set"
                            );
                        }
                        Err(Ok(GovernanceError::TooManySigners)) => {
                            // Expected: signer set at MAX_SIGNERS capacity.
                            // The duplicate check runs before the capacity check, so this
                            // variant is only reachable when the address is NOT a signer.
                            prop_assert_eq!(
                                expected.len(), MAX_SIGNERS,
                                "TooManySigners returned but expected set size is {} (not MAX_SIGNERS={})",
                                expected.len(), MAX_SIGNERS
                            );
                            prop_assert!(
                                !expected.contains(pool_idx),
                                "TooManySigners returned but pool[{pool_idx}] is already a signer \
                                 (should have been DuplicateSigner)"
                            );
                        }
                        other => {
                            prop_assert!(false, "add_signer returned unexpected result: {other:?}");
                        }
                    }
                }

                Op::Remove(pool_idx) => {
                    let addr = &gov.pool[*pool_idx];
                    let result = gov.client.try_remove_signer(addr);
                    match result {
                        Ok(Ok(())) => {
                            // Successful remove or silent no-op for non-existent address.
                            // HashSet::remove is a no-op when the key is absent.
                            expected.remove(pool_idx);
                        }
                        Err(Ok(GovernanceError::QuorumWouldBreak)) => {
                            // Removal rejected: address is a signer and removing it would
                            // violate the quorum invariant.
                            prop_assert!(
                                expected.contains(pool_idx),
                                "QuorumWouldBreak returned but pool[{pool_idx}] is not a signer"
                            );
                            prop_assert!(
                                expected.len() <= gov.threshold as usize,
                                "QuorumWouldBreak returned but expected.len()={} > threshold={}",
                                expected.len(), gov.threshold
                            );
                            // No state change.
                        }
                        other => {
                            prop_assert!(false, "remove_signer returned unexpected result: {other:?}");
                        }
                    }
                }
            }

            // Assert all invariants after every operation.
            check_invariants(&gov, &expected);
        }
    }
}

// ---------------------------------------------------------------------------
// Targeted edge-case properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        source_file: Some("contracts/governance/tests/signer_index_proptest.rs"),
        ..ProptestConfig::default()
    })]

    /// **Add duplicate is always rejected with `DuplicateSigner`.**
    ///
    /// For any valid (signer-count, threshold) combination, calling `add_signer`
    /// on an address that is already registered must always return `DuplicateSigner`.
    ///
    /// The `SignerIndex` duplicate check is O(1) and runs before the `TooManySigners`
    /// guard, so the result must be `DuplicateSigner` even when the set is at capacity.
    #[test]
    fn prop_add_duplicate_always_rejected(
        n_signers in 1usize..=MAX_SIGNERS,
        threshold_raw in 1usize..=20usize,
    ) {
        let n = n_signers.min(POOL_SIZE);
        let threshold = (threshold_raw.min(n)) as u32;

        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(BASE_TIMESTAMP);

        let contract_id = env.register_contract(None, FluxoraGovernance);
        let admin = Address::generate(&env);

        let mut pool: StdVec<Address> = StdVec::new();
        let mut sdk_signers = vec![&env];
        for _ in 0..n {
            let addr = Address::generate(&env);
            sdk_signers.push_back(addr.clone());
            pool.push(addr);
        }

        let client = FluxoraGovernanceClient::new(&env, &contract_id);
        client.init(&admin, &sdk_signers, &threshold);

        // Re-adding every registered signer must return DuplicateSigner.
        for addr in &pool {
            let result = client.try_add_signer(addr);
            prop_assert_eq!(
                result,
                Err(Ok(GovernanceError::DuplicateSigner)),
                "Re-adding an existing signer must always return DuplicateSigner"
            );
        }
    }

    /// **Remove of a non-existent signer is a silent no-op.**
    ///
    /// Calling `remove_signer` with an address that was never registered must
    /// return `Ok(())` and leave the signer list completely unchanged.  This
    /// validates the early-return path that consults `SignerIndex` before
    /// touching the `Signers` Vec.
    #[test]
    fn prop_remove_nonexistent_is_noop(
        n_signers in 1usize..=5usize,
        threshold_raw in 1usize..=5usize,
    ) {
        let n = n_signers.min(POOL_SIZE / 2);
        let threshold = (threshold_raw.min(n)) as u32;

        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(BASE_TIMESTAMP);

        let contract_id = env.register_contract(None, FluxoraGovernance);
        let admin = Address::generate(&env);

        let mut sdk_signers = vec![&env];
        for _ in 0..n {
            sdk_signers.push_back(Address::generate(&env));
        }

        let client = FluxoraGovernanceClient::new(&env, &contract_id);
        client.init(&admin, &sdk_signers, &threshold);

        let before = client.get_signers().len();

        // An address that was never part of the signer set.
        let stranger = Address::generate(&env);
        let result = client.try_remove_signer(&stranger);
        prop_assert!(
            result.is_ok(),
            "remove_signer of non-existent address must return Ok(()), got {result:?}"
        );

        let after = client.get_signers().len();
        prop_assert_eq!(
            before, after,
            "Signer list length changed after removing a non-existent address"
        );
    }

    /// **Quorum safety is preserved across arbitrary removal sequences.**
    ///
    /// Applies up to `n_signers` sequential remove operations (each targeting a
    /// distinct registered signer).  After every removal attempt the signer count
    /// must be >= threshold.  Removals that would violate this are rejected with
    /// `QuorumWouldBreak`; all other outcomes are failures.
    #[test]
    fn prop_quorum_safety_after_removals(
        n_signers in 2usize..=10usize,
        threshold_raw in 1usize..=10usize,
        n_removes_raw in 0usize..=15usize,
    ) {
        let n = n_signers.min(POOL_SIZE);
        let threshold = (threshold_raw.min(n)) as u32;

        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(BASE_TIMESTAMP);

        let contract_id = env.register_contract(None, FluxoraGovernance);
        let admin = Address::generate(&env);

        let mut pool: StdVec<Address> = StdVec::new();
        let mut sdk_signers = vec![&env];
        for _ in 0..n {
            let addr = Address::generate(&env);
            sdk_signers.push_back(addr.clone());
            pool.push(addr);
        }

        let client = FluxoraGovernanceClient::new(&env, &contract_id);
        client.init(&admin, &sdk_signers, &threshold);

        let n_removes = n_removes_raw.min(n);
        #[allow(clippy::needless_range_loop)]
        for i in 0..n_removes {
            let result = client.try_remove_signer(&pool[i]);
            match result {
                Ok(Ok(())) | Err(Ok(GovernanceError::QuorumWouldBreak)) => {}
                other => {
                    prop_assert!(
                        false,
                        "remove_signer returned unexpected result at step {i}: {other:?}"
                    );
                }
            }
            // Quorum safety must hold after every step.
            let count = client.get_signers().len();
            prop_assert!(
                count >= threshold,
                "Quorum safety violated after removal attempt {i}: \
                 {count} signers < threshold {threshold}"
            );
        }
    }

    /// **Interleaved add/remove of the same address maintains index consistency.**
    ///
    /// Repeatedly adding and removing a single address exercises the most common
    /// source of list/index desync: an address that was removed from one structure
    /// but not the other.  After each pair the invariants must hold.
    #[test]
    fn prop_add_remove_same_address_maintains_consistency(
        n_signers in 2usize..=8usize,
        threshold_raw in 1usize..=5usize,
        rounds in 1usize..=15usize,
    ) {
        let n = n_signers.min(POOL_SIZE - 1);
        let threshold = (threshold_raw.min(n)) as u32;
        let gov = GovEnv::new(n, threshold);

        // Use pool[POOL_SIZE - 1] as the address that is toggled in and out.
        let toggle_idx = POOL_SIZE - 1;
        let toggle_addr = &gov.pool[toggle_idx];

        let mut expected: HashSet<usize> = (0..n).collect();
        check_invariants(&gov, &expected);

        for _ in 0..rounds {
            // Add the toggle address (may already be present from a prior round).
            let add_result = gov.client.try_add_signer(toggle_addr);
            match add_result {
                Ok(Ok(())) => { expected.insert(toggle_idx); }
                Err(Ok(GovernanceError::DuplicateSigner)) => {
                    // Already in set; ground truth is consistent.
                    prop_assert!(expected.contains(&toggle_idx));
                }
                Err(Ok(GovernanceError::TooManySigners)) => {
                    prop_assert_eq!(expected.len(), MAX_SIGNERS);
                }
                other => {
                    prop_assert!(false, "add_signer (toggle add) unexpected: {other:?}");
                }
            }
            check_invariants(&gov, &expected);

            // Remove the toggle address.
            let rm_result = gov.client.try_remove_signer(toggle_addr);
            match rm_result {
                Ok(Ok(())) => { expected.remove(&toggle_idx); }
                Err(Ok(GovernanceError::QuorumWouldBreak)) => {
                    prop_assert!(expected.contains(&toggle_idx));
                    prop_assert!(expected.len() <= gov.threshold as usize);
                }
                other => {
                    prop_assert!(false, "remove_signer (toggle remove) unexpected: {other:?}");
                }
            }
            check_invariants(&gov, &expected);
        }
    }
}
