extern crate std;

use fluxora_vesting::{ContractError, FluxoraVesting, FluxoraVestingClient, VestingStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    admin: Address,
    benefactor: Address,
    beneficiary: Address,
    token: TokenClient<'a>,
    client: FluxoraVestingClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraVesting);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let benefactor = Address::generate(&env);
        let beneficiary = Address::generate(&env);

        let client = FluxoraVestingClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&benefactor, &1_000_000_i128);

        let token = TokenClient::new(&env, &token_id);

        Self {
            env,
            contract_id,
            token_id,
            admin,
            benefactor,
            beneficiary,
            token,
            client,
        }
    }

    fn advance(&self, secs: u64) {
        let t = self.env.ledger().timestamp();
        self.env.ledger().with_mut(|l| l.timestamp = t + secs);
    }
}

// ---------------------------------------------------------------------------
// Full lifecycle: create → claim incrementally → complete
// ---------------------------------------------------------------------------

#[test]
fn full_lifecycle_linear_vesting() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    let id = ctx
        .client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1_000_i128,
            &1_i128,
            &now,
            &now,
            &(now + 1_000),
        )
        .unwrap();

    // Claim at 25%, 50%, 75%, 100%
    ctx.advance(250);
    assert_eq!(ctx.client.claim(&id).unwrap(), 250);

    ctx.advance(250);
    assert_eq!(ctx.client.claim(&id).unwrap(), 250);

    ctx.advance(250);
    assert_eq!(ctx.client.claim(&id).unwrap(), 250);

    ctx.advance(250);
    assert_eq!(ctx.client.claim(&id).unwrap(), 250);

    let s = ctx.client.get_schedule(&id).unwrap();
    assert_eq!(s.status, VestingStatus::Completed);
    assert_eq!(s.claimed_amount, 1_000);
    assert_eq!(ctx.token.balance(&ctx.beneficiary), 1_000);
}

// ---------------------------------------------------------------------------
// Cliff boundary: no tokens before cliff, full elapsed after
// ---------------------------------------------------------------------------

#[test]
fn cliff_boundary_exact() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let cliff = now + 300;

    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1_000_i128,
            &1_i128,
            &now,
            &cliff,
            &(now + 1_000),
        )
        .unwrap();

    // One second before cliff: nothing
    ctx.advance(299);
    assert_eq!(ctx.client.get_claimable(&0u64).unwrap(), 0);
    assert_eq!(ctx.client.claim(&0u64).unwrap(), 0);

    // Exactly at cliff: 300 tokens vested (elapsed from start)
    ctx.advance(1);
    assert_eq!(ctx.client.get_claimable(&0u64).unwrap(), 300);
    assert_eq!(ctx.client.claim(&0u64).unwrap(), 300);
}

// ---------------------------------------------------------------------------
// End boundary: accrual stops at end_time
// ---------------------------------------------------------------------------

#[test]
fn end_boundary_accrual_capped() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1_000_i128,
            &1_i128,
            &now,
            &now,
            &(now + 1_000),
        )
        .unwrap();

    ctx.advance(5_000); // well past end
    assert_eq!(ctx.client.get_claimable(&0u64).unwrap(), 1_000);
    assert_eq!(ctx.client.claim(&0u64).unwrap(), 1_000);
}

// ---------------------------------------------------------------------------
// Revoke → beneficiary claims frozen amount → close
// ---------------------------------------------------------------------------

#[test]
fn revoke_then_claim_then_close() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    let id = ctx
        .client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1_000_i128,
            &1_i128,
            &now,
            &now,
            &(now + 1_000),
        )
        .unwrap();

    let bal_before = ctx.token.balance(&ctx.benefactor);
    ctx.advance(600);
    ctx.client.revoke(&id).unwrap();

    // Benefactor gets 400 back
    assert_eq!(ctx.token.balance(&ctx.benefactor), bal_before + 400);

    // Beneficiary claims frozen 600
    assert_eq!(ctx.client.claim(&id).unwrap(), 600);
    assert_eq!(ctx.token.balance(&ctx.beneficiary), 600);

    let s = ctx.client.get_schedule(&id).unwrap();
    assert_eq!(s.status, VestingStatus::Completed);

    // Close the settled schedule
    ctx.client.close_schedule(&id).unwrap();
    assert_eq!(
        ctx.client.try_get_schedule(&id).unwrap_err().unwrap(),
        ContractError::ScheduleNotFound
    );
}

// ---------------------------------------------------------------------------
// Multiple schedules for same beneficiary
// ---------------------------------------------------------------------------

#[test]
fn multiple_schedules_independent() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    let id0 = ctx
        .client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &500_i128,
            &1_i128,
            &now,
            &now,
            &(now + 500),
        )
        .unwrap();

    let id1 = ctx
        .client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1_000_i128,
            &2_i128,
            &now,
            &now,
            &(now + 500),
        )
        .unwrap();

    ctx.advance(250);
    assert_eq!(ctx.client.claim(&id0).unwrap(), 250);
    assert_eq!(ctx.client.claim(&id1).unwrap(), 500);

    ctx.advance(250);
    assert_eq!(ctx.client.claim(&id0).unwrap(), 250);
    assert_eq!(ctx.client.claim(&id1).unwrap(), 500);

    assert_eq!(ctx.token.balance(&ctx.beneficiary), 1_500);
}

// ---------------------------------------------------------------------------
// Revoke before cliff: full refund, beneficiary gets nothing
// ---------------------------------------------------------------------------

#[test]
fn revoke_before_cliff_full_refund() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1_000_i128,
            &1_i128,
            &now,
            &(now + 500),
            &(now + 1_000),
        )
        .unwrap();

    let bal_before = ctx.token.balance(&ctx.benefactor);
    ctx.advance(100);
    ctx.client.revoke(&0u64).unwrap();

    assert_eq!(ctx.token.balance(&ctx.benefactor), bal_before + 1_000);
    // Beneficiary has nothing to claim (0 vested before cliff)
    assert_eq!(ctx.client.claim(&0u64).unwrap(), 0);
}

// ---------------------------------------------------------------------------
// get_vested_at simulation does not change state
// ---------------------------------------------------------------------------

#[test]
fn get_vested_at_is_read_only() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();

    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1_000_i128,
            &1_i128,
            &now,
            &now,
            &(now + 1_000),
        )
        .unwrap();

    let future = now + 700;
    let vested = ctx.client.get_vested_at(&0u64, &future).unwrap();
    assert_eq!(vested, 700);

    // State unchanged: claimed_amount still 0
    let s = ctx.client.get_schedule(&0u64).unwrap();
    assert_eq!(s.claimed_amount, 0);
    assert_eq!(s.status, VestingStatus::Active);
}

// ---------------------------------------------------------------------------
// Double-close is an error
// ---------------------------------------------------------------------------

#[test]
fn double_close_fails() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(1_000);
    ctx.client.claim(&id).unwrap();
    ctx.client.close_schedule(&id).unwrap();
    // Second close: schedule no longer exists
    assert_eq!(
        ctx.client.try_close_schedule(&id).unwrap_err().unwrap(),
        ContractError::ScheduleNotFound
    );
}

// ---------------------------------------------------------------------------
// version and get_config
// ---------------------------------------------------------------------------

#[test]
fn version_is_one() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client.version(), 1u32);
}

#[test]
fn get_config_returns_correct_values() {
    let ctx = Ctx::setup();
    let cfg = ctx.client.get_config().unwrap();
    assert_eq!(cfg.token, ctx.token_id);
    assert_eq!(cfg.admin, ctx.admin);
}

// Helper on Ctx for integration tests
impl<'a> Ctx<'a> {
    fn create_default(&self) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client
            .create_vesting(
                &self.benefactor,
                &self.beneficiary,
                &1_000_i128,
                &1_i128,
                &now,
                &now,
                &(now + 1_000),
            )
            .unwrap()
    }
}
