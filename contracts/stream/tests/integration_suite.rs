    let result = ctx
        .client()
        .try_shorten_stream_end_time(&stream_id, &501u64);
    assert!(result.is_ok(), "new_end_time = now+1 must be accepted");
}

/// top_up_stream role/state matrix: sender/admin/third-party funder across Active/Paused/Cancelled/Completed.
#[test]
fn top_up_stream_role_state_matrix() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_default_stream();

    // --- Active stream: all roles succeed ---
    ctx.env.ledger().set_timestamp(100);
    ctx.client().top_up_stream(&stream_id, &ctx.sender, &200_i128);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_200);
    assert_eq!(state.status, StreamStatus::Active);

    ctx.env.ledger().set_timestamp(200);
    ctx.client().top_up_stream(&stream_id, &ctx.admin, &300_i128);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_500);
    assert_eq!(state.status, StreamStatus::Active);

    let treasury = Address::generate(&ctx.env);
    let sac = StellarAssetClient::new(&ctx.env, &ctx.token_id);
    sac.mint(&treasury, &500_i128);
    ctx.env.ledger().set_timestamp(300);
    ctx.client().top_up_stream(&stream_id, &treasury, &400_i128);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, 1_900);
    assert_eq!(state.status, StreamStatus::Active);

    // --- Paused stream: all roles succeed ---
    ctx.env.ledger().set_timestamp(400);
    ctx.client().pause_stream(&stream_id);
    let state_before_pause = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state_before_pause.status, StreamStatus::Paused);

    ctx.env.ledger().set_timestamp(500);
    ctx.client().top_up_stream(&stream_id, &ctx.sender, &100_i128);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, state_before_pause.deposit_amount + 100);
    assert_eq!(state.status, StreamStatus::Paused);
    assert_eq!(state.start_time, state_before_pause.start_time);
    assert_eq!(state.cliff_time, state_before_pause.cliff_time);
    assert_eq!(state.end_time, state_before_pause.end_time);
    assert_eq!(state.rate_per_second, state_before_pause.rate_per_second);

    ctx.env.ledger().set_timestamp(600);
    ctx.client().top_up_stream(&stream_id, &ctx.admin, &200_i128);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, state_before_pause.deposit_amount + 300);
    assert_eq!(state.status, StreamStatus::Paused);

    ctx.env.ledger().set_timestamp(700);
    sac.mint(&treasury, &300_i128);
    ctx.client().top_up_stream(&stream_id, &treasury, &100_i128);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.deposit_amount, state_before_pause.deposit_amount + 400);
    assert_eq!(state.status, StreamStatus::Paused);

    // --- Cancelled stream: all roles fail with InvalidState ---
    ctx.env.ledger().set_timestamp(800);
    ctx.client().cancel_stream(&stream_id);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Cancelled);

    let sender_before = ctx.token.balance(&ctx.sender);
    let admin_before = ctx.token.balance(&ctx.admin);
    let treasury_before = ctx.token.balance(&treasury);
    let contract_before = ctx.token.balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let result_sender = ctx.client().try_top_up_stream(&stream_id, &ctx.sender, &50_i128);
    let result_admin = ctx.client().try_top_up_stream(&stream_id, &ctx.admin, &50_i128);
    let result_treasury = ctx.client().try_top_up_stream(&stream_id, &treasury, &50_i128);

    assert!(matches!(result_sender, Err(Ok(ContractError::InvalidState))));
    assert!(matches!(result_admin, Err(Ok(ContractError::InvalidState))));
    assert!(matches!(result_treasury, Err(Ok(ContractError::InvalidState))));

    // No state change, no transfer, no new events
    assert_eq!(ctx.token.balance(&ctx.sender), sender_before);
    assert_eq!(ctx.token.balance(&ctx.admin), admin_before);
    assert_eq!(ctx.token.balance(&treasury), treasury_before);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_before);
    assert_eq!(ctx.env.events().all().len(), events_before);

    // --- Completed stream: all roles fail with InvalidState ---
    ctx.env.ledger().set_timestamp(900);
    ctx.client().resume_stream(&stream_id);
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().top_up_stream(&stream_id, &ctx.sender, &100_i128);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Active);
    assert_eq!(state.end_time, 1200);

    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, state.deposit_amount);
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    let sender_before = ctx.token.balance(&ctx.sender);
    let admin_before = ctx.token.balance(&ctx.admin);
    let treasury_before = ctx.token.balance(&treasury);
    let contract_before = ctx.token.balance(&ctx.contract_id);
    let events_before = ctx.env.events().all().len();

    let result_sender = ctx.client().try_top_up_stream(&stream_id, &ctx.sender, &50_i128);
    let result_admin = ctx.client().try_top_up_stream(&stream_id, &ctx.admin, &50_i128);
    let result_treasury = ctx.client().try_top_up_stream(&stream_id, &treasury, &50_i128);

    assert!(matches!(result_sender, Err(Ok(ContractError::InvalidState))));
    assert!(matches!(result_admin, Err(Ok(ContractError::InvalidState))));
    assert!(matches!(result_treasury, Err(Ok(ContractError::InvalidState))));

    // No state change, no transfer, no new events
    assert_eq!(ctx.token.balance(&ctx.sender), sender_before);
    assert_eq!(ctx.token.balance(&ctx.admin), admin_before);
    assert_eq!(ctx.token.balance(&treasury), treasury_before);
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_before);
    assert_eq!(ctx.env.events().all().len(), events_before);
}