use fluxora_governance::{FluxoraGovernance, FluxoraGovernanceClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Bytes, Env, Vec,
};

struct GovGasCtx<'a> {
    env: Env,
    client: FluxoraGovernanceClient<'a>,
    signers: Vec<Address>,
    target: Address,
}

impl<'a> GovGasCtx<'a> {
    fn setup(signer_count: u32, threshold: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000_000);

        let contract_id = env.register_contract(None, FluxoraGovernance);
        let client = FluxoraGovernanceClient::new(&env, &contract_id);

        let mut signers: Vec<Address> = Vec::new(&env);
        for _ in 0..signer_count {
            signers.push_back(Address::generate(&env));
        }

        client.init(&Address::generate(&env), &signers, &threshold);
        let target = Address::generate(&env);

        Self {
            env,
            client,
            signers,
            target,
        }
    }

    fn calldata(&self, tag: &str) -> Bytes {
        Bytes::from_slice(&self.env, tag.as_bytes())
    }
}

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

#[test]
fn test_governance_gas_propose() {
    let sizes = [1u32, 5, 10, 20];
    for &size in &sizes {
        let threshold = if size == 1 { 1 } else { size };
        let ctx = GovGasCtx::setup(size, threshold);
        let (cpu, mem) = measure_budget(&ctx, |ctx| {
            ctx.client
                .propose(&ctx.signers[0], &ctx.target, &ctx.calldata("proposal"));
        });

        println!("GAS_MEASUREMENT: propose: {}: {}", size, cpu);
        println!("GAS_MEASUREMENT: propose_mem: {}: {}", size, mem);

        assert!(cpu < 1_200_000, "propose {} exceeded CPU ceiling", size);
        assert!(mem < 150_000, "propose {} exceeded memory ceiling", size);
    }
}

#[test]
fn test_governance_gas_approve_nonquorum() {
    let sizes = [5u32, 10, 20];
    for &size in &sizes {
        let threshold = size;
        let ctx = GovGasCtx::setup(size, threshold);
        let proposal_id =
            ctx.client
                .propose(&ctx.signers[0], &ctx.target, &ctx.calldata("proposal"));

        let (cpu, mem) = measure_budget(&ctx, |ctx| {
            ctx.client.approve(&ctx.signers[1], &proposal_id);
        });

        println!("GAS_MEASUREMENT: approve_nonquorum: {}: {}", size, cpu);
        println!("GAS_MEASUREMENT: approve_nonquorum_mem: {}: {}", size, mem);

        assert!(
            cpu < 1_300_000,
            "approve_nonquorum {} exceeded CPU ceiling",
            size
        );
        assert!(
            mem < 150_000,
            "approve_nonquorum {} exceeded memory ceiling",
            size
        );
    }
}

#[test]
fn test_governance_gas_approve_quorum() {
    let sizes = [1u32, 5, 10, 20];
    for &size in &sizes {
        let threshold = size;
        let ctx = GovGasCtx::setup(size, threshold);
        let proposal_id =
            ctx.client
                .propose(&ctx.signers[0], &ctx.target, &ctx.calldata("proposal"));

        for signer in 0..(size - 1) {
            ctx.client
                .approve(&ctx.signers[signer as usize], &proposal_id);
        }

        let last_signer = ctx.signers[(size - 1) as usize].clone();
        ctx.env.budget().reset_unlimited();
        ctx.client.approve(&last_signer, &proposal_id);
        let cpu = ctx.env.budget().cpu_instruction_cost();
        let mem = ctx.env.budget().memory_bytes_cost();

        println!("GAS_MEASUREMENT: approve_quorum: {}: {}", size, cpu);
        println!("GAS_MEASUREMENT: approve_quorum_mem: {}: {}", size, mem);

        assert!(
            cpu < 1_600_000,
            "approve_quorum {} exceeded CPU ceiling",
            size
        );
        assert!(
            mem < 180_000,
            "approve_quorum {} exceeded memory ceiling",
            size
        );
    }
}

#[test]
fn test_governance_gas_execute() {
    let sizes = [1u32, 5, 10, 20];
    for &size in &sizes {
        let threshold = size;
        let ctx = GovGasCtx::setup(size, threshold);
        let proposal_id =
            ctx.client
                .propose(&ctx.signers[0], &ctx.target, &ctx.calldata("proposal"));

        for signer in 0..size {
            ctx.client
                .approve(&ctx.signers[signer as usize], &proposal_id);
        }

        ctx.env.ledger().set_timestamp(1_000_000 + 172_800 + 1);
        let (cpu, mem) = measure_budget(&ctx, |ctx| {
            ctx.client
                .execute(&Address::generate(&ctx.env), &proposal_id);
        });

        println!("GAS_MEASUREMENT: execute: {}: {}", size, cpu);
        println!("GAS_MEASUREMENT: execute_mem: {}: {}", size, mem);

        assert!(cpu < 1_800_000, "execute {} exceeded CPU ceiling", size);
        assert!(mem < 200_000, "execute {} exceeded memory ceiling", size);
    }
}
