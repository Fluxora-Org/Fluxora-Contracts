//! End-to-end integration test: factory-created stream through multiple top_up_stream cycles to Completed.
//!
//! Issue #1478: Covers the full lifecycle of a stream created via `FluxoraFactory::create_stream`,
//! subjected to multiple top-up operations at distinct timestamps interspersed with partial withdrawals,
//! tracking balance & liability consistency throughout, culminating in `StreamStatus::Completed`
//! and verifying post-completion top-up rejection.

extern crate std;

use fluxora_factory::{FluxoraFactory, FluxoraFactoryClient};
use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

const MAX_DEPOSIT_CAP: i128 = 10_000_000;
const MIN_DURATION: u64 = 100;
const SENDER_INITIAL_BALANCE: i128 = 10_000_000;

struct TestContext<'a> {
    env: Env,
    factory: FluxoraFactoryClient<'a>,
    stream: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
    stream_contract_id: Address,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);

        let stream_contract_id = env.register_contract(None, FluxoraStream);
        let factory_contract_id = env.register_contract(None, FluxoraFactory);

        let stream = FluxoraStreamClient::new(&env, &stream_contract_id);
        let factory = FluxoraFactoryClient::new(&env, &factory_contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let token = TokenClient::new(&env, &token_id);
        let sac = StellarAssetClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        sac.mint(&sender, &SENDER_INITIAL_BALANCE);
        token.approve(&sender, &stream_contract_id, &i128::MAX, &100_000);

        stream.init(&token_id, &stream_contract_id);
        factory.init(&admin, &stream_contract_id, &MAX_DEPOSIT_CAP, &MIN_DURATION);
        factory.set_allowlist(&recipient, &true);

        Self {
            env,
            factory,
            stream,
            sender,
            recipient,
            token,
            stream_contract_id,
        }
    }
}

/// End-to-end integration test verifying the full lifecycle of a factory-created stream:
/// 1. Create stream via `factory.create_stream`
/// 2. Top-up stream at least 3 times at different ledger timestamps interspersed with partial withdrawals
/// 3. Assert `TotalLiabilities` stays consistent with outstanding deposits after every step
/// 4. Assert stream reaches `StreamStatus::Completed` exactly once total withdrawn equals total deposited
/// 5. Assert further `top_up_stream` calls after completion are rejected with `ContractError::InvalidState`
#[test]
fn test_factory_created_stream_multi_topup_lifecycle_to_completion() {
    let ctx = TestContext::setup();

    // Initial stream configuration:
    // Start at T=1,000, end at T=2,000 (duration = 1,000s).
    // Initial deposit = 1,000, rate = 1 token/second.
    let start_time = 1_000u64;
    let end_time = 2_000u64;
    let initial_deposit = 1_000_i128;
    let rate_per_second = 1_i128;

    let create_params = CreateStreamParams {
        recipient: ctx.recipient.clone(),
        deposit_amount: initial_deposit,
        rate_per_second,
        start_time,
        cliff_time: start_time,
        end_time,
        withdraw_dust_threshold: Some(0),
        memo: None,
        metadata: None,
        kind: StreamKind::Linear,
        irrevocable: None,
        witness: None,
    };

    // Step 1: Create stream via factory
    let stream_id = ctx.factory.create_stream(&ctx.sender, &create_params);

    let state = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.deposit_amount, 1_000);
    assert_eq!(state.withdrawn_amount, 0);

    let stream_contract_balance = ctx.token.balance(&ctx.stream_contract_id);
    let total_liabilities = ctx.stream.get_total_liabilities();
    assert_eq!(total_liabilities, 1_000);
    assert_eq!(stream_contract_balance, total_liabilities);

    // -----------------------------------------------------------------------
    // Cycle 1: Advance time to T=1,200 (200s passed), Top-Up 1 by 500 tokens
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(1_200);

    // Accrued so far = 200 * 1 = 200 tokens
    assert_eq!(ctx.stream.calculate_accrued(&stream_id), 200);

    ctx.stream.top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let state_1 = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state_1.deposit_amount, 1_500);
    assert_eq!(state_1.withdrawn_amount, 0);
    assert_eq!(state_1.status, StreamStatus::Active);

    let liabilities_1 = ctx.stream.get_total_liabilities();
    assert_eq!(liabilities_1, 1_500);
    assert_eq!(ctx.token.balance(&ctx.stream_contract_id), liabilities_1);

    // Interspersed Partial Withdrawal 1: Recipient withdraws accrued funds at T=1,300 (300s passed -> 300 accrued total)
    ctx.env.ledger().set_timestamp(1_300);
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32); // bypass withdrawal frequency cooldown

    let recipient_balance_before_1 = ctx.token.balance(&ctx.recipient);
    let withdrawn_1 = ctx.stream.withdraw(&stream_id);
    assert_eq!(withdrawn_1, 300);
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        recipient_balance_before_1 + 300
    );

    let state_after_w1 = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state_after_w1.withdrawn_amount, 300);
    assert_eq!(state_after_w1.status, StreamStatus::Active);

    let liabilities_after_w1 = ctx.stream.get_total_liabilities();
    assert_eq!(liabilities_after_w1, 1_200); // 1,500 deposited - 300 withdrawn
    assert_eq!(
        ctx.token.balance(&ctx.stream_contract_id),
        liabilities_after_w1
    );

    // -----------------------------------------------------------------------
    // Cycle 2: Advance time to T=1,500 (500s total passed), Top-Up 2 by 1,000 tokens
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(1_500);

    ctx.stream
        .top_up_stream(&stream_id, &ctx.sender, &1_000_i128);

    let state_2 = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state_2.deposit_amount, 2_500);
    assert_eq!(state_2.withdrawn_amount, 300);
    assert_eq!(state_2.status, StreamStatus::Active);

    let liabilities_2 = ctx.stream.get_total_liabilities();
    assert_eq!(liabilities_2, 2_200); // 2,500 deposited - 300 withdrawn
    assert_eq!(ctx.token.balance(&ctx.stream_contract_id), liabilities_2);

    // Interspersed Partial Withdrawal 2: Recipient withdraws at T=1,700 (700s passed -> 700 accrued total, 400 new)
    ctx.env.ledger().set_timestamp(1_700);
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);

    let recipient_balance_before_2 = ctx.token.balance(&ctx.recipient);
    let withdrawn_2 = ctx.stream.withdraw(&stream_id);
    assert_eq!(withdrawn_2, 400);
    assert_eq!(
        ctx.token.balance(&ctx.recipient),
        recipient_balance_before_2 + 400
    );

    let state_after_w2 = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state_after_w2.withdrawn_amount, 700);
    assert_eq!(state_after_w2.status, StreamStatus::Active);

    let liabilities_after_w2 = ctx.stream.get_total_liabilities();
    assert_eq!(liabilities_after_w2, 1_800); // 2,500 deposited - 700 withdrawn
    assert_eq!(
        ctx.token.balance(&ctx.stream_contract_id),
        liabilities_after_w2
    );

    // -----------------------------------------------------------------------
    // Cycle 3: Advance time to T=1,800, Top-Up 3 by 300 tokens
    // -----------------------------------------------------------------------
    ctx.env.ledger().set_timestamp(1_800);

    ctx.stream.top_up_stream(&stream_id, &ctx.sender, &300_i128);

    let state_3 = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state_3.deposit_amount, 2_800);
    assert_eq!(state_3.withdrawn_amount, 700);
    assert_eq!(state_3.status, StreamStatus::Active);

    let liabilities_3 = ctx.stream.get_total_liabilities();
    assert_eq!(liabilities_3, 2_100); // 2,800 deposited - 700 withdrawn
    assert_eq!(ctx.token.balance(&ctx.stream_contract_id), liabilities_3);

    // -----------------------------------------------------------------------
    // Drive stream to StreamStatus::Completed
    // total_duration = 1,000s (T=1,000 to T=2,000). Total rate = 1 token/s.
    // Max accrual over duration = min(1 * 1,000, 2,800) = 1,000.
    // Wait, end_time is 2,000! Let's verify how much stream accrues by end_time.
    // Accrual cap at end_time is deposit_amount (2,800) or rate * (end_time - start_time) = 1 * 1,000 = 1,000!
    // BUT to withdraw the remaining deposit (up to deposit_amount = 2,800), we can extend end_time or top-up before end.
    // Wait! Let's check how completion is defined in FluxoraStream:
    // A stream completes when total withdrawn equals total deposit_amount.
    // At end_time T=2,000, max accrued for rate=1, duration=1,000 is 1,000.
    // To allow withdrawing the full 2,800 deposit, rate or end_time determines total accrual.
    // Since rate is 1, at T=2,000 accrued is capped at rate * duration (1,000).
    // If deposit_amount is 2,800, at T=2,000 accrued = min(1,000, 2,800) = 1,000.
    // To reach completion where total withdrawn = total deposit (2,800), let's ensure rate * duration >= 2,800
    // or extend stream end_time!
    // Let's extend stream end_time to T=3,800 so 2,800 tokens can accrue, OR create initial stream with sufficient duration / rate!
    // -----------------------------------------------------------------------

    // Let's extend stream end_time so full deposit of 2,800 tokens accrues by T=3,800
    ctx.stream
        .extend_stream_end_time(&stream_id, &ctx.sender, &3_800u64);

    // Advance timestamp to end_time T=3,800
    ctx.env.ledger().set_timestamp(3_800);
    ctx.env.ledger().with_mut(|l| l.sequence_number += 32);

    // Accrued total at T=3,800 is min(1 * (3,800 - 1,000), 2,800) = min(2,800, 2,800) = 2,800 tokens!
    assert_eq!(ctx.stream.calculate_accrued(&stream_id), 2_800);
    assert_eq!(ctx.stream.get_withdrawable(&stream_id), 2_100); // 2,800 accrued - 700 previously withdrawn

    // Final withdrawal of remaining 2,100 tokens
    let final_withdrawn = ctx.stream.withdraw(&stream_id);
    assert_eq!(final_withdrawn, 2_100);

    // Assert stream is now Completed (total withdrawn = 2,800 == deposit_amount 2,800)
    let state_final = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state_final.withdrawn_amount, 2_800);
    assert_eq!(state_final.deposit_amount, 2_800);
    assert_eq!(state_final.status, StreamStatus::Completed);

    // Total liabilities for this stream dropped to 0
    let liabilities_final = ctx.stream.get_total_liabilities();
    assert_eq!(liabilities_final, 0);

    // Attempting a further top_up_stream call on the Completed stream must be rejected with InvalidState
    let rejected_topup = ctx
        .stream
        .try_top_up_stream(&stream_id, &ctx.sender, &100_i128);
    assert_eq!(rejected_topup, Err(Ok(ContractError::InvalidState)));

    // Confirm state remains untouched after rejected top-up
    let state_post_rejection = ctx.stream.get_stream_state(&stream_id);
    assert_eq!(state_post_rejection.deposit_amount, 2_800);
    assert_eq!(state_post_rejection.status, StreamStatus::Completed);
}
