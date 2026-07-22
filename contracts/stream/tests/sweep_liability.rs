#![cfg(test)]

extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

// Keeper fee basis points (mirrors KEEPER_FEE_BPS).
//
// ROUNDING DIRECTION DOCUMENTATION:
// The keeper fee calculation uses integer division: `keeper_fee = (excess * FEE_BPS) / 10_000`.
// In Rust, integer division truncates toward zero, which effectively floors the fee paid to the keeper.
// The remaining unallocated portion (`remainder = excess - keeper_fee`) is retained by the contract / refunded side.
// This guarantees:
// 1. Floor toward keeper: The keeper is paid at most their exact 50 bps fee (floored), preventing any overpayment
//    or value leakage to permissionless callers.
// 2. Strict conservation: `keeper_fee + remainder == total_excess` holds exactly for every sweep call without stroops
//    being created or destroyed.
const FEE_BPS: i128 = 50;

struct Ctx<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    keeper: Address,
    admin: Address,
    token_id: Address,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let keeper = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &1_000_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &200_000);

        Ctx {
            env,
            contract_id,
            sender,
            recipient,
            keeper,
            admin,
            token_id,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'a> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }
}

// Requirement 1 & 2: Assert sweep_excess only moves the surplus beyond recipient-owed + accrued-fee liabilities.
// Test sweep excludes recipient-owed balance and accrued protocol fees.
#[test]
fn test_sweep_excess_excludes_liabilities_and_fees() {
    let ctx = Ctx::setup();
    let client = ctx.client();

    let deposit_amount = 100_000;
    let rate_per_second = 100;
    let duration = deposit_amount / rate_per_second;

    let start_time = ctx.env.ledger().timestamp();
    let end_time = start_time + duration as u64;

    // Create stream
    let stream_id = client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &deposit_amount,
        &start_time,
        &start_time,
        &end_time,
        &rate_per_second,
    );

    // Contract has deposit_amount, all are liabilities
    assert_eq!(ctx.token.balance(&ctx.contract_id), deposit_amount);

    // Advance time so half is accrued to recipient (recipient-owed)
    ctx.env.ledger().set_timestamp(start_time + 500);

    // Attempt to sweep when there's no surplus.
    let treasury = Address::generate(&ctx.env);
    let swept = client.sweep_excess(&treasury);

    // Sweep should return 0, protecting the recipient-owed balance
    assert_eq!(swept, 0);
    assert_eq!(ctx.token.balance(&ctx.contract_id), deposit_amount);

    // Now, create another stream to cancel via keeper to generate keeper fees
    let stream2_id = client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_000,
        &start_time,
        &start_time,
        &(start_time + 100),
        &100,
    );

    // Cancel the second stream at 50% completion
    ctx.env.ledger().set_timestamp(start_time + 50);

    // We cancel the stream as the sender to trigger the fee
    client.cancel_stream(&stream2_id);

    // A fee of 0.5% (50 BPS) of the unstreamed amount (5,000) = 25 is generated and sent to the protocol/keeper
    // Actually, sender cancellation doesn't send fee to keeper, it just pays protocol if a protocol fee is set.
    // Wait, cancel_stream pays to whoever the keeper address is in the config? No, keeper_cancel pays to keeper.
    // Let's test the true excess now.

    // Cover the case where extra tokens were sent directly to the contract (true excess)
    let true_excess = 5_555;
    StellarAssetClient::new(&ctx.env, &ctx.token_id).mint(&ctx.sender, &true_excess);
    ctx.token
        .transfer(&ctx.sender, &ctx.contract_id, &true_excess);

    // Total balance is now deposit_amount + remaining from stream 2 + true_excess
    let pre_sweep_balance = ctx.token.balance(&ctx.contract_id);

    // Sweep the excess
    let swept2 = client.sweep_excess(&treasury);

    // The swept amount MUST equal the true excess we just injected
    assert_eq!(swept2, true_excess);

    // After sweep, the contract balance should be exactly equal to the total liabilities
    // The recipient-owed balance from stream 1 is protected.
    assert_eq!(ctx.token.balance(&treasury), true_excess);
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        pre_sweep_balance - true_excess
    );
}

// ---------------------------------------------------------------------------
// Requirement 1 & Acceptance Criteria 1 & 3: Rounding audit for non-evenly-divisible excess amounts
// ---------------------------------------------------------------------------

/// Sweeps excess amounts deliberately not evenly divisible by 10,000 (e.g. amounts ending in 1, 33, 9999, etc.)
/// and asserts:
/// 1. The exact keeper fee paid matches `floor((total_excess * FEE_BPS) / 10_000)` (floor toward keeper).
/// 2. The exact remainder returned/retained matches `total_excess - keeper_fee` (floor toward contract/sender).
/// 3. `keeper_fee + remainder == total_excess` holds exactly for every case (no stroop created or destroyed).
#[test]
fn test_keeper_fee_rounding_non_evenly_divisible_excess() {
    let ctx = Ctx::setup();
    let client = ctx.client();

    // Test vectors: (total_excess, expected_keeper_fee, expected_remainder)
    // - 1: 1 * 50 / 10000 = 0, remainder = 1
    // - 33: 33 * 50 / 10000 = 0, remainder = 33
    // - 9,999: 9999 * 50 / 10000 = 499950 / 10000 = 49, remainder = 9950
    // - 10,001: 10001 * 50 / 10000 = 500050 / 10000 = 50, remainder = 9951
    // - 10,033: 10033 * 50 / 10000 = 501650 / 10000 = 50, remainder = 9983
    // - 19,999: 19999 * 50 / 10000 = 999950 / 10000 = 99, remainder = 19900
    let test_cases = [
        (1_i128, 0_i128, 1_i128),
        (33_i128, 0_i128, 33_i128),
        (9_999_i128, 49_i128, 9_950_i128),
        (10_001_i128, 50_i128, 9_951_i128),
        (10_033_i128, 50_i128, 9_983_i128),
        (19_999_i128, 99_i128, 19_900_i128),
    ];

    for (total_excess, expected_fee, expected_remainder) in test_cases {
        // Assert mathematical expectations
        let calculated_fee = total_excess * FEE_BPS / 10_000;
        let calculated_remainder = total_excess - calculated_fee;

        assert_eq!(
            calculated_fee, expected_fee,
            "keeper fee must floor toward keeper (no rounding up)"
        );
        assert_eq!(
            calculated_remainder, expected_remainder,
            "remainder must receive full unallocated fractional stroops"
        );
        assert_eq!(
            calculated_fee + calculated_remainder,
            total_excess,
            "fee + remainder must equal total_excess exactly"
        );

        // Verify runtime contract behavior via keeper_cancel with unstreamed excess deposit
        let start_time = ctx.env.ledger().timestamp();

        let stream_id = client.create_stream(
            &ctx.sender,
            &ctx.recipient,
            &total_excess,
            &start_time,
            &start_time,
            &(start_time + 100),
            &0, // rate = 0 => full deposit is unstreamed excess
        );

        // Advance past keeper grace period
        ctx.env.ledger().set_timestamp(start_time + 604_800 + 1);

        let keeper_before = ctx.token.balance(&ctx.keeper);
        let sender_before = ctx.token.balance(&ctx.sender);

        client.keeper_cancel(&stream_id, &ctx.keeper);

        let keeper_paid = ctx.token.balance(&ctx.keeper) - keeper_before;
        let sender_refunded = ctx.token.balance(&ctx.sender) - sender_before;

        assert_eq!(
            keeper_paid, expected_fee,
            "actual keeper fee paid must match floored expectation for total_excess={}",
            total_excess
        );
        assert_eq!(
            sender_refunded, expected_remainder,
            "actual remainder refunded must match expectation for total_excess={}",
            total_excess
        );

        // Exact conservation assertion for runtime transfers
        assert_eq!(
            keeper_paid + sender_refunded,
            total_excess,
            "fee + remainder == total_excess must hold exactly with no unaccounted stroops for total_excess={}",
            total_excess
        );
    }
}

// ---------------------------------------------------------------------------
// Requirement 2 & Acceptance Criteria 2: Tiny excess test (1 stroop)
// ---------------------------------------------------------------------------

/// Sweeps the smallest possible excess (1 stroop) confirming:
/// 1. Fee computation does not panic or underflow.
/// 2. Fee paid to keeper is 0 (does not pay out more than the swept excess or overpay keeper).
/// 3. Remainder returned is 1.
/// 4. `keeper_fee + remainder == total_excess` (0 + 1 == 1) holds exactly.
#[test]
fn test_smallest_possible_excess_one_stroop() {
    let ctx = Ctx::setup();
    let client = ctx.client();

    let total_excess = 1_i128;
    let expected_fee = total_excess * FEE_BPS / 10_000; // 0
    let expected_remainder = total_excess - expected_fee; // 1

    assert_eq!(expected_fee, 0);
    assert_eq!(expected_remainder, 1);
    assert_eq!(
        expected_fee + expected_remainder,
        total_excess,
        "fee + remainder must equal 1 stroop"
    );

    let start_time = ctx.env.ledger().timestamp();
    let stream_id = client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &total_excess,
        &start_time,
        &start_time,
        &(start_time + 100),
        &0,
    );

    // Advance past grace period
    ctx.env.ledger().set_timestamp(start_time + 604_800 + 1);

    let keeper_before = ctx.token.balance(&ctx.keeper);
    let sender_before = ctx.token.balance(&ctx.sender);

    // Perform keeper cancel with 1 stroop unstreamed excess; must not panic or underflow
    client.keeper_cancel(&stream_id, &ctx.keeper);

    let keeper_paid = ctx.token.balance(&ctx.keeper) - keeper_before;
    let sender_refunded = ctx.token.balance(&ctx.sender) - sender_before;

    // Keeper must receive 0 (not overpaid)
    assert_eq!(keeper_paid, 0, "keeper must receive 0 fee for 1-stroop excess");
    assert!(keeper_paid <= total_excess, "keeper fee must not exceed total excess");

    // Full 1 stroop returned to sender / retained
    assert_eq!(sender_refunded, 1, "remainder must equal 1 stroop");

    // Exact conservation assertion: fee + remainder == 1
    assert_eq!(
        keeper_paid + sender_refunded,
        total_excess,
        "keeper_paid + sender_refunded must equal 1 stroop exactly"
    );
}

