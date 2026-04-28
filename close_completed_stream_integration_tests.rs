// Integration tests for close_completed_stream function
// These tests verify the protocol semantics for removing a stream from the recipient index
// when a stream is closed after reaching Completed status.

// NOTE: These tests should be appended to contracts/stream/tests/integration_suite.rs
// after the final existing test.

// =========================================================================
// Integration tests — close_completed_stream: recipient index removal
// =========================================================================

/// Integration test: create stream → withdraw fully → close → verify removal from index.
///
/// This end-to-end test covers:
/// - Stream creation and recipient index inclusion
/// - Full withdrawal to mark stream as Completed
/// - Closing the completed stream
/// - Verification that stream is removed from recipient index
/// - Verification that stream can no longer be queried
/// - Verification that recipient's stream count decreases
#[test]
fn integration_close_completed_stream_full_workflow() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create a stream for the recipient
    let stream_id = ctx.create_default_stream();

    // Verify stream is initially in recipient's index
    let streams_before = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams_before.len(), 1);
    assert_eq!(streams_before.get(0).unwrap(), stream_id);

    let count_before = ctx.client().get_recipient_stream_count(&ctx.recipient);
    assert_eq!(count_before, 1);

    // Withdraw fully to mark stream as Completed
    ctx.env.ledger().set_timestamp(1000);
    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 1000);

    // Verify stream status is Completed
    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(state.status, StreamStatus::Completed);

    // Verify stream is still in recipient's index (not removed until close)
    let streams_still_there = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams_still_there.len(), 1);

    // Close the completed stream
    ctx.client().close_completed_stream(&stream_id);

    // Verify stream is now removed from recipient's index
    let streams_after = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams_after.len(), 0);

    let count_after = ctx.client().get_recipient_stream_count(&ctx.recipient);
    assert_eq!(count_after, 0);

    // Verify stream can no longer be queried
    let result = ctx.client().try_get_stream_state(&stream_id);
    assert!(result.is_err(), "closed stream must not be queryable");
}

/// Integration test: multiple completed streams → close some → index updates correctly.
///
/// This test verifies:
/// - Multiple streams created for same recipient
/// - Selective closing of completed streams
/// - Index maintains sorted order after removal
/// - Correct streams remain accessible via index
#[test]
fn integration_close_multiple_completed_streams_selective() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create 4 streams
    let ids = vec![
        ctx.client().create_stream(&ctx.sender, &ctx.recipient, &100_i128, &1_i128, &0u64, &0u64, &100u64,
            &None
        ),
        ctx.client().create_stream(&ctx.sender, &ctx.recipient, &200_i128, &1_i128, &100u64, &100u64, &300u64,
            &None
        ),
        ctx.client().create_stream(&ctx.sender, &ctx.recipient, &150_i128, &1_i128, &0u64, &0u64, &150u64,
            &None
        ),
        ctx.client().create_stream(&ctx.sender, &ctx.recipient, &250_i128, &1_i128, &0u64, &0u64, &250u64,
            &None
        ),
    ];

    // Verify all 4 are in the index
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 4);
    assert_eq!(streams.get(0).unwrap(), ids[0]);
    assert_eq!(streams.get(1).unwrap(), ids[1]);
    assert_eq!(streams.get(2).unwrap(), ids[2]);
    assert_eq!(streams.get(3).unwrap(), ids[3]);

    // Complete streams 0, 1, 3 (leave 2)
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&ids[0]); // stream 0: 100 tokens, fully withdrawn
    ctx.client().withdraw(&ids[1]); // stream 1: 200 tokens, fully withdrawn
    ctx.client().withdraw(&ids[3]); // stream 3: 250 tokens, fully withdrawn
    // stream 2: 150 tokens, partially withdrawn (50 accrued at t=300)

    // Close only streams 0 and 1 (leave 2 and 3 open for now)
    ctx.client().close_completed_stream(&ids[0]);
    ctx.client().close_completed_stream(&ids[1]);

    // Verify only 0 and 1 are removed
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 2);
    assert_eq!(streams.get(0).unwrap(), ids[2]); // still there, still queryable
    assert_eq!(streams.get(1).unwrap(), ids[3]); // still there but not closed yet

    // Verify 0 and 1 are no longer queryable
    assert!(ctx.client().try_get_stream_state(&ids[0]).is_err());
    assert!(ctx.client().try_get_stream_state(&ids[1]).is_err());

    // Verify 2 and 3 are still queryable
    let state2 = ctx.client().get_stream_state(&ids[2]);
    assert_eq!(state2.status, StreamStatus::Active);

    let state3 = ctx.client().get_stream_state(&ids[3]);
    assert_eq!(state3.status, StreamStatus::Completed);

    // Now close stream 3
    ctx.client().close_completed_stream(&ids[3]);

    // Verify only stream 2 remains
    let streams = ctx.client().get_recipient_streams(&ctx.recipient);
    assert_eq!(streams.len(), 1);
    assert_eq!(streams.get(0).unwrap(), ids[2]);
}

/// Integration test: close_completed_stream does not affect other recipients.
///
/// This test verifies:
/// - Multiple recipients each have streams
/// - Closing stream for one recipient doesn't affect others
/// - Index isolation between recipients
#[test]
fn integration_close_completed_stream_per_recipient_isolation() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let recipient2 = Address::generate(&ctx.env);
    let recipient3 = Address::generate(&ctx.env);

    // Create streams for different recipients
    let id_r1 = ctx.client().create_stream(&ctx.sender, &ctx.recipient, &500_i128, &1_i128, &0u64, &0u64, &500u64,
        &None
    );
    let id_r2 = ctx.client().create_stream(&ctx.sender, &recipient2, &600_i128, &1_i128, &0u64, &0u64, &600u64,
        &None
    );
    let id_r3 = ctx.client().create_stream(&ctx.sender, &recipient3, &400_i128, &1_i128, &0u64, &0u64, &400u64,
        &None
    );

    // Verify each recipient has 1 stream
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 1);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient2), 1);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient3), 1);

    // Complete and close only r1's stream
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&id_r1);
    ctx.client().close_completed_stream(&id_r1);

    // Verify only r1's count decreased
    assert_eq!(ctx.client().get_recipient_stream_count(&ctx.recipient), 0);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient2), 1);
    assert_eq!(ctx.client().get_recipient_stream_count(&recipient3), 1);

    // Verify r1's stream is gone but r2 and r3's streams are still there
    assert!(ctx.client().try_get_stream_state(&id_r1).is_err());
    assert_eq!(ctx.client().get_stream_state(&id_r2).stream_id, id_r2);
    assert_eq!(ctx.client().get_stream_state(&id_r3).stream_id, id_r3);
}

/// Integration test: attempt to close completed stream in various recipient scenarios.
///
/// Verifies:
/// - Only the Completed status allows closing
/// - Active, Paused, Cancelled statuses are rejected
/// - Non-existent stream ID is rejected
#[test]
fn integration_close_completed_stream_rejects_invalid_states() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Test 1: Active stream cannot be closed
    let id_active = ctx.create_default_stream();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().close_completed_stream(&id_active);
    }));
    assert!(result.is_err(), "cannot close Active stream");

    // Test 2: Paused stream cannot be closed
    let id_paused = ctx.client().create_stream(&ctx.sender, &ctx.recipient, &500_i128, &1_i128, &0u64, &0u64, &500u64,
        &None
    );
    ctx.env.ledger().set_timestamp(250);
    ctx.client().pause_stream(&id_paused);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().close_completed_stream(&id_paused);
    }));
    assert!(result.is_err(), "cannot close Paused stream");

    // Test 3: Cancelled stream cannot be closed
    let id_cancelled = ctx.client().create_stream(&ctx.sender, &ctx.recipient, &500_i128, &1_i128, &0u64, &0u64, &500u64,
        &None
    );
    ctx.env.ledger().set_timestamp(250);
    ctx.client().cancel_stream(&id_cancelled);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().close_completed_stream(&id_cancelled);
    }));
    assert!(result.is_err(), "cannot close Cancelled stream");

    // Test 4: Non-existent stream ID
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.client().close_completed_stream(&999u64);
    }));
    assert!(result.is_err(), "cannot close non-existent stream");

    // Test 5: Only Completed streams can be closed
    let id_completable = ctx.client().create_stream(&ctx.sender, &ctx.recipient, &500_i128, &1_i128, &0u64, &0u64, &500u64,
        &None
    );
    ctx.env.ledger().set_timestamp(500);
    ctx.client().withdraw(&id_completable);

    let state = ctx.client().get_stream_state(&id_completable);
    assert_eq!(state.status, StreamStatus::Completed);

    // This should succeed
    ctx.client().close_completed_stream(&id_completable);

    // Verify it's removed
    assert!(ctx.client().try_get_stream_state(&id_completable).is_err());
}
