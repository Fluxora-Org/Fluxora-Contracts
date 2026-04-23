#[cfg(test)]
use crate::test::TestContext;
use crate::StreamStatus;
use soroban_sdk::{testutils::Address as _, testutils::Ledger, Address};

#[test]
fn test_withdraw_multiple_partial_withdrawals() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // 1000 total, 1/s

    // t=200: withdraw 200
    ctx.env.ledger().set_timestamp(200);
    let amt1 = ctx.client().withdraw(&stream_id);
    assert_eq!(amt1, 200);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        200
    );

    // t=500: withdraw 300 (500 - 200)
    ctx.env.ledger().set_timestamp(500);
    let amt2 = ctx.client().withdraw(&stream_id);
    assert_eq!(amt2, 300);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        500
    );

    // t=900: withdraw 400 (900 - 500)
    ctx.env.ledger().set_timestamp(900);
    let amt3 = ctx.client().withdraw(&stream_id);
    assert_eq!(amt3, 400);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).withdrawn_amount,
        900
    );

    // t=1000: final withdraw 100
    ctx.env.ledger().set_timestamp(1000);
    let amt4 = ctx.client().withdraw(&stream_id);
    assert_eq!(amt4, 100);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 1000);
    assert_eq!(state.status, StreamStatus::Completed);
}

#[test]
fn test_withdraw_exact_remaining_accrued() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // t=750: withdraw 750
    ctx.env.ledger().set_timestamp(750);
    ctx.client().withdraw(&stream_id);

    // t=1000: remaining is 250
    ctx.env.ledger().set_timestamp(1000);
    let withdrawable = ctx.client().get_withdrawable(&stream_id);
    assert_eq!(withdrawable, 250);

    let amt = ctx.client().withdraw(&stream_id);
    assert_eq!(amt, 250);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );
}

#[test]
fn test_withdraw_cap_contract_balance_safety() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream(); // deposit 1000

    // Advance to t=1000: full accrued (1000)
    ctx.env.ledger().set_timestamp(1000);

    // Artificially drain the contract's balance by 400.
    // Since mock_all_auths is on, we can transfer from the contract.
    let somebody = Address::generate(&ctx.env);
    ctx.token().transfer(&ctx.contract_id, &somebody, &400);

    // Now contract balance is 600, but accrued - withdrawn is 1000.
    // Withdrawal should be capped at 600.
    let _withdrawable = ctx.client().get_withdrawable(&stream_id); // This usually follows the same logic
                                                                   // Wait, get_withdrawable also needs to be updated or it will show 1000.
                                                                   // The requirement said "In withdraw, compute withdrawable as min(...)".
                                                                   // I should also update get_withdrawable for consistency if possible.

    let amt = ctx.client().withdraw(&stream_id);
    assert_eq!(
        amt, 600,
        "Withdrawn amount must be capped by contract balance"
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.withdrawn_amount, 600);
    assert_eq!(
        state.status,
        StreamStatus::Active,
        "Must not be completed if capped by balance"
    );

    // Replenish balance and withdraw remaining
    ctx.sac.mint(&ctx.contract_id, &400);
    let amt2 = ctx.client().withdraw(&stream_id);
    assert_eq!(amt2, 400);
    assert_eq!(
        ctx.client().get_stream_state(&stream_id).status,
        StreamStatus::Completed
    );
}

#[test]
fn test_batch_withdraw_running_balance_cap() {
    let ctx = TestContext::setup();

    // Create two streams with 500 each
    let id1 =
        ctx.client()
            .create_stream(&ctx.sender, &ctx.recipient, &500, &1, &0, &0, &500, &None);
    let id2 =
        ctx.client()
            .create_stream(&ctx.sender, &ctx.recipient, &500, &1, &0, &0, &500, &None);

    ctx.env.ledger().set_timestamp(500); // both fully accrued

    // Contract has 1000. Drain 300. Remaining: 700.
    let somebody = Address::generate(&ctx.env);
    ctx.token().transfer(&ctx.contract_id, &somebody, &300);

    // Batch withdraw from both.
    // Stream 1 should get 500.
    // Stream 2 should get remaining 200 (700 - 500).
    let ids = soroban_sdk::vec![&ctx.env, id1, id2];
    let results = ctx.client().batch_withdraw(&ctx.recipient, &ids);

    assert_eq!(results.get(0).unwrap().amount, 500);
    assert_eq!(results.get(1).unwrap().amount, 200);

    assert_eq!(
        ctx.client().get_stream_state(&id1).status,
        StreamStatus::Completed
    );
    assert_eq!(
        ctx.client().get_stream_state(&id2).status,
        StreamStatus::Active
    );
}
