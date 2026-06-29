// contracts/governance/tests/gas_regression.rs
#![cfg(test)]

use fluxora_governance::{FluxoraGovernance, FluxoraGovernanceClient, GovernanceError};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Bytes, Env, Vec,
};

// These come straight from the governance contract
const MAX_SIGNERS: u32 = 20;
const MAX_CALLDATA_BYTES: u32 = 4_096;
const GOVERNANCE_TIMELOCK_SECONDS: u64 = 172_800;
const MAX_PROPOSAL_AGE_SECONDS: u64 = 2_592_000;

// Gas budgets - we track these in docs/gas.md and CI will fail if we exceed them
// These numbers came from running the tests and adding ~25% headroom
const MAX_CPU_PROPOSE: u64 = 1_000_000;
const MAX_MEM_PROPOSE: u64 = 100_000;

const MAX_CPU_APPROVE_MAX_SIGNERS: u64 = 1_500_000;
const MAX_MEM_APPROVE_MAX_SIGNERS: u64 = 150_000;
const MAX_CPU_APPROVE_PER_SIGNER: u64 = 75_000;
const MAX_MEM_APPROVE_PER_SIGNER: u64 = 7_500;

const MAX_CPU_EXECUTE_MAX_CALLDATA: u64 = 10_000_000;
const MAX_MEM_EXECUTE_MAX_CALLDATA: u64 = 1_000_000;
const MAX_CPU_EXECUTE_PER_KB: u64 = 1_250_000;
const MAX_MEM_EXECUTE_PER_KB: u64 = 125_000;

/// Sets up a test environment with the given number of signers and threshold.
/// We use this across all the gas tests to keep things consistent.
struct GovGasCtx<'a> {
    env: Env,
    client: FluxoraGovernanceClient<'a>,
    signers: Vec<Address>,
    target: Address,
    admin: Address,
}

impl<'a> GovGasCtx<'a> {
    fn setup(signer_count: u32, threshold: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);

        let contract_id = env.register_contract(None, FluxoraGovernance);
        let client = FluxoraGovernanceClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let mut signers: Vec<Address> = Vec::new(&env);
        for _ in 0..signer_count {
            signers.push_back(Address::generate(&env));
        }

        client.init(&admin, &signers, &threshold);
        let target = Address::generate(&env);

        Self {
            env,
            client,
            signers,
            target,
            admin,
        }
    }

    fn calldata(&self, size: usize) -> Bytes {
        Bytes::from_slice(&self.env, &vec![0u8; size])
    }

    fn create_proposal(&self, calldata_size: usize) -> u32 {
        self.client.propose(
            &self.signers[0],
            &self.target,
            &self.calldata(calldata_size),
        )
    }

    fn approve_all(&self, proposal_id: u32) {
        for signer in self.signers.iter() {
            self.client.approve(&signer, &proposal_id);
        }
    }

    fn advance_time(&self, seconds: u64) {
        let current = self.env.ledger().timestamp();
        self.env.ledger().set_timestamp(current + seconds);
    }
}

/// Helper that resets the budget, runs the operation, and returns the CPU and memory costs.
/// Makes the test code cleaner since we do this a lot.
fn measure_budget<F>(ctx: &GovGasCtx, f: F) -> (u64, u64)
where
    F: FnOnce(&GovGasCtx),
{
    ctx.env.budget().reset_unlimited();
    f(ctx);
    (
        ctx.env.budget().cpu_instruction_cost(),
        ctx.env.budget().memory_bytes_cost(),
    )
}

// ============================================================================
// PROPOSE TESTS
// ============================================================================

#[test]
fn test_propose_budget_snapshots() {
    // We want to see how propose scales with both signer count and calldata size.
    // The calldata storage should be the main cost driver here.
    let signer_counts = [1, 5, 10, 15, 20];
    let calldata_sizes = [0, 1024, 2048, 3072, MAX_CALLDATA_BYTES as usize];

    for &signer_count in &signer_counts {
        for &calldata_size in &calldata_sizes {
            let threshold = if signer_count == 1 { 1 } else { 2 };
            let ctx = GovGasCtx::setup(signer_count, threshold);

            let (cpu, mem) = measure_budget(&ctx, |ctx| {
                ctx.client.propose(&ctx.signers[0], &ctx.target, &ctx.calldata(calldata_size));
            });

            // If these fail, we need to either optimize the contract or update the thresholds
            assert!(
                cpu <= MAX_CPU_PROPOSE,
                "Propose CPU exceeded threshold: {} > {} (signers={}, calldata={}B)",
                cpu, MAX_CPU_PROPOSE, signer_count, calldata_size
            );
            assert!(
                mem <= MAX_MEM_PROPOSE,
                "Propose memory exceeded threshold: {} > {} (signers={}, calldata={}B)",
                mem, MAX_MEM_PROPOSE, signer_count, calldata_size
            );

            // These get collected and used to update the documentation
            println!(
                "PROPOSE: signers={:2}, calldata={:4}B, CPU={:8}, MEM={:6}",
                signer_count, calldata_size, cpu, mem
            );
        }
    }
}

#[test]
fn test_propose_rejects_large_calldata() {
    // Make sure we can't create proposals with more than 4KB of calldata
    let ctx = GovGasCtx::setup(1, 1);
    let large_calldata = Bytes::from_slice(&ctx.env, &vec![0u8; (MAX_CALLDATA_BYTES + 1) as usize]);

    let result = ctx
        .client
        .try_propose(&ctx.signers[0], &ctx.target, &large_calldata);

    assert_eq!(result, Err(Ok(GovernanceError::CalldataTooLarge)));
}

// ============================================================================
// APPROVE TESTS
// ============================================================================

#[test]
fn test_approve_budget_snapshots() {
    // The big concern with approve is the linear scan through existing approvals.
    // We need to make sure this stays reasonable even at MAX_SIGNERS.
    let signer_counts = [1, 5, 10, 15, 20];
    let calldata_size = 1024; // Keep calldata fixed so we're only measuring approval cost

    let mut previous_cpu = 0;
    let mut previous_mem = 0;

    for (idx, &signer_count) in signer_counts.iter().enumerate() {
        let threshold = if signer_count == 1 { 1 } else { 2 };
        let ctx = GovGasCtx::setup(signer_count, threshold);

        let proposal_id = ctx.create_proposal(calldata_size);

        // Approve everyone except the last one first
        for i in 0..(signer_count - 1) {
            ctx.client.approve(&ctx.signers[i as usize], &proposal_id);
        }

        // Now measure the cost of that final approval
        let last_signer = ctx.signers[(signer_count - 1) as usize].clone();
        let (cpu, mem) = measure_budget(&ctx, |ctx| {
            ctx.client.approve(&last_signer, &proposal_id);
        });

        let cpu_threshold = MAX_CPU_APPROVE_PER_SIGNER * signer_count as u64;
        let mem_threshold = MAX_MEM_APPROVE_PER_SIGNER * signer_count as u64;

        assert!(
            cpu <= cpu_threshold + MAX_CPU_APPROVE_MAX_SIGNERS,
            "Approve CPU exceeded threshold: {} > {} (signers={})",
            cpu,
            cpu_threshold + MAX_CPU_APPROVE_MAX_SIGNERS,
            signer_count
        );
        assert!(
            mem <= mem_threshold + MAX_MEM_APPROVE_MAX_SIGNERS,
            "Approve memory exceeded threshold: {} > {} (signers={})",
            mem,
            mem_threshold + MAX_MEM_APPROVE_MAX_SIGNERS,
            signer_count
        );

        // Check that costs grow roughly linearly with signer count.
        // If this fails, something weird is happening with the approval loop.
        if idx > 0 {
            let cpu_growth = cpu - previous_cpu;
            let mem_growth = mem - previous_mem;
            let signers_diff = signer_count - signer_counts[idx - 1];

            let expected_cpu_growth = MAX_CPU_APPROVE_PER_SIGNER * signers_diff as u64 + 10_000;
            let expected_mem_growth = MAX_MEM_APPROVE_PER_SIGNER * signers_diff as u64 + 1_000;

            assert!(
                cpu_growth <= expected_cpu_growth,
                "CPU growth not linear: {} for {} new signers (expected <= {})",
                cpu_growth,
                signers_diff,
                expected_cpu_growth
            );
            assert!(
                mem_growth <= expected_mem_growth,
                "Memory growth not linear: {} for {} new signers (expected <= {})",
                mem_growth,
                signers_diff,
                expected_mem_growth
            );
        }

        println!(
            "APPROVE: signers={:2}, CPU={:8}, MEM={:6}, CPU/signer={:6.1}, MEM/signer={:5.1}",
            signer_count,
            cpu,
            mem,
            cpu as f64 / signer_count as f64,
            mem as f64 / signer_count as f64
        );

        previous_cpu = cpu;
        previous_mem = mem;
    }
}

#[test]
fn test_approve_duplicate_error_cost() {
    // Duplicate approval checks should be cheap since we use a Map index for O(1) lookups.
    // If this gets expensive, something went wrong with the storage pattern.
    let ctx = GovGasCtx::setup(5, 2);
    let proposal_id = ctx.create_proposal(1024);

    ctx.client.approve(&ctx.signers[0], &proposal_id);

    let (cpu, mem) = measure_budget(&ctx, |ctx| {
        let _ = ctx.client.try_approve(&ctx.signers[0], &proposal_id);
    });

    assert!(
        cpu < 100_000,
        "Duplicate approve error cost too high: {}",
        cpu
    );
    assert!(
        mem < 20_000,
        "Duplicate approve error memory cost too high: {}",
        mem
    );

    println!("APPROVE_DUPLICATE: CPU={}, MEM={}", cpu, mem);
}

// ============================================================================
// EXECUTE TESTS
// ============================================================================

#[test]
fn test_execute_budget_snapshots() {
    // Execute cost should scale with calldata size since we process the stored data.
    // We test from empty calldata all the way up to the 4KB limit.
    let calldata_sizes = [0, 1024, 2048, 3072, MAX_CALLDATA_BYTES as usize];
    let signer_count = 2; // Minimum needed for quorum

    let mut previous_cpu = 0;
    let mut previous_mem = 0;

    for (idx, &calldata_size) in calldata_sizes.iter().enumerate() {
        let ctx = GovGasCtx::setup(signer_count, 2);
        let proposal_id = ctx.create_proposal(calldata_size);

        ctx.approve_all(proposal_id);

        // Need to wait for the timelock to pass before we can execute
        ctx.advance_time(GOVERNANCE_TIMELOCK_SECONDS + 1);

        let executor = Address::generate(&ctx.env);
        let (cpu, mem) = measure_budget(&ctx, |ctx| {
            ctx.client.execute(&executor, &proposal_id);
        });

        let kb = (calldata_size / 1024 + 1) as u64;
        let cpu_threshold = MAX_CPU_EXECUTE_PER_KB * kb;
        let mem_threshold = MAX_MEM_EXECUTE_PER_KB * kb;

        assert!(
            cpu <= cpu_threshold + MAX_CPU_EXECUTE_MAX_CALLDATA,
            "Execute CPU exceeded threshold: {} > {} (calldata={}B)",
            cpu,
            cpu_threshold + MAX_CPU_EXECUTE_MAX_CALLDATA,
            calldata_size
        );
        assert!(
            mem <= mem_threshold + MAX_MEM_EXECUTE_MAX_CALLDATA,
            "Execute memory exceeded threshold: {} > {} (calldata={}B)",
            mem,
            mem_threshold + MAX_MEM_EXECUTE_MAX_CALLDATA,
            calldata_size
        );

        // Cost should grow roughly linearly with calldata size.
        // Each KB of calldata adds some overhead to processing.
        if idx > 0 {
            let cpu_growth = cpu - previous_cpu;
            let mem_growth = mem - previous_mem;
            let bytes_diff = calldata_size - calldata_sizes[idx - 1];
            let kb_diff = (bytes_diff / 1024 + 1) as u64;

            let expected_cpu_growth = MAX_CPU_EXECUTE_PER_KB * kb_diff + 100_000;
            let expected_mem_growth = MAX_MEM_EXECUTE_PER_KB * kb_diff + 10_000;

            assert!(
                cpu_growth <= expected_cpu_growth,
                "CPU growth not linear with calldata: {} for {} bytes (expected <= {})",
                cpu_growth,
                bytes_diff,
                expected_cpu_growth
            );
            assert!(
                mem_growth <= expected_mem_growth,
                "Memory growth not linear with calldata: {} for {} bytes (expected <= {})",
                mem_growth,
                bytes_diff,
                expected_mem_growth
            );
        }

        println!(
            "EXECUTE: calldata={:4}B, CPU={:8}, MEM={:6}, CPU/KB={:7.1}, MEM/KB={:6.1}",
            calldata_size,
            cpu,
            mem,
            cpu as f64 / (calldata_size as f64 / 1024.0 + 1.0),
            mem as f64 / (calldata_size as f64 / 1024.0 + 1.0)
        );

        previous_cpu = cpu;
        previous_mem = mem;
    }
}

#[test]
fn test_execute_without_quorum_error_cost() {
    // Trying to execute without quorum should be cheap since we fail fast
    let ctx = GovGasCtx::setup(3, 2);
    let proposal_id = ctx.create_proposal(1024);

    // Only 1 approval when we need 2
    ctx.client.approve(&ctx.signers[0], &proposal_id);

    ctx.advance_time(GOVERNANCE_TIMELOCK_SECONDS + 1);

    let executor = Address::generate(&ctx.env);
    let (cpu, mem) = measure_budget(&ctx, |ctx| {
        let _ = ctx.client.try_execute(&executor, &proposal_id);
    });

    assert!(
        cpu < 200_000,
        "Execute without quorum error cost too high: {}",
        cpu
    );
    assert!(
        mem < 30_000,
        "Execute without quorum error memory cost too high: {}",
        mem
    );

    println!("EXECUTE_NO_QUORUM: CPU={}, MEM={}", cpu, mem);
}

#[test]
fn test_execute_pre_timelock_error_cost() {
    // Failing due to timelock should also be cheap
    let ctx = GovGasCtx::setup(2, 2);
    let proposal_id = ctx.create_proposal(1024);

    ctx.approve_all(proposal_id);

    // We don't advance time, so timelock hasn't elapsed
    let executor = Address::generate(&ctx.env);
    let (cpu, mem) = measure_budget(&ctx, |ctx| {
        let _ = ctx.client.try_execute(&executor, &proposal_id);
    });

    assert!(
        cpu < 200_000,
        "Execute pre-timelock error cost too high: {}",
        cpu
    );
    assert!(
        mem < 30_000,
        "Execute pre-timelock error memory cost too high: {}",
        mem
    );

    println!("EXECUTE_PRE_TIMELOCK: CPU={}, MEM={}", cpu, mem);
}

// ============================================================================
// WORST-CASE TESTS
// ============================================================================

#[test]
fn test_worst_case_scenario() {
    // This is the big one: max signers (20) and max calldata (4KB).
    // If this passes, we know governance ops won't break the bank in production.
    println!("\n=== WORST CASE: MAX_SIGNERS + MAX_CALLDATA ===");

    let ctx = GovGasCtx::setup(MAX_SIGNERS, MAX_SIGNERS);
    let calldata_size = MAX_CALLDATA_BYTES as usize;

    // 1. Propose
    let (cpu_propose, mem_propose) = measure_budget(&ctx, |ctx| {
        ctx.create_proposal(calldata_size);
    });

    // 2. Approve all 20 signers
    let proposal_id = ctx.create_proposal(calldata_size);
    let (cpu_approve_all, mem_approve_all) = measure_budget(&ctx, |ctx| {
        ctx.approve_all(proposal_id);
    });

    // 3. Execute
    ctx.advance_time(GOVERNANCE_TIMELOCK_SECONDS + 1);
    let executor = Address::generate(&ctx.env);
    let (cpu_execute, mem_execute) = measure_budget(&ctx, |ctx| {
        ctx.client.execute(&executor, &proposal_id);
    });

    // Check all thresholds
    assert!(
        cpu_propose <= MAX_CPU_PROPOSE,
        "Worst-case propose CPU: {} > {}",
        cpu_propose, MAX_CPU_PROPOSE
    );
    assert!(
        mem_propose <= MAX_MEM_PROPOSE,
        "Worst-case propose memory: {} > {}",
        mem_propose, MAX_MEM_PROPOSE
    );

    assert!(
        cpu_approve_all <= MAX_CPU_APPROVE_MAX_SIGNERS,
        "Worst-case approve CPU: {} > {}",
        cpu_approve_all, MAX_CPU_APPROVE_MAX_SIGNERS
    );
    assert!(
        mem_approve_all <= MAX_MEM_APPROVE_MAX_SIGNERS,
        "Worst-case approve memory: {} > {}",
        mem_approve_all, MAX_MEM_APPROVE_MAX_SIGNERS
    );

    assert!(
        cpu_execute <= MAX_CPU_EXECUTE_MAX_CALLDATA,
        "Worst-case execute CPU: {} > {}",
        cpu_execute, MAX_CPU_EXECUTE_MAX_CALLDATA
    );
    assert!(
        mem_execute <= MAX_MEM_EXECUTE_MAX_CALLDATA,
        "Worst-case execute memory: {} > {}",
        mem_execute, MAX_MEM_EXECUTE_MAX_CALLDATA
    );

    println!("PROPOSE:   CPU={:8}, MEM={:6}", cpu_propose, mem_propose);
    println!("APPROVE:   CPU={:8}, MEM={:6}", cpu_approve_all, mem_approve_all);
    println!("EXECUTE:   CPU={:8}, MEM={:6}", cpu_execute, mem_execute);
}

#[test]
fn test_proposal_expiry_checks_dont_add_hidden_costs() {
    // Make sure the expiry checks don't secretly add overhead to normal operations
    let ctx = GovGasCtx::setup(2, 2);
    let proposal_id = ctx.create_proposal(1024);

    ctx.approve_all(proposal_id);
    ctx.advance_time(GOVERNANCE_TIMELOCK_SECONDS + 1);

    // Run execute once normally
    let executor = Address::generate(&ctx.env);
    let (cpu1, mem1) = measure_budget(&ctx, |ctx| {
        ctx.client.execute(&executor, &proposal_id);
    });

    // Create another proposal and run execute when it's closer to expiry
    let ctx2 = GovGasCtx::setup(2, 2);
    let proposal_id2 = ctx2.create_proposal(1024);
    ctx2.approve_all(proposal_id2);

    // Advance time to just before expiry
    ctx2.advance_time(MAX_PROPOSAL_AGE_SECONDS - 100);
    ctx2.advance_time(GOVERNANCE_TIMELOCK_SECONDS + 1);

    let executor2 = Address::generate(&ctx2.env);
    let (cpu2, mem2) = measure_budget(&ctx2, |ctx| {
        ctx.client.execute(&executor2, &proposal_id2);
    });

    // The costs should be similar - expiry check shouldn't add much overhead
    let cpu_diff = if cpu1 > cpu2 { cpu1 - cpu2 } else { cpu2 - cpu1 };
    let mem_diff = if mem1 > mem2 { mem1 - mem2 } else { mem2 - mem1 };

    assert!(
        cpu_diff < 50_000,
        "Expiry check added unexpected CPU overhead: {}",
        cpu_diff
    );
    assert!(
        mem_diff < 10_000,
        "Expiry check added unexpected memory overhead: {}",
        mem_diff
    );

    println!("EXPIRY_CHECK_OVERHEAD: CPU_diff={}, MEM_diff={}", cpu_diff, mem_diff);
}

#[test]
fn test_budget_reset_is_working_correctly() {
    // Quick sanity check that our budget reset helper actually does what we expect
    let ctx = GovGasCtx::setup(1, 1);

    // First operation
    ctx.env.budget().reset_unlimited();
    ctx.create_proposal(1024);
    let cpu1 = ctx.env.budget().cpu_instruction_cost();

    // Second operation should have similar cost after reset
    ctx.env.budget().reset_unlimited();
    ctx.create_proposal(1024);
    let cpu2 = ctx.env.budget().cpu_instruction_cost();

    let diff = if cpu1 > cpu2 { cpu1 - cpu2 } else { cpu2 - cpu1 };
    assert!(
        diff < 10_000,
        "Budget reset not working - costs differ too much: {} vs {}",
        cpu1, cpu2
    );
}

#[test]
fn test_multiple_proposals_parallel_cost() {
    // Make sure creating multiple proposals doesn't have weird interactions
    let ctx = GovGasCtx::setup(5, 3);
    let calldata_size = 2048;

    // Measure cost of first proposal
    let (cpu1, mem1) = measure_budget(&ctx, |ctx| {
        ctx.create_proposal(calldata_size);
    });

    // Create a bunch of proposals, then measure the cost of another one
    for _ in 0..10 {
        ctx.create_proposal(calldata_size);
    }

    let (cpu2, mem2) = measure_budget(&ctx, |ctx| {
        ctx.create_proposal(calldata_size);
    });

    // Costs should be similar - proposals don't affect each other
    let cpu_diff = if cpu1 > cpu2 { cpu1 - cpu2 } else { cpu2 - cpu1 };
    let mem_diff = if mem1 > mem2 { mem1 - mem2 } else { mem2 - mem1 };

    assert!(
        cpu_diff < 50_000,
        "Proposal cost changed after creating many proposals: {} vs {}",
        cpu1, cpu2
    );
    assert!(
        mem_diff < 10_000,
        "Proposal memory cost changed after creating many proposals: {} vs {}",
        mem1, mem2
    );

    println!("MULTIPLE_PROPOSALS: CPU_diff={}, MEM_diff={}", cpu_diff, mem_diff);
}