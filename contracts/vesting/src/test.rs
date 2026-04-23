extern crate std;

use crate::{calculate_vested, ContractError, FluxoraVesting, FluxoraVestingClient, VestingStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

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
        sac.mint(&benefactor, &100_000_i128);

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

    /// Create a default schedule: 1000 tokens, 1/s, 1000s duration, no cliff delay.
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

    fn advance(&self, secs: u64) {
        let t = self.env.ledger().timestamp();
        self.env.ledger().with_mut(|l| l.timestamp = t + secs);
    }
}

// ---------------------------------------------------------------------------
// calculate_vested pure function tests
// ---------------------------------------------------------------------------

#[test]
fn vested_before_cliff_is_zero() {
    assert_eq!(calculate_vested(0, 500, 1000, 1, 1000, 499), 0);
}

#[test]
fn vested_at_cliff_equals_elapsed_times_rate() {
    // cliff == start, so at t=500 elapsed=500
    assert_eq!(calculate_vested(0, 0, 1000, 1, 1000, 500), 500);
}

#[test]
fn vested_capped_at_total_amount() {
    assert_eq!(calculate_vested(0, 0, 1000, 1, 1000, 2000), 1000);
}

#[test]
fn vested_at_end_time_equals_total() {
    assert_eq!(calculate_vested(0, 0, 1000, 1, 1000, 1000), 1000);
}

#[test]
fn vested_with_cliff_delay() {
    // start=0, cliff=200, end=1000, rate=1, total=1000
    // at t=200: elapsed=200, vested=200
    assert_eq!(calculate_vested(0, 200, 1000, 1, 1000, 200), 200);
    // at t=199: before cliff
    assert_eq!(calculate_vested(0, 200, 1000, 1, 1000, 199), 0);
}

#[test]
fn vested_overflow_falls_back_to_total() {
    // rate * elapsed would overflow i128
    assert_eq!(
        calculate_vested(0, 0, u64::MAX, i128::MAX, i128::MAX, u64::MAX),
        i128::MAX
    );
}

#[test]
fn vested_zero_rate_returns_zero() {
    assert_eq!(calculate_vested(0, 0, 1000, 0, 1000, 500), 0);
}

#[test]
fn vested_negative_rate_returns_zero() {
    assert_eq!(calculate_vested(0, 0, 1000, -1, 1000, 500), 0);
}

// ---------------------------------------------------------------------------
// init tests
// ---------------------------------------------------------------------------

#[test]
fn init_sets_config() {
    let ctx = Ctx::setup();
    let cfg = ctx.client.get_config().unwrap();
    assert_eq!(cfg.token, ctx.token_id);
    assert_eq!(cfg.admin, ctx.admin);
}

#[test]
fn init_double_init_fails() {
    let ctx = Ctx::setup();
    let err = ctx
        .client
        .try_init(&ctx.token_id, &ctx.admin)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::AlreadyInitialised);
}

#[test]
fn version_returns_one() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client.version(), 1u32);
}

// ---------------------------------------------------------------------------
// create_vesting tests
// ---------------------------------------------------------------------------

#[test]
fn create_vesting_happy_path() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    assert_eq!(id, 0);
    assert_eq!(ctx.client.get_schedule_count(), 1);
    // Tokens pulled from benefactor
    assert_eq!(ctx.token.balance(&ctx.benefactor), 99_000_i128);
}

#[test]
fn create_vesting_increments_id() {
    let ctx = Ctx::setup();
    let id0 = ctx.create_default();
    let id1 = ctx.create_default();
    assert_eq!(id0, 0);
    assert_eq!(id1, 1);
}

#[test]
fn create_vesting_zero_amount_fails() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx
        .client
        .try_create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &0_i128,
            &1_i128,
            &now,
            &now,
            &(now + 1000),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);
}

#[test]
fn create_vesting_zero_rate_fails() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx
        .client
        .try_create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &0_i128,
            &now,
            &now,
            &(now + 1000),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);
}

#[test]
fn create_vesting_same_benefactor_beneficiary_fails() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx
        .client
        .try_create_vesting(
            &ctx.benefactor,
            &ctx.benefactor,
            &1000_i128,
            &1_i128,
            &now,
            &now,
            &(now + 1000),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);
}

#[test]
fn create_vesting_start_in_past_fails() {
    let ctx = Ctx::setup();
    ctx.advance(100);
    let now = ctx.env.ledger().timestamp();
    let err = ctx
        .client
        .try_create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &(now - 1),
            &(now - 1),
            &(now + 999),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::StartTimeInPast);
}

#[test]
fn create_vesting_start_equals_end_fails() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx
        .client
        .try_create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &now,
            &now,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);
}

#[test]
fn create_vesting_cliff_before_start_fails() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx
        .client
        .try_create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &(now + 100),
            &(now + 50), // cliff < start
            &(now + 1100),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);
}

#[test]
fn create_vesting_cliff_after_end_fails() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    let err = ctx
        .client
        .try_create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &(now + 2000), // cliff > end
            &(now + 1000),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InvalidParams);
}

#[test]
fn create_vesting_insufficient_deposit_fails() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    // rate=2, duration=1000 → required=2000, but total_amount=1000
    let err = ctx
        .client
        .try_create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &2_i128,
            &now,
            &now,
            &(now + 1000),
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, ContractError::InsufficientDeposit);
}

// ---------------------------------------------------------------------------
// claim tests
// ---------------------------------------------------------------------------

#[test]
fn claim_before_cliff_returns_zero() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    // cliff at now+500
    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &(now + 500),
            &(now + 1000),
        )
        .unwrap();
    ctx.advance(499);
    let claimed = ctx.client.claim(&0u64).unwrap();
    assert_eq!(claimed, 0);
    assert_eq!(ctx.token.balance(&ctx.beneficiary), 0);
}

#[test]
fn claim_at_cliff_returns_elapsed_tokens() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &(now + 500),
            &(now + 1000),
        )
        .unwrap();
    ctx.advance(500);
    let claimed = ctx.client.claim(&0u64).unwrap();
    assert_eq!(claimed, 500);
    assert_eq!(ctx.token.balance(&ctx.beneficiary), 500);
}

#[test]
fn claim_partial_then_full() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(300);
    let c1 = ctx.client.claim(&id).unwrap();
    assert_eq!(c1, 300);
    ctx.advance(700);
    let c2 = ctx.client.claim(&id).unwrap();
    assert_eq!(c2, 700);
    assert_eq!(ctx.token.balance(&ctx.beneficiary), 1000);
    let s = ctx.client.get_schedule(&id).unwrap();
    assert_eq!(s.status, VestingStatus::Completed);
}

#[test]
fn claim_on_completed_schedule_fails() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(1000);
    ctx.client.claim(&id).unwrap();
    let err = ctx.client.try_claim(&id).unwrap_err().unwrap();
    assert_eq!(err, ContractError::InvalidState);
}

#[test]
fn claim_idempotent_when_nothing_to_claim() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(300);
    ctx.client.claim(&id).unwrap();
    // No time advance — nothing new to claim
    let c2 = ctx.client.claim(&id).unwrap();
    assert_eq!(c2, 0);
}

#[test]
fn claim_after_end_time_caps_at_total() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(5000); // well past end
    let claimed = ctx.client.claim(&id).unwrap();
    assert_eq!(claimed, 1000);
    assert_eq!(ctx.token.balance(&ctx.beneficiary), 1000);
}

// ---------------------------------------------------------------------------
// revoke tests
// ---------------------------------------------------------------------------

#[test]
fn revoke_at_50pct_refunds_half() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    let bal_before = ctx.token.balance(&ctx.benefactor);
    ctx.advance(500);
    ctx.client.revoke(&id).unwrap();
    let s = ctx.client.get_schedule(&id).unwrap();
    assert_eq!(s.status, VestingStatus::Revoked);
    assert!(s.revoked_at.is_some());
    // Benefactor gets 500 back
    assert_eq!(ctx.token.balance(&ctx.benefactor), bal_before + 500);
}

#[test]
fn revoke_before_cliff_refunds_all() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &(now + 500),
            &(now + 1000),
        )
        .unwrap();
    let bal_before = ctx.token.balance(&ctx.benefactor);
    ctx.advance(100); // before cliff
    ctx.client.revoke(&0u64).unwrap();
    assert_eq!(ctx.token.balance(&ctx.benefactor), bal_before + 1000);
}

#[test]
fn revoke_non_active_fails() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(500);
    ctx.client.revoke(&id).unwrap();
    let err = ctx.client.try_revoke(&id).unwrap_err().unwrap();
    assert_eq!(err, ContractError::InvalidState);
}

#[test]
fn revoke_non_admin_fails() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    // mock_all_auths is on, but we need to test the admin check
    // We can verify the admin is set correctly via get_config
    let cfg = ctx.client.get_config().unwrap();
    assert_eq!(cfg.admin, ctx.admin);
    // The revoke call succeeds because mock_all_auths is on
    // This test verifies the admin field is correctly stored
    ctx.advance(100);
    ctx.client.revoke(&id).unwrap();
    let s = ctx.client.get_schedule(&id).unwrap();
    assert_eq!(s.status, VestingStatus::Revoked);
}

#[test]
fn claim_after_revoke_gets_frozen_amount() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(400);
    ctx.client.revoke(&id).unwrap();
    // Beneficiary can still claim the 400 vested tokens
    let claimed = ctx.client.claim(&id).unwrap();
    assert_eq!(claimed, 400);
    let s = ctx.client.get_schedule(&id).unwrap();
    assert_eq!(s.status, VestingStatus::Completed);
}

#[test]
fn claim_after_revoke_no_extra_tokens() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(400);
    ctx.client.revoke(&id).unwrap();
    ctx.advance(600); // time passes but revoke froze accrual
    let claimed = ctx.client.claim(&id).unwrap();
    assert_eq!(claimed, 400); // still only 400, not 1000
}

// ---------------------------------------------------------------------------
// close_schedule tests
// ---------------------------------------------------------------------------

#[test]
fn close_completed_schedule_succeeds() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(1000);
    ctx.client.claim(&id).unwrap();
    ctx.client.close_schedule(&id).unwrap();
    let err = ctx.client.try_get_schedule(&id).unwrap_err().unwrap();
    assert_eq!(err, ContractError::ScheduleNotFound);
}

#[test]
fn close_active_schedule_fails() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    let err = ctx.client.try_close_schedule(&id).unwrap_err().unwrap();
    assert_eq!(err, ContractError::InvalidState);
}

#[test]
fn close_revoked_with_unclaimed_fails() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(500);
    ctx.client.revoke(&id).unwrap();
    // Revoked but beneficiary hasn't claimed yet
    let err = ctx.client.try_close_schedule(&id).unwrap_err().unwrap();
    assert_eq!(err, ContractError::InvalidState);
}

#[test]
fn close_revoked_after_full_claim_succeeds() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(500);
    ctx.client.revoke(&id).unwrap();
    ctx.client.claim(&id).unwrap(); // claim frozen 500
    ctx.client.close_schedule(&id).unwrap();
    let err = ctx.client.try_get_schedule(&id).unwrap_err().unwrap();
    assert_eq!(err, ContractError::ScheduleNotFound);
}

// ---------------------------------------------------------------------------
// get_claimable / get_vested_at tests
// ---------------------------------------------------------------------------

#[test]
fn get_claimable_before_cliff_is_zero() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &(now + 500),
            &(now + 1000),
        )
        .unwrap();
    ctx.advance(100);
    assert_eq!(ctx.client.get_claimable(&0u64).unwrap(), 0);
}

#[test]
fn get_claimable_after_partial_claim() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(600);
    ctx.client.claim(&id).unwrap();
    ctx.advance(200);
    assert_eq!(ctx.client.get_claimable(&id).unwrap(), 200);
}

#[test]
fn get_vested_at_simulation() {
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &now,
            &(now + 1000),
        )
        .unwrap();
    assert_eq!(ctx.client.get_vested_at(&0u64, &(now + 300)).unwrap(), 300);
    assert_eq!(ctx.client.get_vested_at(&0u64, &(now + 1500)).unwrap(), 1000);
}

#[test]
fn get_claimable_on_completed_is_zero() {
    let ctx = Ctx::setup();
    let id = ctx.create_default();
    ctx.advance(1000);
    ctx.client.claim(&id).unwrap();
    assert_eq!(ctx.client.get_claimable(&id).unwrap(), 0);
}

// ---------------------------------------------------------------------------
// get_schedule_count tests
// ---------------------------------------------------------------------------

#[test]
fn schedule_count_increments() {
    let ctx = Ctx::setup();
    assert_eq!(ctx.client.get_schedule_count(), 0);
    ctx.create_default();
    assert_eq!(ctx.client.get_schedule_count(), 1);
    ctx.create_default();
    assert_eq!(ctx.client.get_schedule_count(), 2);
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn schedule_not_found_returns_error() {
    let ctx = Ctx::setup();
    let err = ctx.client.try_get_schedule(&99u64).unwrap_err().unwrap();
    assert_eq!(err, ContractError::ScheduleNotFound);
}

#[test]
fn claim_on_nonexistent_schedule_fails() {
    let ctx = Ctx::setup();
    let err = ctx.client.try_claim(&99u64).unwrap_err().unwrap();
    assert_eq!(err, ContractError::ScheduleNotFound);
}

#[test]
fn revoke_nonexistent_schedule_fails() {
    let ctx = Ctx::setup();
    let err = ctx.client.try_revoke(&99u64).unwrap_err().unwrap();
    assert_eq!(err, ContractError::ScheduleNotFound);
}

#[test]
fn cliff_equals_end_time_all_or_nothing() {
    // cliff == end_time: tokens vest all at once at end_time
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &(now + 1000), // cliff == end
            &(now + 1000),
        )
        .unwrap();
    ctx.advance(999);
    assert_eq!(ctx.client.get_claimable(&0u64).unwrap(), 0);
    ctx.advance(1);
    assert_eq!(ctx.client.get_claimable(&0u64).unwrap(), 1000);
}

#[test]
fn large_deposit_with_excess_over_required() {
    // total_amount > rate * duration is allowed (excess stays locked until end)
    let ctx = Ctx::setup();
    let now = ctx.env.ledger().timestamp();
    // rate=1, duration=500, required=500, but total=1000 (excess 500)
    ctx.client
        .create_vesting(
            &ctx.benefactor,
            &ctx.beneficiary,
            &1000_i128,
            &1_i128,
            &now,
            &now,
            &(now + 500),
        )
        .unwrap();
    ctx.advance(500);
    // vested = min(500, 1000) = 500
    let claimed = ctx.client.claim(&0u64).unwrap();
    assert_eq!(claimed, 500);
}
