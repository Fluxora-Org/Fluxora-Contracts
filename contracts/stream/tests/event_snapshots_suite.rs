/// Event Snapshot Tests
///
/// This module contains comprehensive deterministic snapshot tests that assert exact event
/// topics and payload shapes for all emitted events. Each test captures event data at
/// specific ledger points and verifies the shape against the contract's event schema
/// defined in docs/events.md.
///
/// Tests cover:
/// - StreamCreated: topics ["created", stream_id], payload shape verification
/// - Withdrawal: topics ["withdrew", stream_id], payload shape verification  
/// - WithdrawalTo: topics ["wdraw_to", stream_id], payload shape verification
/// - StreamPaused: topics ["paused", stream_id], payload shape verification (with reason)
/// - StreamResumed: topics ["resumed", stream_id], payload shape verification
/// - StreamCancelled: topics ["cancelled", stream_id], payload shape verification
/// - StreamCompleted: topics ["completed", stream_id], payload shape verification
/// - StreamClosed: topics ["closed", stream_id], payload shape verification
/// - RateUpdated: topics ["rate_upd", stream_id], payload shape verification
/// - StreamEndShortened: topics ["end_shrt", stream_id], payload shape verification
/// - StreamEndExtended: topics ["end_ext", stream_id], payload shape verification
/// - StreamToppedUp: topics ["top_up", stream_id], payload shape verification
/// - RecipientUpdated: topics ["recp_upd", stream_id], payload shape verification
/// - AdminUpdated: topics ["admin", "updated"], payload shape verification (tuple)
/// - ContractPaused: topics ["paused_ctl"], payload shape verification (bool)
///
/// Special scenarios:
/// - No event on revert: Operations that fail emit no events
/// - No withdraw event when amount == 0: Only positive withdrawals emit events
/// - Completed after withdrew: Completion emitted after withdrawal in correct order
extern crate std;

use fluxora_stream::{
    ContractPauseChanged, DataKey, FluxoraStream, FluxoraStreamClient, PauseReason, RateUpdated,
    RecipientUpdated, Stream, StreamCreated, StreamEndExtended, StreamEndShortened,
    StreamHealthChanged, StreamPaused, StreamToppedUp, Withdrawal, WithdrawalTo,
};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Symbol, TryFromVal,
};

struct EventTestContext<'a> {
    env: Env,
    contract_id: Address,
    #[allow(dead_code)]
    token_id: Address,
    admin: Address,
    sender: Address,
    recipient: Address,
    #[allow(dead_code)]
    token: TokenClient<'a>,
}

impl<'a> EventTestContext<'a> {
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

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &100_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        Self {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    /// Extract first event topic as a symbol (normalized)
    fn get_first_topic_symbol(
        &self,
        event: &(
            Address,
            soroban_sdk::Vec<soroban_sdk::Val>,
            soroban_sdk::Val,
        ),
    ) -> Option<String> {
        if event.0 != self.contract_id {
            return None;
        }
        if let Some(topic_val) = event.1.iter().next() {
            if let Ok(sym) = Symbol::try_from_val(&self.env, &topic_val) {
                return Some(sym.to_string());
            }
        }
        None
    }

    /// Extract second event topic as u64 if it exists
    fn get_second_topic_u64(
        &self,
        event: &(
            Address,
            soroban_sdk::Vec<soroban_sdk::Val>,
            soroban_sdk::Val,
        ),
    ) -> Option<u64> {
        if event.0 != self.contract_id {
            return None;
        }
        if let Some(topic_val) = event.1.iter().nth(1) {
            if let Ok(stream_id) = u64::try_from_val(&self.env, &topic_val) {
                return Some(stream_id);
            }
        }
        None
    }

    /// Extract event data payload
    fn get_event_data(
        &self,
        event: &(
            Address,
            soroban_sdk::Vec<soroban_sdk::Val>,
            soroban_sdk::Val,
        ),
    ) -> Option<soroban_sdk::Val> {
        if event.0 != self.contract_id {
            return None;
        }
        // Data is the third element of the tuple
        Some(event.2)
    }
}

// =====================================================================
// Event Snapshot Tests: Stream Creation
// =====================================================================

#[test]
fn event_snapshot_stream_created_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let events_before = ctx.env.events().all().len();

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let events = ctx.env.events().all();
    assert!(
        events.len() > events_before,
        "StreamCreated event must be emitted"
    );

    // Find the StreamCreated event
    let mut found_created = false;
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        // Check first topic is "created"
        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "created" {
                // Verify second topic is the stream_id
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(
                        topic_stream_id, stream_id,
                        "Event topic must contain correct stream_id"
                    );

                    // Verify payload contains expected fields
                    if let Some(data) = ctx.get_event_data(&event) {
                        let stream_created = StreamCreated::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to StreamCreated");

                        assert_eq!(stream_created.stream_id, stream_id);
                        assert_eq!(stream_created.sender, ctx.sender);
                        assert_eq!(stream_created.recipient, ctx.recipient);
                        assert_eq!(stream_created.deposit_amount, 1000);
                        assert_eq!(stream_created.rate_per_second, 1);
                        assert_eq!(stream_created.start_time, 0);
                        assert_eq!(stream_created.cliff_time, 0);
                        assert_eq!(stream_created.end_time, 1000);
                        assert_eq!(stream_created.withdraw_dust_threshold, 0);
                        assert!(stream_created.memo.is_none());

                        found_created = true;
                    }
                }
            }
        }
    }

    assert!(
        found_created,
        "StreamCreated event with correct schema must be found"
    );
}

#[test]
fn event_snapshot_stream_created_with_memo() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let memo = Some(soroban_sdk::Bytes::from_slice(&ctx.env, b"payroll-2024-q1"));
    let events_before = ctx.env.events().all().len();

    let _stream_id = ctx.client().create_stream(&ctx.sender, &ctx.recipient, &5000_i128, &2_i128, &0u64, &100u64, &2500u64, &0, &memo, &fluxora_stream::StreamKind::Linear);


    let events = ctx.env.events().all();
    let mut found = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "created" {
                if let Some(data) = ctx.get_event_data(&event) {
                    let stream_created = StreamCreated::try_from_val(&ctx.env, &data)
                        .expect("Data must deserialize to StreamCreated");

                    assert!(
                        stream_created.memo.is_some(),
                        "Memo must be present in event"
                    );
                    let memo_bytes = stream_created.memo.unwrap();
                    let expected = soroban_sdk::Bytes::from_slice(&ctx.env, b"payroll-2024-q1");
                    assert_eq!(memo_bytes, expected);
                    found = true;
                }
            }
        }
    }

    assert!(found, "StreamCreated event with memo must be found");
}

// =====================================================================
// Event Snapshot Tests: Withdrawal Operations
// =====================================================================

#[test]
fn event_snapshot_withdrawal_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    ctx.env.ledger().set_timestamp(500);
    let events_before = ctx.env.events().all().len();

    let withdrawn_amount = ctx.client().withdraw(&stream_id);

    let events = ctx.env.events().all();
    assert_eq!(withdrawn_amount, 500);

    let mut found_withdrew = false;
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "withdrew" {
                // Verify second topic is the stream_id
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);

                    if let Some(data) = ctx.get_event_data(&event) {
                        let withdrawal = Withdrawal::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to Withdrawal");

                        assert_eq!(withdrawal.stream_id, stream_id);
                        assert_eq!(withdrawal.recipient, ctx.recipient);
                        assert_eq!(withdrawal.amount, 500);

                        found_withdrew = true;
                    }
                }
            }
        }
    }

    assert!(
        found_withdrew,
        "Withdrawal event with correct schema must be found"
    );
}

#[test]
fn event_snapshot_no_withdrawal_event_when_amount_zero() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &500u64, // Cliff at 500
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Try to withdraw before cliff - amount should be 0
    ctx.env.ledger().set_timestamp(100);
    let events_before = ctx.env.events().all().len();

    let withdrawn = ctx.client().withdraw(&stream_id);
    assert_eq!(withdrawn, 0, "Withdrawal before cliff must return 0");

    let events = ctx.env.events().all();
    let mut saw_withdrew_event = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "withdrew" {
                saw_withdrew_event = true;
            }
        }
    }

    assert!(
        !saw_withdrew_event,
        "No withdrawal event should be emitted when amount is 0"
    );
}

#[test]
fn event_snapshot_withdrawal_to_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let destination = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(400);
    let events_before = ctx.env.events().all().len();

    let withdrawn = ctx.client().withdraw_to(&stream_id, &destination);

    let events = ctx.env.events().all();
    assert_eq!(withdrawn, 400);

    let mut found_wdraw_to = false;
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "wdraw_to" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);

                    if let Some(data) = ctx.get_event_data(&event) {
                        let withdrawal_to = WithdrawalTo::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to WithdrawalTo");

                        assert_eq!(withdrawal_to.stream_id, stream_id);
                        assert_eq!(withdrawal_to.recipient, ctx.recipient);
                        assert_eq!(withdrawal_to.destination, destination);
                        assert_eq!(withdrawal_to.amount, 400);

                        found_wdraw_to = true;
                    }
                }
            }
        }
    }

    assert!(
        found_wdraw_to,
        "WithdrawalTo event with correct schema must be found"
    );
}

// =====================================================================
// Event Snapshot Tests: Stream Pause/Resume/Cancel
// =====================================================================

#[test]
fn event_snapshot_stream_paused_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let events_before = ctx.env.events().all().len();
    ctx.client()
        .pause_stream(&stream_id, &PauseReason::Operational);

    let events = ctx.env.events().all();
    let mut found_paused = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "paused" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);

                    if let Some(data) = ctx.get_event_data(&event) {
                        let stream_paused = StreamPaused::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to StreamPaused");

                        assert_eq!(stream_paused.stream_id, stream_id);
                        assert_eq!(stream_paused.reason, PauseReason::Operational);

                        found_paused = true;
                    }
                }
            }
        }
    }

    assert!(
        found_paused,
        "StreamPaused event with correct schema must be found"
    );
}

#[test]
fn event_snapshot_stream_paused_as_admin_has_administrative_reason() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let events_before = ctx.env.events().all().len();
    ctx.client()
        .pause_stream_as_admin(&stream_id, &PauseReason::Administrative);

    let events = ctx.env.events().all();
    let mut found_admin_paused = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "paused" {
                if let Some(data) = ctx.get_event_data(&event) {
                    let stream_paused = StreamPaused::try_from_val(&ctx.env, &data)
                        .expect("Data must deserialize to StreamPaused");

                    assert_eq!(stream_paused.reason, PauseReason::Administrative);
                    found_admin_paused = true;
                }
            }
        }
    }

    assert!(
        found_admin_paused,
        "StreamPaused event with Administrative reason must be found"
    );
}

#[test]
fn event_snapshot_stream_resumed_has_correct_topics() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    ctx.client()
        .pause_stream(&stream_id, &PauseReason::Operational);

    let events_before = ctx.env.events().all().len();
    ctx.client().resume_stream(&stream_id);

    let events = ctx.env.events().all();
    let mut found_resumed = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "resumed" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);
                    found_resumed = true;
                }
            }
        }
    }

    assert!(found_resumed, "StreamResumed event must be found");
}

#[test]
fn event_snapshot_stream_cancelled_has_correct_topics() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    ctx.env.ledger().set_timestamp(500);
    let events_before = ctx.env.events().all().len();
    ctx.client().cancel_stream(&stream_id);

    let events = ctx.env.events().all();
    let mut found_cancelled = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "cancelled" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);
                    found_cancelled = true;
                }
            }
        }
    }

    assert!(found_cancelled, "StreamCancelled event must be found");
}

// =====================================================================
// Event Snapshot Tests: Stream Completion and Closure
// =====================================================================

#[test]
fn event_snapshot_stream_completed_emitted_after_withdrew() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Partial withdrawal first
    ctx.env.ledger().set_timestamp(300);
    ctx.client().withdraw(&stream_id);

    // Final withdrawal that completes the stream
    ctx.env.ledger().set_timestamp(1000);
    let events_before = ctx.env.events().all().len();
    ctx.client().withdraw(&stream_id);

    let events = ctx.env.events().all();
    let mut withdrew_idx: Option<usize> = None;
    let mut completed_idx: Option<usize> = None;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "withdrew" && withdrew_idx.is_none() {
                withdrew_idx = Some(i as usize);
            } else if sym == "completed" && completed_idx.is_none() {
                completed_idx = Some(i as usize);
            }
        }
    }

    assert!(withdrew_idx.is_some(), "withdrew event must be emitted");
    assert!(completed_idx.is_some(), "completed event must be emitted");
    assert!(
        withdrew_idx.unwrap() < completed_idx.unwrap(),
        "withdrew event must come before completed event"
    );
}

#[test]
fn event_snapshot_stream_closed_has_correct_topics() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Complete the stream
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let events_before = ctx.env.events().all().len();
    ctx.client().close_completed_stream(&stream_id);

    let events = ctx.env.events().all();
    let mut found_closed = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "closed" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);
                    found_closed = true;
                }
            }
        }
    }

    assert!(found_closed, "StreamClosed event must be found");
}

// =====================================================================
// Event Snapshot Tests: Rate and Schedule Updates
// =====================================================================

#[test]
fn event_snapshot_rate_updated_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Deposit enough to support rate increase from 1/s to 2/s over 1000s.
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    ctx.env.ledger().set_timestamp(100);
    let events_before = ctx.env.events().all().len();

    ctx.client().update_rate_per_second(&stream_id, &2_i128);

    let events = ctx.env.events().all();
    let mut found_rate_updated = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "rate_upd" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);

                    if let Some(data) = ctx.get_event_data(&event) {
                        let rate_updated = RateUpdated::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to RateUpdated");

                        assert_eq!(rate_updated.stream_id, stream_id);
                        assert_eq!(rate_updated.old_rate_per_second, 1);
                        assert_eq!(rate_updated.new_rate_per_second, 2);
                        assert_eq!(rate_updated.effective_time, 100);

                        found_rate_updated = true;
                    }
                }
            }
        }
    }

    assert!(
        found_rate_updated,
        "RateUpdated event with correct schema must be found"
    );
}

#[test]
fn event_snapshot_stream_end_shortened_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let events_before = ctx.env.events().all().len();
    ctx.client().shorten_stream_end_time(&stream_id, &500u64);

    let events = ctx.env.events().all();
    let mut found_end_shortened = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "end_shrt" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);

                    if let Some(data) = ctx.get_event_data(&event) {
                        let end_shortened = StreamEndShortened::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to StreamEndShortened");

                        assert_eq!(end_shortened.stream_id, stream_id);
                        assert_eq!(end_shortened.old_end_time, 1000);
                        assert_eq!(end_shortened.new_end_time, 500);
                        assert_eq!(end_shortened.refund_amount, 500);

                        found_end_shortened = true;
                    }
                }
            }
        }
    }

    assert!(
        found_end_shortened,
        "StreamEndShortened event with correct schema must be found"
    );
}

#[test]
fn event_snapshot_stream_end_extended_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Deposit enough to support extending end_time from 1000 to 2000 at rate 1/s.
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &2000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let events_before = ctx.env.events().all().len();
    ctx.client().extend_stream_end_time(&stream_id, &2000u64);

    let events = ctx.env.events().all();
    let mut found_end_extended = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "end_ext" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);

                    if let Some(data) = ctx.get_event_data(&event) {
                        let end_extended = StreamEndExtended::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to StreamEndExtended");

                        assert_eq!(end_extended.stream_id, stream_id);
                        assert_eq!(end_extended.old_end_time, 1000);
                        assert_eq!(end_extended.new_end_time, 2000);

                        found_end_extended = true;
                    }
                }
            }
        }
    }

    assert!(
        found_end_extended,
        "StreamEndExtended event with correct schema must be found"
    );
}

// =====================================================================
// Event Snapshot Tests: Stream Funding and Recipient Updates
// =====================================================================

#[test]
fn event_snapshot_stream_topped_up_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let events_before = ctx.env.events().all().len();
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let events = ctx.env.events().all();
    let mut found_topped_up = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "top_up" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);

                    if let Some(data) = ctx.get_event_data(&event) {
                        let topped_up = StreamToppedUp::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to StreamToppedUp");

                        assert_eq!(topped_up.stream_id, stream_id);
                        assert_eq!(topped_up.top_up_amount, 500);
                        assert_eq!(topped_up.new_deposit_amount, 1500);

                        found_topped_up = true;
                    }
                }
            }
        }
    }

    assert!(
        found_topped_up,
        "StreamToppedUp event with correct schema must be found"
    );
}

#[test]
fn event_snapshot_recipient_updated_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    let new_recipient = Address::generate(&ctx.env);
    let events_before = ctx.env.events().all().len();
    ctx.client().update_recipient(&stream_id, &new_recipient);
    ctx.client().accept_recipient_update(&stream_id);

    let events = ctx.env.events().all();
    let mut found_recipient_updated = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "recp_upd" {
                if let Some(topic_stream_id) = ctx.get_second_topic_u64(&event) {
                    assert_eq!(topic_stream_id, stream_id);

                    if let Some(data) = ctx.get_event_data(&event) {
                        let recipient_updated = RecipientUpdated::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to RecipientUpdated");

                        assert_eq!(recipient_updated.stream_id, stream_id);
                        assert_eq!(recipient_updated.old_recipient, ctx.recipient);
                        assert_eq!(recipient_updated.new_recipient, new_recipient);

                        found_recipient_updated = true;
                    }
                }
            }
        }
    }

    assert!(
        found_recipient_updated,
        "RecipientUpdated event with correct schema must be found"
    );
}

// =====================================================================
// Event Snapshot Tests: Admin Operations
// =====================================================================

#[test]
fn event_snapshot_admin_updated_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();

    let new_admin = Address::generate(&ctx.env);
    let events_before = ctx.env.events().all().len();

    ctx.client().set_admin(&new_admin);

    let events = ctx.env.events().all();
    let mut found_admin_updated = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        // AdminUpdated uses single topic ["AdminUpdated"]
        let topics_vec = event.1.clone();
        if let Some(topic_val) = topics_vec.iter().next() {
            if let Ok(first_sym) = Symbol::try_from_val(&ctx.env, &topic_val) {
                if first_sym.to_string() == "AdminUpdated" {
                    if let Some(data) = ctx.get_event_data(&event) {
                        // AdminUpdated payload is a tuple (old_admin, new_admin)
                        if let Ok((old_admin, new_admin_from_event)) =
                            <(Address, Address)>::try_from_val(&ctx.env, &data)
                        {
                            assert_eq!(old_admin, ctx.admin);
                            assert_eq!(new_admin_from_event, new_admin);
                            found_admin_updated = true;
                        }
                    }
                }
            }
        }
    }

    assert!(
        found_admin_updated,
        "AdminUpdated event with correct schema must be found"
    );
}

// =====================================================================
// Event Snapshot Tests: Contract-Level Operations
// =====================================================================

#[test]
fn event_snapshot_contract_paused_has_correct_topics_and_payload() {
    let ctx = EventTestContext::setup();

    let events_before = ctx.env.events().all().len();
    ctx.client().set_contract_paused(&true);

    let events = ctx.env.events().all();
    let mut found_paused_ctl = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "paused_ctl" {
                if let Some(data) = ctx.get_event_data(&event) {
                    if let Ok(payload) = ContractPauseChanged::try_from_val(&ctx.env, &data) {
                        assert!(payload.paused);
                        found_paused_ctl = true;
                    }
                }
            }
        }
    }

    assert!(
        found_paused_ctl,
        "ContractPaused event with correct schema must be found"
    );
}

#[test]
fn event_snapshot_contract_resumed_has_correct_topics() {
    let ctx = EventTestContext::setup();

    ctx.client().set_contract_paused(&true);

    let events_before = ctx.env.events().all().len();
    ctx.client().set_contract_paused(&false);

    let events = ctx.env.events().all();
    let mut found_paused_ctl = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "paused_ctl" {
                if let Some(data) = ctx.get_event_data(&event) {
                    if let Ok(payload) = ContractPauseChanged::try_from_val(&ctx.env, &data) {
                        assert!(!payload.paused);
                        found_paused_ctl = true;
                    }
                }
            }
        }
    }

    assert!(
        found_paused_ctl,
        "ContractPaused event (resumed) with correct schema must be found"
    );
}

// =====================================================================
// Event Snapshot Tests: Special Scenarios
// =====================================================================

#[test]
fn event_snapshot_no_events_on_failed_create_stream() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let events_before = ctx.env.events().all().len();

    // Try to create stream with insufficient deposit (will fail)
    let result = ctx.client().try_create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10_i128, // Too small for 1000 seconds at 1 token/sec
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    assert!(
        result.is_err(),
        "Stream creation should fail with insufficient deposit"
    );

    let events = ctx.env.events().all();
    let mut saw_created = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "created" {
                saw_created = true;
            }
        }
    }

    assert!(
        !saw_created,
        "No StreamCreated event should be emitted on failed creation"
    );
}

#[test]
fn event_snapshot_no_events_on_failed_operations() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Try to pause an already completed stream (should fail)
    ctx.env.ledger().set_timestamp(1000);
    ctx.client().withdraw(&stream_id);

    let events_before = ctx.env.events().all().len();
    let result = ctx
        .client()
        .try_pause_stream(&stream_id, &PauseReason::Operational);

    assert!(result.is_err(), "Pause of completed stream should fail");

    let events = ctx.env.events().all();
    let mut saw_new_paused = false;

    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }

        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "paused" {
                saw_new_paused = true;
            }
        }
    }

    assert!(
        !saw_new_paused,
        "No new pause event should be emitted on failed pause operation"
    );
}

// =====================================================================
// Event Snapshot Tests: StreamHealthChanged
// =====================================================================

/// Helper: find the last StreamHealthChanged event emitted after `events_before`.
fn find_health_changed_event(
    ctx: &EventTestContext,
    events_before: u32,
) -> Option<StreamHealthChanged> {
    let events = ctx.env.events().all();
    let mut result: Option<StreamHealthChanged> = None;
    for i in events_before..events.len() {
        let event = events.get(i).unwrap();
        if event.0 != ctx.contract_id {
            continue;
        }
        if let Some(sym) = ctx.get_first_topic_symbol(&event) {
            if sym == "hlth_chg" {
                if let Some(data) = ctx.get_event_data(&event) {
                    result = Some(
                        StreamHealthChanged::try_from_val(&ctx.env, &data)
                            .expect("Data must deserialize to StreamHealthChanged"),
                    );
                }
            }
        }
    }
    result
}

/// Helper: manually sets the deposit amount of a stream in persistent storage.
fn set_stream_deposit_in_storage(ctx: &EventTestContext, stream_id: u64, amount: i128) {
    ctx.env.as_contract(&ctx.contract_id, || {
        let key = DataKey::Stream(stream_id);
        let mut stream: Stream = ctx.env.storage().persistent().get(&key).unwrap();
        stream.deposit_amount = amount;
        ctx.env.storage().persistent().set(&key, &stream);
    });
}

/// Top-up heals an underfunded stream: health transitions from underfunded → funded.
#[test]
fn event_snapshot_health_changed_top_up_heals_underfunded_stream() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create a stream: deposit=1000, rate=1/s, duration=1000s.
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Advance to t=100.
    ctx.env.ledger().set_timestamp(100);

    // Manually lower deposit to 500 in storage to make it underfunded.
    // remaining_balance = 500. remaining_time = 900. 500 < 900.
    set_stream_deposit_in_storage(&ctx, stream_id, 500);

    // Top up to heal the stream by adding 600 tokens.
    // new deposit = 1100. remaining_balance = 1100. 1100 >= 900. Funded!
    let events_before = ctx.env.events().all().len();
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &600_i128);

    let health = find_health_changed_event(&ctx, events_before)
        .expect("StreamHealthChanged event must be emitted when healing");

    assert_eq!(health.stream_id, stream_id);
    assert_eq!(health.is_underfunded, false, "Stream should now be funded");
    assert_eq!(health.remaining_balance, 1100);
    assert_eq!(health.seconds_remaining, 900);
}

/// Shorten heals an underfunded stream: health transitions from underfunded → funded.
#[test]
fn event_snapshot_health_changed_shorten_heals_underfunded_stream() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream: deposit=1000, rate=1/s, duration=1000s. Funded.
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Advance to t=100.
    ctx.env.ledger().set_timestamp(100);

    // Set deposit to 500.
    // remaining_balance = 500. remaining_time = 900. 500 < 900 (underfunded).
    set_stream_deposit_in_storage(&ctx, stream_id, 500);

    // Shorten end_time to 500.
    // new_max_streamable = 1 * 500 = 500.
    // deposit becomes 500. remaining_balance = 500. remaining_time = 400. 500 >= 400 (funded).
    let events_before = ctx.env.events().all().len();
    ctx.client().shorten_stream_end_time(&stream_id, &500u64);

    let health = find_health_changed_event(&ctx, events_before)
        .expect("StreamHealthChanged event must be emitted when shorten heals");

    assert_eq!(health.stream_id, stream_id);
    assert_eq!(health.is_underfunded, false, "Stream should now be funded");
    assert_eq!(health.seconds_remaining, 400);
    assert_eq!(health.remaining_balance, 500);
}

/// Decrease rate heals an underfunded stream: health transitions from underfunded → funded.
#[test]
fn event_snapshot_health_changed_decrease_rate_heals_underfunded_stream() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream: deposit=10000, rate=10/s, duration=1000s. Funded.
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &10000_i128,
        &10_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Advance to t=100.
    ctx.env.ledger().set_timestamp(100);

    // Set deposit to 2000 in storage.
    // remaining_balance = 2000. remaining_time = 900. required = 10 * 900 = 9000.
    // 2000 < 9000 (underfunded).
    set_stream_deposit_in_storage(&ctx, stream_id, 2000);

    // Decrease rate from 10 to 1.
    // accrued_now = 1000. remaining_seconds = 900. future_accrual = 1 * 900 = 900.
    // new deposit = 1900. remaining_balance = 1900. remaining_time = 900. required = 900.
    // 1900 >= 900 (funded).
    let events_before = ctx.env.events().all().len();
    ctx.client().decrease_rate_per_second(&stream_id, &1_i128);

    let health = find_health_changed_event(&ctx, events_before)
        .expect("StreamHealthChanged event must be emitted when decreasing rate heals");

    assert_eq!(health.stream_id, stream_id);
    assert_eq!(health.is_underfunded, false, "Stream should now be funded");
    assert_eq!(health.remaining_balance, 1900);
    assert_eq!(health.seconds_remaining, 900);
}

/// Cancel heals an underfunded stream: terminal state → not underfunded.
#[test]
fn event_snapshot_health_changed_cancel_heals_underfunded_stream() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream: deposit=1000, rate=1/s, duration=1000s.
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Advance to t=100.
    ctx.env.ledger().set_timestamp(100);

    // Set deposit to 500.
    // remaining_balance = 500. remaining_time = 900. 500 < 900 (underfunded).
    set_stream_deposit_in_storage(&ctx, stream_id, 500);

    // Cancel: terminal → seconds_remaining=0, required=0, never underfunded.
    let events_before = ctx.env.events().all().len();
    ctx.client().cancel_stream(&stream_id);

    let health = find_health_changed_event(&ctx, events_before)
        .expect("StreamHealthChanged event must be emitted when cancel heals");

    assert_eq!(health.stream_id, stream_id);
    assert_eq!(
        health.is_underfunded, false,
        "Terminal stream is never underfunded"
    );
    assert_eq!(health.seconds_remaining, 0);
}

/// No health event emitted when health status does not change (stays funded).
#[test]
fn event_snapshot_health_changed_not_emitted_when_no_transition() {
    let ctx = EventTestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    // Create stream: deposit=1000, rate=1/s, duration=1000s. Funded.
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &fluxora_stream::StreamKind::Linear,
        );

    // Top up an already-funded stream. Health stays funded → no event.
    let events_before = ctx.env.events().all().len();
    ctx.client()
        .top_up_stream(&stream_id, &ctx.sender, &500_i128);

    let health = find_health_changed_event(&ctx, events_before);
    assert!(
        health.is_none(),
        "StreamHealthChanged must NOT be emitted when health doesn't change"
    );
}
